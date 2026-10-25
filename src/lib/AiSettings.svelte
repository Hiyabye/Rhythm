<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { AiPreferences, AiSnapshot } from './types';
  import { archiveError, archiveSize } from './archive-format';

  let { desktop, onsetup }: { desktop: boolean; onsetup?: () => void } = $props();
  let data = $state<AiSnapshot | null>(null);
  let busy = $state(false);
  let error = $state('');
  let feedback = $state('');
  let path = $state('');
  let removal = $state<string | null>(null);
  let disposed = false;
  let refreshing = false;
  const modelStatus: Record<string, string> = { pending: '다운로드 대기', downloading: '내려받는 중', verifying: '무결성 확인 중', installed: '설치 완료', failed: '설치 실패', cancelled: '중단됨 · 이어받기 가능', missing: '파일 확인 필요' };

  onMount(() => {
    if (desktop) void refresh();
    const timer = window.setInterval(() => { if (desktop && !busy) void refresh(); }, 1500);
    return () => { disposed = true; window.clearInterval(timer); };
  });

  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    try { const next = await invoke<AiSnapshot>('ai_snapshot'); if (!disposed) data = next; }
    catch (reason) { if (!disposed) error = archiveError(reason); }
    finally { refreshing = false; }
  }
  async function action(command: string, args: Record<string, unknown> = {}, success = '') {
    if (busy) return;
    busy = true; error = ''; feedback = '';
    try {
      await invoke(command, args);
      if (!disposed) { feedback = success; await refresh(); if (data?.preferences.setup_completed) onsetup?.(); }
    } catch (reason) { if (!disposed) error = archiveError(reason); }
    finally { if (!disposed) busy = false; }
  }
  async function preference(patch: Partial<AiPreferences>) {
    if (!data) return;
    await action('ai_preferences', { preferences: { ...data.preferences, ...patch } }, 'AI 처리 설정을 저장했습니다.');
  }
</script>

<section id="ai" class="settings-section" aria-busy={busy}>
  <div class="section-heading"><h2 tabindex="-1">AI 모델</h2><p>공지 분석은 이 기기에서만 실행됩니다. 문서와 생성 결과를 외부 추론 서버로 보내지 않습니다.</p></div>
  {#if !desktop}<p class="muted">모델 설치와 로컬 추론은 데스크톱 앱에서 사용할 수 있습니다.</p>
  {:else if !data && !error}<p class="loading" role="status">모델 정보를 불러오는 중입니다…</p>{/if}
  {#if error}<p class="archive-error" role="alert">{error}</p><button class="text-button" disabled={busy} onclick={refresh}>모델 정보 다시 확인</button>{/if}
  <p class="archive-feedback" role="status">{feedback}</p>
  {#if data}
    {#if !data.preferences.setup_completed}<div class="ai-setup"><h3>로컬 AI 시작하기</h3><p>모델 선택 → 다운로드와 무결성 확인 → 불러오기 → 실제 추론 확인 순서로 준비합니다. 인터넷이 없어도 보관된 공지는 계속 읽을 수 있습니다.</p><p>설치는 아래 버튼을 누른 경우에만 시작합니다. 준비는 나중에 이어서 할 수 있습니다.</p></div>{/if}
    <div class="setting-row"><div><h3>내 기기</h3><p>{data.hardware.os} · {data.hardware.arch} · 메모리 {archiveSize(data.hardware.ram_bytes)}<br />남은 저장 공간 {archiveSize(data.hardware.disk_available_bytes)} · 빌드 백엔드 {data.hardware.backend}</p></div></div>
    <div class="setting-row"><div><h3>AI 엔진</h3><p>{data.runtime.state === 'busy' ? '공지 분석 중' : data.runtime.state === 'loading' ? '모델을 불러오는 중' : data.runtime.state === 'ready' ? (data.runtime.model ? '모델 준비됨' : '모델 없이 대기 중') : '엔진 확인 필요'}</p>{#if data.runtime.detail}<p>{data.runtime.detail}</p>{/if}</div><div class="inline-controls"><button class="text-button" disabled={busy || data.busy} onclick={() => action('ai_restart', {}, 'AI 엔진을 다시 시작했습니다.')}>엔진 다시 시작</button><button class="text-button" disabled={busy} aria-label="AI 엔진 상태 새로고침" onclick={refresh}>상태 새로고침</button></div></div>
    <div class="ai-catalog" aria-label="모델 카탈로그">
      {#each data.catalog as entry}
        {@const installed = data.models.find((model) => model.catalog_id === entry.id)}
        <article class="ai-model-option">
          {#if entry.compatibility === 'verified' && entry.parameters === '4B' && data.hardware.ram_bytes >= entry.memory_bytes && data.hardware.disk_available_bytes >= entry.bytes}<p class="ai-tag">균형형 선택지 · 이 기기의 메모리와 저장 공간을 확인했습니다</p>{/if}
          <div class="ai-model-heading"><h3>{entry.name} <span class="muted">{entry.quantization}</span></h3><span class="ai-tag">{entry.compatibility === 'verified' ? '실제 추론 검증됨' : entry.compatibility === 'experimental' ? '실험적 · 추론 미검증' : '현재 지원하지 않음'}</span></div>
          <p>{entry.description}</p><p class="footnote">{entry.use_case} · 다운로드 {archiveSize(entry.bytes)} · 예상 메모리 {archiveSize(entry.memory_bytes)}</p>
          {#if entry.reason}<p class="footnote">{entry.reason}</p>{/if}
          <details><summary>출처 · 파일 · 라이선스</summary><p>{entry.repository} · {entry.license}</p><code>{entry.filename}<br />리비전 {entry.revision}<br />SHA-256 {entry.sha256}</code><p class="footnote">텍스트 분석용입니다. 이미지 이해 기능은 제공하지 않습니다.</p></details>
          {#if !installed || installed.status !== 'installed'}<button class="button secondary small" disabled={busy || data.busy || entry.compatibility !== 'verified'} onclick={() => action('ai_install', { catalogId: entry.id }, '모델 다운로드를 시작했습니다. 앱을 닫아도 다음 실행에서 이어받을 수 있습니다.')}>{installed ? `${entry.name} 설치 다시 시도 / 이어받기` : `${entry.name} 다운로드 및 설치`}</button>{/if}
        </article>
      {/each}
    </div>
    <h3 class="ai-subheading">내 모델</h3>
    {#if !data.models.length}<p class="muted">아직 설치된 모델이 없습니다. 모델 없이도 보관된 공지를 읽고 검색할 수 있습니다.</p>{/if}
    {#each data.models as model}
      <article class="ai-installed">
        <div class="ai-model-heading"><h3>{model.name}</h3><span class="ai-tag">{data.preferences.active_model_id === model.id ? '선택한 모델 · ' : ''}{modelStatus[model.status] ?? model.status}</span></div>
        <p>{model.ownership === 'external' ? '외부 파일 참조 · Rhythm에서 원본 파일을 삭제하지 않습니다.' : 'Rhythm이 관리하는 파일'} · {archiveSize(model.bytes || model.downloaded_bytes)}</p>
        <code>{model.path}</code>
        {#if model.status === 'downloading' || model.status === 'verifying' || model.status === 'pending'}
          <label class="ai-progress">{model.status === 'verifying' ? '모델 무결성 확인' : '모델 다운로드'}<progress max={model.expected_bytes || 1} value={model.downloaded_bytes}></progress><span>{archiveSize(model.downloaded_bytes)} / {archiveSize(model.expected_bytes ?? 0)}</span></label>
          <button class="text-button" disabled={busy} onclick={() => action('ai_cancel_download', {}, '다운로드 중단을 요청했습니다. 받은 부분은 이어받기를 위해 유지합니다.')}>다운로드 중단</button>
        {/if}
        {#if model.error}<p class="archive-error" role="alert">{model.error}</p>{/if}
        <div class="ai-actions">
          {#if model.status === 'installed'}<button class="button secondary small" disabled={busy || data.busy || data.preferences.active_model_id === model.id} onclick={() => action('ai_select', { id: model.id }, '사용할 모델을 선택했습니다.')}>{data.preferences.active_model_id === model.id ? '선택됨' : `${model.name} 선택`}</button>{/if}
          {#if removal !== model.id}<button class="text-button" disabled={busy || data.busy} onclick={() => removal = model.id}>{model.ownership === 'external' ? '참조 해제' : '모델 삭제'}</button>{/if}
        </div>
        {#if removal === model.id}<div class="ai-confirm" role="group" aria-label={`${model.name} 삭제 확인`}><p>{model.ownership === 'external' ? '등록된 참조만 해제합니다. 외부 원본 파일은 삭제하지 않습니다.' : '모델 파일을 삭제할까요? 공지와 기존 AI 분석은 유지됩니다.'}</p><button class="button secondary small" disabled={busy || data.busy} onclick={async () => { await action('ai_remove', { id: model.id, confirmed: true }, '모델 등록을 정리했습니다. 기존 분석은 유지됩니다.'); removal = null; }}>확인하고 {model.ownership === 'external' ? '참조 해제' : '삭제'}</button><button class="text-button" onclick={() => removal = null}>취소</button></div>{/if}
      </article>
    {/each}
    <div class="ai-actions"><button class="button small" disabled={busy || data.busy || !data.preferences.active_model_id} onclick={() => action('ai_load', {}, '모델을 불러왔습니다.')}>선택한 모델 불러오기</button><button class="button secondary small" disabled={busy || data.busy || !data.runtime.model} onclick={() => action('ai_unload', {}, '모델 메모리를 해제했습니다.')}>모델 메모리 해제</button></div>
    {#if !data.preferences.setup_completed}<button class="text-button" disabled={busy || data.busy || !data.runtime.model} onclick={() => preference({ setup_completed: true })}>실제 추론 확인 후 AI 설정 완료</button><p class="footnote">설정 완료 시 짧은 로컬 추론으로 모델 동작을 확인합니다. 실패하면 공지는 그대로 유지됩니다.</p>{/if}
    <details class="ai-register"><summary>이미 가진 로컬 GGUF 등록</summary><p>파일을 복사하지 않고 참조합니다. 지원되는 GGUF 파일의 전체 경로를 입력하세요. 외부 파일은 앱에서 삭제하지 않습니다.</p><label class="field-label" for="local-model-path">로컬 GGUF 전체 경로</label><input id="local-model-path" type="text" bind:value={path} placeholder="/전체/경로/model.gguf" spellcheck="false" /><button class="button secondary small" disabled={busy || data.busy || !path.trim()} onclick={() => action('ai_register', { path: path.trim() }, '로컬 모델을 등록했습니다. 선택한 뒤 불러와 호환성을 확인하세요.')}>외부 파일 참조로 등록</button></details>
  {/if}
</section>

<section id="ai-processing" class="settings-section">
  <div class="section-heading"><h2>AI 처리</h2><p>전체 보관함을 한꺼번에 분석하지 않습니다. 공지를 읽고 필요할 때 요청하세요.</p></div>
  {#if data}
    <label class="field-label" for="ai-processing-mode">분석 방식</label><select id="ai-processing-mode" value={data.preferences.processing_mode} disabled={busy} onchange={(event) => preference({ processing_mode: event.currentTarget.value as AiPreferences['processing_mode'] })}><option value="explicit">직접 요청한 공지만</option><option value="recent">앱을 사용하는 동안 최근 공지 소량 분석</option></select>
    <label class="field-label" for="ai-automatic-limit">자동 처리 상한</label><input id="ai-automatic-limit" type="number" min="0" max="20" value={data.preferences.automatic_limit} disabled={busy} onchange={(event) => preference({ automatic_limit: Number(event.currentTarget.value) })} /><p class="footnote">앱 실행 중 적용되는 상한입니다. 절전 프로필에서는 직접 요청을 우선합니다. 앱이 종료되면 분석하지 않습니다.</p>
    <details><summary>고급 · 출력 길이</summary><label class="field-label" for="ai-max-tokens">최대 출력 토큰</label><input id="ai-max-tokens" type="number" min="128" max="4096" step="128" value={data.preferences.max_tokens} disabled={busy} onchange={(event) => preference({ max_tokens: Number(event.currentTarget.value) })} /><p class="footnote">길이를 바꾸면 새 분석에 적용되며 기존 결과는 유지됩니다.</p></details>
    <h3 class="ai-subheading">처리 대기열</h3>
    {#if !data.jobs.length}<p class="muted">요청한 분석이 없습니다.</p>{/if}
    <ul class="ai-queue">{#each data.jobs as job}<li><div><strong>공지 #{job.document_id}</strong><span>{job.status === 'running' ? '분석 중' : job.status === 'pending' ? '대기 중' : job.status === 'completed' ? '완료' : job.status === 'cancelled' ? '취소됨' : job.status === 'stale' ? '이전 버전' : '분석 실패'}</span>{#if job.error}<p class="archive-error">{job.error}</p>{/if}</div>{#if ['pending', 'running'].includes(job.status)}<button class="text-button" disabled={busy} onclick={() => action('ai_cancel', { id: job.id }, '분석 취소를 요청했습니다.')}>분석 취소</button>{/if}</li>{/each}</ul>
  {:else}<p class="muted">데스크톱 앱에서 AI 처리 설정을 확인할 수 있습니다.</p>{/if}
</section>
