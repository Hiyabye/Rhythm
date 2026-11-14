<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { AiModel, AiPreferences, AiSnapshot } from './types';
  import { archiveError, archiveSize } from './archive-format';
  import { isWindows } from './platform';
  import Hint from './Hint.svelte';
  import Progress from './Progress.svelte';
  import { fade } from './motion';

  let { desktop }: { desktop: boolean } = $props();
  let data = $state<AiSnapshot | null>(null);
  let busy = $state(false);
  let error = $state('');
  let fetchError = $state('');
  let cancelling = $state(false);
  let controlsBusy = $derived(busy || Boolean(data?.busy));
  let feedback = $state('');
  let removal = $state<string | null>(null);
  let executionMode = $state<AiPreferences['execution_mode']>('auto');
  let advancedOpen = $state(false);
  let disposed = false;
  let refreshing = false;
  const modelStatus: Record<string, string> = { pending: '다운로드 대기', downloading: '내려받는 중', verifying: '무결성 확인 중', installed: '설치됨', failed: '설치 실패', cancelled: '중단됨 · 이어받기 가능', missing: '파일 확인 필요' };
  let embeddingEntry = $derived(data?.catalog.find((entry) => entry.kind === 'embedding') ?? null);
  let embeddingModel = $derived(data?.models.find((model) => model.kind === 'embedding' && model.id === data?.preferences.embedding_model_id)
    ?? data?.models.find((model) => model.kind === 'embedding') ?? null);
  // Summary models are no longer offered (#226); installed copies stay until the user removes them.
  let retiredModels = $derived(data?.models.filter((model) => model.kind !== 'embedding') ?? []);
  let classificationState = $derived(data?.classification?.state ?? 'idle');
  const classificationLabel: Record<string, string> = { idle: '대기 중', classifying: '공지를 분류하는 중', waiting: '분류할 공지를 기다리는 중', disabled: '꺼짐', failed: '분류 모델을 확인하지 못했습니다' };

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
      if (!disposed) { data = next; fetchError = ''; executionMode = next.preferences.execution_mode ?? 'auto'; }
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
    if (controlsBusy) return;
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
</script>

{#snippet removeConfirm(model: AiModel)}
  <div class="ai-confirm" in:fade role="group" aria-label={`${model.name} 삭제 확인`}>
    <p>{model.kind === 'embedding' ? (model.ownership === 'external' ? '분류 모델 참조만 해제합니다. 외부 원본 파일과 공지는 유지되며, 이 모델의 분류 결과는 삭제됩니다.' : '분류 모델 파일을 삭제할까요? 공지는 유지되며, 이 모델의 분류 결과는 삭제됩니다.') : (model.ownership === 'external' ? '등록된 참조만 해제합니다. 외부 원본 파일은 삭제하지 않습니다.' : '모델 파일을 삭제할까요? 공지는 그대로 유지됩니다.')}</p>
    {#if model.ownership === 'managed'}<p class="footnote">폴더에 접근할 수 없으면 파일을 남기고 목록에서 제거하세요.</p>{/if}
    <div class="popover-actions">
      <button class="button secondary small" disabled={controlsBusy} onclick={async () => { await action('ai_remove', { id: model.id, confirmed: true }, model.ownership === 'external' ? '모델 참조를 해제했습니다.' : '모델을 삭제했습니다.'); removal = null; }}>확인하고 {model.ownership === 'external' ? '참조 해제' : '삭제'}</button>
      {#if model.ownership === 'managed'}<button class="text-button" disabled={controlsBusy} onclick={async () => { await action('ai_remove', { id: model.id, confirmed: true, keepFiles: true }, '파일을 남기고 모델을 목록에서 제거했습니다.'); removal = null; }}>파일을 남기고 목록에서 제거</button>{/if}
      <button class="text-button" onclick={() => removal = null}>취소</button>
    </div>
  </div>
{/snippet}

<div id="ai" class="ai-settings" aria-busy={desktop && !data && !fetchError}>
  {#if !desktop}<p class="muted">모델 설치와 공지 분류는 데스크톱 앱에서 사용할 수 있습니다.</p>
  {:else if !data && !fetchError}<p class="loading" role="status">모델 정보를 불러오는 중입니다…</p>{/if}
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  {#if fetchError}<p class="archive-error" role="alert">{fetchError}</p><button class="text-button" onclick={refresh}>모델 정보 다시 확인</button>{/if}
  <p class="archive-feedback" role="status">{feedback}</p>
  {#if data}
    <h3 class="settings-group-title" id="ai-classification">공지 분류</h3>
    <div class="settings-group" role="group" aria-label="공지 분류">
      {#if !embeddingEntry && !embeddingModel}
        <div class="settings-row"><span class="row-label">분류 모델을 제공하지 않는 빌드입니다.</span></div>
      {:else}
        <div class="settings-row stacked">
          <span class="row-label">{embeddingModel?.name ?? embeddingEntry?.name}<Hint label="공지 분류">공지의 제목과 본문을 작은 임베딩 모델로 비교해 8개 분류 중 하나를 고릅니다. 설치한 뒤에는 새로 보관되는 공지와 아직 분류하지 않은 공지를 자동으로 분류하고, 비슷한 공지와 뜻으로 찾는 검색에도 씁니다. 결과가 불확실하면 기타로 표시합니다.</Hint><span class="row-detail">{embeddingModel?.status === 'installed' ? (data.preferences.classification_enabled ? classificationLabel[classificationState] ?? classificationState : '꺼짐') : embeddingModel ? (modelStatus[embeddingModel.status] ?? embeddingModel.status) : '설치하지 않음'}{#if data.classification.processed > 0}{' · 이번 실행에서 '}{data.classification.processed.toLocaleString('ko-KR')}{'건'}{/if}</span></span>
          {#if embeddingModel?.status === 'installed'}
            <p class="row-detail">모델 {archiveSize(embeddingModel.bytes || embeddingEntry?.bytes || 0)} · {embeddingEntry ? `${embeddingEntry.repository} · ${embeddingEntry.license}` : '목록에서 내려간 모델'}</p>
            <div class="settings-row no-separator">
              <span class="row-label"><label for="ai-classification-enabled">새 공지 자동 분류</label><Hint label="자동 분류">모델을 설치·삭제하는 동안에는 기다립니다. 공지 수집은 분류를 기다리지 않습니다. 분류 모델은 1분 넘게 할 일이 없으면 메모리를 반환합니다.</Hint></span>
              <input id="ai-classification-enabled" type="checkbox" checked={data.preferences.classification_enabled} disabled={controlsBusy} onchange={(event) => action('ai_classification_preferences', { enabled: event.currentTarget.checked }, event.currentTarget.checked ? '새 공지 자동 분류를 켰습니다.' : '새 공지 자동 분류를 껐습니다. 기존 분류는 유지됩니다.')} />
            </div>
            {#if removal !== embeddingModel.id}<button class="text-button" disabled={controlsBusy} onclick={() => removal = embeddingModel!.id}>분류 모델 삭제</button>
            {:else}{@render removeConfirm(embeddingModel)}{/if}
            {#if classificationState === 'failed'}<button class="text-button" disabled={controlsBusy} onclick={() => action('ai_classification_preferences', { enabled: data?.preferences.classification_enabled ?? true }, '분류를 다시 준비했습니다.')}>분류 다시 시도</button>{/if}
          {:else if embeddingModel && ['pending', 'downloading'].includes(embeddingModel.status)}
            <label class="ai-progress"><span class="sr-only">분류 모델 다운로드</span><Progress max={embeddingModel.expected_bytes || 1} value={embeddingModel.downloaded_bytes} /><span>{archiveSize(embeddingModel.downloaded_bytes)} / {archiveSize(embeddingModel.expected_bytes ?? 0)}</span></label>
            <button class="button secondary small" disabled={cancelling} onclick={() => interrupt('ai_cancel_download', {}, '분류 모델 다운로드를 중단했습니다. 받은 부분은 이어받기용으로 남겨 둡니다.')}>중단</button>
          {:else if embeddingModel && embeddingModel.status === 'verifying'}
            <p class="row-detail" role="status">무결성 확인 중</p>
          {:else if embeddingEntry}
            <p class="row-detail">다운로드 {archiveSize(embeddingEntry.bytes)} · 예상 메모리 {archiveSize(embeddingEntry.memory_bytes)}(실측 최대치 아님)</p>
            <button class="button small" disabled={controlsBusy} onclick={() => action('ai_install', { catalogId: embeddingEntry!.id }, '분류 모델 다운로드를 시작했습니다. 완료되면 자동으로 선택됩니다.')}>{embeddingModel ? '다시 설치' : '분류 모델 설치'}</button>
          {/if}
          {#if embeddingModel?.error}<p class="archive-error" role="alert">{embeddingModel.error}</p>{/if}
          {#if embeddingModel?.status !== 'installed'}<p class="footnote">AI 요약과 원격 AI 연결은 더 이상 제공하지 않습니다. 원격 분류를 쓰고 있었다면 분류 모델을 설치해야 다시 분류합니다.</p>{/if}
        </div>
      {/if}
    </div>

    {#if retiredModels.length}
      <h3 class="settings-group-title">더 이상 제공하지 않는 모델</h3>
      <ul class="settings-group" aria-label="더 이상 제공하지 않는 모델">
        {#each retiredModels as model (model.id)}
          <li class="settings-row stacked">
            <span class="row-label">{model.name}<span class="row-detail">{model.ownership === 'external' ? '외부 파일' : 'Rhythm이 관리하는 파일'} · {archiveSize(model.bytes || model.downloaded_bytes)}{#if model.status !== 'installed'} · {modelStatus[model.status] ?? model.status}{/if}</span></span>
            <p class="row-detail">AI 요약 기능을 더 이상 제공하지 않아 이 모델은 쓰이지 않습니다. 저장 공간을 비우려면 삭제하세요.</p>
            {#if removal !== model.id}<div class="popover-actions"><button class="text-button" disabled={controlsBusy} onclick={() => removal = model.id}>{model.ownership === 'external' ? '참조 해제' : '모델 삭제'}</button></div>
            {:else}{@render removeConfirm(model)}{/if}
          </li>
        {/each}
      </ul>
    {/if}

    <details class="advanced" bind:open={advancedOpen}>
      <summary>고급</summary>
      <div class="settings-group">
        <div class="settings-row"><span class="row-label">이 기기<Hint label="이 기기">물리 메모리는 지금 쓸 수 있는 메모리와 다릅니다. 다른 앱 사용량에 따라 분류 모델을 불러오지 못할 수 있습니다. 추론 백엔드: {data.hardware.backend}. NVIDIA GPU는 Vulkan으로 실행하며 모든 기종에서 검증된 것은 아닙니다.</Hint><span class="row-detail">{data.hardware.os} · {data.hardware.arch} · 물리 메모리 {data.hardware.ram_bytes ? archiveSize(data.hardware.ram_bytes) : '확인하지 못함'} · 현재 남은 저장 공간 {data.hardware.disk_available_bytes ? archiveSize(data.hardware.disk_available_bytes) : '확인하지 못함'}</span></span></div>
        {#if isWindows}
          <div class="settings-row">
            <span class="row-label"><label for="ai-execution-mode">AI 실행 장치</label><Hint label="AI 실행 장치">바꾸면 실행 중인 분류 모델을 멈추고 다음 분류부터 적용합니다. GPU 전용은 GPU를 쓸 수 없을 때 오류를 알립니다.</Hint></span>
            <select id="ai-execution-mode" bind:value={executionMode} disabled={controlsBusy} onchange={(event) => action('ai_preferences', { preferences: { ...data!.preferences, execution_mode: event.currentTarget.value as AiPreferences['execution_mode'] } }, 'AI 실행 장치를 저장했습니다.')}>
              <option value="auto">자동 · NVIDIA GPU 우선, 사용할 수 없으면 CPU</option>
              <option value="cpu">CPU만 사용</option>
              <option value="gpu">NVIDIA GPU만 사용</option>
            </select>
          </div>
        {/if}
      </div>

      <h3 class="settings-group-title">모델 설치 폴더</h3>
      <div class="settings-group" role="group" aria-label="모델 설치 폴더">
        <div class="settings-row stacked">
          <span class="row-label">{data.model_storage.custom ? '선택한 위치' : '기본 위치'}<Hint label="모델 설치 폴더">새로 설치하는 모델에만 적용됩니다. 이미 설치한 모델은 옮기지 않고, 중단한 다운로드는 원래 폴더에서 이어받습니다. 기본 위치로 되돌려도 파일을 옮기거나 지우지 않습니다.</Hint><span class="row-detail">남은 공간 {data.model_storage.available_bytes === null ? '확인하지 못함' : archiveSize(data.model_storage.available_bytes)}</span></span>
          <code class="path">{data.model_storage.directory}</code>
          {#if data.model_storage.error}<p class="archive-error" role="alert">{data.model_storage.error}</p>{/if}
          <div class="popover-actions">
            <button class="button secondary small" disabled={controlsBusy} onclick={chooseModelDirectory}>설치 폴더 선택</button>
            <button class="text-button" disabled={controlsBusy || !data.model_storage.custom} onclick={() => action('ai_reset_model_directory', {}, '새 모델의 설치 폴더를 기본 위치로 되돌렸습니다. 기존 모델은 옮기지 않습니다.')}>기본 위치로 되돌리기</button>
          </div>
        </div>
      </div>
    </details>
  {/if}
</div>
