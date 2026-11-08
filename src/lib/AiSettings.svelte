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
  let projectorRemoval = $state<string | null>(null);
  let executionMode = $state<AiPreferences['execution_mode']>('auto');
  let remoteBaseUrl = $state('');
  let remoteModel = $state('');
  let remoteEmbeddingModel = $state('');
  let remoteKey = $state('');
  let keyPresent = $state<boolean | null>(null);
  let keyChecked = false;
  let remoteInitialized = false;
  let testing = $state(false);
  let advancedOpen = $state(false);
  let disposed = false;
  let refreshing = false;
  let setupReported = false;
  const operationPhase: Record<string, string> = { verifying: '모델 파일 무결성을 확인하고 있습니다', waiting: 'AI 엔진이 준비되기를 기다리고 있습니다', restarting: 'AI 엔진을 다시 준비하고 있습니다', loading: '모델을 메모리에 불러오고 있습니다', setup: '짧은 로컬 추론으로 설치를 확인하고 있습니다', cancelling: '작업을 중단하고 메모리를 정리하고 있습니다' };
  const operationName: Record<string, string> = { load: '모델 불러오기', setup: 'AI 설정 확인', register: '외부 모델 등록' };
  const modelStatus: Record<string, string> = { pending: '다운로드 대기', downloading: '내려받는 중', verifying: '무결성 확인 중', installed: '설치됨', failed: '설치 실패', cancelled: '중단됨 · 이어받기 가능', missing: '파일 확인 필요' };
  const projectorStatus: Record<string, string> = { pending: '이미지 인식 모듈 다운로드 대기', downloading: '이미지 인식 모듈을 내려받는 중', failed: '이미지 인식 모듈 설치 실패', cancelled: '이미지 인식 모듈 중단됨 · 이어받기 가능', missing: '이미지 인식 모듈 파일 확인 필요' };
  const installable = (entry: CatalogEntry) => ['verified', 'available'].includes(entry.compatibility) && entry.kind !== 'embedding';
  // One tile per offered model, plus every inventory row (external, retired or in progress) that no offered tile covers.
  let tiles = $derived.by(() => {
    if (!data) return [];
    const offered = data.catalog.filter(installable);
    const list: { entry: CatalogEntry | null; model: AiModel | null }[] = offered.map((entry) => ({ entry, model: data!.models.find((model) => model.catalog_id === entry.id) ?? null }));
    for (const model of data.models) {
      if (model.kind === 'embedding') continue;
      if (!offered.some((entry) => entry.id === model.catalog_id)) list.push({ entry: data.catalog.find((entry) => entry.id === model.catalog_id) ?? null, model });
    }
    return list;
  });
  let activeModel = $derived(data?.models.find((model) => model.id === data?.preferences.active_model_id) ?? null);
  let hasInstalled = $derived(data?.models.some((model) => model.kind !== 'embedding' && model.status === 'installed') ?? false);
  // Completing setup loads the selected model and runs a short inference itself; it must not
  // depend on a model that the idle unload may already have released.
  let setupReady = $derived(activeModel?.status === 'installed' && activeModel.kind !== 'embedding');
  let embeddingEntry = $derived(data?.catalog.find((entry) => entry.kind === 'embedding') ?? null);
  let embeddingModel = $derived(data?.models.find((model) => model.kind === 'embedding' && model.id === data?.preferences.embedding_model_id)
    ?? data?.models.find((model) => model.kind === 'embedding') ?? null);
  let classificationState = $derived(data?.classification?.state ?? 'idle');
  const classificationLabel: Record<string, string> = { idle: '대기 중', classifying: '공지를 분류하는 중', waiting: '분류할 공지를 기다리는 중', disabled: '꺼짐', failed: '분류 모델을 확인하지 못했습니다' };

  $effect(() => { if (data?.model_storage.error) advancedOpen = true; });
  $effect(() => {
    if (data && !remoteInitialized) {
      remoteInitialized = true;
      remoteBaseUrl = data.preferences.remote_base_url ?? '';
      remoteModel = data.preferences.remote_model ?? '';
      remoteEmbeddingModel = data.preferences.remote_embedding_model ?? '';
    }
  });
  $effect(() => {
    if (desktop && data?.preferences.engine === 'openai-compatible' && !keyChecked) {
      keyChecked = true;
      void checkKey();
    }
  });

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
        if ((next.preferences.setup_completed || next.preferences.engine === 'openai-compatible') && !setupReported) { setupReported = true; onsetup?.(); }
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
  async function checkKey() {
    if (!desktop || disposed) return;
    try {
      const value = await invoke<boolean>('ai_remote_status');
      if (!disposed) keyPresent = value;
    } catch { if (!disposed) keyPresent = null; }
  }
  async function saveRemoteKey() {
    if (controlsBusy || !remoteKey.trim()) return;
    busy = true; error = ''; feedback = '';
    try {
      await invoke('ai_remote_key', { key: remoteKey });
      if (!disposed) { remoteKey = ''; keyPresent = true; feedback = 'API 키를 운영체제 보안 저장소에 저장했습니다.'; await refresh(); }
    } catch (reason) { if (!disposed) { error = archiveError(reason); await refresh(); } }
    finally { if (!disposed) busy = false; }
  }
  async function clearRemoteKey() {
    if (controlsBusy) return;
    busy = true; error = ''; feedback = '';
    try {
      await invoke('ai_remote_key_clear');
      if (!disposed) { keyPresent = false; feedback = '저장된 API 키를 삭제했습니다.'; await refresh(); }
    } catch (reason) { if (!disposed) { error = archiveError(reason); await refresh(); } }
    finally { if (!disposed) busy = false; }
  }
  async function testRemote() {
    if (controlsBusy || testing) return;
    testing = true; error = ''; feedback = '';
    try {
      const message = await invoke<string>('ai_remote_test');
      if (!disposed) { feedback = message; keyPresent = true; }
    } catch (reason) { if (!disposed) error = archiveError(reason); }
    finally { if (!disposed) testing = false; }
  }
  async function saveRemoteBase() {
    await preference({ remote_base_url: remoteBaseUrl.trim() || null }, 'API 주소를 저장했습니다. 주소가 바뀌면 다시 동의해야 합니다.');
    if (!disposed) {
      remoteBaseUrl = data?.preferences.remote_base_url ?? remoteBaseUrl.trim();
      keyPresent = data?.remote_key_present ?? null;
      keyChecked = false;
    }
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
  async function preference(patch: Partial<Omit<AiPreferences, 'model_directory'>>, success = 'AI 설정을 저장했습니다.') {
    if (!data) return;
    await action('ai_preferences', { preferences: { ...data.preferences, ...patch } }, success);
  }
  function engineLabel(snapshot: AiSnapshot) {
    const state = snapshot.runtime.state;
    return state === 'busy' ? '로컬 AI 작업 중' : state === 'loading' ? '모델을 불러오는 중' : state === 'ready' ? (snapshot.runtime.model ? '모델 준비됨' : '모델 없이 대기 중') : '엔진 확인 필요';
  }
  function compatibilityLabel(entry: CatalogEntry) {
    return entry.compatibility === 'available' ? '설치 가능 · Windows CPU 검증 범위 확인' : entry.compatibility === 'verified' ? '일부 환경에서 실제 추론 확인' : entry.compatibility === 'experimental' ? '실험적 · 추론 미검증' : '현재 지원하지 않음';
  }
</script>

{#snippet removeConfirm(model: AiModel)}
  <div class="ai-confirm" role="group" aria-label={`${model.name} 삭제 확인`}>
    <p>{model.kind === 'embedding' ? (model.ownership === 'external' ? '분류 모델 참조만 해제합니다. 외부 원본 파일과 공지·요약 결과는 유지되며, 이 모델의 분류 결과는 삭제됩니다.' : '분류 모델 파일을 삭제할까요? 공지와 요약 결과는 유지되며, 이 모델의 분류 결과는 삭제됩니다.') : (model.ownership === 'external' ? '등록된 참조만 해제합니다. 외부 원본 파일은 삭제하지 않습니다.' : '모델 파일을 삭제할까요? 공지와 기존 AI 분석은 유지됩니다.')}</p>
    {#if model.ownership === 'managed'}<p class="footnote">폴더에 접근할 수 없거나 다른 위치에 다시 설치하려면 파일을 남기고 목록에서 제거하세요.</p>{/if}
    <div class="popover-actions">
      <button class="button secondary small" disabled={busy || data?.busy} onclick={async () => { await action('ai_remove', { id: model.id, confirmed: true }, '모델 등록을 정리했습니다. 기존 분석은 유지됩니다.'); removal = null; }}>확인하고 {model.ownership === 'external' ? '참조 해제' : '삭제'}</button>
      {#if model.ownership === 'managed'}<button class="text-button" disabled={busy || data?.busy} onclick={async () => { await action('ai_remove', { id: model.id, confirmed: true, keepFiles: true }, '파일을 남기고 모델을 목록에서 제거했습니다. 기존 분석은 유지됩니다.'); removal = null; }}>파일을 남기고 목록에서 제거</button>{/if}
      <button class="text-button" onclick={() => removal = null}>취소</button>
    </div>
  </div>
{/snippet}

<div id="ai" class="ai-settings" aria-busy={desktop && !data && !fetchError}>
  {#if !desktop}<p class="muted">모델 설치와 로컬 추론은 데스크톱 앱에서 사용할 수 있습니다.</p>
  {:else if !data && !fetchError}<p class="loading" role="status">모델 정보를 불러오는 중입니다…</p>{/if}
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  {#if fetchError}<p class="archive-error" role="alert">{fetchError}</p><button class="text-button" onclick={refresh}>모델 정보 다시 확인</button>{/if}
  <p class="archive-feedback" role="status">{feedback}</p>
  {#if data}
    <h3 class="settings-group-title">AI 엔진 선택</h3>
    <div class="settings-group" role="group" aria-label="AI 엔진 선택">
      <div class="settings-row">
        <span class="row-label"><label for="ai-engine">추론 엔진</label><Hint label="추론 엔진">로컬은 앱에 포함된 llama.cpp를 이 기기에서 실행합니다. OpenAI 호환 API는 사용자가 지정한 외부 주소로 공지 제목과 본문을 보내 분석합니다. 로컬이 기본이며 언제든 되돌릴 수 있습니다.</Hint><span class="row-detail">{data.preferences.engine === 'openai-compatible' ? `원격 · ${data.remote_host ?? '주소 미설정'}` : '이 기기에서 실행'}</span></span>
        <select id="ai-engine" value={data.preferences.engine} disabled={controlsBusy} onchange={(event) => preference({ engine: event.currentTarget.value as AiPreferences['engine'] }, event.currentTarget.value === 'openai-compatible' ? '원격 AI 엔진을 선택했습니다. 주소와 모델을 설정하고 동의해 주세요.' : '로컬 AI 엔진으로 되돌렸습니다.')}>
          <option value="local">로컬 (기본)</option>
          <option value="openai-compatible">OpenAI 호환 API</option>
        </select>
      </div>
      {#if data.preferences.engine === 'openai-compatible'}
        <div class="settings-row stacked">
          <span class="row-label"><label for="ai-remote-base">API 주소</label><Hint label="API 주소">OpenAI, OpenRouter, Groq, Ollama, LM Studio, vLLM처럼 OpenAI 호환 API를 제공하는 주소입니다. HTTPS만 사용하며, 로컬 서버는 http://127.0.0.1, http://[::1], http://localhost만 허용합니다. 리디렉션과 프록시는 따르지 않습니다.</Hint></span>
          <div class="field-with-button"><input id="ai-remote-base" type="text" bind:value={remoteBaseUrl} placeholder="https://api.example.com/v1" spellcheck="false" disabled={controlsBusy} /><button class="button secondary small" disabled={controlsBusy} onclick={saveRemoteBase}>주소 저장</button></div>
          {#if data.remote_host}<p class="row-detail">현재 연결 대상: {data.remote_host}</p>{/if}
        </div>
        <div class="settings-row">
          <span class="row-label"><label for="ai-remote-model">모델 이름</label><Hint label="모델 이름">제공자 콘솔에 표시되는 모델 ID를 입력합니다. 기본값을 미리 채우지 않습니다.</Hint></span>
          <div class="field-with-button"><input id="ai-remote-model" type="text" bind:value={remoteModel} placeholder="모델 ID" spellcheck="false" disabled={controlsBusy} onchange={() => preference({ remote_model: remoteModel.trim() || null }, '원격 모델 이름을 저장했습니다.')} /></div>
        </div>
        <div class="settings-row">
          <span class="row-label"><label for="ai-remote-timeout">요청 제한 시간(초)</label><Hint label="요청 제한 시간">10초에서 600초 사이입니다. 시간 초과 시 요청을 중단하며 자동으로 다시 보내지 않습니다.</Hint></span>
          <input id="ai-remote-timeout" class="number-field" type="number" min="10" max="600" value={data.preferences.remote_timeout_seconds} disabled={controlsBusy} onchange={(event) => preference({ remote_timeout_seconds: Number(event.currentTarget.value) }, '요청 제한 시간을 저장했습니다.')} />
        </div>
        <div class="settings-row stacked">
          <span class="row-label">API 키<Hint label="API 키">운영체제 보안 저장소(Keychain 또는 Credential Manager)에만 저장합니다. Rhythm 데이터베이스, 백업, 진단에 포함되지 않으며 화면에 다시 표시하지 않습니다.</Hint><span class="row-detail">{keyPresent === true ? '저장된 키 사용 중' : keyPresent === false ? '키가 저장되어 있지 않습니다' : '키 상태 확인 필요'}</span></span>
          <div class="field-with-button"><input id="ai-remote-key" aria-label="API 키" type="password" bind:value={remoteKey} placeholder={keyPresent ? '새 키를 입력해 교체' : 'API 키 입력'} spellcheck="false" autocomplete="off" disabled={controlsBusy} /><button class="button secondary small" disabled={controlsBusy || !remoteKey.trim()} onclick={saveRemoteKey}>키 저장</button></div>
          <div class="popover-actions">
            <button class="text-button" disabled={controlsBusy || keyPresent !== true} onclick={clearRemoteKey}>저장된 키 삭제</button>
            <button class="text-button" disabled={controlsBusy || testing || !data.preferences.remote_consent || !data.preferences.remote_model} onclick={testRemote}>{testing ? '확인하는 중…' : '연결 확인'}</button>
          </div>
          <p class="row-detail">연결 확인은 공지 내용을 보내지 않고 짧은 시험 요청 한 번만 보냅니다.</p>
        </div>
        {#if !data.preferences.remote_consent}
          <div class="ai-confirm" role="group" aria-label="원격 AI 사용 동의">
            <p><strong>{data.remote_host ?? '입력한 API 주소'}</strong>로 공지 제목·본문과 분석 지시문을 보냅니다. 관심사, 프로필, 피드백, 저장·읽음 상태, 보관함 식별자, 기기 정보는 보내지 않습니다. 보낸 내용에는 제공자의 데이터 정책이 적용됩니다.</p>
            <p class="footnote">주소를 바꾸면 동의가 초기화됩니다. 원격 전송을 원하지 않으면 로컬 엔진을 사용하세요.</p>
            <div class="popover-actions"><button class="button secondary small" disabled={controlsBusy || !data.preferences.remote_base_url || !data.preferences.remote_model} onclick={() => preference({ remote_consent: true }, '원격 AI 사용에 동의했습니다.')}>동의하고 사용</button></div>
          </div>
        {:else}
          <div class="settings-row"><span class="row-label">원격 전송에 동의함<span class="row-detail">API 주소를 바꾸면 다시 동의해야 합니다.</span></span></div>
        {/if}
        <div class="settings-row">
          <span class="row-label"><label for="ai-remote-automatic">자동 원격 분석</label><Hint label="자동 원격 분석">최근 공지 소량 분석이나 쉬는 동안 전체 분석이 원격 API를 호출하면 비용이 발생할 수 있습니다. 그래서 별도로 켜야 하며 하루 한도가 있습니다. 직접 요청한 분석은 한도에 포함되지 않습니다.</Hint></span>
          <input id="ai-remote-automatic" type="checkbox" checked={data.preferences.remote_automatic} disabled={controlsBusy || !data.preferences.remote_consent || keyPresent !== true || !data.preferences.remote_model} onchange={(event) => preference({ remote_automatic: event.currentTarget.checked }, event.currentTarget.checked ? '자동 원격 분석을 켰습니다. 하루 한도까지 실행합니다.' : '자동 원격 분석을 껐습니다. 직접 요청만 원격으로 실행합니다.')} />
        </div>
        {#if data.preferences.remote_automatic}
          <div class="settings-row"><label class="row-label" for="ai-remote-limit">하루 자동 분석 한도</label><input id="ai-remote-limit" class="number-field" type="number" min="1" max="200" value={data.preferences.remote_daily_limit} disabled={controlsBusy} onchange={(event) => preference({ remote_daily_limit: Number(event.currentTarget.value) }, '하루 자동 분석 한도를 저장했습니다.')} /></div>
          <p class="row-detail">오늘 자동 시도 {data.remote_usage_today ?? 0}회 / 최대 {data.preferences.remote_daily_limit}회. 직접 요청은 포함되지 않습니다.</p>
        {/if}
        <div class="settings-row">
          <span class="row-label"><label for="ai-remote-embeddings">원격 공지 분류</label><Hint label="원격 공지 분류">공지 제목과 본문 앞부분을 원격 임베딩 API로 보내 분류합니다. 자동으로 실행되며, 끄면 설치한 로컬 분류 모델을 사용합니다. 분류 결과는 추천 순위나 알림에 사용하지 않습니다.</Hint></span>
          <input id="ai-remote-embeddings" type="checkbox" checked={data.preferences.remote_embeddings} disabled={controlsBusy || !data.preferences.remote_consent || keyPresent !== true} onchange={(event) => preference({ remote_embeddings: event.currentTarget.checked }, event.currentTarget.checked ? '원격 공지 분류를 켰습니다.' : '원격 공지 분류를 껐습니다. 로컬 분류 모델을 사용합니다.')} />
        </div>
        {#if data.preferences.remote_embeddings}
          <div class="settings-row stacked">
            <span class="row-label"><label for="ai-remote-embedding-model">원격 분류 모델 이름</label><Hint label="원격 분류 모델">임베딩 API의 모델 ID입니다. 요약 모델과 다를 수 있습니다.</Hint></span>
            <input id="ai-remote-embedding-model" type="text" bind:value={remoteEmbeddingModel} placeholder="임베딩 모델 ID" spellcheck="false" disabled={controlsBusy} onchange={() => preference({ remote_embedding_model: remoteEmbeddingModel.trim() || null }, '원격 분류 모델 이름을 저장했습니다.')} />
          </div>
          <p class="row-detail">원격 분류는 새 공지와 아직 분류하지 않은 공지를 자동으로 처리합니다. 원격 전송을 원하지 않으면 이 스위치를 끄세요.</p>
        {/if}
      {/if}
    </div>

    {#if data.preferences.engine === 'local' && !data.preferences.setup_completed}
      <div class="ai-setup">
        <div class="setup-title"><h3>로컬 AI 시작하기</h3><Hint label="로컬 AI 시작하기">모델 설치는 직접 누를 때만 시작합니다. 마지막 단계에서 짧은 로컬 추론으로 실제 동작을 확인합니다. 준비하지 않아도 보관한 공지는 읽고 검색할 수 있습니다.</Hint></div>
        <ol class="setup-steps">
          <li class:done={hasInstalled}><span aria-hidden="true">{#if hasInstalled}<Icon name="check" size={12} />{:else}1{/if}</span>모델 설치</li>
          <li class:done={setupReady}><span aria-hidden="true">{#if setupReady}<Icon name="check" size={12} />{:else}2{/if}</span>모델 선택</li>
          <li><span aria-hidden="true">3</span>동작 확인</li>
        </ol>
        <button class="button small" disabled={busy || data.busy || !setupReady} aria-label="실제 추론 확인 후 AI 설정 완료" onclick={() => preference({ setup_completed: true })}>동작 확인하고 완료</button>
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
        {@const projectorSpec = entry?.projector ?? null}
        {@const projector = projectorSpec && model ? data.projectors.find((row) => row.model_id === model.id) ?? null : null}
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
          {#if model && projectorSpec && model.status === 'installed' && model.ownership !== 'external'}
            {#if projector && ['pending', 'downloading'].includes(projector.status)}
              <p class="model-note" role="status">{projectorStatus[projector.status]}</p>
              <label class="ai-progress"><span class="sr-only">이미지 인식 모듈 다운로드</span><progress max={projector.expected_bytes || 1} value={projector.downloaded_bytes}></progress><span>{archiveSize(projector.downloaded_bytes)} / {archiveSize(projector.expected_bytes)}</span></label>
            {:else if projector?.status === 'verifying'}<p class="model-note" role="status">이미지 인식 모듈 확인 중</p>
            {:else if projector?.status === 'installed'}<p class="model-note">이미지 인식 사용 중 · 예상 메모리 {archiveSize(projectorSpec.memory_bytes)}(실측 최대치 아님)</p>
            {:else if projector}<p class="model-note">{projectorStatus[projector.status] ?? projector.status}</p>{#if projector.error}<p class="archive-error" role="alert">{projector.error}</p>{/if}{/if}
          {/if}
          <div class="model-actions">
            {#if !model && entry}<button class="button small" disabled={busy || data.busy} aria-label={`${entry.name} 다운로드 및 설치`} onclick={() => action('ai_install', { catalogId: entry.id }, '모델 다운로드를 시작했습니다. 앱을 닫아도 다음 실행에서 이어받습니다.')}>설치</button>
            {:else if model && ['downloading', 'pending', 'verifying'].includes(model.status)}{#if model.status !== 'verifying'}<button class="button secondary small" disabled={cancelling} aria-label="다운로드 중단" onclick={() => interrupt('ai_cancel_download', {}, '다운로드를 중단했습니다. 받은 부분은 이어받기용으로 남겨 둡니다.')}>중단</button>{/if}
            {:else if model && model.status !== 'installed' && entry && !retired}<button class="button small" disabled={busy || data.busy} aria-label={`${entry.name} 설치 다시 시도 / 이어받기`} onclick={() => action('ai_install', { catalogId: entry.id }, '모델 다운로드를 다시 시작했습니다.')}>이어받기</button>{/if}
            {#if model && removal !== model.id && !['downloading', 'pending', 'verifying'].includes(model.status)}<button class="text-button" disabled={busy || data.busy} onclick={() => removal = model.id}>{model.ownership === 'external' ? '참조 해제' : '모델 삭제'}</button>{/if}
            <button class="row-icon model-info" popovertarget={infoId} aria-label={`${name} 정보`}><Icon name="info" size={15} /></button>
          </div>
          {#if model && removal === model.id}{@render removeConfirm(model)}{/if}
          {#if model && projectorSpec && model.status === 'installed' && model.ownership !== 'external'}
            <div class="model-actions model-projector">
              {#if !projector}<button class="button secondary small" disabled={busy || data.busy} aria-label={`${name} 이미지 인식 추가`} onclick={() => action('ai_install_projector', { modelId: model.id }, '이미지 인식 모듈 다운로드를 시작했습니다.')}>이미지 인식 추가 ({archiveSize(projectorSpec.bytes)})</button>
              {:else if ['pending', 'downloading'].includes(projector.status)}<button class="button secondary small" disabled={cancelling} aria-label="이미지 인식 모듈 다운로드 중단" onclick={() => interrupt('ai_cancel_download', {}, '이미지 인식 모듈 다운로드를 중단했습니다. 받은 부분은 이어받기용으로 남겨 둡니다.')}>중단</button>
              {:else if projector.status === 'verifying'}{:else if projector.status === 'installed'}{#if projectorRemoval !== model.id}<button class="text-button" disabled={busy || data.busy} onclick={() => projectorRemoval = model.id}>이미지 인식 제거</button>{/if}
              {:else}<button class="button small" disabled={busy || data.busy} aria-label={`${name} 이미지 인식 다시 설치`} onclick={() => action('ai_install_projector', { modelId: model.id }, '이미지 인식 모듈 다운로드를 다시 시작했습니다.')}>다시 설치</button>{/if}
            </div>
            {#if projectorRemoval === model.id}
              <div class="ai-confirm" role="group" aria-label={`${model.name} 이미지 인식 모듈 제거 확인`}>
                <p>이미지 인식 모듈 파일을 삭제할까요? 모델과 기존 분석은 유지되며, 이후 분석은 텍스트만 읽습니다.</p>
                <div class="popover-actions">
                  <button class="button secondary small" disabled={busy || data.busy} onclick={async () => { await action('ai_remove_projector', { modelId: model.id, confirmed: true }, '이미지 인식 모듈을 제거했습니다. 이후 분석은 텍스트만 읽습니다.'); projectorRemoval = null; }}>확인하고 제거</button>
                  <button class="text-button" onclick={() => projectorRemoval = null}>취소</button>
                </div>
              </div>
            {/if}
          {/if}
          <Popover id={infoId} label={`${name} 정보`} wide>
            <p class="popover-title">{name}</p>
            {#if entry}<p>{entry.description} · {compatibilityLabel(entry)}</p><p>다운로드 {archiveSize(entry.bytes)} · 예상 메모리 {archiveSize(entry.memory_bytes)}(실측 최대치 아님) · {entry.projector ? '이미지 인식 모듈을 설치하면 보관된 이미지도 함께 읽습니다. 설치하지 않으면 텍스트만 분석합니다.' : '텍스트 분석 전용'}</p>{#if entry.projector}<p>이미지 인식 모듈 {archiveSize(entry.projector.bytes)} · 예상 추가 메모리 {archiveSize(entry.projector.memory_bytes)}(실측 최대치 아님)</p>{/if}{#if entry.reason}<p>{entry.reason}</p>{/if}{/if}
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

    <h3 class="settings-group-title" id="ai-classification">공지 분류</h3>
    <div class="settings-group" role="group" aria-label="공지 분류">
      {#if data.preferences.engine === 'openai-compatible' && data.preferences.remote_embeddings}
        <div class="settings-row stacked"><span class="row-label">원격 분류 사용 중<Hint label="원격 분류">공지 제목과 본문 앞부분을 원격 임베딩 API로 보냅니다. 아래 로컬 분류 모델은 사용하지 않으며, 위 AI 엔진 선택에서 원격 분류를 끄면 다시 사용합니다.</Hint><span class="row-detail">{data.preferences.remote_embedding_model ?? '분류 모델 이름을 입력해 주세요.'}{#if data.classification.detail} · {data.classification.detail}{/if}</span></span></div>
      {:else if !embeddingEntry && !embeddingModel}
        <div class="settings-row"><span class="row-label">분류 모델을 제공하지 않는 빌드입니다.</span></div>
      {:else}
        <div class="settings-row stacked">
          <span class="row-label">{embeddingModel?.name ?? embeddingEntry?.name}<Hint label="공지 분류">공지의 제목과 본문을 작은 임베딩 모델로 비교해 8개 분류 중 하나를 고릅니다. 요약 모델과 별도로 실행되며, 설치한 뒤에는 새로 보관되는 공지와 아직 분류하지 않은 공지를 자동으로 분류합니다. 결과가 불확실하면 기타로 표시합니다.</Hint><span class="row-detail">{embeddingModel?.status === 'installed' ? (data.preferences.classification_enabled ? classificationLabel[classificationState] ?? classificationState : '꺼짐') : embeddingModel ? (modelStatus[embeddingModel.status] ?? embeddingModel.status) : '설치하지 않음'}{#if data.classification.processed > 0}{' · 이번 실행에서 '}{data.classification.processed.toLocaleString('ko-KR')}{'건'}{/if}</span></span>
          {#if embeddingModel?.status === 'installed'}
            <p class="row-detail">모델 {archiveSize(embeddingModel.bytes || embeddingEntry?.bytes || 0)} · {embeddingEntry ? `${embeddingEntry.repository} · ${embeddingEntry.license}` : '목록에서 내려간 모델'}</p>
            <div class="settings-row no-separator">
              <span class="row-label"><label for="ai-classification-enabled">새 공지 자동 분류</label><Hint label="자동 분류">요약 모델이 분석 중이거나 모델을 설치·삭제하는 동안에는 기다립니다. 공지 수집은 분류를 기다리지 않습니다. 분류 모델은 1분 넘게 할 일이 없으면 메모리를 반환합니다.</Hint></span>
              <input id="ai-classification-enabled" type="checkbox" checked={data.preferences.classification_enabled} disabled={controlsBusy} onchange={(event) => action('ai_classification_preferences', { enabled: event.currentTarget.checked }, event.currentTarget.checked ? '새 공지 자동 분류를 켰습니다.' : '새 공지 자동 분류를 껐습니다. 기존 분류는 유지됩니다.')} />
            </div>
            {#if removal !== embeddingModel.id}<button class="text-button" disabled={busy || data.busy} onclick={() => removal = embeddingModel!.id}>분류 모델 삭제</button>
            {:else}{@render removeConfirm(embeddingModel)}{/if}
            {#if classificationState === 'failed'}<button class="text-button" disabled={busy || data.busy} onclick={() => action('ai_classification_preferences', { enabled: data?.preferences.classification_enabled ?? true }, '분류를 다시 준비했습니다.')}>분류 다시 시도</button>{/if}
          {:else if embeddingModel && ['pending', 'downloading'].includes(embeddingModel.status)}
            <label class="ai-progress"><span class="sr-only">분류 모델 다운로드</span><progress max={embeddingModel.expected_bytes || 1} value={embeddingModel.downloaded_bytes}></progress><span>{archiveSize(embeddingModel.downloaded_bytes)} / {archiveSize(embeddingModel.expected_bytes ?? 0)}</span></label>
            <button class="button secondary small" disabled={cancelling} onclick={() => interrupt('ai_cancel_download', {}, '분류 모델 다운로드를 중단했습니다. 받은 부분은 이어받기용으로 남겨 둡니다.')}>중단</button>
          {:else if embeddingModel && embeddingModel.status === 'verifying'}
            <p class="row-detail" role="status">무결성 확인 중</p>
          {:else if embeddingEntry}
            <p class="row-detail">다운로드 {archiveSize(embeddingEntry.bytes)} · 예상 메모리 {archiveSize(embeddingEntry.memory_bytes)}(실측 최대치 아님)</p>
            <button class="button small" disabled={busy || data.busy} onclick={() => action('ai_install', { catalogId: embeddingEntry!.id }, '분류 모델 다운로드를 시작했습니다. 완료되면 자동으로 선택됩니다.')}>{embeddingModel ? '다시 설치' : '분류 모델 설치'}</button>
          {/if}
          {#if embeddingModel?.error}<p class="archive-error" role="alert">{embeddingModel.error}</p>{/if}
        </div>
      {/if}
    </div>

    <h3 class="settings-group-title">분석</h3>
    <div class="settings-group">
      <div class="settings-row">
        <span class="row-label"><label for="ai-processing-mode">분석 방식</label><Hint label="분석 방식">직접 요청은 공지에서 분석을 누를 때만 실행합니다. 최근 공지 소량 분석은 앱을 쓰는 동안 새로 보관되거나 바뀐 공지를 상한만큼만 분석합니다. 쉬는 동안 전체 공지 분석은 앱을 쓰는 동안에는 자동으로 분석하지 않고, 컴퓨터를 5분 이상 쓰지 않을 때 아직 분석하지 않은 공지를 최신 게시일부터 시간당 20건까지 분석합니다. 절전 모드, 배터리 사용 중, 앱 종료 후에는 분석하지 않으며 이미 시도한 공지는 다시 시도하지 않습니다.</Hint></span>
        <select id="ai-processing-mode" value={data.preferences.processing_mode} disabled={controlsBusy} onchange={(event) => preference({ processing_mode: event.currentTarget.value as AiPreferences['processing_mode'] })}><option value="explicit">직접 요청한 공지만</option><option value="recent">최근 공지 소량 분석</option><option value="backlog">쉬는 동안 전체 공지 분석</option></select>
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
