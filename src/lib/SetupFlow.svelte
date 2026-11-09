<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import Icon from './Icon.svelte';
  import Hint from './Hint.svelte';
  import SourceSettings from './SourceSettings.svelte';
  import PersonalizationSettings from './PersonalizationSettings.svelte';
  import ProactiveSettings from './ProactiveSettings.svelte';
  import ExtensionSettings from './ExtensionSettings.svelte';
  import Progress from './Progress.svelte';
  import { fade, fly } from './motion';
  import { contributed, enabledExtensions } from './extensions';
  import { archiveError, archiveSize } from './archive-format';
  import type { AiSnapshot, AutoUpdateCheck, CatalogEntry, ClockFormat, EnergyProfile, ProactiveActivation, ProactiveSnapshot, Settings, Snapshot, SourceInfo, Theme } from './types';

  // Every control saves through the same command its Settings pane uses, so leaving midway loses nothing.
  let { snapshot, theme, busy, error, onsave, ontoggleextension, onactivation, onfinish }: {
    snapshot: Snapshot; theme: Theme; busy: boolean; error: string;
    onsave: (patch: Partial<Settings>) => void; ontoggleextension: (id: string, enabled: boolean) => void;
    onactivation: (activation: ProactiveActivation) => void; onfinish: () => void;
  } = $props();

  // The core steps frame the steps each chosen extension contributes, in this order.
  type StepId = 'start' | 'extensions' | 'boards' | 'cybercampus' | 'interests' | 'ai' | 'background' | 'notifications' | 'done';
  const allSteps: { id: StepId; label: string; title: string; line: string; extension?: string }[] = [
    { id: 'start', label: '시작', title: 'Rhythm에 오신 것을 환영합니다', line: '계정이나 클라우드 없이 학교 소식과 일정을 이 기기에 모아 봅니다. 여기서 고른 설정은 나중에 설정에서 언제든 바꿀 수 있습니다.' },
    { id: 'extensions', label: '기능', title: '사용할 기능 고르기', line: '켠 기능만 메뉴와 오늘 화면에 나타납니다. 처음에는 모두 꺼져 있고, 설정 → 확장 기능에서 언제든 바꿀 수 있습니다.' },
    { id: 'boards', label: '게시판', title: '받아 볼 게시판', line: '구독한 게시판만 가져옵니다. 구독만으로는 아무것도 내려받지 않습니다.', extension: 'sogang_notices' },
    { id: 'cybercampus', label: '사이버캠퍼스', title: '사이버캠퍼스 연결', line: '본인 강좌의 공지와 과제·마감을 가져옵니다. 로그인 정보는 운영체제 보안 저장소에만 저장합니다.', extension: 'cybercampus' },
    { id: 'interests', label: '관심사', title: '관심사', line: '추천 순위에만 쓰이며 이 기기에만 저장됩니다. 비워 두어도 됩니다.', extension: 'recommendations' },
    { id: 'ai', label: 'AI', title: 'AI 요약', line: '공지 요약과 일정 근거를 만듭니다. AI 없이도 읽기, 검색, 추천을 모두 쓸 수 있습니다.', extension: 'ai' },
    { id: 'background', label: '백그라운드', title: '백그라운드 실행', line: '처음에는 모두 꺼져 있습니다. 켜더라도 Rhythm이 실행 중이고 기기가 깨어 있을 때만 동작합니다.', extension: 'proactive' },
    { id: 'notifications', label: '알림', title: '알림', line: '새 공지를 운영체제 알림으로 받습니다. 처음에는 꺼져 있습니다.', extension: 'proactive' },
    { id: 'done', label: '완료', title: '준비가 끝났습니다', line: '지금까지 고른 설정입니다. 설정 → 일반에서 초기 설정을 다시 열 수 있습니다.' },
  ];
  const themes: { id: Theme; label: string; icon: string }[] = [
    { id: 'light', label: '라이트', icon: 'today' },
    { id: 'dark', label: '다크', icon: 'moon' },
    { id: 'system', label: '시스템', icon: 'monitor' },
  ];
  const clockFormats: { id: ClockFormat; label: string }[] = [
    { id: '12h', label: '12시간' },
    { id: '24h', label: '24시간' },
  ];
  const energyProfiles: { id: EnergyProfile; label: string }[] = [
    { id: 'eco', label: '절전' },
    { id: 'balanced', label: '균형' },
    { id: 'performance', label: '성능' },
  ];
  type AiChoice = 'local' | 'remote' | 'later';
  const aiChoices: { id: AiChoice; label: string; meta: string }[] = [
    { id: 'local', label: '이 기기에서', meta: '로컬 모델 · 공지가 기기를 떠나지 않음' },
    { id: 'remote', label: 'OpenAI 호환 API', meta: '직접 지정한 외부 주소로 전송' },
    { id: 'later', label: '나중에', meta: 'AI 없이 시작' },
  ];
  const aiChoiceLabel: Record<AiChoice, string> = { local: '이 기기에서 실행', remote: 'OpenAI 호환 API', later: '나중에 설정' };

  let enabled = $derived(enabledExtensions(snapshot.extensions));
  let steps = $derived(contributed(allSteps, enabled));
  // Tracked by id, so turning extensions on or off on the 기능 step keeps the place.
  let current = $state<StepId>('start');
  let step = $derived(Math.max(0, steps.findIndex((item) => item.id === current)));
  // Each step slides in from the side the user is moving towards.
  let direction = $state(1);
  let sourcesOn = $derived(enabled.has('sogang_notices') || enabled.has('cybercampus'));
  let chosen = $derived(snapshot.extensions.filter((extension) => extension.enabled).map((extension) => extension.display_name));

  let ai = $state<AiSnapshot | null>(null);
  let aiChoice = $state<AiChoice | null>(null);
  let aiBusy = $state(false);
  let aiError = $state('');
  let removal = $state<string | null>(null);
  let offered = $derived(ai?.catalog.filter((entry) => ['verified', 'available'].includes(entry.compatibility) && entry.kind !== 'embedding') ?? []);

  let updaterConfigured = $state(false);
  let autoCheck = $state<boolean | null>(null);

  let sources = $state<SourceInfo[]>([]);
  let proactive = $state<ProactiveSnapshot | null>(null);
  let subscribed = $derived(sources.filter((source) => source.supported && source.subscribed));
  let syncMessage = $state('');
  let syncBusy = $state(false);
  let summaryError = $state('');

  onMount(() => {
    void focusHeading();
    void invoke<boolean>('updater_configured').then(async (configured) => {
      updaterConfigured = configured;
      if (configured) autoCheck = (await invoke<AutoUpdateCheck>('updater_auto_check')).enabled;
    }).catch(() => { updaterConfigured = false; });
  });

  // The AI step polls like the AI pane, so download progress stays current.
  $effect(() => {
    if (current !== 'ai') return;
    let active = true;
    let issued = 0;
    let applied = 0;
    const refresh = () => {
      const request = ++issued;
      return invoke<AiSnapshot>('ai_snapshot').then((next) => {
        // A slow answer must not replace a snapshot from a later request.
        if (!active || request < applied) return;
        applied = request;
        ai = next;
        aiChoice ??= next.preferences.engine === 'openai-compatible' ? 'remote'
          : next.models.some((model) => model.kind !== 'embedding') ? 'local' : null;
      }).catch((reason) => { if (active && request >= applied) aiError = archiveError(reason); });
    };
    void refresh();
    const timer = window.setInterval(refresh, 1000);
    return () => { active = false; window.clearInterval(timer); };
  });

  $effect(() => {
    if (current !== 'done') return;
    summaryError = '';
    void Promise.all([invoke<SourceInfo[]>('archive_sources'), invoke<ProactiveSnapshot>('proactive_snapshot')])
      .then(([nextSources, nextProactive]) => { sources = nextSources; proactive = nextProactive; })
      .catch((reason) => { summaryError = archiveError(reason); });
  });

  async function focusHeading() {
    await tick();
    window.scrollTo({ top: 0, behavior: 'instant' });
    document.getElementById('setup-title')?.focus({ preventScroll: true });
  }

  function go(next: number) {
    direction = next < step ? -1 : 1;
    current = steps[next].id;
    void focusHeading();
  }

  async function aiCommand(command: string, args: Record<string, unknown>) {
    if (aiBusy) return;
    aiBusy = true;
    aiError = '';
    try {
      await invoke(command, args);
      ai = await invoke<AiSnapshot>('ai_snapshot');
    } catch (reason) { aiError = archiveError(reason); }
    finally { aiBusy = false; }
  }

  async function chooseAi(choice: AiChoice) {
    if (!ai || aiBusy) return;
    aiChoice = choice;
    // "나중에" keeps whatever engine is already saved.
    const engine = choice === 'remote' ? 'openai-compatible' : choice === 'local' ? 'local' : ai.preferences.engine;
    if (engine !== ai.preferences.engine) await aiCommand('ai_preferences', { preferences: { ...ai.preferences, engine } });
  }

  function install(entry: CatalogEntry) {
    return aiCommand('ai_install', { catalogId: entry.id });
  }

  async function remove(id: string) {
    await aiCommand('ai_remove', { id, confirmed: true });
    removal = null;
  }

  async function toggleAutoCheck() {
    if (autoCheck === null) return;
    try { autoCheck = await invoke<boolean>('updater_set_auto_check', { enabled: !autoCheck }); }
    catch { /* The row keeps its previous state; Settings shows update errors. */ }
  }

  async function syncNow() {
    syncBusy = true;
    summaryError = '';
    try { syncMessage = (await invoke<{ message: string }>('archive_start', { sourceIds: null, mode: 'refresh' })).message; }
    catch (reason) { summaryError = archiveError(reason); }
    finally { syncBusy = false; }
  }
</script>

<div class="setup-flow">
  <div class="setup-column">
    <div class="brand"><span class="beat" aria-hidden="true"><i></i><i></i><i></i><i></i></span><span>Rhythm</span></div>
    <ol class="setup-steps setup-progress" aria-label="초기 설정 단계">
      {#each steps as item, index (item.id)}
        <li class:done={index < step} class:current={index === step} aria-current={index === step ? 'step' : undefined}><span aria-hidden="true">{#if index < step}<Icon name="check" size={12} />{:else}{index + 1}{/if}</span>{item.label}</li>
      {/each}
    </ol>

    <main id="main" class="settings-pane setup-body" tabindex="-1" aria-labelledby="setup-title">
      <header class="setup-header"><h1 id="setup-title" tabindex="-1">{steps[step].title}</h1><p>{steps[step].line}</p></header>
      {#if error}<p class="archive-error" role="alert">{error}</p>{/if}

      {#key current}<div in:fly={{ x: 12 * direction, y: 0 }}>
      {#if current === 'start'}
        <div class="settings-group">
          <div class="settings-row"><span class="row-label">화면 테마</span><div class="segmented" role="group" aria-label="화면 테마">{#each themes as option}<button disabled={busy} class:selected={theme === option.id} aria-pressed={theme === option.id} onclick={() => onsave({ theme: option.id })}><Icon name={option.icon} size={15} />{option.label}</button>{/each}</div></div>
          <div class="settings-row"><span class="row-label">시간 표시<span class="row-detail">{snapshot.settings.clock_format === '12h' ? '오후 9:30' : '21:30'}</span></span><div class="segmented" role="group" aria-label="시간 표시">{#each clockFormats as option}<button disabled={busy} class:selected={snapshot.settings.clock_format === option.id} aria-pressed={snapshot.settings.clock_format === option.id} onclick={() => onsave({ clock_format: option.id })}>{option.label}</button>{/each}</div></div>
          <div class="settings-row"><span class="row-label">전원 모드<Hint label="전원 모드">AI 자동 분석과 자동 동기화 간격에 적용됩니다. 절전은 자동 분석을 하지 않고 60분마다, 균형은 30분, 성능은 15분마다 확인합니다.</Hint></span><div class="segmented" role="group" aria-label="전원 모드">{#each energyProfiles as option}<button disabled={busy} class:selected={snapshot.settings.energy_profile === option.id} aria-pressed={snapshot.settings.energy_profile === option.id} onclick={() => onsave({ energy_profile: option.id })}>{option.label}</button>{/each}</div></div>
          {#if updaterConfigured}<div class="settings-row"><span class="row-label">자동으로 업데이트 확인<Hint label="자동으로 업데이트 확인">앱이 실행 중일 때 하루에 한 번 GitHub에서 새 버전을 확인해 알려 드립니다. 내려받기와 설치는 직접 고를 때만 진행합니다.</Hint></span><button class="switch" class:on={autoCheck === true} role="switch" aria-label="자동으로 업데이트 확인" aria-checked={autoCheck === true} disabled={autoCheck === null} onclick={toggleAutoCheck}><span></span></button></div>{/if}
        </div>
      {:else if current === 'extensions'}
        <ExtensionSettings extensions={snapshot.extensions} {busy} ontoggle={ontoggleextension} compact />
      {:else if current === 'boards'}
        <SourceSettings enabled extension="sogang_notices" />
      {:else if current === 'cybercampus'}
        <SourceSettings enabled extension="cybercampus" />
      {:else if current === 'interests'}
        <PersonalizationSettings desktop compact />
      {:else if current === 'ai'}
        {#if aiError}<p class="archive-error" role="alert">{aiError}</p>{/if}
        {#if ai}
          <ul class="model-grid setup-choices" aria-label="AI 사용 방식">
            {#each aiChoices as option (option.id)}
              <li class="model-tile installed" class:selected={aiChoice === option.id}>
                <button class="model-select" aria-pressed={aiChoice === option.id} aria-label={`AI ${option.label}`} disabled={aiBusy} onclick={() => chooseAi(option.id)}></button>
                <div class="model-heading"><h4>{option.label}</h4>{#if aiChoice === option.id}<span class="model-check"><Icon name="check" size={13} /><span>선택함</span></span>{/if}</div>
                <p class="model-meta">{option.meta}</p>
              </li>
            {/each}
          </ul>
          {#if aiChoice === 'local'}
            <div class="settings-group-title"><h3>모델</h3><Hint label="모델">설치는 누를 때만 시작하며, 앱을 닫아도 다음 실행에서 이어받습니다. 불러오기와 동작 확인은 설정 → AI에서 마칩니다.</Hint></div>
            <ul class="model-grid" aria-label="설치할 수 있는 모델">
              {#each offered as entry (entry.id)}
                {@const model = ai.models.find((row) => row.catalog_id === entry.id) ?? null}
                {@const external = model?.ownership === 'external'}
                <li class="model-tile">
                  <div class="model-heading"><h4>{entry.name}</h4>{#if model?.status === 'installed'}<span class="model-check"><Icon name="check" size={13} /><span>설치됨</span></span>{/if}</div>
                  <p class="model-meta">{entry.use_case} · {archiveSize(entry.bytes)}</p>
                  {#if model && ['downloading', 'pending'].includes(model.status)}
                    <label class="ai-progress"><span class="sr-only">{entry.name} 다운로드</span><Progress max={model.expected_bytes || 1} value={model.downloaded_bytes} /><span>{archiveSize(model.downloaded_bytes)} / {archiveSize(model.expected_bytes ?? 0)}</span></label>
                  {:else if model?.status === 'verifying'}<p class="model-note" role="status">무결성 확인 중</p>{/if}
                  {#if model && removal === model.id}
                    <div class="model-confirm" role="group" aria-label={`${entry.name} 삭제 확인`} in:fade>
                      <p class="model-note">{external ? '등록된 참조만 해제합니다. 외부 원본 파일은 삭제하지 않습니다.' : '모델 파일을 삭제할까요? 공지와 기존 AI 분석은 유지됩니다.'}</p>
                      <div class="model-actions"><button class="button secondary small" disabled={aiBusy || ai.busy} onclick={() => remove(model.id)}>확인하고 {external ? '참조 해제' : '삭제'}</button><button class="text-button" onclick={() => removal = null}>취소</button></div>
                    </div>
                  {:else if !model || !['downloading', 'pending', 'verifying'].includes(model.status)}
                    <div class="model-actions">
                      {#if !model || ['failed', 'cancelled'].includes(model.status)}<button class="button small" disabled={aiBusy || ai.busy} aria-label={`${entry.name} 다운로드 및 설치`} onclick={() => install(entry)}>{model ? '이어받기' : '설치'}</button>{/if}
                      {#if model}<button class="text-button" disabled={aiBusy || ai.busy} aria-label={`${entry.name} ${external ? '참조 해제' : '삭제'}`} onclick={() => removal = model.id}>{external ? '참조 해제' : '모델 삭제'}</button>{/if}
                    </div>
                  {/if}
                </li>
              {/each}
            </ul>
          {:else if aiChoice === 'remote'}
            <p class="footnote setup-note">API 주소, 키, 전송 동의는 설정 → AI에서 입력합니다. 동의하기 전에는 아무것도 보내지 않습니다.</p>
          {/if}
        {:else if !aiError}<p class="loading" role="status">모델 정보를 불러오는 중입니다…</p>{/if}
      {:else if current === 'background'}
        <ProactiveSettings desktop part="background" compact onopen={() => {}} {onactivation} />
      {:else if current === 'notifications'}
        <ProactiveSettings desktop part="notifications" compact onopen={() => {}} {onactivation} />
      {:else}
        {#if summaryError}<p class="archive-error" role="alert">{summaryError}</p>{/if}
        <div class="settings-group" role="group" aria-label="선택한 설정">
          <div class="settings-row"><span class="row-label">켠 기능<span class="row-detail">{chosen.length ? chosen.join(', ') : '오늘만 표시합니다. 설정 → 확장 기능에서 언제든 켤 수 있습니다.'}</span></span><span class="value">{chosen.length.toLocaleString('ko-KR')}개</span></div>
          {#if sourcesOn}<div class="settings-row"><span class="row-label">구독한 게시판</span><span class="value">{subscribed.length.toLocaleString('ko-KR')}개</span></div>{/if}
          {#if enabled.has('ai')}<div class="settings-row"><span class="row-label">AI</span><span class="value">{aiChoice ? aiChoiceLabel[aiChoice] : '나중에 설정'}</span></div>{/if}
          {#if enabled.has('proactive')}
            <div class="settings-row"><span class="row-label">자동 동기화</span><span class="value">{proactive ? (proactive.settings.scheduler_enabled ? '켜짐' : '꺼짐') : '—'}</span></div>
            <div class="settings-row"><span class="row-label">알림</span><span class="value">{proactive ? (proactive.settings.notifications_enabled ? '켜짐' : '꺼짐') : '—'}</span></div>
          {/if}
          {#if sourcesOn}<div class="settings-row"><span class="row-label">지금 공지 가져오기<Hint label="지금 공지 가져오기">구독한 게시판의 새 소식을 확인하고 이전 공지를 차례로 가져옵니다. 진행 상황은 사이드바와 각 게시판 설정에 표시됩니다.</Hint>{#if syncMessage}<span class="row-detail" role="status">{syncMessage}</span>{/if}</span><button class="button secondary small" disabled={syncBusy || Boolean(syncMessage) || !subscribed.length} onclick={syncNow}>가져오기</button></div>{/if}
        </div>
      {/if}
      </div>{/key}
    </main>

    <footer class="setup-footer">
      {#if step < steps.length - 1}<button class="text-button" onclick={onfinish}>건너뛰기</button>{/if}
      <span class="setup-footer-actions">
        {#if step > 0}<button class="button secondary" onclick={() => go(step - 1)}>이전</button>{/if}
        {#if step < steps.length - 1}<button class="button" onclick={() => go(step + 1)}>다음</button>
        {:else}<button class="button" onclick={onfinish}>시작하기</button>{/if}
      </span>
    </footer>
  </div>
</div>
