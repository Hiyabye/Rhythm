<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { CatalogEntry, AiModel, AiPreferences, AiSnapshot } from './types';
  import { archiveError, archiveSize } from './archive-format';
  import { isWindows } from './platform';
  import Icon from './Icon.svelte';
  import Hint from './Hint.svelte';
  import Popover from './Popover.svelte';

  let { desktop, onsetup }: { desktop: boolean; onsetup?: () => void } = $props();
  let data = $state<AiSnapshot | null>(null);
  let busy = $state(false);
  let error = $state('');
  let fetchError = $state('');
  let cancelling = $state(false);
  let controlsBusy = $derived(busy || Boolean(data?.operation));
  let feedback = $state('');
  let path = $state('');
  let removal = $state<string | null>(null);
  let executionMode = $state<AiPreferences['execution_mode']>('auto');
  let advancedOpen = $state(false);
  let disposed = false;
  let refreshing = false;
  let setupReported = false;
  const operationPhase: Record<string, string> = { verifying: '모델 파일 무결성을 확인하고 있습니다', waiting: 'AI 엔진이 준비되기를 기다리고 있습니다', restarting: 'AI 엔진을 다시 준비하고 있습니다', loading: '모델을 메모리에 불러오고 있습니다', setup: '짧은 로컬 추론으로 설치를 확인하고 있습니다', cancelling: '작업을 중단하고 메모리를 정리하고 있습니다' };
  const operationName: Record<string, string> = { load: '모델 불러오기', setup: 'AI 설정 확인', register: '외부 모델 등록' };
  const modelStatus: Record<string, string> = { pending: '다운로드 대기', downloading: '내려받는 중', verifying: '무결성 확인 중', installed: '설치됨', failed: '설치 실패', cancelled: '중단됨 · 이어받기 가능', missing: '파일 확인 필요' };
  const installable = (entry: CatalogEntry) => ['verified', 'available'].includes(entry.compatibility);
  // One tile per offered model, plus every inventory row (external, retired or in progress) that no offered tile covers.
  let tiles = $derived.by(() => {
    if (!data) return [];
    const offered = data.catalog.filter(installable);
    const list: { entry: CatalogEntry | null; model: AiModel | null }[] = offered.map((entry) => ({ entry, model: data!.models.find((model) => model.catalog_id === entry.id) ?? null }));
    for (const model of data.models) {
      if (!offered.some((entry) => entry.id === model.catalog_id)) list.push({ entry: data.catalog.find((entry) => entry.id === model.catalog_id) ?? null, model });
    }
    return list;
  });
  let activeModel = $derived(data?.models.find((model) => model.id === data?.preferences.active_model_id) ?? null);
  let hasInstalled = $derived(data?.models.some((model) => model.status === 'installed') ?? false);

  $effect(() => { if (data?.model_storage.error) advancedOpen = true; });

  onMount(() => {
    if (desktop) void refresh();
    const timer = window.setInterval(() => { if (desktop) void refresh(); }, 1000);
    return () => { disposed = true; window.clearInterval(timer); };
  });

  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    try {
      const next = await invoke<AiSnapshot>('ai_snapshot');
      if (!disposed) {
        data = next; fetchError = '';
        executionMode = next.preferences.execution_mode ?? 'auto';
        if (next.preferences.setup_completed && !setupReported) { setupReported = true; onsetup?.(); }
      }
    }
    catch (reason) { if (!disposed) fetchError = archiveError(reason); }
    finally { refreshing = false; }
  }
  async function action(command: string, args: Record<string, unknown> = {}, success = '') {
    if (controlsBusy) return;
    busy = true; error = ''; feedback = '';
    try {
      await invoke(command, args);
      if (!disposed) { feedback = success; await refresh(); }
    } catch (reason) { if (!disposed) { error = archiveError(reason); await refresh(); } }
    finally { if (!disposed) busy = false; }
  }
  async function interrupt(command: string, args: Record<string, unknown>, message: string) {
    if (cancelling) return;
    cancelling = true;
    try {
      await invoke(command, args);
      if (!disposed) { feedback = message; await refresh(); }
    } catch (reason) { if (!disposed) error = archiveError(reason); }
    finally { if (!disposed) cancelling = false; }
  }
  async function chooseModelDirectory() {
    if (controlsBusy || data?.busy) return;
    busy = true; error = ''; feedback = '';
    try {
      const selected = await invoke<boolean>('ai_choose_model_directory');
      if (!disposed) {
        if (selected) feedback = '새 모델을 설치할 폴더를 바꿨습니다. 기존 모델은 옮기지 않습니다.';
        await refresh();
      }
    } catch (reason) { if (!disposed) { error = archiveError(reason); await refresh(); } }
    finally { if (!disposed) busy = false; }
  }
  async function preference(patch: Partial<Omit<AiPreferences, 'model_directory'>>) {
    if (!data) return;
    await action('ai_preferences', { preferences: { ...data.preferences, ...patch } }, 'AI 설정을 저장했습니다.');
  }
  function engineLabel(snapshot: AiSnapshot) {
    const state = snapshot.runtime.state;
    return state === 'busy' ? '로컬 AI 작업 중' : state === 'loading' ? '모델을 불러오는 중' : state === 'ready' ? (snapshot.runtime.model ? '모델 준비됨' : '모델 없이 대기 중') : '엔진 확인 필요';
  }
  function compatibilityLabel(entry: CatalogEntry) {
    return entry.compatibility === 'available' ? '설치 가능 · Windows CPU 검증 범위 확인' : entry.compatibility === 'verified' ? '일부 환경에서 실제 추론 확인' : entry.compatibility === 'experimental' ? '실험적 · 추론 미검증' : '현재 지원하지 않음';
  }
</script>

<div id="ai" class="ai-settings" aria-busy={desktop && !data && !fetchError}>
  {#if !desktop}<p class="muted">모델 설치와 로컬 추론은 데스크톱 앱에서 사용할 수 있습니다.</p>
  {:else if !data && !fetchError}<p class="loading" role="status">모델 정보를 불러오는 중입니다…</p>{/if}
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  {#if fetchError}<p class="archive-error" role="alert">{fetchError}</p><button class="text-button" onclick={refresh}>모델 정보 다시 확인</button>{/if}
  <p class="archive-feedback" role="status">{feedback}</p>
  {#if data}
    {#if !data.preferences.setup_completed}
      <div class="ai-setup">
        <div class="setup-title"><h3>로컬 AI 시작하기</h3><Hint label="로컬 AI 시작하기">모델 설치는 직접 누를 때만 시작합니다. 마지막 단계에서 짧은 로컬 추론으로 실제 동작을 확인합니다. 준비하지 않아도 보관한 공지는 읽고 검색할 수 있습니다.</Hint></div>
        <ol class="setup-steps">
          <li class:done={hasInstalled}><span aria-hidden="true">{#if hasInstalled}<Icon name="check" size={12} />{:else}1{/if}</span>모델 설치</li>
          <li class:done={Boolean(data.runtime.model)}><span aria-hidden="true">{#if data.runtime.model}<Icon name="check" size={12} />{:else}2{/if}</span>불러오기</li>
          <li><span aria-hidden="true">3</span>동작 확인</li>
        </ol>
        <button class="button small" disabled={busy || data.busy || !data.runtime.model} aria-label="실제 추론 확인 후 AI 설정 완료" onclick={() => preference({ setup_completed: true })}>동작 확인하고 완료</button>
      </div>
    {/if}

    {#if data.operation}
      <div class="ai-operation" role="group" aria-label="진행 중인 AI 작업">
        <div><strong>{operationName[data.operation.kind] ?? '로컬 AI 작업'}</strong><p role="status">{operationPhase[data.operation.cancelling ? 'cancelling' : data.operation.phase] ?? '로컬 AI 작업을 처리하고 있습니다'} · 경과 {(data.operation.elapsed_ms / 1000).toFixed(0)}초</p></div>
        <button class="button secondary small" disabled={cancelling || data.operation.cancelling} aria-label="현재 AI 작업 취소" onclick={() => interrupt('ai_cancel_operation', {}, '작업 중단을 요청했습니다. 정리가 끝날 때까지 기다려 주세요.')}>{data.operation.cancelling ? '중단하는 중…' : '취소'}</button>
      </div>
    {/if}

    <div class="settings-group">
      <div class="settings-row">
        <span class="row-label">AI 엔진<span class="row-detail">{engineLabel(data)}{#if data.runtime.model && activeModel} · {activeModel.name}{/if}{#if data.runtime.detail} · {data.runtime.detail}{/if}</span></span>
        <button class="row-icon" aria-label="AI 엔진 상태 새로고침" onclick={refresh}><Icon name="refresh" size={15} /></button>
        {#if data.runtime.model}<button class="button secondary small" disabled={busy || data.busy} aria-label="모델 메모리 해제" onclick={() => action('ai_unload', {}, '모델 메모리를 해제했습니다.')}>메모리 해제</button>
        {:else}<button class="button small" disabled={busy || data.busy || !data.preferences.active_model_id} aria-label="선택한 모델 불러오기" onclick={() => action('ai_load', {}, '모델을 불러왔습니다.')}>불러오기</button>{/if}
      </div>
    </div>

    <h3 class="settings-group-title">모델</h3>
    <ul class="model-grid" aria-label="모델">
      {#each tiles as { entry, model } (model?.id ?? entry?.id)}
        {@const name = model?.name ?? entry?.name ?? ''}
        {@const selected = Boolean(model) && data.preferences.active_model_id === model?.id}
        {@const retired = Boolean(model?.catalog_id) && !data.catalog.some((item) => item.id === model?.catalog_id)}
        {@const infoId = `model-${(model?.id ?? entry?.id ?? '').replace(/[^A-Za-z0-9-]/g, '-')}`}
        <li class="model-tile" class:selected class:installed={model?.status === 'installed'}>
          {#if model?.status === 'installed'}
            <button class="model-select" aria-pressed={selected} aria-label={`${name} 선택`} disabled={busy || data.busy} onclick={() => { if (!selected) void action('ai_select', { id: model.id }, '사용할 모델을 선택했습니다.'); }}></button>
          {/if}
          <div class="model-heading"><h4>{name}</h4>{#if selected}<span class="model-check"><Icon name="check" size={13} /><span>사용 중</span></span>{/if}</div>
          <p class="model-meta">{model?.ownership === 'external' ? '외부 파일' : entry?.use_case ?? ''} · {archiveSize(model ? (model.bytes || model.downloaded_bytes || entry?.bytes || 0) : entry?.bytes ?? 0)}</p>
          {#if retired}<p class="model-note">더 이상 제공하지 않는 모델</p>{/if}
          {#if model && ['downloading', 'pending'].includes(model.status)}
            <label class="ai-progress"><span class="sr-only">모델 다운로드</span><progress max={model.expected_bytes || 1} value={model.downloaded_bytes}></progress><span>{archiveSize(model.downloaded_bytes)} / {archiveSize(model.expected_bytes ?? 0)}</span></label>
          {:else if model?.status === 'verifying'}<p class="model-note" role="status">무결성 확인 중</p>
          {:else if model && model.status !== 'installed'}<p class="model-note">{modelStatus[model.status] ?? model.status}</p>{/if}
          {#if model?.error}<p class="archive-error" role="alert">{model.error}</p>{/if}
          <div class="model-actions">
            {#if !model && entry}<button class="button small" disabled={busy || data.busy} aria-label={`${entry.name} 다운로드 및 설치`} onclick={() => action('ai_install', { catalogId: entry.id }, '모델 다운로드를 시작했습니다. 앱을 닫아도 다음 실행에서 이어받습니다.')}>설치</button>
            {:else if model && ['downloading', 'pending', 'verifying'].includes(model.status)}{#if model.status !== 'verifying'}<button class="button secondary small" disabled={cancelling} aria-label="다운로드 중단" onclick={() => interrupt('ai_cancel_download', {}, '다운로드를 중단했습니다. 받은 부분은 이어받기용으로 남겨 둡니다.')}>중단</button>{/if}
            {:else if model && model.status !== 'installed' && entry && !retired}<button class="button small" disabled={busy || data.busy} aria-label={`${entry.name} 설치 다시 시도 / 이어받기`} onclick={() => action('ai_install', { catalogId: entry.id }, '모델 다운로드를 다시 시작했습니다.')}>이어받기</button>{/if}
            {#if model && removal !== model.id && !['downloading', 'pending', 'verifying'].includes(model.status)}<button class="text-button" disabled={busy || data.busy} onclick={() => removal = model.id}>{model.ownership === 'external' ? '참조 해제' : '모델 삭제'}</button>{/if}
            <button class="row-icon model-info" popovertarget={infoId} aria-label={`${name} 정보`}><Icon name="info" size={15} /></button>
          </div>
          <Popover id={infoId} label={`${name} 정보`} wide>
            <p class="popover-title">{name}</p>
            {#if entry}<p>{entry.description} · {compatibilityLabel(entry)}</p><p>다운로드 {archiveSize(entry.bytes)} · 예상 메모리 {archiveSize(entry.memory_bytes)}(실측 최대치 아님) · 텍스트 분석 전용</p>{#if entry.reason}<p>{entry.reason}</p>{/if}{/if}
            {#if retired}<p>목록에서 내려간 모델입니다. 파일과 기존 분석은 그대로 두며, 다시 설치하거나 이어받을 수 없습니다.</p>{/if}
            {#if model}
              <p>{model.ownership === 'external' ? '외부 파일 참조 · Rhythm에서 원본을 삭제하지 않습니다.' : 'Rhythm이 관리하는 파일'}</p>
              {#if model.ownership === 'managed' && model.storage_directory}<p>저장 폴더</p><code class="popover-code">{model.storage_directory}</code>{/if}
              <p>{model.ownership === 'external' ? '참조 파일' : '저장 파일'}</p><code class="popover-code">{model.path}</code>
            {/if}
            {#if entry}<p>{entry.repository} · {entry.license}</p><code class="popover-code">{entry.filename} · 리비전 {entry.revision}<br />SHA-256 {entry.sha256}</code>{/if}
          </Popover>
        </li>
      {/each}
    </ul>
    {#each data.models.filter((model) => model.id === removal) as model (model.id)}
      <div class="ai-confirm" role="group" aria-label={`${model.name} 삭제 확인`}>
        <p>{model.ownership === 'external' ? '등록된 참조만 해제합니다. 외부 원본 파일은 삭제하지 않습니다.' : '모델 파일을 삭제할까요? 공지와 기존 AI 분석은 유지됩니다.'}</p>
        {#if model.ownership === 'managed'}<p class="footnote">폴더에 접근할 수 없거나 다른 위치에 다시 설치하려면 파일을 남기고 목록에서 제거하세요.</p>{/if}
        <div class="popover-actions">
          <button class="button secondary small" disabled={busy || data.busy} onclick={async () => { await action('ai_remove', { id: model.id, confirmed: true }, '모델 등록을 정리했습니다. 기존 분석은 유지됩니다.'); removal = null; }}>확인하고 {model.ownership === 'external' ? '참조 해제' : '삭제'}</button>
          {#if model.ownership === 'managed'}<button class="text-button" disabled={busy || data.busy} onclick={async () => { await action('ai_remove', { id: model.id, confirmed: true, keepFiles: true }, '파일을 남기고 모델을 목록에서 제거했습니다. 기존 분석은 유지됩니다.'); removal = null; }}>파일을 남기고 목록에서 제거</button>{/if}
          <button class="text-button" onclick={() => removal = null}>취소</button>
        </div>
      </div>
    {/each}

    <h3 class="settings-group-title">분석</h3>
    <div class="settings-group">
      <div class="settings-row">
        <span class="row-label"><label for="ai-processing-mode">분석 방식</label><Hint label="분석 방식">직접 요청은 공지에서 분석을 누를 때만 실행합니다. 최근 공지 소량 분석은 앱을 쓰는 동안 새로 보관되거나 바뀐 공지를 상한만큼만 분석합니다. 과거 자료와 이미 시도한 공지는 제외하며, 절전 모드나 앱 종료 후에는 분석하지 않습니다.</Hint></span>
        <select id="ai-processing-mode" value={data.preferences.processing_mode} disabled={controlsBusy} onchange={(event) => preference({ processing_mode: event.currentTarget.value as AiPreferences['processing_mode'] })}><option value="explicit">직접 요청한 공지만</option><option value="recent">최근 공지 소량 분석</option></select>
      </div>
      {#if data.preferences.processing_mode === 'recent'}
        <div class="settings-row"><label class="row-label" for="ai-automatic-limit">한 번에 자동 분석할 공지</label><input id="ai-automatic-limit" class="number-field" type="number" min="0" max="20" value={data.preferences.automatic_limit} disabled={controlsBusy} onchange={(event) => preference({ automatic_limit: Number(event.currentTarget.value) })} /></div>
      {/if}
    </div>

    {#if data.jobs.length}
      <h3 class="settings-group-title">분석 대기열</h3>
      <ul class="settings-group ai-queue" aria-label="분석 대기열">{#each data.jobs as job}<li class="settings-row"><span class="row-label">공지 #{job.document_id}<span class="row-detail">{job.status === 'running' ? '분석 중' : job.status === 'pending' ? '대기 중' : job.status === 'completed' ? '완료' : job.status === 'cancelled' ? '취소됨' : job.status === 'stale' ? '이전 버전' : '분석 실패'}{#if job.error} · {job.error}{/if}</span></span>{#if ['pending', 'running'].includes(job.status)}<button class="text-button" disabled={cancelling} onclick={() => interrupt('ai_cancel', { id: job.id }, '분석 취소를 요청했습니다.')}>분석 취소</button>{/if}</li>{/each}</ul>
    {/if}

    <details class="advanced" bind:open={advancedOpen}>
      <summary>고급</summary>
      <div class="settings-group">
        <div class="settings-row"><span class="row-label">이 기기<Hint label="이 기기">물리 메모리는 지금 쓸 수 있는 메모리와 다릅니다. 다른 앱 사용량에 따라 모델을 불러오지 못할 수 있으니 처음에는 2B가 가볍습니다. 추론 백엔드: {data.hardware.backend}. NVIDIA GPU는 Vulkan으로 실행하며 모든 기종에서 검증된 것은 아닙니다.</Hint><span class="row-detail">{data.hardware.os} · {data.hardware.arch} · 물리 메모리 {data.hardware.ram_bytes ? archiveSize(data.hardware.ram_bytes) : '확인하지 못함'} · 현재 남은 저장 공간 {data.hardware.disk_available_bytes ? archiveSize(data.hardware.disk_available_bytes) : '확인하지 못함'}</span></span></div>
        {#if isWindows}
          <div class="settings-row">
            <span class="row-label"><label for="ai-execution-mode">AI 실행 장치</label><Hint label="AI 실행 장치">바꾸면 불러온 모델을 해제하고 다음 분석부터 적용합니다. GPU 전용은 GPU를 쓸 수 없을 때 오류를 알립니다. 실제 실행 장치와 CPU 전환 이유는 AI 엔진 상태에 표시됩니다.</Hint></span>
            <select id="ai-execution-mode" bind:value={executionMode} disabled={busy || data.busy} onchange={(event) => preference({ execution_mode: event.currentTarget.value as AiPreferences['execution_mode'] })}>
              <option value="auto">자동 · NVIDIA GPU 우선, 사용할 수 없으면 CPU</option>
              <option value="cpu">CPU만 사용</option>
              <option value="gpu">NVIDIA GPU만 사용</option>
            </select>
          </div>
        {/if}
        <div class="settings-row"><span class="row-label">AI 엔진 다시 시작</span><button class="button secondary small" disabled={busy || data.busy} onclick={() => action('ai_restart', {}, 'AI 엔진을 다시 시작했습니다.')}>다시 시작</button></div>
        <div class="settings-row"><span class="row-label"><label for="ai-max-tokens">최대 출력 토큰</label><Hint label="최대 출력 토큰">새 분석부터 적용되며 기존 결과는 그대로 둡니다.</Hint></span><input id="ai-max-tokens" class="number-field" type="number" min="128" max="4096" step="128" value={data.preferences.max_tokens} disabled={controlsBusy} onchange={(event) => preference({ max_tokens: Number(event.currentTarget.value) })} /></div>
      </div>

      <h3 class="settings-group-title">모델 설치 폴더</h3>
      <div class="settings-group" role="group" aria-label="모델 설치 폴더">
        <div class="settings-row stacked">
          <span class="row-label">{data.model_storage.custom ? '선택한 위치' : '기본 위치'}<Hint label="모델 설치 폴더">새로 설치하는 모델에만 적용됩니다. 이미 설치한 모델은 옮기지 않고, 중단한 다운로드는 원래 폴더에서 이어받습니다. 기본 위치로 되돌려도 파일을 옮기거나 지우지 않습니다.</Hint><span class="row-detail">남은 공간 {data.model_storage.available_bytes === null ? '확인하지 못함' : archiveSize(data.model_storage.available_bytes)}</span></span>
          <code class="path">{data.model_storage.directory}</code>
          {#if data.model_storage.error}<p class="archive-error" role="alert">{data.model_storage.error}</p>{/if}
          <div class="popover-actions">
            <button class="button secondary small" disabled={controlsBusy || data.busy} onclick={chooseModelDirectory}>설치 폴더 선택</button>
            <button class="text-button" disabled={controlsBusy || data.busy || !data.model_storage.custom} onclick={() => action('ai_reset_model_directory', {}, '새 모델의 설치 폴더를 기본 위치로 되돌렸습니다. 기존 모델은 옮기지 않습니다.')}>기본 위치로 되돌리기</button>
          </div>
        </div>
      </div>

      <h3 class="settings-group-title">다른 모델</h3>
      <div class="settings-group">
        <div class="settings-row stacked">
          <span class="row-label"><label for="local-model-path">이미 가진 GGUF 파일 등록</label><Hint label="GGUF 파일 등록">파일을 복사하지 않고 그 위치를 참조합니다. 참조를 해제해도 원본 파일은 지우지 않습니다.</Hint></span>
          <div class="field-with-button"><input id="local-model-path" type="text" bind:value={path} placeholder={isWindows ? 'C:\\모델\\model.gguf' : '/전체/경로/model.gguf'} spellcheck="false" /><button class="button secondary small" disabled={busy || data.busy || !path.trim()} aria-label="외부 파일 참조로 등록" onclick={() => action('ai_register', { path: path.trim() }, '로컬 모델을 등록했습니다. 선택한 뒤 불러와 호환성을 확인하세요.')}>등록</button></div>
        </div>
      </div>
    </details>
  {/if}
</div>
