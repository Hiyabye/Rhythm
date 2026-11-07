use super::*;

pub(super) async fn install(
    store: &Arc<Mutex<Store>>,
    root: &Path,
    entry: &CatalogEntry,
    model: &mut AiModel,
    cancel: &Arc<AtomicBool>,
    transport: &impl Transport,
) -> Result<(), String> {
    check_cancel(cancel)?;
    let full = super::super::model_storage::managed_path(
        root,
        &model.path,
        model.storage_directory.as_deref(),
        true,
    )?;
    if full.exists() {
        // A chosen folder may be shared with another profile or hold a file the
        // user registered as external. Adopting it as managed would let model
        // removal delete bytes Rhythm never wrote, so only adopt there when this
        // inventory already received every byte (a crash after the final rename).
        if model.storage_directory.is_some() && model.downloaded_bytes != entry.bytes {
            return Err("선택한 폴더에 같은 모델 파일이 이미 있습니다. 삭제 권한을 넘겨받지 않도록 설치하지 않았습니다. 기존 파일은 '모델 파일 등록'으로 추가해 주세요.".into());
        }
        let file = full.clone();
        let expected = entry.clone();
        let cancelled = cancel.clone();
        tokio::task::spawn_blocking(move || {
            verify_file(&file, Some((&expected.sha256, expected.bytes)), &cancelled)
        })
        .await
        .map_err(|_| "모델 검증 작업이 중단되었습니다.")??;
        check_cancel(cancel)?;
        model.status = "installed".into();
        model.sha256 = Some(entry.sha256.clone());
        model.bytes = entry.bytes;
        model.downloaded_bytes = entry.bytes;
        return with_store(store, |s| s.ai_upsert_model(model));
    }
    let part = full.with_extension("part");
    let actual = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if actual == entry.bytes {
        return finalize(store, entry, model, cancel, &part, &full).await;
    }
    let offset = if actual < entry.bytes && model.resume_etag.as_deref().is_some_and(strong_etag) {
        actual
    } else {
        0
    };
    require_space(
        available_space(full.parent().ok_or("모델 저장 폴더가 없습니다.")?)?,
        entry.bytes - offset,
    )?;
    model.status = "downloading".into();
    model.downloaded_bytes = offset;
    model.error = None;
    with_store(store, |s| s.ai_upsert_model(model))?;
    let url = download_url(entry)?;
    let mut response = tokio::select! {r=transport.response(&url,entry.bytes,offset,model.resume_etag.as_deref(),cancel)=>r?,_=cancelled(cancel)=>return check_cancel(cancel)};
    let append =
        match validate_response(&response, offset, entry.bytes, model.resume_etag.as_deref()) {
            Ok(append) => append,
            Err(error) => {
                if response.status == 206 || response.status == 416 {
                    // A changed validator/range cannot be retried as the same prefix.
                    tokio::fs::remove_file(&part)
                        .await
                        .map_err(|_| "잘못된 이어받기 파일을 정리하지 못했습니다.")?;
                    model.downloaded_bytes = 0;
                    model.resume_etag = None;
                }
                return Err(error);
            }
        };
    let mut received = if append { offset } else { 0 };
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!append)
        .append(append)
        .open(&part)
        .await
        .map_err(|_| "임시 모델 파일을 열지 못했습니다.")?;
    model.resume_etag = response.etag.take();
    model.downloaded_bytes = received;
    with_store(store, |s| s.ai_upsert_model(model))?;
    let start = Instant::now();
    let mut progress = Instant::now();
    loop {
        let next = tokio::select! {
            r = tokio::time::timeout(Duration::from_secs(45), response.body.chunk()) => r.unwrap_or_else(|_| Err("모델 다운로드 응답 시간이 초과되었습니다.".into())),
            _ = cancelled(cancel) => Err("모델 설치를 취소했습니다. 다시 선택하면 이어받습니다.".into()),
        };
        let chunk = match next {
            Ok(chunk) if !cancel.load(Ordering::Acquire) => chunk,
            outcome => {
                // Tokio writes may still be buffered when a network error arrives.
                file.flush()
                    .await
                    .map_err(|_| "임시 모델 저장에 실패했습니다.")?;
                return Err(outcome
                    .err()
                    .unwrap_or_else(|| "모델 설치를 취소했습니다.".into()));
            }
        };
        let Some(chunk) = chunk else { break };
        if start.elapsed() > Duration::from_secs(2 * 60 * 60)
            || chunk.len() as u64 > entry.bytes.saturating_sub(received)
        {
            return Err("모델 다운로드 크기 또는 시간 제한을 초과했습니다.".into());
        }
        file.write_all(&chunk)
            .await
            .map_err(|_| "모델 파일을 저장하지 못했습니다. 저장 공간과 권한을 확인해 주세요.")?;
        received += chunk.len() as u64;
        model.downloaded_bytes = received;
        if progress.elapsed() >= Duration::from_millis(350) {
            with_store(store, |s| s.ai_upsert_model(model))?;
            progress = Instant::now();
        }
    }
    file.flush()
        .await
        .map_err(|_| "모델 파일 저장을 마무리하지 못했습니다.")?;
    file.sync_all()
        .await
        .map_err(|_| "모델 파일을 디스크에 저장하지 못했습니다.")?;
    drop(file);
    if received != entry.bytes {
        return Err("모델 파일을 끝까지 받지 못했습니다. 다시 선택하면 이어받습니다.".into());
    }
    finalize(store, entry, model, cancel, &part, &full).await
}
pub(super) async fn finalize(
    store: &Arc<Mutex<Store>>,
    entry: &CatalogEntry,
    model: &mut AiModel,
    cancel: &Arc<AtomicBool>,
    part: &Path,
    full: &Path,
) -> Result<(), String> {
    model.status = "verifying".into();
    with_store(store, |s| s.ai_upsert_model(model))?;
    let file = part.to_path_buf();
    let expected = entry.clone();
    let cancelled = cancel.clone();
    let checked = tokio::task::spawn_blocking(move || {
        verify_file(&file, Some((&expected.sha256, expected.bytes)), &cancelled)
    })
    .await
    .map_err(|_| "모델 무결성 검증이 중단되었습니다.")?;
    if let Err(error) = checked {
        if !cancel.load(Ordering::Acquire) {
            // A failed full-file digest must not be reused as a supposedly safe prefix.
            tokio::fs::remove_file(part)
                .await
                .map_err(|_| "손상된 임시 모델을 정리하지 못했습니다.")?;
            model.downloaded_bytes = 0;
            model.resume_etag = None;
        }
        return Err(error);
    }
    check_cancel(cancel)?;
    tokio::fs::rename(part, full)
        .await
        .map_err(|_| "검증한 모델 설치를 마무리하지 못했습니다.")?;
    model.status = "installed".into();
    model.sha256 = Some(entry.sha256.clone());
    model.bytes = entry.bytes;
    model.downloaded_bytes = entry.bytes;
    model.error = None;
    with_store(store, |s| s.ai_upsert_model(model))?;
    if entry.kind == "embedding" {
        // An explicit embedding install is the opt-in: select it for
        // classification, but never touch the generation model.
        if let Err(error) = with_store(store, |s| {
            let mut preferences = s.ai_preferences()?;
            preferences.embedding_model_id = Some(model.id.clone());
            s.ai_save_preferences(&preferences)
        }) {
            eprintln!("Embedding model selection: {error}");
        }
    }
    Ok(())
}
/// Installs (or resumes) the vision projector of an installed model. It is a
/// separate artifact: failure never changes the model weights or selection.
pub(super) async fn install_projector(
    store: &Arc<Mutex<Store>>,
    root: &Path,
    model: &AiModel,
    entry: &CatalogEntry,
    cancel: &Arc<AtomicBool>,
    transport: &impl Transport,
) -> Result<(), String> {
    check_cancel(cancel)?;
    let projector = entry
        .projector
        .clone()
        .ok_or("이 모델은 이미지 인식을 지원하는 배포 항목이 없습니다.")?;
    let full = super::super::model_storage::managed_path(
        root,
        &format!("models/{}.gguf", projector.sha256),
        model.storage_directory.as_deref(),
        true,
    )?;
    let mut record = projector_record(model, entry)?;
    if let Some(old) = with_store(store, |s| s.ai_projector(&model.id))? {
        if old.status == "installed" {
            return Err("이미 이미지 인식 모듈이 설치되어 있습니다.".into());
        }
        if old.path == record.path && old.expected_sha256 == record.expected_sha256 {
            record.resume_etag = old.resume_etag;
            record.downloaded_bytes = old.downloaded_bytes;
        }
    }
    with_store(store, |s| s.ai_upsert_projector(&record))?;
    let outcome = install_projector_bytes(
        store,
        entry,
        &projector,
        &mut record,
        cancel,
        transport,
        &full,
    )
    .await;
    if let Err(error) = outcome {
        record.status = if cancel.load(Ordering::Acquire) {
            "cancelled"
        } else {
            "failed"
        }
        .into();
        record.error = Some(error.clone());
        if let Err(error) = with_store(store, |s| s.ai_upsert_projector(&record)) {
            eprintln!("AI projector status: {error}");
        }
        return Err(error);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn install_projector_bytes(
    store: &Arc<Mutex<Store>>,
    entry: &CatalogEntry,
    projector: &ProjectorEntry,
    record: &mut AiProjector,
    cancel: &Arc<AtomicBool>,
    transport: &impl Transport,
    full: &Path,
) -> Result<(), String> {
    if full.exists() {
        // A crash after the final rename: adopt only bytes this inventory wrote.
        // A chosen shared folder could already hold a same-named file, so it is
        // adopted only when the recorded download had received every byte.
        if record.downloaded_bytes != projector.bytes {
            return Err("선택한 폴더에 같은 이미지 인식 모듈 파일이 이미 있습니다. 삭제 권한을 넘겨받지 않도록 설치하지 않았습니다.".into());
        }
        let file = full.to_path_buf();
        let expected = projector.clone();
        let cancelled = cancel.clone();
        tokio::task::spawn_blocking(move || {
            verify_projector_file(&file, Some((&expected.sha256, expected.bytes)), &cancelled)
        })
        .await
        .map_err(|_| "이미지 인식 모듈 검증 작업이 중단되었습니다.")??;
        check_cancel(cancel)?;
        record.status = "installed".into();
        record.sha256 = Some(projector.sha256.clone());
        record.bytes = projector.bytes;
        record.downloaded_bytes = projector.bytes;
        record.error = None;
        return with_store(store, |s| s.ai_upsert_projector(record));
    }
    let part = full.with_extension("part");
    let actual = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if actual == projector.bytes {
        return finalize_projector(store, projector, record, cancel, &part, full).await;
    }
    let offset =
        if actual < projector.bytes && record.resume_etag.as_deref().is_some_and(strong_etag) {
            actual
        } else {
            0
        };
    require_space(
        available_space(full.parent().ok_or("모델 저장 폴더가 없습니다.")?)?,
        projector.bytes - offset,
    )?;
    record.status = "downloading".into();
    record.downloaded_bytes = offset;
    record.error = None;
    with_store(store, |s| s.ai_upsert_projector(record))?;
    let (url, _) = projector_url(entry)?;
    let mut response = tokio::select! {r=transport.response(&url,projector.bytes,offset,record.resume_etag.as_deref(),cancel)=>r?,_=cancelled(cancel)=>return check_cancel(cancel)};
    let append = match validate_response(
        &response,
        offset,
        projector.bytes,
        record.resume_etag.as_deref(),
    ) {
        Ok(append) => append,
        Err(error) => {
            if response.status == 206 || response.status == 416 {
                tokio::fs::remove_file(&part)
                    .await
                    .map_err(|_| "잘못된 이어받기 파일을 정리하지 못했습니다.")?;
                record.downloaded_bytes = 0;
                record.resume_etag = None;
            }
            return Err(error);
        }
    };
    let mut received = if append { offset } else { 0 };
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!append)
        .append(append)
        .open(&part)
        .await
        .map_err(|_| "임시 이미지 인식 모듈 파일을 열지 못했습니다.")?;
    record.resume_etag = response.etag.take();
    record.downloaded_bytes = received;
    with_store(store, |s| s.ai_upsert_projector(record))?;
    let start = Instant::now();
    let mut progress = Instant::now();
    loop {
        let next = tokio::select! {
            r = tokio::time::timeout(Duration::from_secs(45), response.body.chunk()) => r.unwrap_or_else(|_| Err("이미지 인식 모듈 다운로드 응답 시간이 초과되었습니다.".into())),
            _ = cancelled(cancel) => Err("이미지 인식 모듈 설치를 취소했습니다. 다시 선택하면 이어받습니다.".into()),
        };
        let chunk = match next {
            Ok(chunk) if !cancel.load(Ordering::Acquire) => chunk,
            outcome => {
                file.flush()
                    .await
                    .map_err(|_| "임시 이미지 인식 모듈 저장에 실패했습니다.")?;
                return Err(outcome
                    .err()
                    .unwrap_or_else(|| "이미지 인식 모듈 설치를 취소했습니다.".into()));
            }
        };
        let Some(chunk) = chunk else { break };
        if start.elapsed() > Duration::from_secs(2 * 60 * 60)
            || chunk.len() as u64 > projector.bytes.saturating_sub(received)
        {
            return Err("이미지 인식 모듈 다운로드 크기 또는 시간 제한을 초과했습니다.".into());
        }
        file.write_all(&chunk).await.map_err(|_| {
            "이미지 인식 모듈 파일을 저장하지 못했습니다. 저장 공간과 권한을 확인해 주세요."
        })?;
        received += chunk.len() as u64;
        record.downloaded_bytes = received;
        if progress.elapsed() >= Duration::from_millis(350) {
            with_store(store, |s| s.ai_upsert_projector(record))?;
            progress = Instant::now();
        }
    }
    file.flush()
        .await
        .map_err(|_| "이미지 인식 모듈 파일 저장을 마무리하지 못했습니다.")?;
    file.sync_all()
        .await
        .map_err(|_| "이미지 인식 모듈 파일을 디스크에 저장하지 못했습니다.")?;
    drop(file);
    if received != projector.bytes {
        return Err(
            "이미지 인식 모듈 파일을 끝까지 받지 못했습니다. 다시 선택하면 이어받습니다.".into(),
        );
    }
    finalize_projector(store, projector, record, cancel, &part, full).await
}

pub(super) async fn finalize_projector(
    store: &Arc<Mutex<Store>>,
    projector: &ProjectorEntry,
    record: &mut AiProjector,
    cancel: &Arc<AtomicBool>,
    part: &Path,
    full: &Path,
) -> Result<(), String> {
    record.status = "verifying".into();
    with_store(store, |s| s.ai_upsert_projector(record))?;
    let file = part.to_path_buf();
    let expected = projector.clone();
    let cancelled = cancel.clone();
    let checked = tokio::task::spawn_blocking(move || {
        verify_projector_file(&file, Some((&expected.sha256, expected.bytes)), &cancelled)
    })
    .await
    .map_err(|_| "이미지 인식 모듈 무결성 검증이 중단되었습니다.")?;
    if let Err(error) = checked {
        if !cancel.load(Ordering::Acquire) {
            tokio::fs::remove_file(part)
                .await
                .map_err(|_| "손상된 이미지 인식 모듈 파일을 정리하지 못했습니다.")?;
            record.downloaded_bytes = 0;
            record.resume_etag = None;
        }
        return Err(error);
    }
    check_cancel(cancel)?;
    tokio::fs::rename(part, full)
        .await
        .map_err(|_| "검증한 이미지 인식 모듈 설치를 마무리하지 못했습니다.")?;
    record.status = "installed".into();
    record.sha256 = Some(projector.sha256.clone());
    record.bytes = projector.bytes;
    record.downloaded_bytes = projector.bytes;
    record.error = None;
    with_store(store, |s| s.ai_upsert_projector(record))
}
