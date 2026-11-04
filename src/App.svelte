<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import Icon from './lib/Icon.svelte';
  import ArchiveView from './lib/ArchiveView.svelte';
  import SourceSettings from './lib/SourceSettings.svelte';
  import AiSettings from './lib/AiSettings.svelte';
  import PersonalizationSettings from './lib/PersonalizationSettings.svelte';
  import BackupSettings from './lib/BackupSettings.svelte';
  import UpdaterSettings from './lib/UpdaterSettings.svelte';
  import ProactiveSettings from './lib/ProactiveSettings.svelte';
  import Hint from './lib/Hint.svelte';
  import './lib/ai.css';
  import { archiveError as message, archiveDay, localDayKey } from './lib/archive-format';
  import type { ArchiveSummary, EnergyProfile, Settings, Snapshot, SourceInfo, SyncStatus, Theme, WorkerStatus, AiSnapshot, ProactiveActivation, ProactiveSnapshot, RestoreOutcome, AutoUpdateCheck, UpdateInfo } from './lib/types';

  const navigation = [
    { id: 'today', label: '오늘' },
    { id: 'inbox', label: '받은 소식' },
    { id: 'search', label: '탐색' },
    { id: 'bookmark', label: '보관함' },
    { id: 'settings', label: '설정' },
  ];
  type SettingsPane = 'general' | 'boards' | 'ai' | 'personalization' | 'notifications' | 'data';
  const settingsPanes: { id: SettingsPane; label: string; icon: string }[] = [
    { id: 'general', label: '일반', icon: 'settings' },
    { id: 'boards', label: '게시판', icon: 'boards' },
    { id: 'ai', label: 'AI', icon: 'cpu' },
    { id: 'personalization', label: '개인화', icon: 'person' },
    { id: 'notifications', label: '알림', icon: 'bell' },
    { id: 'data', label: '데이터 및 정보', icon: 'database' },
  ];
  const themes: { id: Theme; label: string; icon: string }[] = [
    { id: 'light', label: '라이트', icon: 'today' },
    { id: 'dark', label: '다크', icon: 'moon' },
    { id: 'system', label: '시스템', icon: 'monitor' },
  ];
  const energyProfiles: { id: EnergyProfile; label: string }[] = [
    { id: 'eco', label: '절전' },
    { id: 'balanced', label: '균형' },
    { id: 'performance', label: '성능' },
  ];
  let page = $state('today');
  let snapshot = $state<Snapshot | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let desktop = $state(false);
  let proactiveActivation = $state<ProactiveActivation | null>(null);
  const handledActivations = new Set<string>();
  let setupNeeded = $state(false);
  // Kept while the app is open, so returning to Settings shows the last pane.
  let settingsPane = $state<SettingsPane>('general');
  let paneLabel = $derived(settingsPanes.find((item) => item.id === settingsPane)!.label);
  let restoreOutcome = $state<RestoreOutcome | null>(null);
  // Found by the automatic check; "나중에" hides it for this version until restart.
  let availableUpdate = $state<UpdateInfo | null>(null);
  let dismissedUpdate = $state('');
  let noticesEnabled = $derived(snapshot?.extensions.some((extension) => extension.id === 'sogang_notices' && extension.enabled) ?? false);
  let theme = $state<Theme>('system');
  let systemDark = $state(false);
  let heading = $derived(navigation.find((item) => item.id === page)!);
  let dark = $derived(theme === 'dark' || (theme === 'system' && systemDark));
  let date = new Intl.DateTimeFormat('ko-KR', { month: 'long', day: 'numeric', weekday: 'long' }).format(new Date());
  // Archive context reported by ArchiveView; the sidebar reads it but never fetches it separately.
  let summary = $state<ArchiveSummary | null>(null);
  let sources = $state<SourceInfo[]>([]);
  let syncStatus = $state<SyncStatus | null>(null);
  // A new token asks ArchiveView to reset its filters, optionally scoped to one board.
  let sourceRequest = $state({ id: '', token: 0 });
  let activeSource = $state('');
  let weekPane = $state(false);
  let boards = $derived(sources.filter((item) => item.supported && item.subscribed));
  // Board names repeat across institutions ("학사 공지" / "학사공지"), so the sidebar groups them by organization.
  let boardGroups = $derived.by(() => {
    const groups = new Map<string, SourceInfo[]>();
    for (const board of boards) {
      const label = organizationLabel(board.organization);
      groups.set(label, [...(groups.get(label) ?? []), board]);
    }
    return [...groups];
  });
  let scopedBoard = $derived(page === 'inbox' && activeSource ? sources.find((item) => item.id === activeSource) ?? null : null);
  let lastChecked = $derived(boards.reduce<string | null>((latest, item) => !item.last_success || (latest && latest >= item.last_success) ? latest : item.last_success, null));

  $effect(() => {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
  });

  onMount(() => {
    desktop = isTauri();
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    let mounted = true;
    let unlistenNotification: (() => void) | undefined;
    let unlistenUpdate: (() => void) | undefined;
    systemDark = media.matches;
    const update = () => { systemDark = media.matches; };
    media.addEventListener('change', update);
    if (desktop) {
      // Subscribe before asking, so a check finishing in between is not lost.
      // The announcement is optional; failures leave the rest of the app alone.
      void listen<UpdateInfo>('updater://available', (event) => { availableUpdate = event.payload; }).then((unlisten) => {
        if (mounted) unlistenUpdate = unlisten;
        else unlisten();
        return invoke<AutoUpdateCheck>('updater_auto_check');
      }).then((value) => {
        if (mounted && value?.available) availableUpdate ??= value.available;
      }).catch(() => {});
      void listen<ProactiveActivation>('proactive://notification-clicked', (event) => {
        handleProactiveActivation(event.payload);
      }).then((unlisten) => {
        if (mounted) {
          unlistenNotification = unlisten;
          void load();
        } else unlisten();
      }).catch((reason) => {
        if (mounted) {
          void load().then(() => {
            if (mounted) error = `알림 클릭 기능을 시작하지 못했습니다: ${message(reason)}`;
          });
        }
      });
    } else loading = false;
    const healthTimer = window.setInterval(() => {
      if (desktop && snapshot && page === 'settings' && !busy) {
        void invoke<WorkerStatus>('worker_status').then((status) => {
          if (snapshot) snapshot.worker = status;
        }).catch(() => { /* Explicit refresh also exposes transport errors. */ });
      }
    }, 5000);
    return () => {
      mounted = false;
      unlistenNotification?.();
      unlistenUpdate?.();
      media.removeEventListener('change', update);
      window.clearInterval(healthTimer);
    };
  });

  async function navigate(destination: string, sourceId = '') {
    page = destination;
    sourceRequest = { id: sourceId, token: sourceRequest.token + 1 };
    activeSource = sourceId;
    notice = '';
    history.replaceState(null, '', location.pathname + location.search);
    await tick();
    window.scrollTo({ top: 0, behavior: 'instant' });
    document.getElementById('main')?.focus({ preventScroll: true });
  }

  function handleProactiveActivation(payload: ProactiveActivation) {
    if (!Number.isSafeInteger(payload.document_id) || payload.document_id <= 0
      || !Number.isSafeInteger(payload.revision_id) || payload.revision_id <= 0
      || !Number.isSafeInteger(payload.event_id) || payload.event_id <= 0
      || !Number.isSafeInteger(payload.attempt_count) || payload.attempt_count < 0) return;
    // The same activation can arrive through the event and the one-shot snapshot,
    // even long after the first delivery. A later attempt is a distinct click.
    const activationKey = `${payload.event_id}:${payload.attempt_count}`;
    if (handledActivations.has(activationKey)) return;
    handledActivations.add(activationKey);
    const eventIds = Array.isArray(payload.event_ids)
      ? payload.event_ids.filter((id) => Number.isSafeInteger(id) && id > 0) : [payload.event_id];
    void openProactiveDocument(payload.document_id, payload.revision_id, payload.event_id, payload.attempt_count, eventIds);
  }

  async function load() {
    loading = true;
    error = '';
    try {
      snapshot = await invoke<Snapshot>('get_snapshot');
      theme = snapshot.settings.theme;
      try {
        const proactive = await invoke<ProactiveSnapshot>('proactive_snapshot');
        if (proactive.pending_activation) handleProactiveActivation(proactive.pending_activation);
      } catch { /* The archive remains usable if proactive state is unavailable. */ }
      try { restoreOutcome = await invoke<RestoreOutcome | null>('backup_restore_outcome'); }
      catch { /* A missing report must not block the restored archive. */ }
      try { setupNeeded = !(await invoke<AiSnapshot>('ai_snapshot')).preferences.setup_completed; }
      catch { /* AI availability must not prevent archive access. Settings exposes recovery. */ }
    } catch (reason) {
      error = message(reason);
    } finally {
      loading = false;
    }
  }

  async function save(patch: Partial<Settings>) {
    if (!snapshot || busy) return;
    busy = true;
    error = '';
    notice = '';
    if (patch.theme) theme = patch.theme;
    try {
      snapshot = await invoke<Snapshot>('save_settings', { settings: { ...snapshot.settings, ...patch } });
      theme = snapshot.settings.theme;
      notice = '설정을 저장했습니다.';
    } catch (reason) {
      theme = snapshot.settings.theme;
      error = message(reason);
    } finally {
      busy = false;
    }
  }

  async function toggleExtension(id: string, enabled: boolean) {
    if (busy) return;
    busy = true;
    error = '';
    notice = '';
    try {
      snapshot = await invoke<Snapshot>('set_extension', { id, enabled });
      notice = '확장 기능 사용 설정을 저장했습니다.';
    } catch (reason) {
      error = message(reason);
    } finally { busy = false; }
  }

  // Opens a settings pane from elsewhere and moves focus to its heading (or a target inside it).
  async function openSettings(pane: SettingsPane, target = 'settings-pane-title') {
    await navigate('settings');
    settingsPane = pane;
    await tick();
    const element = document.getElementById(target);
    element?.scrollIntoView({ block: 'start', behavior: 'instant' });
    element?.focus({ preventScroll: true });
  }

  function aiSettings() {
    return openSettings('ai');
  }

  function updateSettings() {
    return openSettings('data', 'about-title');
  }

  async function openProactiveDocument(documentId: number, revisionId: number, eventId: number, attemptCount = 1, eventIds: number[] = [eventId]) {
    if (eventIds.length > 1) {
      proactiveActivation = null;
      await openSettings('notifications', 'notification-history-title');
      return;
    }
    proactiveActivation = { document_id: documentId, revision_id: revisionId, event_id: eventId, attempt_count: attemptCount, event_ids: eventIds };
    page = 'inbox';
    notice = '';
    history.replaceState(null, '', location.pathname + location.search);
    await tick();
    window.scrollTo({ top: 0, behavior: 'instant' });
    document.getElementById('main')?.focus({ preventScroll: true });
  }

  function organizationLabel(organization: string): string {
    return organization.replace(/^서강대학교\s*·\s*/, '') || organization;
  }

  function checkedLabel(value: string): string {
    const day = archiveDay(value);
    if (!day) return value;
    if (day !== localDayKey(new Date())) return `${Number(day.slice(5, 7))}월 ${Number(day.slice(8, 10))}일`;
    const time = new Date(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(value) ? `${value.replace(' ', 'T')}Z` : value);
    return Number.isNaN(time.getTime()) ? '오늘' : `오늘 ${new Intl.DateTimeFormat('ko-KR', { hour: '2-digit', minute: '2-digit', hour12: false }).format(time)}`;
  }

  function storageSize(bytes: number) {
    return bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
</script>

<svelte:head><title>Rhythm</title><meta name="description" content="나의 하루를 정돈하는 로컬 퍼스트 개인 커맨드 센터" /></svelte:head>

<a class="skip-link" href="#main">본문으로 건너뛰기</a>
<div class="app-shell">
  <aside class="sidebar" aria-label="주 메뉴">
    <div class="brand"><span class="beat" aria-hidden="true"><i></i><i></i><i></i><i></i></span><span>Rhythm</span></div>
    <nav aria-label="기본 탐색">
      <ul class="source-list">
        {#each navigation.slice(0, 4) as item}
          {@const count = item.id === 'inbox' ? summary?.unread : item.id === 'bookmark' ? summary?.saved : undefined}
          <li><button class="source-item" class:active={page === item.id && !activeSource} aria-label={item.label} aria-current={page === item.id && !activeSource ? 'page' : undefined} aria-describedby={count ? `nav-count-${item.id}` : undefined} onclick={() => navigate(item.id)}>
            <Icon name={item.id} size={17} /><span class="source-label">{item.label}</span>
            {#if count}<span class="count" aria-hidden="true">{count.toLocaleString('ko-KR')}</span><span class="sr-only" id={`nav-count-${item.id}`}>{item.id === 'inbox' ? `읽지 않은 공지 ${count}개` : `저장한 공지 ${count}개`}</span>{/if}
          </button></li>
        {/each}
      </ul>
      {#each snapshot?.extensions ?? [] as extension}
        <p class="source-group-heading" id={`group-${extension.id}`}>{extension.display_name}</p>
        <ul class="source-list" aria-labelledby={`group-${extension.id}`}>
          {#if !extension.enabled}
            <li><button class="source-item quiet" onclick={() => openSettings('boards')}><span class="source-label">꺼져 있음 · 설정에서 켜기</span></button></li>
          {:else if extension.id === 'sogang_notices' && boards.length}
            {#each boardGroups as [organization, members] (organization)}
              <li class="board-organization" aria-hidden="true">{organization}</li>
              {#each members as board (board.id)}
                <li><button class="source-item board" class:active={page === 'inbox' && activeSource === board.id} aria-current={page === 'inbox' && activeSource === board.id ? 'page' : undefined} aria-label={`${organizationLabel(board.organization)} ${board.name}`} onclick={() => navigate('inbox', board.id)}>
                  <span class={`board-swatch family-${board.family}`} aria-hidden="true"></span><span class="source-label">{board.name}</span>
                </button></li>
              {/each}
            {/each}
          {:else}
            <li><button class="source-item quiet" onclick={() => openSettings('boards')}><span class="source-label">구독한 게시판 없음 · 설정</span></button></li>
          {/if}
        </ul>
      {/each}
      <div class="sidebar-bottom">
        <p class="sync-line" role="status">
          {#if syncStatus?.running}<Icon name="refresh" size={14} /><span>{syncStatus.message}{#if syncStatus.page > 0} · {syncStatus.page}쪽{/if}</span>
          {:else if lastChecked}<span>마지막 확인 {checkedLabel(lastChecked)}</span>
          {:else if boards.length}<span>아직 가져온 적 없음</span>{/if}
        </p>
        <button class="source-item" class:active={page === 'settings'} aria-label="설정" aria-current={page === 'settings' ? 'page' : undefined} onclick={() => navigate('settings')}><Icon name="settings" size={17} /><span class="source-label">설정</span></button>
      </div>
    </nav>
  </aside>

  <main id="main" tabindex="-1" class:with-week-pane={weekPane}>
    <header class="page-header"><h1>{scopedBoard?.name ?? heading.label}</h1>{#if scopedBoard}<p>{organizationLabel(scopedBoard.organization)} · 받은 소식</p>{/if}{#if page === 'today'}<p>{date}{#if summary}{' · '}읽지 않은 공지 {summary.unread.toLocaleString('ko-KR')}{#if summary.new_since_sync}{' · '}최근 동기화로 새 공지 {summary.new_since_sync.toLocaleString('ko-KR')}{/if}{/if}</p>{/if}</header>
    {#if !desktop && !loading}<div class="banner browser-notice" role="status"><Icon name="info" /><div><strong>데스크톱 앱에서 시작해 주세요</strong><p>브라우저에서는 화면만 둘러볼 수 있습니다. 설정 저장과 로컬 데이터 접근은 Rhythm 앱에서 사용할 수 있습니다.</p></div></div>{/if}
    {#if error}<div class="banner error" role="alert"><Icon name="info" /><div><strong>작업을 완료하지 못했습니다</strong><p>{error}</p></div>{#if !snapshot && desktop}<button class="text-button" onclick={load}>다시 시도</button>{/if}</div>{/if}
    {#if restoreOutcome}<div class="banner" role="status"><Icon name="info" /><div><strong>백업 복원을 마쳤습니다</strong><p>공지 {restoreOutcome.documents.toLocaleString('ko-KR')}건과 첨부 자료 {restoreOutcome.resources.toLocaleString('ko-KR')}개를 복원하고 무결성을 확인했습니다.{#if restoreOutcome.unavailable_resources} 백업 당시 내려받지 못한 첨부 자료 {restoreOutcome.unavailable_resources.toLocaleString('ko-KR')}개는 포함되지 않았습니다.{/if}{#if restoreOutcome.models_needing_reinstall} 사용할 수 없는 AI 모델 {restoreOutcome.models_needing_reinstall.toLocaleString('ko-KR')}개는 다시 등록하거나 설치해 주세요.{/if} 이전 데이터는 데이터 폴더에 복구용으로 보관되어 있습니다.</p></div><button class="text-button" onclick={() => restoreOutcome = null}>닫기</button></div>{/if}
    {#if desktop && availableUpdate && dismissedUpdate !== availableUpdate.version && page !== 'settings'}<div class="banner" role="status"><Icon name="info" /><div><strong>Rhythm {availableUpdate.version} 버전을 설치할 수 있습니다</strong><p>설정 → 데이터 및 정보에서 내려받으면 업데이트 서명을 확인한 뒤 설치 여부를 묻습니다. 그 전에는 아무것도 바뀌지 않습니다.</p></div><button class="text-button" onclick={updateSettings}>업데이트 보기</button><button class="text-button" onclick={() => dismissedUpdate = availableUpdate?.version ?? ''}>나중에</button></div>{/if}
    {#if desktop && setupNeeded && page !== 'settings'}<div class="banner ai-setup-banner" role="status"><Icon name="cpu" /><div><strong>나의 기기에서 공지의 핵심을 정리하세요</strong><p>로컬 AI 모델을 선택하면 한국어 요약과 일정의 근거를 확인할 수 있습니다. 준비 전이나 오프라인에서도 공지는 계속 읽을 수 있습니다.</p></div><button class="text-button" onclick={aiSettings}>로컬 AI 설정하기</button><button class="text-button" onclick={() => setupNeeded = false}>나중에</button></div>{/if}
    <div class="save-notice" class:visible={Boolean(notice)} role="status" aria-live="polite">{#if notice}<Icon name="check" size={15} />{notice}{/if}</div>

    {#if page === 'settings'}
      <div class="settings-layout">
        <nav class="settings-nav" aria-label="설정 항목">
          <ul>
            {#each settingsPanes as item (item.id)}
              <li><button class="pane-item" class:active={settingsPane === item.id} aria-current={settingsPane === item.id ? 'page' : undefined} onclick={() => settingsPane = item.id}><Icon name={item.icon} size={17} /><span>{item.label}</span></button></li>
            {/each}
          </ul>
        </nav>
        <section class="settings-pane" aria-labelledby="settings-pane-title" aria-busy={busy || loading}>
          <h2 id="settings-pane-title" class="pane-title" tabindex="-1">{paneLabel}</h2>
          {#if loading}<p class="loading" role="status">저장된 설정을 불러오는 중입니다…</p>{/if}
          {#if settingsPane === 'general'}
            <div class="settings-group">
              <div class="settings-row"><span class="row-label">화면 테마</span><div class="segmented" role="group" aria-label="화면 테마">{#each themes as option}<button disabled={!snapshot || busy} class:selected={theme === option.id} aria-pressed={theme === option.id} onclick={() => save({ theme: option.id })}><Icon name={option.icon} size={15} />{option.label}</button>{/each}</div></div>
              <div class="settings-row"><span class="row-label">전원 모드<Hint label="전원 모드">AI 자동 분석과 자동 동기화 간격에 적용됩니다. 절전은 자동 분석을 하지 않고 60분마다, 균형은 30분, 성능은 15분마다 확인합니다. AI는 한 번에 한 공지만 분석합니다.</Hint></span><div class="segmented" role="group" aria-label="전원 모드">{#each energyProfiles as option}<button disabled={!snapshot || busy} class:selected={snapshot?.settings.energy_profile === option.id} aria-pressed={snapshot?.settings.energy_profile === option.id} onclick={() => save({ energy_profile: option.id })}>{option.label}</button>{/each}</div></div>
            </div>
            <ProactiveSettings {desktop} part="background" onopen={openProactiveDocument} onactivation={handleProactiveActivation} />
          {:else if settingsPane === 'boards'}
            {#if snapshot}
              <div class="settings-group">
                {#each snapshot.extensions as extension}
                  <div class="settings-row"><span class="row-label">{extension.display_name}<Hint label={extension.display_name}>{extension.description} 꺼도 보관한 공지와 구독은 그대로 남습니다.{#if !extension.available} 지금은 사용할 수 없습니다.{/if}</Hint></span><button class="switch" class:on={extension.enabled} role="switch" aria-checked={extension.enabled} aria-label={`${extension.display_name} 사용 설정`} disabled={busy} onclick={() => toggleExtension(extension.id, !extension.enabled)}><span></span></button></div>
                {/each}
              </div>
            {:else if !loading}<p class="muted">데스크톱 앱에서 게시판을 구독할 수 있습니다.</p>{/if}
            {#if noticesEnabled}<SourceSettings enabled={noticesEnabled} />{/if}
          {:else if settingsPane === 'ai'}
            <AiSettings {desktop} onsetup={() => setupNeeded = false} />
          {:else if settingsPane === 'personalization'}
            <PersonalizationSettings {desktop} />
          {:else if settingsPane === 'notifications'}
            <ProactiveSettings {desktop} part="notifications" onopen={openProactiveDocument} onactivation={handleProactiveActivation} />
          {:else}
            <div class="settings-group">
              <div class="settings-row"><span class="row-label">로컬 데이터<Hint label="로컬 데이터">프로필, 관심사, 피드백과 공지는 이 기기의 SQLite{#if snapshot} (스키마 {snapshot.storage.schema_version}){/if}에만 저장합니다. 클라우드 저장소, 원격 추론, 사용 정보 수집은 없습니다.{#if snapshot}<code class="popover-code">{snapshot.storage.path}</code>{/if}</Hint></span><span class="value">{snapshot ? storageSize(snapshot.storage.bytes) : '—'}</span></div>
            </div>
            <BackupSettings {desktop} />
            <UpdaterSettings {desktop} />
          {/if}
        </section>
      </div>
    {:else}
      <ArchiveView {page} {desktop} enabled={noticesEnabled} {sourceRequest} activation={proactiveActivation} onactivationhandled={() => proactiveActivation = null} onsettings={(pane) => openSettings(pane ?? 'boards')} onaisettings={aiSettings}
        onsourcechange={(id) => activeSource = id}
        onweekpane={(visible) => weekPane = visible}
        oncontext={(context) => { if (context.summary) summary = context.summary; if (context.sources) sources = context.sources; syncStatus = context.status; }} />
    {/if}
  </main>
</div>
