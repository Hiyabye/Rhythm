<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import Icon from './lib/Icon.svelte';
  import ArchiveView from './lib/ArchiveView.svelte';
  import SourceSettings from './lib/SourceSettings.svelte';
  import FeedSettings from './lib/FeedSettings.svelte';
  import AiSettings from './lib/AiSettings.svelte';
  import PersonalizationSettings from './lib/PersonalizationSettings.svelte';
  import BackupSettings from './lib/BackupSettings.svelte';
  import UpdaterSettings from './lib/UpdaterSettings.svelte';
  import ProactiveSettings from './lib/ProactiveSettings.svelte';
  import CybercampusTasks from './lib/CybercampusTasks.svelte';
  import StudiumView from './lib/StudiumView.svelte';
  import SetupFlow from './lib/SetupFlow.svelte';
  import ExtensionSettings from './lib/ExtensionSettings.svelte';
  import { contributed, enabledExtensions, navigation, settingsPanes, sourceExtensions, type SettingsPane } from './lib/extensions';
  import Hint from './lib/Hint.svelte';
  import { fade, fly, slide } from './lib/motion';
  import { archiveError as message, archiveDay, groupScope, localDayKey, openTask, organizationLabel, scopeGroup, timeOptions } from './lib/archive-format';
  import { clock } from './lib/clock.svelte';
  import type { ArchiveSummary, AutoTrashMonths, ClockFormat, EnergyProfile, Settings, Snapshot, SourceGroup, SourceInfo, SyncStatus, FeedStatus, Theme, ProactiveActivation, ProactiveSnapshot, RestoreOutcome, AutoUpdateCheck, UpdateInfo, CybercampusCourse, CybercampusTask, TodoPrefill } from './lib/types';

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
  const clockFormats: { id: ClockFormat; label: string }[] = [
    { id: '12h', label: '12시간' },
    { id: '24h', label: '24시간' },
  ];
  const autoTrashChoices: { months: AutoTrashMonths; label: string }[] = [
    { months: 0, label: '안 함' },
    { months: 1, label: '1개월' },
    { months: 3, label: '3개월' },
    { months: 6, label: '6개월' },
    { months: 12, label: '1년' },
    { months: 24, label: '2년' },
  ];
  let page = $state('today');
  let snapshot = $state<Snapshot | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let desktop = $state(false);
  let proactiveActivation = $state<ProactiveActivation | null>(null);
  let studiumActivation = $state(0);
  let pendingStudiumRoom: string | null = null;
  let pendingReminderDay: string | null = null;
  let studiumEventGeneration = 0;
  const handledActivations = new Set<string>();
  // The first-run setup flow replaces the shell until it is finished or skipped.
  let showSetup = $state(false);
  // Kept while the app is open, so returning to Settings shows the last pane.
  let settingsPane = $state<SettingsPane>('extensions');
  // Only the core shell and what enabled extensions add are shown.
  let enabled = $derived(enabledExtensions(snapshot?.extensions));
  let panes = $derived(contributed(settingsPanes, enabled));
  let paneLabel = $derived(panes.find((item) => item.id === settingsPane)?.label ?? '');
  let restoreOutcome = $state<RestoreOutcome | null>(null);
  // Found by the automatic check; "나중에" hides it for this version until restart.
  let availableUpdate = $state<UpdateInfo | null>(null);
  let dismissedUpdate = $state('');
  let noticesEnabled = $derived(enabled.has('sogang_notices'));
  let theme = $state<Theme>('system');
  let systemDark = $state(false);
  let heading = $derived(page === 'settings' ? '설정' : navigation.find((item) => item.id === page)?.label ?? '');
  let dark = $derived(theme === 'dark' || (theme === 'system' && systemDark));
  // The header date follows the local day; the periodic timer below refreshes the key across midnight.
  let dayKey = $state(localDayKey(new Date()));
  let date = $derived(dayKey && new Intl.DateTimeFormat('ko-KR', { month: 'long', day: 'numeric', weekday: 'long' }).format(new Date()));
  // Archive context reported by ArchiveView; the sidebar reads it but never fetches it separately.
  let summary = $state<ArchiveSummary | null>(null);
  let sources = $state<SourceInfo[]>([]);
  let sourceGroups = $state<SourceGroup[]>([]);
  let syncStatus = $state<SyncStatus | null>(null);
  // A new token asks ArchiveView to reset its filters, optionally scoped to one board or group (`groupScope`).
  let sourceRequest = $state({ id: '', category: '', token: 0 });
  let activeSource = $state('');
  // A Cybercampus course entry scopes the inbox to its board and the course's category.
  let activeCategory = $state('');
  let weekPane = $state(false);
  // A new token asks the calendar page to select `day`.
  let calendarRequest = $state<{ day: string; token: number; prefill: TodoPrefill | null }>({ day: '', token: 0, prefill: null });
  let boards = $derived(sources.filter((item) => item.supported && item.subscribed));
  // 휴지통 belongs to the notice sources, but stays reachable while it still holds notices.
  let primaryNavigation = $derived(contributed(navigation, enabled).filter((item) => item.id !== 'trash'
    || sourceExtensions.some((owner) => enabled.has(owner.id)) || Boolean(summary?.trashed)));
  // Each enabled source extension gets a sidebar section: one feed per settings group holding a
  // subscribed board, then ungrouped boards by organization ("학사 공지" / "학사공지" repeat across them).
  // A group feed lists every board placed directly in it, so its count covers them all.
  let sidebarGroups = $derived(sourceExtensions.filter((owner) => enabled.has(owner.id)).map((owner) => {
    const owned = boards.filter((item) => item.extension === owner.id);
    const feeds = sourceGroups.filter((group) => owned.some((board) => board.group === group.id)).map((group) => {
      const members = sources.filter((item) => item.group === group.id);
      return { group, family: members[0].family, unread: members.reduce((sum, item) => sum + item.unread, 0) };
    });
    const organizations = new Map<string, SourceInfo[]>();
    for (const board of owned.filter((item) => !sourceGroups.some((group) => group.id === item.group))) {
      const label = organizationLabel(board.organization);
      organizations.set(label, [...(organizations.get(label) ?? []), board]);
    }
    return { ...owner, name: snapshot?.extensions.find((extension) => extension.id === owner.id)?.display_name ?? owner.id, feeds, organizations: [...organizations] };
  }));
  // A page or pane whose extension was just turned off falls back to the core shell.
  // A board scope keeps the inbox open: its owner may be Cybercampus alone.
  $effect(() => {
    if (!snapshot) return;
    if (page !== 'settings' && !primaryNavigation.some((item) => item.id === page) && !(page === 'inbox' && boards.some((board) => board.id === activeSource || groupScope(board.group ?? '') === activeSource))) page = 'today';
    if (!panes.some((item) => item.id === settingsPane)) settingsPane = 'extensions';
  });
  let scopedBoard = $derived(page === 'inbox' && activeSource ? sources.find((item) => item.id === activeSource) ?? null : null);
  // The synced term's courses, one sidebar entry each under 사이버캠퍼스.
  let courses = $state<CybercampusCourse[]>([]);
  let scopedCourse = $derived(scopedBoard?.family === 'cybercampus' && courses.some((course) => course.name === activeCategory) ? activeCategory : null);
  let scopedGroup = $derived(page === 'inbox' ? sourceGroups.find((group) => group.id === scopeGroup(activeSource)) ?? null : null);
  // A board chosen inside a group feed keeps the group's sidebar entry current.
  let currentFeed = $derived(page === 'inbox' ? scopeGroup(activeSource) ?? scopedBoard?.group ?? null : null);
  // Only the open-task count is shown; the 과제·마감 page loads its own list.
  let tasks = $state<CybercampusTask[]>([]);
  let now = $state(Date.now());
  let openTasks = $derived(tasks.filter((task) => openTask(task, now)).length);
  // Each context refresh (sync finished, reader closed) and each page change reloads them, like the archive counts.
  $effect(() => {
    void sources; void page;
    if (!desktop || !enabled.has('cybercampus')) { tasks = []; courses = []; return; }
    let current = true;
    void invoke<CybercampusTask[]>('cybercampus_tasks').then((value) => { if (current) tasks = value; }).catch(() => { /* The 과제·마감 page reports load failures. */ });
    // Without courses (never synced, or a failed read) the sidebar keeps the single board entry.
    void invoke<CybercampusCourse[]>('cybercampus_courses').then((value) => { if (current) courses = value; }).catch(() => { if (current) courses = []; });
    return () => { current = false; };
  });
  function navCount(id: string): { count: number; label: string } | null {
    const entry = id === 'today' ? summary && { count: summary.new_since_sync, label: '최근 동기화로 새 공지' }
      : id === 'inbox' ? summary && { count: summary.unread, label: '읽지 않은 공지' }
      : id === 'search' ? summary && { count: summary.documents, label: '보관된 공지' }
      : id === 'bookmark' ? summary && { count: summary.saved, label: '저장한 공지' }
      : id === 'tasks' ? { count: openTasks, label: '남은 과제' }
      : id === 'trash' ? summary && { count: summary.trashed, label: '휴지통의 공지' } : null;
    return entry?.count ? entry : null;
  }
  // Public boards arrive through the feed, so its last successful check stands for them (#274);
  // Cybercampus keeps its own last run. Reloaded on each context refresh, which follows every finished feed sync.
  let feedChecked = $state<string | null>(null);
  $effect(() => {
    void sources;
    if (!desktop || !enabled.has('sogang_notices')) { feedChecked = null; return; }
    let current = true;
    void invoke<FeedStatus>('feed_status').then((value) => { if (current) feedChecked = value.checked_at; }).catch(() => { /* The feed pane reports failures. */ });
    return () => { current = false; };
  });
  let lastChecked = $derived([
    ...boards.filter((item) => item.id === 'cybercampus').map((item) => item.last_success),
    boards.some((item) => item.id !== 'cybercampus') ? feedChecked : null,
  ].reduce<string | null>((latest, value) => !value || (latest && latest >= value) ? latest : value, null));

  $effect(() => {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
  });

  onMount(() => {
    desktop = isTauri();
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    let mounted = true;
    let unlistenNotification: (() => void) | undefined;
    let unlistenStudiumNotification: (() => void) | undefined;
    let unlistenReminder: (() => void) | undefined;
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
      void listen<string>('studium://notification-clicked', (event) => {
        if (!isStudiumRoom(event.payload)) return;
        studiumEventGeneration++;
        handleStudiumActivation(event.payload);
      }).then((unlisten) => {
        if (!mounted) { unlisten(); return null; }
        unlistenStudiumNotification = unlisten;
        const generation = studiumEventGeneration;
        return invoke<string | null>('studium_notification_activation').then((roomId) => {
          if (mounted && generation === studiumEventGeneration && roomId) handleStudiumActivation(roomId);
        });
      }).catch(() => { /* Optional click recovery must not block the shell. */ });
      void listen<string>('calendar://reminder-clicked', (event) => handleReminderActivation(event.payload)).then((unlisten) => {
        if (!mounted) { unlisten(); return null; }
        unlistenReminder = unlisten;
        return invoke<string | null>('calendar_reminder_activation').then((day) => {
          if (mounted && day) handleReminderActivation(day);
        });
      }).catch(() => { /* Optional click recovery must not block the shell. */ });
    } else loading = false;
    const healthTimer = window.setInterval(() => {
      dayKey = localDayKey(new Date());
      now = Date.now();
    }, 5000);
    return () => {
      mounted = false;
      unlistenNotification?.();
      unlistenStudiumNotification?.();
      unlistenReminder?.();
      unlistenUpdate?.();
      media.removeEventListener('change', update);
      window.clearInterval(healthTimer);
    };
  });

  async function navigate(destination: string, sourceId = '', category = '') {
    page = destination;
    sourceRequest = { id: sourceId, category, token: sourceRequest.token + 1 };
    activeSource = sourceId;
    activeCategory = category;
    notice = '';
    await showTop();
  }

  // Opens the calendar on one day, optionally with a prefilled new-todo form.
  async function openCalendar(day: string, prefill: TodoPrefill | null = null) {
    await navigate('calendar');
    calendarRequest = { day, token: calendarRequest.token + 1, prefill };
  }

  async function showTop() {
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

  function isStudiumRoom(roomId: unknown): roomId is string {
    return typeof roomId === 'string' && ['19', '5', '6', '11', '15', '16', '18'].includes(roomId);
  }

  // A clicked todo reminder opens the calendar on the todo's day.
  function handleReminderActivation(day: string) {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(day)) return;
    if (!snapshot || showSetup) { pendingReminderDay = day; return; }
    pendingReminderDay = null;
    if (enabled.has('calendar')) void openCalendar(day);
  }

  function handleStudiumActivation(roomId: string) {
    if (!isStudiumRoom(roomId)) return;
    if (!snapshot || showSetup) { pendingStudiumRoom = roomId; return; }
    pendingStudiumRoom = null;
    if (!enabled.has('studium')) return;
    // Native activation already validated the token and persisted the view room.
    // Refresh an open page to read it, with no new seat check or notification.
    studiumActivation++;
    void navigate('studium');
  }

  $effect(() => { if (snapshot) clock.format = snapshot.settings.clock_format; });

  async function load() {
    loading = true;
    error = '';
    try {
      snapshot = await invoke<Snapshot>('get_snapshot');
      theme = snapshot.settings.theme;
      showSetup = !snapshot.onboarding_completed;
      try {
        const proactive = await invoke<ProactiveSnapshot>('proactive_snapshot');
        if (proactive.pending_activation) handleProactiveActivation(proactive.pending_activation);
      } catch { /* The archive remains usable if proactive state is unavailable. */ }
      try { restoreOutcome = await invoke<RestoreOutcome | null>('backup_restore_outcome'); }
      catch { /* A missing report must not block the restored archive. */ }
      if (pendingStudiumRoom) handleStudiumActivation(pendingStudiumRoom);
      if (pendingReminderDay) handleReminderActivation(pendingReminderDay);
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

  function openSetup() {
    error = '';
    notice = '';
    showSetup = true;
  }

  // Finishing and skipping both end the flow; a rerun ends the same way.
  async function finishSetup() {
    error = '';
    try {
      snapshot = await invoke<Snapshot>('complete_onboarding');
      showSetup = false;
      await navigate('today');
      if (pendingStudiumRoom) handleStudiumActivation(pendingStudiumRoom);
      if (pendingReminderDay) handleReminderActivation(pendingReminderDay);
    } catch (reason) { error = message(reason); }
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
    await showTop();
  }

  function checkedLabel(value: string): string {
    const day = archiveDay(value);
    if (!day) return value;
    if (day !== localDayKey(new Date())) return `${Number(day.slice(5, 7))}월 ${Number(day.slice(8, 10))}일`;
    const time = new Date(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(value) ? `${value.replace(' ', 'T')}Z` : value);
    return Number.isNaN(time.getTime()) ? '오늘' : `오늘 ${new Intl.DateTimeFormat('ko-KR', timeOptions(clock.format)).format(time)}`;
  }

  function storageSize(bytes: number) {
    return bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
</script>

<svelte:head><title>Rhythm</title><meta name="description" content="나의 하루를 정돈하는 로컬 퍼스트 개인 커맨드 센터" /></svelte:head>

<a class="skip-link" href="#main">본문으로 건너뛰기</a>
{#if showSetup && snapshot}
<SetupFlow {snapshot} {theme} {busy} {error} onsave={save} ontoggleextension={toggleExtension} onactivation={handleProactiveActivation} onfinish={finishSetup} />
{:else}
<div class="app-shell">
  <aside class="sidebar" aria-label="주 메뉴">
    <div class="brand"><span class="beat" aria-hidden="true"><i></i><i></i><i></i><i></i></span><span>Rhythm</span></div>
    <nav aria-label="기본 탐색">
      <ul class="source-list">
        {#each primaryNavigation as item}
          {@const count = navCount(item.id)}
          <li><button class="source-item" class:active={page === item.id && !activeSource} aria-label={item.label} aria-current={page === item.id && !activeSource ? 'page' : undefined} aria-describedby={count ? `nav-count-${item.id}` : undefined} onclick={() => navigate(item.id)}>
            <Icon name={item.id === 'studium' ? 'boards' : item.id} size={17} /><span class="source-label">{item.label}</span>
            {#if count}<span class="count" aria-hidden="true">{count.count.toLocaleString('ko-KR')}</span><span class="sr-only" id={`nav-count-${item.id}`}>{count.label} {count.count}개</span>{/if}
          </button></li>
        {/each}
      </ul>
      {#each sidebarGroups as group (group.id)}
        <p class="source-group-heading" id={`group-${group.id}`}>{group.name}</p>
        <ul class="source-list" aria-labelledby={`group-${group.id}`}>
          {#each group.feeds as feed (feed.group.id)}
            <li><button class="source-item board" class:active={currentFeed === feed.group.id} aria-current={currentFeed === feed.group.id ? 'page' : undefined} aria-label={feed.group.name} aria-describedby={feed.unread ? `feed-count-${feed.group.id}` : undefined} onclick={() => navigate('inbox', groupScope(feed.group.id))}>
              <span class={`board-swatch family-${feed.family}`} aria-hidden="true"></span><span class="source-label">{feed.group.name}</span>
              {#if feed.unread}<span class="count" aria-hidden="true">{feed.unread.toLocaleString('ko-KR')}</span><span class="sr-only" id={`feed-count-${feed.group.id}`}>읽지 않은 공지 {feed.unread}개</span>{/if}
            </button></li>
          {/each}
          {#each group.organizations as [organization, members] (organization)}
            <!-- An organization named like its section (사이버캠퍼스) would only repeat the heading. -->
            {#if organization !== group.name}<li class="board-organization" aria-hidden="true">{organization}</li>{/if}
            {#each members as board (board.id)}
              {#if board.family === 'cybercampus' && courses.length}
                <!-- Cybercampus is one board; each course is listed as its own feed (its notices' category). -->
                {#each courses as course (course.course_key)}
                  {@const current = scopedCourse === course.name}
                  <li><button class="source-item board" class:active={current} aria-current={current ? 'page' : undefined} aria-label={course.name} aria-describedby={course.unread ? `course-count-${course.course_key}` : undefined} onclick={() => navigate('inbox', board.id, course.name)}>
                    <span class={`board-swatch family-${board.family}`} aria-hidden="true"></span><span class="source-label">{course.name}</span>
                    {#if course.unread}<span class="count" aria-hidden="true">{course.unread.toLocaleString('ko-KR')}</span><span class="sr-only" id={`course-count-${course.course_key}`}>읽지 않은 공지 {course.unread}개</span>{/if}
                  </button></li>
                {/each}
              {:else}
              <li><button class="source-item board" class:active={page === 'inbox' && activeSource === board.id} aria-current={page === 'inbox' && activeSource === board.id ? 'page' : undefined} aria-label={`${organizationLabel(board.organization)} ${board.name}`} aria-describedby={board.unread ? `board-count-${board.id}` : undefined} onclick={() => navigate('inbox', board.id)}>
                <span class={`board-swatch family-${board.family}`} aria-hidden="true"></span><span class="source-label">{board.name}</span>
                {#if board.unread}<span class="count" aria-hidden="true">{board.unread.toLocaleString('ko-KR')}</span><span class="sr-only" id={`board-count-${board.id}`}>읽지 않은 공지 {board.unread}개</span>{/if}
              </button></li>
              {/if}
            {/each}
          {:else}
            {#if !group.feeds.length}<li><button class="source-item quiet" onclick={() => openSettings(group.pane)}><span class="source-label">구독한 게시판 없음 · 설정</span></button></li>{/if}
          {/each}
        </ul>
      {/each}
      <div class="sidebar-bottom">
        <p class="sync-line" role="status">
          {#if syncStatus?.running}<Icon name="refresh" size={14} /><span>{syncStatus.message}{#if syncStatus.active.length === 1 && syncStatus.active[0].page > 0} · {syncStatus.active[0].page}쪽{:else if syncStatus.active.length > 1} · 게시판 {syncStatus.active.length}개{/if}</span>
          {:else if lastChecked}<span>마지막 확인 {checkedLabel(lastChecked)}</span>
          {:else if boards.length}<span>아직 가져온 적 없음</span>{/if}
        </p>
        <button class="source-item" class:active={page === 'settings'} aria-label="설정" aria-current={page === 'settings' ? 'page' : undefined} onclick={() => navigate('settings')}><Icon name="settings" size={17} /><span class="source-label">설정</span></button>
      </div>
    </nav>
  </aside>

  <main id="main" tabindex="-1" class:with-week-pane={weekPane}>
    <header class="page-header"><h1>{scopedCourse ?? scopedGroup?.name ?? scopedBoard?.name ?? heading}</h1>{#if scopedGroup}<p>게시판 {boards.filter((board) => board.group === scopedGroup.id).length}개 · 받은 소식</p>{:else if scopedBoard}<p>{organizationLabel(scopedBoard.organization)} · 받은 소식</p>{/if}{#if page === 'today'}<p>{date}{#if summary}{' · '}읽지 않은 공지 {summary.unread.toLocaleString('ko-KR')}{#if summary.new_since_sync}{' · '}최근 동기화로 새 공지 {summary.new_since_sync.toLocaleString('ko-KR')}{/if}{/if}</p>{/if}</header>
    {#if !desktop && !loading}<div class="banner browser-notice" role="status"><Icon name="info" /><div><strong>데스크톱 앱에서 시작해 주세요</strong><p>브라우저에서는 화면만 둘러볼 수 있습니다. 설정 저장과 로컬 데이터 접근은 Rhythm 앱에서 사용할 수 있습니다.</p></div></div>{/if}
    {#if error}<div class="banner error" role="alert" transition:slide><Icon name="info" /><div><strong>작업을 완료하지 못했습니다</strong><p>{error}</p></div>{#if !snapshot && desktop}<button class="text-button" onclick={load}>다시 시도</button>{/if}</div>{/if}
    {#if restoreOutcome}<div class="banner" role="status" transition:slide><Icon name="info" /><div><strong>백업 복원을 마쳤습니다</strong><p>공지 {restoreOutcome.documents.toLocaleString('ko-KR')}건과 첨부 자료 {restoreOutcome.resources.toLocaleString('ko-KR')}개를 복원하고 무결성을 확인했습니다.{#if restoreOutcome.unavailable_resources} 백업 당시 내려받지 못한 첨부 자료 {restoreOutcome.unavailable_resources.toLocaleString('ko-KR')}개는 포함되지 않았습니다.{/if}{#if restoreOutcome.models_needing_reinstall} 사용할 수 없는 AI 모델 {restoreOutcome.models_needing_reinstall.toLocaleString('ko-KR')}개는 다시 설치해 주세요.{/if} 이전 데이터는 데이터 폴더에 복구용으로 보관되어 있습니다.</p></div><button class="text-button" onclick={() => restoreOutcome = null}>닫기</button></div>{/if}
    {#if desktop && availableUpdate && dismissedUpdate !== availableUpdate.version && page !== 'settings'}<div class="banner" role="status" transition:slide><Icon name="info" /><div><strong>Rhythm {availableUpdate.version} 버전을 설치할 수 있습니다</strong><p>설정 → 데이터 및 정보에서 내려받으면 업데이트 서명을 확인한 뒤 설치 여부를 묻습니다. 그 전에는 아무것도 바뀌지 않습니다.</p></div><button class="text-button" onclick={updateSettings}>업데이트 보기</button><button class="text-button" onclick={() => dismissedUpdate = availableUpdate?.version ?? ''}>나중에</button></div>{/if}
    <div class="save-notice" role="status" aria-live="polite">{#if notice}<span class="save-notice-body" transition:fly={{ y: 8 }}><Icon name="check" size={15} />{notice}</span>{/if}</div>

    {#if page === 'settings'}
      <div class="settings-layout" in:fade>
        <nav class="settings-nav" aria-label="설정 항목">
          <ul>
            {#each panes as item (item.id)}
              <li><button class="pane-item" class:active={settingsPane === item.id} aria-current={settingsPane === item.id ? 'page' : undefined} onclick={() => settingsPane = item.id}><Icon name={item.icon} size={17} /><span>{item.label}</span></button></li>
            {/each}
          </ul>
        </nav>
        <section class="settings-pane" aria-labelledby="settings-pane-title" aria-busy={busy || loading}>
          <h2 id="settings-pane-title" class="pane-title" tabindex="-1">{paneLabel}</h2>
          {#if loading}<p class="loading" role="status">저장된 설정을 불러오는 중입니다…</p>{/if}
          {#key settingsPane}<div in:fade>
          {#if settingsPane === 'general'}
            <div class="settings-group">
              <div class="settings-row"><span class="row-label">화면 테마</span><div class="segmented" role="group" aria-label="화면 테마">{#each themes as option}<button disabled={!snapshot || busy} class:selected={theme === option.id} aria-pressed={theme === option.id} onclick={() => save({ theme: option.id })}><Icon name={option.icon} size={15} />{option.label}</button>{/each}</div></div>
              <div class="settings-row"><span class="row-label">시간 표시<span class="row-detail">{clock.format === '12h' ? '오후 9:30' : '21:30'}</span></span><div class="segmented" role="group" aria-label="시간 표시">{#each clockFormats as option}<button disabled={!snapshot || busy} class:selected={clock.format === option.id} aria-pressed={clock.format === option.id} onclick={() => save({ clock_format: option.id })}>{option.label}</button>{/each}</div></div>
              <div class="settings-row"><span class="row-label">전원 모드<Hint label="전원 모드">사이버캠퍼스 자동 동기화 간격에 적용됩니다. 절전은 60분마다, 균형은 30분, 성능은 15분마다 확인합니다. 학교 게시판 공지 피드는 전원 모드와 관계없이 30분마다 받습니다.</Hint></span><div class="segmented" role="group" aria-label="전원 모드">{#each energyProfiles as option}<button disabled={!snapshot || busy} class:selected={snapshot?.settings.energy_profile === option.id} aria-pressed={snapshot?.settings.energy_profile === option.id} onclick={() => save({ energy_profile: option.id })}>{option.label}</button>{/each}</div></div>
            </div>
            <div class="settings-group">
              <div class="settings-row"><span class="row-label">초기 설정<Hint label="초기 설정">처음 실행할 때 본 설정 단계를 다시 엽니다. 현재 설정을 그대로 보여 주며 아무것도 초기화하지 않습니다.</Hint></span><button class="button secondary small" disabled={!snapshot || busy} onclick={openSetup}>다시 실행</button></div>
            </div>
          {:else if settingsPane === 'extensions'}
            {#if snapshot}<ExtensionSettings extensions={snapshot.extensions} {busy} ontoggle={toggleExtension} />
            {:else if !loading}<p class="muted">데스크톱 앱에서 확장 기능을 켤 수 있습니다.</p>{/if}
          {:else if settingsPane === 'boards'}
            {#if desktop}<FeedSettings enabled={noticesEnabled} />{/if}
            <SourceSettings enabled={noticesEnabled} extension="sogang_notices" />
          {:else if settingsPane === 'cybercampus'}
            <SourceSettings enabled={enabled.has('cybercampus')} extension="cybercampus" />
          {:else if settingsPane === 'ai'}
            <AiSettings {desktop} />
          {:else if settingsPane === 'personalization'}
            <PersonalizationSettings {desktop} />
          {:else if settingsPane === 'notifications'}
            <ProactiveSettings {desktop} part="background" onopen={openProactiveDocument} onactivation={handleProactiveActivation} />
            <ProactiveSettings {desktop} part="notifications" onopen={openProactiveDocument} onactivation={handleProactiveActivation} />
          {:else}
            <div class="settings-group">
              <div class="settings-row"><span class="row-label"><label for="auto-trash">오래된 공지 정리</label><Hint label="오래된 공지 정리">게시일이 선택한 기간보다 오래된 공지를 휴지통으로 옮깁니다. 저장한 공지는 옮기지 않습니다. 휴지통의 공지는 내용이 그대로 남아 언제든 복원할 수 있고, 저절로 영구 삭제되지 않습니다. 설정을 바꿀 때, 동기화가 끝날 때, 앱이 열려 있는 동안 1시간마다 확인합니다.</Hint></span><select id="auto-trash" value={snapshot?.settings.auto_trash_months ?? 0} disabled={!snapshot || busy} onchange={(event) => save({ auto_trash_months: Number(event.currentTarget.value) as AutoTrashMonths })}>{#each autoTrashChoices as choice}<option value={choice.months}>{choice.label}</option>{/each}</select></div>
            </div>
            <div class="settings-group">
              <div class="settings-row"><span class="row-label">로컬 데이터<Hint label="로컬 데이터">프로필, 관심사, 피드백과 공지는 이 기기의 SQLite{#if snapshot} (스키마 {snapshot.storage.schema_version}){/if}에만 저장합니다. 클라우드 저장소, 원격 추론, 사용 정보 수집은 없습니다.{#if snapshot}<code class="popover-code">{snapshot.storage.path}</code>{/if}</Hint></span><span class="value">{snapshot ? storageSize(snapshot.storage.bytes) : '—'}</span></div>
            </div>
            <BackupSettings {desktop} />
            <UpdaterSettings {desktop} />
          {/if}
          </div>{/key}
        </section>
      </div>
    {:else if page === 'tasks'}
      <CybercampusTasks {desktop} onsettings={() => openSettings('cybercampus')}
        oncalendar={enabled.has('calendar') ? (prefill) => void openCalendar(prefill.todo.day, prefill) : undefined} />
    {:else if page === 'studium'}
      <StudiumView {desktop} activation={studiumActivation} proactiveEnabled={enabled.has('proactive')} onactivation={handleProactiveActivation} onsettings={() => openSettings(enabled.has('proactive') ? 'notifications' : 'extensions')} />
    {:else}
      <ArchiveView {page} {desktop} {enabled} onsetup={openSetup} onextensions={() => openSettings('extensions')} {sourceRequest} activation={proactiveActivation} onactivationhandled={() => proactiveActivation = null} onsettings={(pane) => openSettings(pane ?? 'extensions')}
        onsourcechange={(id, category) => { activeSource = id; activeCategory = category; }}
        onweekpane={(visible) => weekPane = visible}
        ontasks={() => void navigate('tasks')}
        {calendarRequest}
        oncalendar={(day, prefill) => void openCalendar(day, prefill)}
        oncontext={(context) => { if (context.summary) summary = context.summary; if (context.sources) sources = context.sources; if (context.groups) sourceGroups = context.groups; syncStatus = context.status; }} />
    {/if}
  </main>
</div>
{/if}
