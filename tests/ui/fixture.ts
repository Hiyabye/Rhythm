import type { Page } from '@playwright/test';
import type { CalendarTodo, ClockFormat, NoticeDate, CybercampusTask, DocumentItem, PersonalInterest, ResourceRecord, ProactiveHistoryItem, ProactiveSettings, ProactiveActivation, StudiumMiniStatus, StudiumSeatLayout, StudiumSnapshot, Theme } from '../../src/lib/types';

// Invented regular marker origins, unrelated to the school's real floor plans.
export function syntheticStudiumLayout(total: number): StudiumSeatLayout {
  const columns = Math.min(16, Math.ceil(Math.sqrt(total * 1.3)));
  const points = Array.from({ length: total }, (_, index) => ({ number: index + 1, x: (index % columns) * 36_000, y: Math.floor(index / columns) * 36_000 }));
  return { width: Math.max(...points.map((point) => point.x)), height: Math.max(...points.map((point) => point.y)), points };
}

export interface FixtureOptions {
  clockFormat?: ClockFormat;
  studium?: Partial<StudiumSnapshot>;
  studiumSaveError?: boolean;
  studiumStartError?: boolean;
  studiumAlertSaveError?: boolean;
  pendingStudiumActivation?: string;
  studiumActivationGate?: boolean;
  studiumSnapshotError?: boolean;
  mini?: Partial<StudiumMiniStatus>;
  miniOpenError?: boolean;
  miniOnTopError?: boolean;
  miniCloseError?: boolean;
  miniShowMainError?: boolean;
  miniSettingsGate?: boolean;
  count?: number;
  saved?: boolean;
  queryError?: boolean;
  readError?: boolean;
  settingsError?: boolean;
  // Controls `sogang_notices` alone; `extensions` overrides any extension.
  enabled?: boolean;
  // Per-extension on/off; Studium stays opt-in while existing extensions default on.
  extensions?: Record<string, boolean>;
  body?: string;
  unsupportedBody?: boolean;
  imageError?: boolean;
  imageDelay?: number;
  queryGate?: boolean;
  // A retired Qwen3.5 4B summary model is still installed.
  aiInstalled?: boolean;
  aiEmbeddingInstalled?: boolean;
  aiClassificationLow?: boolean;
  // Notice 2 is a cross-post of notice 1 and notice 3 is only similar; needs count >= 3.
  aiSimilar?: boolean;
  // With aiEmbeddingInstalled, relevance search adds one meaning-only notice.
  aiSemanticSearch?: boolean;
  aiRetiredModel?: boolean;
  // A fresh profile that has not finished the first-run setup flow.
  onboardingNeeded?: boolean;
  // Windows-flavored defaults (paths); pair with a Windows user agent.
  windows?: boolean;
  aiModelDirectoryGate?: boolean;
  aiModelStorageAvailable?: number | null;
  sourceCount?: number;
  // Replaces the default two-board catalog.
  sourceList?: { id: string; name: string; organization: string; supported: boolean; subscribed?: boolean; group?: string | null; family?: string; last_error?: string | null; skipped_count?: number; skipped?: { external_id: string; url: string; reason: string; skipped_at: string; archived: boolean }[] }[];
  sourceDelay?: number;
  initialNotificationsEnabled?: boolean;
  capabilitiesError?: boolean;
  aiExecutionMode?: 'auto' | 'cpu' | 'gpu';
  aiExecutionError?: boolean;
  aiModelDirectory?: string;
  aiModelDirectoryChoice?: string | null;
  aiModelDirectoryError?: boolean;
  aiModelStorageError?: string;
  aiRemoveError?: boolean;
  aiInstallError?: boolean;
  aiDelay?: number;
  revisions?: boolean;
  aiLowResources?: boolean;
  initialInterests?: PersonalInterest[];
  feedbackWriteDelay?: number;
  profileSaveDelay?: number;
  interestSaveDelay?: number;
  interestRemovalDelay?: number;
  personalizationResetDelay?: number;
  proactiveSaveError?: boolean;
  proactiveClearError?: boolean;
  lifecycleError?: boolean;
  schedulerError?: boolean;
  pendingActivation?: ProactiveActivation;
  notificationAuthorization?: string;
  notificationRequestResult?: string;
  notificationTestRequested?: boolean;
  restoreSupportError?: string;
  notificationRequestError?: boolean;
  backupInspectError?: boolean;
  backupRestoreError?: boolean;
  restoreOutcome?: { documents: number; resources: number; unavailable_resources: number; models_needing_reinstall: number };
  // Real builds embed no update key; only explicit tests exercise a configured channel.
  updaterConfigured?: boolean;
  dialogError?: boolean;
  updaterCheckError?: string;
  updaterInstallError?: boolean;
  updaterRestartError?: boolean;
  updateAvailable?: boolean;
  // Automatic update checks: the stored preference (default on) and whether a
  // check already found 1.1.1 before the window subscribed.
  autoUpdateCheck?: boolean;
  autoUpdateFound?: boolean;
  // Saved Cybercampus login ID; `cybercampus_save_login` accepts only the password "right".
  cybercampusId?: string | null;
  cybercampusTasks?: CybercampusTask[];
  calendarTodos?: CalendarTodo[];
  // Dates found in notice text; `saved: false` ones appear on the calendar only as deadlines with every notice's deadlines on.
  noticeDates?: (NoticeDate & { saved?: boolean })[];
  calendarAllDeadlines?: boolean;
  // Nested board groups; sources name theirs through `group`.
  sourceGroups?: { id: string; name: string; parent: string | null; enabled?: boolean }[];
}

// Only the IPC boundary is faked: tests exercise the production Svelte build,
// DOM reconstruction, reactivity and browser keyboard/layout behavior.
export async function mockDesktop(page: Page, options: FixtureOptions = {}) {
  await page.addInitScript(({ options }: { options: FixtureOptions }) => {
    const copy = <T,>(value: T): T => JSON.parse(JSON.stringify(value));
    const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
    const entries: DocumentItem[] = Array.from({ length: options.count ?? 2 }, (_, index) => ({
      id: index + 1, source_id: 'computing-updates', source_name: '소프트웨어융합대학 소식',
      title: `한국어 공지 ${String(index + 1).padStart(2, '0')}`, author: '교학팀', category: index % 2 ? '학사' : '장학',
      published_at: '2026-10-24', collected_at: '2026-10-24 12:00:00', is_read: false,
      saved: options.saved ?? false, historical: true, snippet: '컴퓨터 Computer 장학금 경제 공지 본문', content_status: 'complete',
      trashed: false,
    }));
    let sources = [{ id: 'computing-updates', name: '소식', organization: '소프트웨어융합대학',
      description: '학교 공개 소식', url: 'https://computing.sogang.ac.kr/ko/community/news/list', family: 'computing', supported: true,
      subscribed: false, last_attempt: null, last_success: null, last_error: null, next_page: 1, history_complete: false,
      status: 'limited', discovered: 0, archived: 0, failed: 0, skipped_count: 0, skipped: [] as unknown[] },
    { id: 'unsupported-board', name: '미검증 게시판', organization: '다른 학과', description: '아직 검증되지 않은 게시판',
      url: 'https://example.invalid', family: 'cms', supported: false, subscribed: false, last_attempt: null, last_success: null,
      last_error: null, next_page: 1, history_complete: false, status: 'idle', discovered: 0, archived: 0, failed: 0, skipped_count: 0, skipped: [] as unknown[] }];
    if (options.sourceList) {
      sources = options.sourceList.map((source) => ({ ...sources[0], family: 'computing', subscribed: false, last_error: null, ...source }));
    }
    for (const source of sources) Object.assign(source, { extension: source.family === 'cybercampus' ? 'cybercampus' : 'sogang_notices', group: (source as { group?: string | null }).group ?? null, selected: source.subscribed });
    let sourceGroups = (options.sourceGroups ?? []).map((group) => ({ enabled: true, ...group }));
    // As the store: a grouped board is subscribed only while every enclosing group is on.
    const groupOn = (id: string | null): boolean => {
      const group = sourceGroups.find((item) => item.id === id);
      return !group || (group.enabled && groupOn(group.parent));
    };
    const reconcile = () => { sources = sources.map((source) => ({ ...source, subscribed: source.selected && groupOn(source.group) })); };
    if ((options.sourceCount ?? 0) > sources.length) {
      sources.push(...Array.from({ length: options.sourceCount! - sources.length }, (_, index) => ({
        ...sources[0], id: `extra-source-${index}`, name: `추가 정보원 ${index + 1}`,
      })));
    }
    const proactiveSettings: ProactiveSettings = { close_to_background: false, launch_at_login: false, start_hidden: false,
      scheduler_enabled: false, scheduler_paused: false, scheduler_interval_minutes: 0, scheduler_retry_minutes: 15,
      notifications_enabled: options.initialNotificationsEnabled ?? false, notification_sensitivity: 'important',
      quiet_hours: { enabled: false, start: '22:00', end: '07:00' }, updated_at: '2026-10-27T12:00:00Z' };
    const scheduler = { enabled: false, paused: false, interval_minutes: 60, retry_minutes: 15,
      notifications_enabled: false, notification_sensitivity: 'important', quiet_hours: copy(proactiveSettings.quiet_hours),
      pending_events: 1, due_tasks: 0, next_task_at: null as string | null, last_run_at: '2026-10-27T12:00:00Z',
      last_outcome: options.schedulerError ? 'failure' as string : 'success' as string, last_error: options.schedulerError ? '자동 동기화에 실패했습니다.' : null as string | null, consecutive_failures: options.schedulerError ? 1 : 0 };
    let history: ProactiveHistoryItem[] = [{ id: 12, document_id: 1, revision_id: 1, event_type: 'new', state: 'attempted',
      attempt_count: 1, claimed_at: null, last_attempt_at: '2026-10-27T12:00:00Z', deferred_until: null,
      delivered_at: null, error: null, reason: null, created_at: '2026-10-27T12:00:00Z',
      updated_at: '2026-10-27T12:00:01Z', title: entries[0]?.title ?? '한국어 공지 01', category: '장학', source_id: 'computing-updates' }];
    let pendingActivation = options.pendingActivation ? copy(options.pendingActivation) : null;
    let notificationAuthorization = options.notificationAuthorization ?? 'not_determined';
    const proactiveSnapshot = (consumeActivation = false) => {
      const snapshot = { settings: copy(proactiveSettings), scheduler: copy(scheduler), history: copy(history),
        notification_permission: 'unknown' as const, pending_activation: copy(pendingActivation) };
      if (consumeActivation) pendingActivation = null;
      return snapshot;
    };
    const snapshot = { settings: { theme: 'light', energy_profile: 'eco', interests: '', clock_format: options.clockFormat ?? '12h', auto_trash_months: 0 },
      extensions: ([['sogang_notices', '서강대학교 공지'], ['cybercampus', '사이버캠퍼스'], ['search', '탐색'], ['saved', '보관함'],
        ['recommendations', '나를 위한 소식'], ['week', '앞으로 7일'], ['calendar', '캘린더'], ['ai', 'AI 분류'], ['proactive', '알림·백그라운드'], ['studium', '열람실']] as const).map(([id, display_name]) => ({
        id, display_name, description: `${display_name} 설명`, available: true,
        enabled: options.extensions?.[id] ?? (id === 'sogang_notices' ? options.enabled ?? true : id !== 'studium') })),
      storage: { path: '/isolated/rhythm.sqlite3', bytes: 4096, schema_version: 5 },
      proactive: copy(proactiveSettings),
      onboarding_completed: !options.onboardingNeeded };
    const studium: StudiumSnapshot = {
      rooms: [
        { id: '19', name: '111 로욜라 인벤툼Ⅰ', label: '로욜라 인벤툼 I', capacity: 113 },
        { id: '5', name: '133 로욜라 인벤툼 II', label: '로욜라 인벤툼 II', capacity: 102 },
        { id: '6', name: 'K관 열람실', label: 'K관 열람실', capacity: 311 },
        { id: '11', name: 'PA관 열람실', label: 'PA관 열람실', capacity: 101 },
        { id: '15', name: 'J관 일반열람실', label: 'J관 일반열람실', capacity: 207 },
        { id: '16', name: 'J관 노트북전용실', label: 'J관 노트북전용실', capacity: 92 },
        { id: '18', name: 'MA관 열람실', label: 'MA관 열람실', capacity: 174 },
      ],
      preferences: { room_ids: ['5'], view_room: '5' }, job_id: null, running: false, results: [],
      automatic: { active: false, interval_minutes: 5, next_check_at: null, error: null },
      favorites: Object.fromEntries(['19', '5', '6', '11', '15', '16', '18'].map((id) => [id, []])),
      alert_rules: Object.fromEntries(['19', '5', '6', '11', '15', '16', '18'].map((id) => [id, { enabled: false, kind: 'remaining' as const, minimum: 1, version: 0 }])),
      alert_error: null,
      ...copy(options.studium ?? {}),
    };
    const studiumStates = (status: 'not_checked' | 'loading' | 'cancelled') => studium.preferences.room_ids.map((room_id) => ({ room_id, status, result: null, error: null, checked_at: null }));
    if (!options.studium?.results) studium.results = studiumStates('not_checked');
    type Board = { source_id: string; mode: string; page: number; message: string };
    let sync = { running: false, mode: 'historical', message: '수집 대기 중입니다.', active: [] as Board[] };
    const board = (mode: string, page = 1, source_id = 'computing-updates'): Board => ({ source_id, mode, page, message: '공개 공지를 가져오는 중입니다.' });
    const separator = options.windows ? '\\' : '/';
    const defaultModelDirectory = options.windows ? 'C:\\Rhythm-test\\models' : '/isolated/models';
    const chosenModelDirectory = options.windows ? 'D:\\로컬 모델\\Rhythm' : '/Volumes/로컬 모델/Rhythm';
    const modelStorage = { directory: options.aiModelDirectory ?? defaultModelDirectory, custom: Boolean(options.aiModelDirectory), available_bytes: (options.aiModelStorageAvailable === undefined ? 27000000000 : options.aiModelStorageAvailable) as number | null, error: options.aiModelStorageError ?? null as string | null };
    const modelNames: Record<string, string> = { 'qwen35-2b': 'Qwen3.5 2B', 'qwen35-4b': 'Qwen3.5 4B', 'qwen35-9b': 'Qwen3.5 9B', 'qwen3-embedding-0-6b': 'Qwen3-Embedding 0.6B' };
    const model = (id = 'qwen35-4b', kind: 'generation' | 'embedding' = 'generation') => ({ id, catalog_id: id, name: modelNames[id] ?? id, ownership: 'managed', kind, path: `models/${id}.gguf`, storage_directory: null as string | null, sha256: 'a'.repeat(64), expected_sha256: 'a'.repeat(64), expected_bytes: kind === 'embedding' ? 639150592 : 2740937888, bytes: kind === 'embedding' ? 639150592 : 2740937888, source_revision: 'pinned-revision', source_filename: `${id}.gguf`, status: 'installed', downloaded_bytes: kind === 'embedding' ? 639150592 : 2740937888, resume_etag: null, error: null as string | null, created_at: '2026-10-24 12:00:00', updated_at: '2026-10-24 12:00:00' });
    const embeddingCatalog = { id: 'qwen3-embedding-0-6b', kind: 'embedding', name: 'Qwen3-Embedding 0.6B', parameters: '0.6B', quantization: 'Q8_0', bytes: 639150592, memory_bytes: 2147483648, description: '공지 분류용 임베딩 모델', use_case: '공지 주제 분류', repository: 'Qwen/Qwen3-Embedding-0.6B-GGUF', revision: 'pinned-revision', filename: 'Qwen3-Embedding-0.6B-Q8_0.gguf', sha256: 'd'.repeat(64), license: 'Apache-2.0', compatibility: 'verified', reason: null, pooling: 'last', embedding_dim: 1024 };
    const ai = { model_storage: modelStorage, catalog: [embeddingCatalog], models: [...(options.aiInstalled ? [model()] : []), ...(options.aiRetiredModel ? [model('qwen35-9b')] : []), ...(options.aiEmbeddingInstalled ? [model('qwen3-embedding-0-6b', 'embedding')] : [])], preferences: { execution_mode: options.aiExecutionMode ?? 'auto', model_directory: options.aiModelDirectory ?? null as string | null, embedding_model_id: options.aiEmbeddingInstalled ? 'qwen3-embedding-0-6b' : null as string | null, classification_enabled: true }, hardware: { os: 'macOS', arch: 'arm64', ram_bytes: options.aiLowResources ? 4294967296 : 17179869184, disk_available_bytes: options.aiLowResources ? 0 : 27000000000, backend: 'cpu' }, busy: false, classification: { state: options.aiEmbeddingInstalled ? 'idle' : 'disabled', processed: 0 } };
    const classifications = entries.map((item) => ({
      document_id: item.id, revision_id: 1, category: '장학',
      low_confidence: Boolean(options.aiClassificationLow), created_at: '2026-10-24 12:00:00',
    }));
    const similarNotice = (id: number, duplicate: boolean) => ({ document_id: id, source_id: entries[id - 1].source_id, source_name: id === 2 ? '공과대학 공지' : entries[id - 1].source_name, title: entries[id - 1].title, published_at: entries[id - 1].published_at, duplicate });
    const similar: Record<number, ReturnType<typeof similarNotice>[]> = options.aiSimilar && options.aiEmbeddingInstalled ? { 1: [similarNotice(2, true), similarNotice(3, false)], 2: [similarNotice(1, true)], 3: [similarNotice(1, false)] } : {};
    let personalization = { profile: { university: '', department: '', study_level: '' }, interests: copy(options.initialInterests ?? []), learning_enabled: true };
    const documentFeedback = new Map<number, string>();
    let lifecycle = { close_to_background: false, start_hidden: false, launch_at_login: false, autostart_enabled: false as boolean | null, tray_available: true };
    const callbacks = new Map<number, (event: { event: string; id: number; payload: unknown }) => void>();
    let callbackId = 0;
    let eventId = 0;
    let updaterHandler: number | null = null;
    let availableHandler: number | null = null;
    let autoUpdateCheck = options.autoUpdateCheck ?? true;
    let notificationHandler: number | null = null;
    let studiumNotificationHandler: number | null = null;
    let pendingStudiumActivation = options.pendingStudiumActivation ?? null;
    const stateListeners = new Map<number, number>();
    const themeListeners = new Map<number, number>();
    const clockListeners = new Map<number, number>();
    const mini: StudiumMiniStatus = { open: new URLSearchParams(location.search).get('studium-mini') === '1', always_on_top: false, ...copy(options.mini ?? {}) };
    const emitStudiumState = () => {
      for (const [id, handler] of stateListeners) callbacks.get(handler)?.({ event: 'studium://state-changed', id, payload: null });
    };
    const emitTheme = (value: Theme) => {
      snapshot.settings.theme = value;
      for (const [id, handler] of themeListeners) callbacks.get(handler)?.({ event: 'rhythm://theme-changed', id, payload: value });
    };
    const emitClockFormat = (value: ClockFormat) => {
      snapshot.settings.clock_format = value;
      for (const [id, handler] of clockListeners) callbacks.get(handler)?.({ event: 'rhythm://clock-format-changed', id, payload: value });
    };
    const fixture = { ai, options, entries, studium, mini, emitStudiumState, emitTheme, emitClockFormat, studiumStateListenerCount: () => stateListeners.size, themeListenerCount: () => themeListeners.size, clockListenerCount: () => clockListeners.size, miniSettingsResolvers: [] as Array<() => void>, releaseMiniSettings() { this.miniSettingsResolvers.splice(0).forEach((release) => release()); }, studiumSnapshotGate: false, studiumSnapshotResolvers: [] as Array<() => void>, releaseStudiumSnapshots() { this.studiumSnapshotResolvers.splice(0).forEach((release) => release()); }, studiumStatusDelay: 0, studiumSaveError: options.studiumSaveError ?? false, studiumAlertSaveError: options.studiumAlertSaveError ?? false, studiumCancelRequested: null as number | null, studiumJobAutomatic: studium.automatic.active && studium.running, calls: [] as { command: string; args: any }[], sourceDelay: options.sourceDelay ?? 0,
      finishStudiumCancel() {
        if (this.studiumCancelRequested !== studium.job_id || !studium.running) return;
        studium.running = false;
        studium.results = studium.results.map((value) => value.status === 'loading' ? { ...value, status: 'cancelled', result: null } : value);
        this.studiumCancelRequested = null;
        this.studiumJobAutomatic = false;
        if (studium.automatic.active) studium.automatic.next_check_at = new Date(Date.now() + studium.automatic.interval_minutes * 60_000).toISOString();
        emitStudiumState();
      },
      finishStudiumCycle(results: StudiumSnapshot['results']) {
        studium.results = copy(results);
        studium.running = false;
        this.studiumJobAutomatic = false;
        if (studium.automatic.active) studium.automatic.next_check_at = new Date(Date.now() + studium.automatic.interval_minutes * 60_000).toISOString();
        emitStudiumState();
      },
      setNotificationAuthorization: (status: string) => { notificationAuthorization = status; },
      setNotificationSettings: (patch: Partial<ProactiveSettings>) => { Object.assign(proactiveSettings, copy(patch)); },
      studiumActivationResolvers: [] as Array<() => void>,
      studiumActivationCompleted: false,
      releaseStudiumActivation() { this.studiumActivationResolvers.splice(0).forEach((release) => release()); },
      modelDirectoryResolvers: [] as Array<() => void>,
      releaseModelDirectory() { this.modelDirectoryResolvers.splice(0).forEach((release) => release()); },
      dialogs: [] as string[], dialogAnswers: [] as ('ok' | 'cancel')[], trashCalls: [] as { command: string; ids: number[] | null }[],
      stateDelay: 0, subscribeError: false, snapshotFailures: 0, retryMessage: '자료를 내려받지 못했습니다. 연결을 확인해 주세요.',
      queryResolvers: [] as Array<() => void>,
      emitUpdater: (payload: unknown) => {
        if (updaterHandler !== null) callbacks.get(updaterHandler)?.({ event: 'updater://progress', id: ++eventId, payload: copy(payload) });
      },
      emitAvailable: (payload: unknown) => {
        if (availableHandler !== null) callbacks.get(availableHandler)?.({ event: 'updater://available', id: ++eventId, payload: copy(payload) });
      },
      releaseQueries() { this.queryResolvers.splice(0).forEach((release) => release()); },
      emitNotification: (payload: { event_id: number; document_id: number; revision_id: number; attempt_count?: number }, keepPending = false) => {
        const activation = { ...copy(payload), attempt_count: payload.attempt_count ?? 1, event_ids: [payload.event_id] };
        if (keepPending) pendingActivation = copy(activation);
        if (notificationHandler !== null) callbacks.get(notificationHandler)?.({ event: 'proactive://notification-clicked', id: eventId, payload: activation });
      },
      emitStudiumNotification: (roomId: string, keepPending = false) => {
        if (studium.preferences.room_ids.includes(roomId) && snapshot.extensions.some((extension) => extension.id === 'studium' && extension.enabled)) studium.preferences.view_room = roomId;
        if (keepPending) pendingStudiumActivation = roomId;
        if (studiumNotificationHandler !== null) callbacks.get(studiumNotificationHandler)?.({ event: 'studium://notification-clicked', id: ++eventId, payload: roomId });
      },
      completeSync: () => { sync = { ...sync, running: false, message: '가져오기를 마쳤습니다.', active: [] }; },
      syncMode: (mode: string, page = 1) => { sync = { ...sync, running: true, mode, active: [board(mode, page)] }; },
      syncBoards: (ids: string[]) => { sync = { ...sync, running: true, mode: 'refresh', active: ids.map((id) => board('historical', 1, id)) }; } };
    let calendarTodos = copy(options.calendarTodos ?? []);
    const noticeDates = copy(options.noticeDates ?? []);
    let calendarAllDeadlines = options.calendarAllDeadlines ?? false;
    const noticeDate = ({ saved: _, ...date }: NoticeDate & { saved?: boolean }): NoticeDate => date;
    let calendarId = Math.max(0, ...calendarTodos.map((todo) => todo.id));
    const cybercampus = { id: options.cybercampusId ?? null, rejected: false, courses: options.cybercampusId ? [{ course_key: 'A1', name: '자료구조실습', year: 2026, term: '3' }] : [] };
    Object.assign(window, { isTauri: true, __fixture: fixture, __TAURI_EVENT_PLUGIN_INTERNALS__: {
      unregisterListener: (event: string, id: number) => {
        if (event === 'updater://progress') updaterHandler = null;
        if (event === 'updater://available') availableHandler = null;
        if (event === 'proactive://notification-clicked') notificationHandler = null;
        if (event === 'studium://notification-clicked') studiumNotificationHandler = null;
        if (event === 'studium://state-changed') stateListeners.delete(id);
        if (event === 'rhythm://theme-changed') themeListeners.delete(id);
        if (event === 'rhythm://clock-format-changed') clockListeners.delete(id);
      },
    }, __TAURI_INTERNALS__: {
      transformCallback: (callback: (event: { event: string; id: number; payload: unknown }) => void) => {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      invoke: async (command: string, args: any = {}) => {
        fixture.calls.push({ command, args: copy(args) });
        switch (command) {
          case 'plugin:event|listen':
            if (args.event === 'updater://progress') updaterHandler = args.handler;
            if (args.event === 'updater://available') availableHandler = args.handler;
            if (args.event === 'proactive://notification-clicked') notificationHandler = args.handler;
            if (args.event === 'studium://notification-clicked') studiumNotificationHandler = args.handler;
            if (args.event === 'studium://state-changed') stateListeners.set(eventId + 1, args.handler);
            if (args.event === 'rhythm://theme-changed') themeListeners.set(eventId + 1, args.handler);
            if (args.event === 'rhythm://clock-format-changed') clockListeners.set(eventId + 1, args.handler);
            eventId += 1;
            return eventId;
          case 'plugin:event|unlisten':
            if (args.event === 'updater://progress') updaterHandler = null;
            if (args.event === 'updater://available') availableHandler = null;
            if (args.event === 'proactive://notification-clicked') notificationHandler = null;
            if (args.event === 'studium://notification-clicked') studiumNotificationHandler = null;
            if (args.event === 'studium://state-changed') stateListeners.delete(args.eventId);
            if (args.event === 'rhythm://theme-changed') themeListeners.delete(args.eventId);
            if (args.event === 'rhythm://clock-format-changed') clockListeners.delete(args.eventId);
            return null;
          case 'plugin:dialog|message': {
            if (options.dialogError) throw '대화상자를 열 권한이 없습니다.';
            fixture.dialogs.push(args.message);
            // Unanswered confirmations are cancelled, like dismissing a native alert.
            const ok = fixture.dialogAnswers.shift() === 'ok';
            const custom = args.buttons?.OkCancelCustom as [string, string] | undefined;
            return custom ? custom[ok ? 0 : 1] : ok ? 'Ok' : 'Cancel';
          }
          case 'plugin:dialog|open': return '/fixtures/Rhythm-backup.rhythm-backup';
          case 'plugin:dialog|save': return '/fixtures/Rhythm-backup.rhythm-backup';
          case 'backup_restore_support':
            if (options.restoreSupportError) throw options.restoreSupportError;
            return null;
          case 'platform_capabilities': {
            if (options.capabilitiesError) throw '사용 가능 여부를 확인하지 못했습니다.';
            const granted = ['authorized', 'granted', 'provisional', 'ephemeral'].includes(notificationAuthorization);
            return {
              notifications: { available: granted, reason: granted ? null : '운영체제의 알림 설정에서 Rhythm 알림을 허용해 주세요.' },
              restore: { available: !options.restoreSupportError, reason: options.restoreSupportError ?? null },
              updater: { available: Boolean(options.updaterConfigured), reason: options.updaterConfigured ? null : '새 버전을 내려받아 직접 설치합니다.' },
            };
          }
          case 'backup_inspect':
            if (options.backupInspectError) throw '백업 파일의 형식이 올바르지 않습니다.';
            return { format_version: 1, created_at: '1793404800', database_schema: 6, documents: 12, resources: 3,
              resource_bytes: 1536, includes_model_weights: false };
          case 'backup_restore':
            if (options.backupRestoreError) throw '백업 데이터를 복원하지 못했습니다.';
            return null;
          case 'backup_cancel': return true;
          case 'backup_restore_outcome': return options.restoreOutcome ?? null;
          case 'updater_configured': return options.updaterConfigured ?? false;
          case 'updater_check':
            if (options.updaterCheckError) throw options.updaterCheckError;
            return options.updateAvailable === false ? null : { version: '1.1.1', currentVersion: '1.1.0', date: null, body: '안정 버전 업데이트' };
          case 'updater_auto_check':
            return { enabled: autoUpdateCheck, available: options.updaterConfigured && options.autoUpdateFound
              ? { version: '1.1.1', currentVersion: '1.1.0', date: null, body: '안정 버전 업데이트' } : null };
          case 'updater_set_auto_check':
            autoUpdateCheck = Boolean(args.enabled);
            return autoUpdateCheck;
          case 'updater_install':
            if (options.updaterInstallError) throw '서명된 업데이트를 설치하지 못했습니다.';
            return { version: '1.1.1', currentVersion: '1.1.0', date: null, body: '안정 버전 업데이트' };
          case 'updater_restart':
            if (options.updaterRestartError) throw '업데이트를 적용하며 다시 시작하지 못했습니다.';
            return null;
          case 'notification_authorization': return notificationAuthorization;
          case 'notification_request_permission':
            if (options.notificationRequestError) throw '알림 권한 요청을 사용할 수 없습니다.';
            notificationAuthorization = options.notificationRequestResult ?? 'authorized';
            return { status: notificationAuthorization, test_notification_requested: options.notificationTestRequested ?? Boolean(options.windows) };
          case 'get_snapshot': {
            const value = copy(snapshot);
            if (options.miniSettingsGate) await new Promise<void>((resolve) => fixture.miniSettingsResolvers.push(resolve));
            return value;
          }
          case 'proactive_snapshot': return copy(proactiveSnapshot(true));
          case 'proactive_history': return copy(history.slice(0, args.limit ?? 200));
          case 'proactive_save':
            if (options.proactiveSaveError) throw '백그라운드 설정을 저장하지 못했습니다.';
            Object.assign(proactiveSettings, copy(args.settings));
            Object.assign(scheduler, { enabled: proactiveSettings.scheduler_enabled, paused: proactiveSettings.scheduler_paused,
              interval_minutes: proactiveSettings.scheduler_interval_minutes === 0 ? 60 : proactiveSettings.scheduler_interval_minutes,
              notifications_enabled: proactiveSettings.notifications_enabled,
              notification_sensitivity: proactiveSettings.notification_sensitivity, quiet_hours: copy(proactiveSettings.quiet_hours) });
            snapshot.proactive = copy(proactiveSettings);
            return copy(proactiveSnapshot());
          case 'proactive_clear_history':
            if (options.proactiveClearError) throw '알림 기록을 비우지 못했습니다.';
            history = [];
            return copy(proactiveSnapshot());
          case 'lifecycle_status': return copy(lifecycle);
          case 'lifecycle_set_close_to_background':
          case 'lifecycle_set_start_hidden':
          case 'lifecycle_set_launch_at_login': {
            if (options.lifecycleError) throw '운영체제 시작 설정을 바꾸지 못했습니다.';
            if (command === 'lifecycle_set_close_to_background' && args.enabled && !lifecycle.tray_available) throw '메뉴 막대 아이콘을 사용할 수 없습니다.';
            const field = command === 'lifecycle_set_close_to_background' ? 'close_to_background'
              : command === 'lifecycle_set_start_hidden' ? 'start_hidden' : 'launch_at_login';
            lifecycle[field] = args.enabled;
            if (field === 'launch_at_login') lifecycle.autostart_enabled = args.enabled;
            proactiveSettings[field] = args.enabled;
            snapshot.proactive = copy(proactiveSettings);
            return copy(lifecycle);
          }
          case 'save_settings':
            if (options.settingsError) throw '설정을 저장하지 못했습니다.';
            snapshot.settings = copy(args.settings); emitTheme(snapshot.settings.theme as Theme); emitClockFormat(snapshot.settings.clock_format); return copy(snapshot);
          case 'set_extension':
            snapshot.extensions.find((extension) => extension.id === args.id)!.enabled = args.enabled;
            if (args.id === 'studium' && !args.enabled) {
              studium.running = false; studium.job_id = null; studium.results = studiumStates('not_checked');
              studium.automatic = { ...studium.automatic, active: false, next_check_at: null, error: null };
              fixture.studiumCancelRequested = null; fixture.studiumJobAutomatic = false;
              mini.open = false; mini.always_on_top = false;
              emitStudiumState();
            }
            return copy(snapshot);
          case 'studium_open_mini':
            if (options.miniOpenError) throw '미니 창을 열지 못했습니다.';
            mini.open = true;
            return copy(mini);
          case 'studium_close_mini':
            if (options.miniCloseError) throw '미니 창을 닫지 못했습니다.';
            mini.open = false; mini.always_on_top = false; emitStudiumState();
            return null;
          case 'studium_mini_status': return copy(mini);
          case 'studium_mini_set_on_top':
            if (options.miniOnTopError) throw '항상 위 표시를 바꾸지 못했습니다.';
            mini.always_on_top = Boolean(args.enabled);
            return copy(mini);
          case 'studium_mini_show_main':
            if (options.miniShowMainError) throw '기본 창을 열지 못했습니다.';
            return null;
          case 'studium_snapshot': {
            if (options.studiumSnapshotError) throw '좌석 현황을 불러오지 못했습니다.';
            const value = copy(studium);
            if (fixture.studiumSnapshotGate) await new Promise<void>((resolve) => fixture.studiumSnapshotResolvers.push(resolve));
            return value;
          }
          case 'studium_notification_activation': {
            const roomId = pendingStudiumActivation;
            pendingStudiumActivation = null;
            if (roomId && studium.preferences.room_ids.includes(roomId) && snapshot.extensions.some((extension) => extension.id === 'studium' && extension.enabled)) studium.preferences.view_room = roomId;
            if (options.studiumActivationGate) await new Promise<void>((resolve) => fixture.studiumActivationResolvers.push(resolve));
            fixture.studiumActivationCompleted = true;
            return roomId;
          }
          case 'studium_status': {
            const value = copy(studium);
            await wait(fixture.studiumStatusDelay);
            return value;
          }
          case 'studium_save_preferences': {
            if (studium.running) throw '확인 중에는 대상 열람실을 바꿀 수 없습니다.';
            if (fixture.studiumSaveError) throw '열람실 설정을 저장하지 못했습니다.';
            const ids = args.roomIds as string[];
            if (!ids.length || ids.length > 7 || new Set(ids).size !== ids.length || ids.some((id) => !studium.rooms.some((room) => room.id === id)) || !ids.includes(args.viewRoom)) throw '확인할 열람실을 확인해 주세요.';
            studium.preferences = { room_ids: studium.rooms.filter((room) => ids.includes(room.id)).map((room) => room.id), view_room: args.viewRoom };
            studium.results = studium.preferences.room_ids.map((room_id) => studium.results.find((value) => value.room_id === room_id) ?? { room_id, status: 'not_checked', result: null, error: null, checked_at: null });
            emitStudiumState();
            return copy(studium);
          }
          case 'studium_set_view_room':
            if (fixture.studiumSaveError) throw '표시할 열람실을 저장하지 못했습니다.';
            if (!studium.preferences.room_ids.includes(args.roomId)) throw '선택한 열람실을 확인해 주세요.';
            studium.preferences.view_room = args.roomId;
            emitStudiumState();
            return copy(studium);
          case 'studium_save_favorites': {
            if (fixture.studiumSaveError) throw '선호 좌석을 저장하지 못했습니다.';
            const room = studium.rooms.find((value) => value.id === args.roomId);
            const numbers = args.seatNumbers as number[];
            if (!room || !Array.isArray(numbers) || numbers.some((number) => !Number.isSafeInteger(number) || number < 1 || number > room.capacity) || new Set(numbers).size !== numbers.length) throw '선호 좌석 번호를 확인해 주세요.';
            studium.favorites[args.roomId] = [...numbers].sort((a, b) => a - b);
            emitStudiumState();
            return copy(studium);
          }
          case 'studium_save_alert': {
            if (fixture.studiumAlertSaveError) throw '좌석 알림 조건을 저장하지 못했습니다.';
            const room = studium.rooms.find((value) => value.id === args.roomId);
            if (!room || !['remaining', 'preferred'].includes(args.kind) || !Number.isSafeInteger(args.minimum) || args.minimum < 1 || args.minimum > room.capacity || (args.enabled && args.kind === 'preferred' && !studium.favorites[args.roomId]?.length)) throw '좌석 알림 조건을 확인해 주세요.';
            studium.alert_rules[args.roomId] = { enabled: Boolean(args.enabled), kind: args.kind, minimum: args.minimum, version: (studium.alert_rules[args.roomId]?.version ?? 0) + 1 };
            emitStudiumState();
            return copy(studium);
          }
          case 'studium_save_interval':
            if (fixture.studiumSaveError) throw '자동 확인 간격을 저장하지 못했습니다.';
            if (!Number.isInteger(args.intervalMinutes) || args.intervalMinutes < 2 || args.intervalMinutes > 60) throw '2부터 60 사이의 정수를 입력해 주세요.';
            studium.automatic.interval_minutes = args.intervalMinutes;
            emitStudiumState();
            return copy(studium);
          case 'studium_automatic_start':
            if (options.studiumStartError) throw '좌석 확인을 시작하지 못했습니다.';
            if (!studium.automatic.active) {
              studium.automatic = { ...studium.automatic, active: true, next_check_at: null, error: null };
              if (!studium.running) {
                studium.job_id = (studium.job_id ?? 0) + 1;
                studium.running = true;
                studium.results = studiumStates('loading');
                fixture.studiumJobAutomatic = true;
              }
            }
            emitStudiumState();
            return copy(studium);
          case 'studium_automatic_pause':
            studium.automatic.active = false;
            studium.automatic.next_check_at = null;
            if (studium.running && fixture.studiumJobAutomatic) {
              fixture.studiumCancelRequested = studium.job_id;
              fixture.finishStudiumCancel();
            }
            emitStudiumState();
            return copy(studium);
          case 'studium_start':
            if (options.studiumStartError) throw '좌석 확인을 시작하지 못했습니다.';
            if (studium.running) throw '이미 좌석을 확인하고 있습니다.';
            studium.job_id = (studium.job_id ?? 0) + 1;
            studium.running = true;
            fixture.studiumCancelRequested = null;
            fixture.studiumJobAutomatic = false;
            studium.automatic.next_check_at = null;
            studium.results = studiumStates('loading');
            emitStudiumState();
            return copy(studium);
          case 'studium_cancel':
            if (args.jobId === studium.job_id) {
              // A cancellation request does not complete the owning task. Tests
              // explicitly release it after checking the still-running state.
              fixture.studiumCancelRequested = args.jobId;
              emitStudiumState();
            }
            return copy(studium);
          case 'complete_onboarding': snapshot.onboarding_completed = true; return copy(snapshot);
          case 'personalization_get': return copy(personalization);
          case 'personalization_save_profile':
            await wait(options.profileSaveDelay ?? 0);
            personalization.profile = copy(args.profile); return copy(personalization);
          case 'personalization_save_interest': {
            await wait(options.interestSaveDelay ?? 0);
            const interest = args.interest;
            const id = interest.id ?? Math.max(0, ...personalization.interests.map((item) => item.id)) + 1;
            const existing = personalization.interests.find((item) => item.id === id);
            const value = { id, name: interest.name, aliases: interest.aliases, priority: interest.priority, polarity: interest.polarity ?? 'positive', origin: 'explicit' as const, evidence_count: 0, evidence_summary: null, updated_at: '2026-10-27' };
            if (existing) Object.assign(existing, value); else personalization.interests.push(value);
            return copy(personalization);
          }
          case 'personalization_remove_interest':
            await wait(options.interestRemovalDelay ?? 0);
            personalization.interests = personalization.interests.filter((item) => item.id !== args.id); return copy(personalization);
          case 'personalization_set_learning': personalization.learning_enabled = args.enabled; return copy(personalization);
          case 'personalization_reset': {
            await wait(options.personalizationResetDelay ?? 0);
            personalization = { profile: { university: '', department: '', study_level: '' }, interests: [], learning_enabled: true };
            snapshot.settings.interests = '';
            documentFeedback.clear();
            return copy(personalization);
          }
          case 'personalization_get_feedback':
            if (options.feedbackReadDelay) await wait(options.feedbackReadDelay);
            return documentFeedback.get(args.documentId) ?? null;
          case 'personalization_set_feedback':
            await wait(options.feedbackWriteDelay ?? 0);
            if (args.feedback) documentFeedback.set(args.documentId, args.feedback); else documentFeedback.delete(args.documentId);
            return documentFeedback.get(args.documentId) ?? null;
          case 'personalization_recommendations': {
            const hasPreferences = Boolean(personalization.interests.length || personalization.profile.university
              || personalization.profile.department || personalization.profile.study_level
              || entries.some((item) => item.saved) || documentFeedback.size || sources.some((source) => source.subscribed));
            const items = options.enabled === false ? [] : entries.slice(0, args.limit).map((item) => {
              const searchable = `${item.title} ${item.category ?? ''} ${item.snippet}`.toLowerCase();
              const reasons = hasPreferences ? personalization.interests
                .filter((interest) => [interest.name, ...interest.aliases].some((term) => searchable.includes(term.toLowerCase())))
                .map((interest) => interest.polarity === 'negative'
                  ? `덜 보고 싶은 분야 ‘${interest.name}’ 관련 공지`
                  : `관심 분야 ‘${interest.name}’에 맞는 공지`) : [];
              if (sources.find((source) => source.id === item.source_id)?.subscribed) reasons.push('구독 중인 정보원');
              const feedback = documentFeedback.get(item.id) ?? null;
              if (feedback === 'interested') reasons.push('관심 표시한 공지');
              if (feedback === 'not_interested') reasons.push('관심 없음 표시한 공지');
              if (item.saved) reasons.push('저장한 공지');
              if (hasPreferences && !reasons.length) reasons.push('관심사 일치 신호가 없어 게시일 기준으로 보조 정렬');
              return { item: copy(item), reasons, feedback };
            });
            return { items, has_preferences: hasPreferences };
          }
          case 'ai_snapshot': {
            if (fixture.snapshotFailures > 0) { fixture.snapshotFailures -= 1; throw '모델 정보를 일시적으로 확인하지 못했습니다.'; }
            ai.busy = ai.models.some((model) => ['pending', 'downloading', 'verifying'].includes(model.status));
            return copy(ai);
          }
          case 'ai_classifications': return copy(classifications.filter((row) => args.documentIds.includes(row.document_id)));
          case 'ai_classification': return copy(classifications.find((row) => row.document_id === args.documentId && row.revision_id === args.revisionId) ?? null);
          case 'ai_similar_notices': return copy(similar[args.documentId] ?? []);
          case 'ai_duplicate_notices': return copy(args.documentIds.flatMap((id: number) => { const duplicates = (similar[id] ?? []).filter((row) => row.duplicate); return duplicates.length ? [{ document_id: id, duplicates }] : []; }));
          case 'ai_classification_preferences': ai.preferences.classification_enabled = Boolean(args.enabled); ai.classification = { state: args.enabled ? 'idle' : 'disabled', processed: ai.classification.processed }; return null;
          case 'ai_install': {
            const entry = ai.catalog.find((value) => value.id === args.catalogId);
            let installed = ai.models.find((value) => value.id === args.catalogId);
            if (!installed) {
              installed = model(args.catalogId, entry?.kind === 'embedding' ? 'embedding' : 'generation');
              installed.storage_directory = ai.model_storage.directory;
              installed.path = `${ai.model_storage.directory}${separator}${args.catalogId}.gguf`;
              ai.models.push(installed);
            }
            const total = entry?.bytes ?? 2740937888;
            installed.status = 'downloading'; installed.downloaded_bytes = Math.min(1200000000, total); installed.bytes = 0; installed.error = null;
            setTimeout(() => {
              if (installed!.status !== 'downloading') return;
              installed!.status = options.aiInstallError ? 'failed' : 'installed';
              installed!.error = options.aiInstallError ? '모델 다운로드에 실패했습니다. 연결을 확인하고 이어받아 주세요.' : null;
              installed!.bytes = options.aiInstallError ? 0 : total;
              installed!.downloaded_bytes = options.aiInstallError ? 0 : total;
              if (!options.aiInstallError && installed!.kind === 'embedding') ai.preferences.embedding_model_id = installed!.id;
            }, options.aiDelay ?? 1800);
            return null;
          }
          case 'ai_cancel_download': ai.models.forEach((model) => { if (model.status === 'downloading') model.status = 'cancelled'; }); return null;
          case 'ai_choose_model_directory': {
            if (options.aiModelDirectoryGate) await new Promise<void>((resolve) => fixture.modelDirectoryResolvers.push(resolve));
            if (options.aiModelDirectoryChoice === null) return false;
            if (options.aiModelDirectoryError) throw '모델 설치 폴더를 저장하지 못했습니다. 기존 위치를 유지합니다.';
            const directory = options.aiModelDirectoryChoice ?? chosenModelDirectory;
            ai.preferences.model_directory = directory;
            Object.assign(ai.model_storage, { directory, custom: true, error: null });
            return true;
          }
          case 'ai_reset_model_directory':
            if (options.aiModelDirectoryError) throw '모델 설치 폴더를 저장하지 못했습니다. 기존 위치를 유지합니다.';
            ai.preferences.model_directory = null;
            Object.assign(ai.model_storage, { directory: defaultModelDirectory, custom: false, error: null });
            return null;
          case 'ai_preferences':
            if (args.preferences.model_directory !== ai.preferences.model_directory) throw '모델 설치 폴더는 폴더 선택 명령으로 변경해 주세요.';
            if (args.preferences.execution_mode !== ai.preferences.execution_mode && options.aiExecutionError) throw '실행 장치를 저장하지 못했습니다. 다시 시도해 주세요.';
            Object.assign(ai.preferences, args.preferences); return null;
          case 'ai_remove':
            if (options.aiRemoveError && !args.keepFiles && ai.models.find((model) => model.id === args.id)?.ownership === 'managed') throw '모델 폴더에 접근할 수 없습니다.';
            ai.models = ai.models.filter((model) => model.id !== args.id); if (ai.preferences.embedding_model_id === args.id) ai.preferences.embedding_model_id = null; return null;
          case 'archive_sources': { const result = copy(sources).map((source) => ({ ...source, unread: entries.filter((x) => x.source_id === source.id && !x.is_read && !x.trashed).length })); await wait(fixture.sourceDelay); return result; }
          case 'archive_summary': {
            // As the store: notices in 휴지통 count only as trashed.
            const live = entries.filter((x) => !x.trashed);
            return { documents: live.length, unread: live.filter((x) => !x.is_read).length,
              saved: live.filter((x) => x.saved).length, new_since_sync: 0, resource_bytes: 0, trashed: entries.length - live.length };
          }
          case 'archive_status': return copy(sync);
          case 'archive_query':
          case 'archive_ranked_query': {
            const query = args.query;
            if (options.queryError) throw '로컬 검색을 완료하지 못했습니다.';
            const terms = query.query.trim().split(/\s+/).filter(Boolean);
            const passes = (entry: DocumentItem) => entry.trashed === Boolean(query.trashed) && (!query.saved_only || entry.saved) && (!query.unread_only || !entry.is_read)
              && (!query.source_id || entry.source_id === query.source_id) && (!query.category || entry.category === query.category);
            const filtered = copy(entries.filter((entry) => passes(entry)
              && terms.every((term: string) => `${entry.title} ${entry.snippet}`.toLowerCase().includes(term.toLowerCase()))));
            // The ranked query adds the first notice without the terms as a
            // meaning-only hit when the embedding model can embed the query.
            const semantic = command === 'archive_ranked_query' && Boolean(options.aiSemanticSearch && options.aiEmbeddingInstalled);
            const meaning = semantic ? copy(entries.filter((entry) => passes(entry) && !filtered.some((item) => item.id === entry.id)).slice(0, 1)) : [];
            const ranked = [...meaning, ...filtered];
            if (options.queryGate) {
              const { promise, resolve } = Promise.withResolvers<void>();
              fixture.queryResolvers.push(resolve);
              await promise;
            } else await wait(options.queryDelay ?? 0);
            return { items: ranked.slice(query.offset, query.offset + query.limit), total: ranked.length, categories: ['장학', '학사'],
              meaning_only: meaning.map((item) => item.id), semantic };
          }
          case 'archive_document': {
            const item = entries.find((entry) => entry.id === args.id);
            if (!item) throw '공지 없음';
            const unsupported = options.unsupportedBody && (args.revisionId ?? 2) === 2;
            return { item: copy(item), canonical_url: options.unsupportedBody
              ? 'https://www.sogang.ac.kr/notice/original/551681'
              : 'https://computing.sogang.ac.kr/ko/community/news/detail/282',
              body_html: unsupported ? '' : options.body ?? '<p>보관된 한국어 본문입니다.</p>',
              body_text: unsupported ? '' : '보관된 한국어 본문입니다. 2026년 10월 30일까지 신청서를 제출하세요.',
              revision_id: args.revisionId ?? (options.unsupportedBody ? 2 : 1),
              revisions: options.unsupportedBody
                ? [{ id: 2, content_hash: 'def', collected_at: '2026-10-24 12:00:00' }, { id: 1, content_hash: 'abc', collected_at: '2026-10-23 12:00:00' }]
                : options.revisions
                  ? [{ id: 1, content_hash: 'abc', collected_at: '2026-10-24 12:00:00' }, { id: 2, content_hash: 'def', collected_at: '2026-10-23 12:00:00' }]
                  : [{ id: 1, content_hash: 'abc', collected_at: '2026-10-24 12:00:00' }],
              resources: copy(options.resources ?? []) };
          }
          case 'archive_state': {
            await wait(fixture.stateDelay);
            if (args.isRead !== null && options.readError) throw '읽음 상태를 저장하지 못했습니다. 디스크 공간을 확인해 주세요.';
            const item = entries.find((entry) => entry.id === args.id)!;
            if (args.isRead !== null) item.is_read = args.isRead;
            if (args.saved !== null) item.saved = args.saved;
            return null;
          }
          case 'archive_trash':
          case 'archive_restore':
            for (const entry of entries) if (args.ids.includes(entry.id)) entry.trashed = command === 'archive_trash';
            fixture.trashCalls.push({ command, ids: [...args.ids] });
            return null;
          case 'archive_purge': {
            if (args.ids && args.ids.some((id: number) => !entries.find((entry) => entry.id === id)?.trashed)) throw '휴지통에 있는 공지만 영구 삭제할 수 있습니다.';
            fixture.trashCalls.push({ command, ids: args.ids === null ? null : [...args.ids] });
            for (let index = entries.length - 1; index >= 0; index -= 1) {
              if (entries[index].trashed && (args.ids === null || args.ids.includes(entries[index].id))) entries.splice(index, 1);
            }
            return null;
          }
          case 'archive_subscribe':
            await wait(fixture.subscribeDelay);
            if (fixture.subscribeError) throw '구독 설정을 저장하지 못했습니다.';
            if (!groupOn(sources.find((source) => source.id === args.id)!.group)) throw '상위 그룹을 먼저 켜 주세요.';
            sources = sources.map((source) => source.id === args.id ? { ...source, selected: args.subscribed } : source);
            reconcile();
            return copy({ sources, groups: sourceGroups, joined: args.subscribed && sync.running && sync.mode === 'refresh' });
          case 'archive_source_groups': return copy(sourceGroups);
          case 'archive_set_group':
            await wait(fixture.subscribeDelay);
            sourceGroups = sourceGroups.map((group) => group.id === args.id ? { ...group, enabled: args.enabled } : group);
            reconcile();
            return copy({ sources, groups: sourceGroups, joined: false });
          case 'archive_start': sync = { running: true, mode: args.mode, message: '공개 공지를 가져오는 중입니다.', active: [board(args.mode)] }; return copy(sync);
          case 'archive_cancel': sync = { ...sync, running: false, message: '가져오기를 중단했습니다.', active: [] }; return copy(sync);
          case 'archive_resource_image':
            await wait(options.imageDelay ?? 0);
            if (options.imageError) throw '로컬 자료가 손상되었습니다. 다시 내려받아 주세요.';
            return 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a4WQAAAAASUVORK5CYII=';
          case 'archive_retry_resource':
            sync = { ...sync, running: true, message: '첨부 자료를 다시 내려받고 있습니다.' };
            setTimeout(() => { sync = { ...sync, running: false, message: fixture.retryMessage }; }, 100);
            return copy(sync);
          case 'archive_reveal_resource':
          case 'archive_open_original': return null;
          case 'cybercampus_status': return copy(cybercampus);
          case 'cybercampus_save_login':
            if (args.password !== 'right') { cybercampus.rejected = true; throw '사이버캠퍼스 로그인에 실패했습니다. 아이디와 비밀번호를 확인해 다시 저장해 주세요.'; }
            Object.assign(cybercampus, { id: args.id, rejected: false, courses: [{ course_key: 'A1', name: '자료구조실습', year: 2026, term: '3' }] });
            return copy(cybercampus);
          case 'cybercampus_forget_login': cybercampus.id = null; return copy(cybercampus);
          case 'cybercampus_tasks': return copy(options.cybercampusTasks ?? []);
          // Mirrors storage/calendar: inclusive range, untimed first, then time and id.
          case 'calendar_todos': return copy(calendarTodos.filter((todo) => todo.day >= args.from && todo.day <= args.to)
            .sort((a, b) => a.day.localeCompare(b.day) || Number(a.time !== null) - Number(b.time !== null) || (a.time ?? '').localeCompare(b.time ?? '') || a.id - b.id));
          // Mirrors storage/notice_dates: saved notices' dates, plus every deadline when chosen.
          case 'calendar_notice_dates': return { all_deadlines: calendarAllDeadlines, dates: noticeDates
            .filter((date) => date.day >= args.from && date.day <= args.to && (date.saved !== false || (calendarAllDeadlines && date.kind === 'deadline')))
            .sort((a, b) => a.day.localeCompare(b.day) || a.document_id - b.document_id || a.ordinal - b.ordinal).map(noticeDate) };
          case 'calendar_set_all_deadlines': calendarAllDeadlines = args.enabled; return null;
          case 'notice_dates': return noticeDates.filter((date) => date.document_id === args.documentId).map(noticeDate);
          case 'calendar_create_todo': {
            if (!args.todo.title.trim()) throw '할 일 제목은 1자에서 200자 사이로 입력해 주세요.';
            const todo = { id: ++calendarId, title: args.todo.title.trim(), day: args.todo.day, time: args.todo.time || null, memo: args.todo.memo.trim(), done: false, document_id: args.todo.document_id ?? null };
            calendarTodos.push(todo);
            return copy(todo);
          }
          case 'calendar_update_todo': {
            const todo = calendarTodos.find((item) => item.id === args.id);
            if (!todo) throw '할 일을 찾을 수 없습니다. 이미 삭제되었을 수 있습니다.';
            Object.assign(todo, { title: args.todo.title.trim(), day: args.todo.day, time: args.todo.time || null, memo: args.todo.memo.trim() });
            return copy(todo);
          }
          case 'calendar_set_todo_done': {
            const todo = calendarTodos.find((item) => item.id === args.id)!;
            todo.done = args.done;
            return copy(todo);
          }
          case 'calendar_delete_todo': calendarTodos = calendarTodos.filter((item) => item.id !== args.id); return null;
          default: throw `테스트에서 예상하지 않은 명령: ${command}`;
        }
      },
    } });
    // The desktop dialog plugin replaces these with async IPC wrappers, so a
    // synchronous-looking `window.confirm` returns an always-truthy Promise.
    window.alert = (text?: unknown) => { void (window as any).__TAURI_INTERNALS__.invoke('plugin:dialog|message', { message: String(text) }); };
    window.confirm = (async (text?: unknown) => 'Ok' === await (window as any).__TAURI_INTERNALS__.invoke('plugin:dialog|message', { message: String(text), buttons: 'OkCancel' })) as unknown as typeof window.confirm;
  }, { options });
}

export async function callCount(page: Page, command: string) {
  return page.evaluate((command) => (window as any).__fixture.calls.filter((call: any) => call.command === command).length, command);
}

/** Reports a later refresh step from the mocked backend, as `archive_status` would. */
export async function syncMode(page: Page, mode: string, step = 1) {
  await page.evaluate(({ mode, step }) => {
    // addInitScript installs this test fixture on the page's window.
    const pageWindow = window as unknown as { __fixture: { syncMode: (mode: string, step: number) => void } };
    pageWindow.__fixture.syncMode(mode, step);
  }, { mode, step });
}

type DialogWindow = Window & { __fixture: { dialogs: string[]; dialogAnswers: ('ok' | 'cancel')[] } };

/** Queues answers for upcoming native confirmations, in order. */
export async function answerDialogs(page: Page, ...answers: ('ok' | 'cancel')[]) {
  await page.evaluate((queued) => { (window as DialogWindow).__fixture.dialogAnswers.push(...queued); }, answers);
}

export async function dialogMessages(page: Page) {
  return page.evaluate(() => [...(window as DialogWindow).__fixture.dialogs]);
}

export type SettingsPane = '확장 기능' | '일반' | '게시판' | '사이버캠퍼스' | '개인화' | 'AI' | '알림·백그라운드' | '데이터 및 정보';
/** Opens Settings from the sidebar and shows one pane. */
export async function openSettings(page: Page, pane: SettingsPane) {
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '설정', exact: true }).click();
  await page.getByRole('navigation', { name: '설정 항목', exact: true }).getByRole('button', { name: pane, exact: true }).click();
}
