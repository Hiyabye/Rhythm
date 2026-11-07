use super::Cancellation;
use crate::storage::intelligence::AiModel;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

/// User-facing messages for the artifact being hashed.
pub(super) struct HashErrors {
    read: &'static str,
    grew: &'static str,
    format: &'static str,
}
/// Streams the complete file once, checking the GGUF magic/version of the first
/// block, and returns the lowercase SHA-256 hex digest. `expected_bytes` bounds
/// growth while reading.
pub(super) fn hash_gguf(
    file: &mut std::fs::File,
    expected_bytes: u64,
    cancellation: &Cancellation,
    errors: &HashErrors,
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut first = true;
    let mut received = 0u64;
    loop {
        cancellation.check()?;
        let count = file.read(&mut buffer).map_err(|_| errors.read)?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > expected_bytes {
            return Err(errors.grew.into());
        }
        if first {
            if count < 8
                || &buffer[..4] != b"GGUF"
                || ![2u32, 3].contains(&u32::from_le_bytes(buffer[4..8].try_into().unwrap()))
            {
                return Err(errors.format.into());
            }
            first = false;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(super) fn verified_path(
    root: &Path,
    model: &AiModel,
    cancellation: &Cancellation,
) -> Result<PathBuf, String> {
    let path = super::super::model_storage::resolve_model_path(root, model)?
        .canonicalize()
        .map_err(|_| "모델 파일을 찾을 수 없습니다.")?;
    let mut file = std::fs::File::open(&path).map_err(|_| "모델 파일을 읽을 수 없습니다.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "모델 파일 정보를 읽을 수 없습니다.")?;
    if !metadata.is_file() || metadata.len() != model.bytes || model.bytes < 8 {
        return Err(
            "모델 파일 크기가 변경되었습니다. 모델을 다시 설치하거나 등록해 주세요.".into(),
        );
    }
    let digest = hash_gguf(
        &mut file,
        model.bytes,
        cancellation,
        &HashErrors {
            read: "모델 파일 무결성을 검사하지 못했습니다.",
            grew: "검사 중 모델 파일 크기가 변경되었습니다.",
            format: "지원하는 GGUF 모델 파일이 아닙니다.",
        },
    )?;
    if model.sha256.as_deref() != Some(digest.as_str()) {
        return Err(
            "모델 파일의 무결성이 일치하지 않습니다. 다시 설치하거나 등록해 주세요.".into(),
        );
    }
    Ok(path)
}

/// Same containment and full-file digest contract as the model weights, for
/// the optional projector artifact. Its GGUF architecture is `clip`.
pub(super) fn verified_projector_path(
    root: &Path,
    model: &AiModel,
    projector: &crate::storage::intelligence::AiProjector,
    cancellation: &Cancellation,
) -> Result<PathBuf, String> {
    let path = super::super::model_storage::managed_path(
        root,
        &projector.path,
        model.storage_directory.as_deref(),
        false,
    )?
    .canonicalize()
    .map_err(|_| "이미지 인식 모듈 파일을 찾을 수 없습니다.")?;
    let mut file =
        std::fs::File::open(&path).map_err(|_| "이미지 인식 모듈 파일을 읽을 수 없습니다.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "이미지 인식 모듈 파일 정보를 읽을 수 없습니다.")?;
    if !metadata.is_file() || metadata.len() != projector.expected_bytes {
        return Err("이미지 인식 모듈 파일 크기가 변경되었습니다. 다시 설치해 주세요.".into());
    }
    let digest = hash_gguf(
        &mut file,
        projector.expected_bytes,
        cancellation,
        &HashErrors {
            read: "이미지 인식 모듈 무결성을 검사하지 못했습니다.",
            grew: "검사 중 이미지 인식 모듈 파일 크기가 변경되었습니다.",
            format: "지원하는 이미지 인식 모듈 파일이 아닙니다.",
        },
    )?;
    if projector.sha256.as_deref() != Some(digest.as_str()) {
        return Err("이미지 인식 모듈의 무결성이 일치하지 않습니다. 다시 설치해 주세요.".into());
    }
    Ok(path)
}
