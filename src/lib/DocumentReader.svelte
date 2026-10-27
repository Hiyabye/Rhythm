<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { DocumentDetail, SyncStatus } from './types';
  import { archiveDate, archiveError, archiveSize, resourceStatus } from './archive-format';
  import Icon from './Icon.svelte';
  import AnalysisPanel from './AnalysisPanel.svelte';

  let { id, enabled, onclose, onchange, onaisettings }: {
    id: number; enabled: boolean; onclose: () => void; onchange: () => void; onaisettings: () => void;
  } = $props();
  let detail = $state<DocumentDetail | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let resourceBusy = $state(false);
  let localResourceErrors = $state<Record<number, string>>({});
  let error = $state('');
  let feedback = $state('');
  let documentFeedback = $state<string | null>(null);
  let safeHtml = $state('');
  let selectedRevision = $state('');
  let heading = $state<HTMLHeadingElement>();
  let generation = 0;
  let disposed = false;
  let feedbackGeneration = 0;
  let feedbackWriteGeneration = 0;
  let busyGeneration = 0;
  let pendingFeedbackDocumentId: number | null = null;

  onMount(() => {
    void load(null, true);
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !(event.target instanceof HTMLSelectElement)) onclose();
    };
    window.addEventListener('keydown', key);
    const timer = window.setInterval(() => {
      if (resourceBusy) void invoke<SyncStatus>('archive_status').then(async (status) => {
        if (!status.running && !disposed) {
          resourceBusy = false;
          feedback = archiveError(status.message);
          await load(detail?.revision_id ?? null);
          onchange();
        }
      }).catch((reason) => { error = archiveError(reason); resourceBusy = false; });
    }, 1000);
    return () => {
      disposed = true;
      generation++;
      feedbackGeneration++;
      feedbackWriteGeneration++;
      busyGeneration++;
      pendingFeedbackDocumentId = null;
      window.removeEventListener('keydown', key);
      window.clearInterval(timer);
    };
  });

  async function load(revisionId: number | null, markRead = false) {
    const request = ++generation;
    const documentId = id;
    const feedbackRequest = ++feedbackGeneration;
    const writeGeneration = feedbackWriteGeneration;
    const feedbackWritePending = pendingFeedbackDocumentId === documentId;
    loading = true;
    error = '';
    try {
      const result = await invoke<DocumentDetail>('archive_document', { id: documentId, revisionId });
      if (disposed || request !== generation || documentId !== id) return;
      const rendered = await localHtml(result, request);
      if (disposed || request !== generation || documentId !== id) return;
      detail = result;
      selectedRevision = String(result.revision_id);
      safeHtml = rendered.html;
      localResourceErrors = rendered.errors;
      await tick();
      heading?.focus({ preventScroll: true });
      // A failed preference/state write must never make readable archival text
      // unavailable (for example when the disk is full or becomes read-only).
      if (markRead && !result.item.is_read) {
        const busyRequest = ++busyGeneration;
        busy = true;
        try {
          await invoke('archive_state', { id: documentId, isRead: true, saved: null });
          // A user can return before this write completes. Refresh the parent
          // even then so an unread-only list cannot retain the just-read row.
          onchange();
          if (disposed || request !== generation || documentId !== id) return;
          detail.item.is_read = true;
        } catch (reason) {
          if (!disposed && request === generation && documentId === id) error = archiveError(reason);
        } finally { if (!disposed && busyRequest === busyGeneration) busy = false; }
      }
    } catch (reason) { if (!disposed && request === generation && documentId === id) error = archiveError(reason); }
    if (!feedbackWritePending) {
      try {
        const savedFeedback = await invoke<string | null>('personalization_get_feedback', { documentId });
        if (!disposed && request === generation && documentId === id
          && feedbackRequest === feedbackGeneration && writeGeneration === feedbackWriteGeneration) {
          documentFeedback = savedFeedback;
        }
      } catch { /* Feedback controls remain usable if a read fails. */ }
    }
    if (!disposed && request === generation) loading = false;
  }

  async function localHtml(document: DocumentDetail, request: number): Promise<{ html: string; errors: Record<number, string> }> {
    // A reload alone does not verify an attachment. Keep observed failures until
    // its local bytes are successfully read or explicitly revealed again.
    const errors = Object.fromEntries(document.resources
      .filter((resource) => localResourceErrors[resource.id])
      .map((resource) => [resource.id, localResourceErrors[resource.id]]));
    const images = new Map<number, Promise<string>>();
    const parsed = new DOMParser().parseFromString(document.body_html, 'text/html');
    const output = window.document.createElement('div');
    const allowed = new Set(['p', 'div', 'span', 'br', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'ul', 'ol', 'li', 'table',
      'thead', 'tbody', 'tfoot', 'tr', 'td', 'th', 'caption', 'strong', 'b', 'em', 'i', 'u', 's', 'sub', 'sup', 'blockquote', 'pre', 'code', 'hr', 'a', 'img']);
    const blocked = new Set(['script', 'style', 'iframe', 'object', 'embed', 'svg', 'math', 'form', 'input', 'button', 'link', 'meta']);
    async function copy(node: Node, parent: HTMLElement) {
      if (disposed || request !== generation) return;
      if (node.nodeType === Node.TEXT_NODE) { parent.append(window.document.createTextNode(node.textContent ?? '')); return; }
      if (!(node instanceof Element)) return;
      const tag = node.tagName.toLowerCase();
      if (blocked.has(tag)) return;
      if (!allowed.has(tag)) { for (const child of [...node.childNodes]) await copy(child, parent); return; }
      if (tag === 'img') {
        const url = node.getAttribute('data-resource-url');
        // v0.2.0 could retain only the attachment role for a URL also used inline.
        // The backend independently validates raster bytes, so these archives are
        // safe to hydrate without rewriting any stored resource metadata.
        const resource = document.resources.find((item) => item.kind === 'image' && item.url === url)
          ?? document.resources.find((item) => item.url === url);
        if (resource?.status === 'downloaded') {
          try {
            let imageData = images.get(resource.id);
            if (!imageData) {
              imageData = invoke<string>('archive_resource_image', { id: resource.id });
              images.set(resource.id, imageData);
            }
            const data = await imageData;
            if (disposed || request !== generation) return;
            if (/^data:image\/(?:png|jpeg|gif|webp);base64,[A-Za-z0-9+/=]+$/.test(data)) {
              const image = window.document.createElement('img');
              image.src = data;
              image.alt = node.getAttribute('alt') || resource.filename || '공지 이미지';
              image.loading = 'lazy';
              parent.append(image);
              delete errors[resource.id];
              return;
            }
            errors[resource.id] = '로컬 이미지 형식을 확인하지 못했습니다. 다시 내려받아 주세요.';
          } catch (reason) {
            errors[resource.id] = archiveError(reason);
          }
        }
        const placeholder = window.document.createElement('p');
        placeholder.className = 'reader-missing-image';
        placeholder.textContent = `[이미지: ${node.getAttribute('alt') || resource?.filename || '로컬 파일 없음'} · 첨부 자료에서 상태 확인]`;
        parent.append(placeholder);
        return;
      }
      const element = window.document.createElement(tag);
      if (tag === 'a') {
        try {
          const url = new URL(node.getAttribute('href') ?? '');
          if (['https:', 'http:'].includes(url.protocol) && !url.username && !url.password) {
            element.setAttribute('href', '#');
            element.dataset.externalUrl = url.href;
            element.setAttribute('title', `외부 링크: ${url.hostname}`);
          }
        } catch { /* Unsafe or unresolved links remain plain text. */ }
      }
      if (tag === 'td' || tag === 'th') {
        for (const attribute of ['colspan', 'rowspan']) {
          const value = node.getAttribute(attribute);
          if (value && /^\d{1,2}$/.test(value) && Number(value) > 0) element.setAttribute(attribute, value);
        }
      }
      for (const child of [...node.childNodes]) await copy(child, element);
      if (element.dataset.externalUrl) element.append(window.document.createTextNode(' ↗'));
      parent.append(element);
    }
    for (const node of [...parsed.body.childNodes]) await copy(node, output);
    return { html: output.innerHTML, errors };
  }

  function scrollBody(event: KeyboardEvent) {
    if (event.target !== event.currentTarget || event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
      const node = event.currentTarget as HTMLElement;
      if (node.scrollWidth > node.clientWidth) {
        event.preventDefault();
        node.scrollBy({ left: event.key === 'ArrowRight' ? 80 : -80, behavior: 'instant' });
      }
    }
  }

  function explicitLinks(node: HTMLElement) {
    const click = (event: MouseEvent) => {
      const link = event.target instanceof Element ? event.target.closest<HTMLAnchorElement>('a[data-external-url]') : null;
      if (!link) return;
      event.preventDefault();
      void external(link.dataset.externalUrl!);
    };
    node.addEventListener('click', click);
    return { destroy() { node.removeEventListener('click', click); } };
  }

  async function external(url: string) {
    try { await invoke('archive_open_original', { url }); }
    catch (reason) { error = archiveError(reason); }
  }

  async function updateState(field: 'is_read' | 'saved') {
    if (!detail || busy) return;
    const request = ++busyGeneration;
    const documentId = id;
    busy = true;
    error = '';
    try {
      const value = !detail.item[field];
      await invoke('archive_state', { id: documentId, isRead: field === 'is_read' ? value : null, saved: field === 'saved' ? value : null });
      onchange();
      if (disposed || request !== busyGeneration || documentId !== id || !detail) return;
      detail.item[field] = value;
      feedback = field === 'saved' ? value ? '보관함에 저장했습니다.' : '보관함에서 해제했습니다.' : value ? '읽음으로 표시했습니다.' : '읽지 않음으로 표시했습니다.';
    } catch (reason) {
      if (!disposed && request === busyGeneration && documentId === id) error = archiveError(reason);
    } finally { if (!disposed && request === busyGeneration) busy = false; }
  }
  async function setDocumentFeedback(value: string | null) {
    if (busy) return;
    const request = ++feedbackWriteGeneration;
    const documentId = id;
    const busyRequest = ++busyGeneration;
    pendingFeedbackDocumentId = documentId;
    busy = true; error = ''; feedback = '';
    try {
      const savedFeedback = await invoke<string | null>('personalization_set_feedback', { documentId, feedback: value });
      onchange();
      if (disposed || request !== feedbackWriteGeneration || documentId !== id) return;
      documentFeedback = savedFeedback;
      feedback = documentFeedback ? '피드백을 저장했습니다. 언제든 취소하거나 바꿀 수 있습니다.' : '피드백을 취소했습니다.';
    } catch (reason) {
      if (!disposed && request === feedbackWriteGeneration && documentId === id) error = archiveError(reason);
    } finally {
      if (request === feedbackWriteGeneration && pendingFeedbackDocumentId === documentId) pendingFeedbackDocumentId = null;
      if (!disposed && busyRequest === busyGeneration) busy = false;
    }
  }

  async function retry(resourceId: number) {
    if (!enabled || resourceBusy) return;
    error = '';
    try {
      const result = await invoke<SyncStatus>('archive_retry_resource', { id: resourceId });
      resourceBusy = result.running;
      feedback = result.message;
      if (!result.running) await load(detail?.revision_id ?? null);
    } catch (reason) { error = archiveError(reason); }
  }

  async function reveal(resourceId: number) {
    try {
      await invoke('archive_reveal_resource', { id: resourceId });
      const remaining = { ...localResourceErrors };
      delete remaining[resourceId];
      localResourceErrors = remaining;
    } catch (reason) {
      localResourceErrors = { ...localResourceErrors, [resourceId]: archiveError(reason) };
    }
  }
</script>

<section class="archive-reader" aria-label="보관된 공지 읽기" aria-busy={loading}>
  <button class="text-button reader-back" onclick={onclose}>← 목록으로 돌아가기 <span class="muted">Esc</span></button>
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  <p class="archive-feedback" role="status">{feedback}</p>
  {#if loading}<p class="loading" role="status">로컬 공지를 불러오는 중입니다…</p>{/if}
  {#if detail}
    <header class="reader-header">
      <span class="section-kicker">{detail.item.source_name}</span>
      <h2 bind:this={heading} tabindex="-1">{detail.item.title}</h2>
      <div class="notice-meta"><time datetime={detail.item.published_at}>{archiveDate(detail.item.published_at)}</time>{#if detail.item.author}<span>{detail.item.author}</span>{/if}{#if detail.item.category}<span>{detail.item.category}</span>{/if}</div>
      <div class="reader-actions">
        <button class="button secondary small" disabled={busy} aria-pressed={detail.item.is_read} onclick={() => updateState('is_read')}>{detail.item.is_read ? '읽지 않음으로 표시' : '읽음으로 표시'}</button>
        <button class="button secondary small" disabled={busy} aria-pressed={detail.item.saved} onclick={() => updateState('saved')}><Icon name="bookmark" size={16} />{detail.item.saved ? '저장 해제' : '보관함에 저장'}</button>
        <button class="text-button" onclick={() => external(detail!.canonical_url)}>학교 원문 열기 ↗</button>
      </div>
      <div class="reader-actions" role="group" aria-label="관심 피드백">
        <span>이 공지가 도움이 되었나요?</span>
        <button class="button secondary small" aria-pressed={documentFeedback === 'interested'} disabled={busy} onclick={() => void setDocumentFeedback(documentFeedback === 'interested' ? null : 'interested')}>관심 있어요</button>
        <button class="button secondary small" aria-pressed={documentFeedback === 'not_interested'} disabled={busy} onclick={() => void setDocumentFeedback(documentFeedback === 'not_interested' ? null : 'not_interested')}>관심 없어요</button>
        {#if documentFeedback}<button class="text-button" disabled={busy} onclick={() => void setDocumentFeedback(null)}>피드백 취소</button>{/if}
      </div>
      <p class="archive-local-status">본문은 로컬에 보관되어 있습니다. {detail.resources.some((item) => localResourceErrors[item.id]) ? '일부 로컬 자료가 없거나 손상되었을 수 있습니다. 아래에서 확인하거나 다시 내려받아 주세요.' : detail.resources.every((item) => item.status === 'downloaded') ? '연결된 자료도 모두 저장되어 있습니다.' : '일부 자료는 로컬에 없습니다. 아래에서 상태를 확인하세요.'}</p>
      {#if detail.revisions.length > 1}
        <label class="revision-label">보관 이력
          <select bind:value={selectedRevision} onchange={() => load(Number(selectedRevision))} disabled={loading}>
            {#each detail.revisions as revision, index}<option value={revision.id}>{archiveDate(revision.collected_at, true)} · 버전 {detail.revisions.length - index}</option>{/each}
          </select>
        </label>
      {/if}
    </header>
    {#key detail.revision_id}<AnalysisPanel documentId={detail.item.id} revisionId={detail.revision_id} onsettings={onaisettings} {onchange} />{/key}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (Wide document tables need an explicitly focusable keyboard scroll region.) -->
    <div class="reader-body" role="region" tabindex="0" aria-label="공지 본문 · 넓은 표는 좌우로 이동할 수 있습니다" onkeydown={scrollBody} use:explicitLinks>{@html safeHtml}</div>
    {#if !safeHtml && !loading}<div class="reader-plain">{detail.body_text}</div>{/if}
    {#if detail.resources.length}
      <section class="reader-resources" aria-labelledby="resource-heading">
        <h3 id="resource-heading">첨부 자료 및 이미지 <span class="muted">{detail.resources.length}</span></h3>
        <p class="footnote">저장된 파일은 폴더에서 직접 확인할 수 있습니다. 파일은 자동으로 실행되지 않습니다.</p>
        {#each detail.resources as resource}
          <div class="resource-row">
            <div><strong>{resource.filename || (resource.kind === 'image' ? '본문 이미지' : '첨부 파일')}</strong><p>{localResourceErrors[resource.id] ? '로컬 자료 확인 필요' : resourceStatus(resource.status)} · {archiveSize(resource.bytes)}{#if resource.mime} · {resource.mime}{/if}</p>{#if localResourceErrors[resource.id]}<p class="archive-error" role="alert">{localResourceErrors[resource.id]}</p>{:else if resource.error}<p class="archive-error">{resource.error}</p>{/if}{#if resource.sha256}<details><summary>무결성 정보</summary><code>SHA-256 {resource.sha256}</code></details>{/if}</div>
            {#if resource.status === 'downloaded'}<button class="text-button" onclick={() => reveal(resource.id)}>{localResourceErrors[resource.id] ? '로컬 파일 다시 확인' : '저장 폴더에서 보기'}</button>{/if}
            <button class="text-button" disabled={!enabled || resourceBusy} aria-label={`${resource.filename || '첨부 자료'} 다시 내려받기`} onclick={() => retry(resource.id)}>{resourceBusy ? '자료 다운로드 중…' : '다시 내려받기'}</button>
          </div>
        {/each}
        {#if !enabled}<p class="footnote">자료를 추가로 받으려면 설정에서 서강대학교 공지 확장 기능을 켜 주세요.</p>{/if}
      </section>
    {/if}
    <footer class="reader-archive-meta"><span>처음 수집: {archiveDate(detail.item.collected_at, true)}</span><span>이 버전 보관: {archiveDate(detail.revisions.find((revision) => revision.id === detail?.revision_id)?.collected_at ?? null, true)}</span></footer>
  {/if}
</section>
