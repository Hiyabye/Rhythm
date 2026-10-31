import type { Page } from '@playwright/test';
import analysisContract from '../../src/lib/analysis-contract.json' with { type: 'json' };
import type { AiOperation, DocumentItem, PersonalInterest, ResourceRecord, ProactiveHistoryItem, ProactiveSettings, ProactiveActivation } from '../../src/lib/types';

export interface FixtureOptions {
  count?: number;
  saved?: boolean;
  queryError?: boolean;
  readError?: boolean;
  settingsError?: boolean;
  enabled?: boolean;
  body?: string;
  unsupportedBody?: boolean;
  imageError?: boolean;
  imageDelay?: number;
  queryGate?: boolean;
  aiInstalled?: boolean;
  aiResult?: boolean;
  aiPartial?: boolean;
  aiSetupNeeded?: boolean;
  aiInstallError?: boolean;
  aiInferenceError?: boolean;
  aiDelay?: number;
  aiText?: string;
  revisions?: boolean;
  aiResultLag?: boolean;
  aiLoadDelay?: number;
  aiSetupDelay?: number;
  aiLowResources?: boolean;
  analysisContract?: 'current' | 'prompt-v2';
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
  notificationRequestError?: boolean;
  backupInspectError?: boolean;
  backupRestoreError?: boolean;
  updaterCheckError?: string;
  updaterInstallError?: boolean;
  updaterRestartError?: boolean;
  updateAvailable?: boolean;
}

// Only the IPC boundary is faked: tests exercise the production Svelte build,
// DOM reconstruction, reactivity and browser keyboard/layout behavior.
export async function mockDesktop(page: Page, options: FixtureOptions = {}) {
  await page.addInitScript(({ options, contract }: { options: FixtureOptions; contract: typeof analysisContract }) => {
    const copy = <T,>(value: T): T => JSON.parse(JSON.stringify(value));
    const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
    const entries: DocumentItem[] = Array.from({ length: options.count ?? 2 }, (_, index) => ({
      id: index + 1, source_id: 'computing-updates', source_name: '소프트웨어융합대학 소식',
      title: `한국어 공지 ${String(index + 1).padStart(2, '0')}`, author: '교학팀', category: index % 2 ? '학사' : '장학',
      published_at: '2026-10-24', collected_at: '2026-10-24 12:00:00', is_read: false,
      saved: options.saved ?? false, historical: true, snippet: '컴퓨터 Computer 장학금 경제 공지 본문', content_status: 'complete',
    }));
    let sources = [{ id: 'computing-updates', name: '소식', organization: '소프트웨어융합대학',
      description: '학교 공개 소식', url: 'https://computing.sogang.ac.kr/ko/community/news/list', family: 'computing', supported: true,
      subscribed: false, last_attempt: null, last_success: null, last_error: null, next_page: 1, history_complete: false,
      status: 'limited', discovered: 0, archived: 0, failed: 0 },
    { id: 'unsupported-board', name: '미검증 게시판', organization: '다른 학과', description: '아직 검증되지 않은 게시판',
      url: 'https://example.invalid', family: 'cms', supported: false, subscribed: false, last_attempt: null, last_success: null,
      last_error: null, next_page: 1, history_complete: false, status: 'idle', discovered: 0, archived: 0, failed: 0 }];
    const proactiveSettings: ProactiveSettings = { close_to_background: false, launch_at_login: false, start_hidden: false,
      scheduler_enabled: false, scheduler_paused: false, scheduler_interval_minutes: 0, scheduler_retry_minutes: 15,
      notifications_enabled: false, notification_sensitivity: 'important',
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
    const snapshot = { settings: { theme: 'light', energy_profile: 'eco', interests: '' },
      extensions: [{ id: 'sogang_notices', display_name: '서강대학교 공지', description: '학교 소식 보관', available: true, enabled: options.enabled ?? true }],
      storage: { path: '/isolated/rhythm.sqlite3', bytes: 4096, schema_version: 5 },
      worker: { state: 'ready', runtime_available: true, model: null, detail: null }, proactive: copy(proactiveSettings) };
    let sync = { running: false, source_id: null as string | null, mode: 'historical', page: 0, message: '수집 대기 중입니다.' };
    const model = (id = 'qwen35-4b') => ({ id, catalog_id: id, name: id === 'qwen35-4b' ? 'Qwen3.5 4B' : 'Qwen3.5 2B', ownership: 'managed', path: `models/${id}.gguf`, sha256: 'a'.repeat(64), expected_sha256: 'a'.repeat(64), expected_bytes: 2740937888, bytes: 2740937888, source_revision: 'pinned-revision', source_filename: `${id}.gguf`, status: 'installed', downloaded_bytes: 2740937888, resume_etag: null, error: null as string | null, created_at: '2026-10-24 12:00:00', updated_at: '2026-10-24 12:00:00' });
    const analysis = (id: number, documentId: number, revisionId: number) => ({
      id, job_id: id, document_id: documentId, revision_id: revisionId, task: 'notice_analysis',
      model_sha256: 'a'.repeat(64),
      prompt_version: options.analysisContract === 'prompt-v2' ? 2 : contract.prompt_version,
      schema_version: contract.schema_version,
      configuration_hash: 'b'.repeat(64),
      model_identity: { name: 'Qwen3.5 4B' },
      result: {
        short_summary: options.aiText ?? '장학금 신청 일정과 제출 서류를 확인하세요.',
        summary: '대상 학생은 안내된 기간에 신청서를 제출해야 합니다.',
        category: '장학', uncertainty: 'clear',
        facts: [{
          kind: 'deadline', label: '신청 마감', value: '2026년 10월 30일까지 신청',
          evidence: '2026년 10월 30일까지 신청서를 제출하세요.',
          date: '2026-10-30', uncertainty: 'clear',
        }],
      },
      partial: options.aiPartial ?? false,
      processed_chars: 100,
      total_chars: options.aiPartial ? 10000 : 100,
      duration_ms: 1250,
      created_at: '2026-10-24 12:00:00',
    });
    const ai = { catalog: ['qwen35-4b', 'qwen35-2b'].map((id) => ({ id, name: id === 'qwen35-4b' ? 'Qwen3.5 4B' : 'Qwen3.5 2B', parameters: id === 'qwen35-4b' ? '4B' : '2B', quantization: 'Q4_K_M', bytes: 2740937888, memory_bytes: 6000000000, description: '한국어 공지 분석용 로컬 모델', use_case: '균형 잡힌 공지 분석', repository: 'unsloth/Qwen3.5-4B-GGUF', revision: 'pinned-revision', filename: `${id}.gguf`, sha256: 'a'.repeat(64), license: 'Apache-2.0', compatibility: 'verified', reason: null })), models: options.aiInstalled ? [model()] : [], preferences: { active_model_id: options.aiInstalled ? 'qwen35-4b' : null as string | null, setup_completed: !options.aiSetupNeeded, processing_mode: 'explicit', automatic_limit: 3, max_tokens: 1024 }, jobs: [] as any[], runtime: { state: 'ready', runtime_available: true, model: null as string | null, detail: null }, hardware: { os: 'macOS', arch: 'arm64', ram_bytes: options.aiLowResources ? 4294967296 : 17179869184, disk_available_bytes: options.aiLowResources ? 0 : 27000000000, backend: 'cpu' }, busy: false, operation: null as AiOperation | null, progress: null as any, analysis_contract: { prompt_version: contract.prompt_version, schema_version: contract.schema_version, configuration_hash: 'b'.repeat(64) } };
    const analyses = options.aiResult ? [analysis(1, 1, 1)] : [];
    let personalization = { profile: { university: '', department: '', study_level: '' }, interests: copy(options.initialInterests ?? []), learning_enabled: true };
    const documentFeedback = new Map<number, string>();
    let resultLagUsed = false;
    let lifecycle = { close_to_background: false, start_hidden: false, launch_at_login: false, autostart_enabled: false as boolean | null, tray_available: true };
    const callbacks = new Map<number, (event: { event: string; id: number; payload: unknown }) => void>();
    let callbackId = 0;
    let eventId = 0;
    let updaterHandler: number | null = null;
    let notificationHandler: number | null = null;
    const fixture = { ai, analyses, options, entries, calls: [] as { command: string; args: any }[], sourceDelay: 0,
      stateDelay: 0, subscribeError: false, snapshotFailures: 0, resultsFailures: 0, documentJobs: [] as any[], retryMessage: '자료를 내려받지 못했습니다. 연결을 확인해 주세요.',
      queryResolvers: [] as Array<() => void>,
      emitUpdater: (payload: unknown) => {
        if (updaterHandler !== null) callbacks.get(updaterHandler)?.({ event: 'updater://progress', id: ++eventId, payload: copy(payload) });
      },
      releaseQueries() { this.queryResolvers.splice(0).forEach((release) => release()); },
      emitNotification: (payload: { event_id: number; document_id: number; revision_id: number }) => {
        if (notificationHandler !== null) callbacks.get(notificationHandler)?.({ event: 'proactive://notification-clicked', id: eventId, payload: { ...copy(payload), attempt_count: 1, event_ids: [payload.event_id] } });
      },
      completeSync: () => { sync = { ...sync, running: false, message: '가져오기를 마쳤습니다.' }; } };
    let operationStarted = 0;
    let analysisStarted = 0;
    async function operation(kind: AiOperation['kind'], delay: number) {
      const current: AiOperation = { kind, phase: kind === 'setup' ? 'setup' : 'verifying', elapsed_ms: 0, cancelling: false };
      ai.operation = current; operationStarted = Date.now();
      try {
        while (Date.now() - operationStarted < delay && !current.cancelling) await wait(25);
        if (current.cancelling) throw 'AI 작업을 취소했습니다.';
      } finally { if (ai.operation === current) ai.operation = null; }
    }
    Object.assign(window, { isTauri: true, __fixture: fixture, __TAURI_EVENT_PLUGIN_INTERNALS__: {
      unregisterListener: (event: string, _id: number) => {
        if (event === 'updater://progress') updaterHandler = null;
        if (event === 'proactive://notification-clicked') notificationHandler = null;
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
            if (args.event === 'proactive://notification-clicked') notificationHandler = args.handler;
            eventId += 1;
            return eventId;
          case 'plugin:event|unlisten':
            if (args.event === 'updater://progress') updaterHandler = null;
            if (args.event === 'proactive://notification-clicked') notificationHandler = null;
            return null;
          case 'plugin:dialog|open': return '/fixtures/Rhythm-backup.rhythm-backup';
          case 'plugin:dialog|save': return '/fixtures/Rhythm-backup.rhythm-backup';
          case 'backup_inspect':
            if (options.backupInspectError) throw '백업 파일의 형식이 올바르지 않습니다.';
            return { format_version: 1, created_at: '1793404800', database_schema: 6, documents: 12, resources: 3,
              resource_bytes: 1536, includes_model_weights: false };
          case 'backup_restore':
            if (options.backupRestoreError) throw '백업 데이터를 복원하지 못했습니다.';
            return null;
          case 'backup_cancel': return true;
          case 'updater_check':
            if (options.updaterCheckError) throw options.updaterCheckError;
            return options.updateAvailable === false ? null : { version: '0.9.1', currentVersion: '0.9.0', date: null, body: '안정 버전 업데이트' };
          case 'updater_install':
            if (options.updaterInstallError) throw '서명된 업데이트를 설치하지 못했습니다.';
            return { version: '0.9.1', currentVersion: '0.9.0', date: null, body: '안정 버전 업데이트' };
          case 'updater_restart':
            if (options.updaterRestartError) throw '업데이트를 적용하며 다시 시작하지 못했습니다.';
            return null;
          case 'notification_authorization': return notificationAuthorization;
          case 'notification_request_permission':
            if (options.notificationRequestError) throw '알림 권한 요청을 사용할 수 없습니다.';
            notificationAuthorization = options.notificationRequestResult ?? 'authorized';
            return notificationAuthorization;
          case 'get_snapshot': return copy(snapshot);
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
            snapshot.settings = copy(args.settings); return copy(snapshot);
          case 'set_extension': snapshot.extensions[0].enabled = args.enabled; return copy(snapshot);
          case 'worker_status': return copy(snapshot.worker);
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
            if (ai.operation) ai.operation.elapsed_ms = Date.now() - operationStarted;
            if (ai.progress && analysisStarted) ai.progress.elapsed_ms = Date.now() - analysisStarted;
            ai.busy = Boolean(ai.operation) || ai.jobs.some((job) => ['pending', 'running'].includes(job.status));
            return copy(ai);
          }
          case 'ai_document_jobs': return copy([...ai.jobs, ...fixture.documentJobs].filter((job) => job.document_id === args.documentId && job.revision_id === args.revisionId).sort((a, b) => b.id - a.id));
          case 'ai_diagnostics': return copy(ai.progress?.job_id === args.jobId ? ai.progress : null);
          case 'ai_recent': return copy(analyses.filter((result) => result.revision_id === 1));
          case 'ai_results':
            if (fixture.resultsFailures > 0) { fixture.resultsFailures -= 1; throw '분석 정보를 일시적으로 확인하지 못했습니다.'; }
            if (options.aiResultLag && !resultLagUsed && ai.jobs.some((job) => job.status === 'completed')) { resultLagUsed = true; return []; }
            return copy(analyses.filter((result) => result.document_id === args.documentId && result.revision_id === args.revisionId).sort((a, b) => b.id - a.id));
          case 'ai_install': {
            let installed = ai.models.find((value) => value.id === args.catalogId);
            if (!installed) { installed = model(args.catalogId); ai.models.push(installed); }
            installed.status = 'downloading'; installed.downloaded_bytes = 1200000000; installed.bytes = 0; installed.error = null;
            setTimeout(() => { if (installed!.status !== 'downloading') return; installed!.status = options.aiInstallError ? 'failed' : 'installed'; installed!.error = options.aiInstallError ? '모델 다운로드에 실패했습니다. 연결을 확인하고 이어받아 주세요.' : null; installed!.bytes = options.aiInstallError ? 0 : 2740937888; }, options.aiDelay ?? 1800);
            return null;
          }
          case 'ai_cancel_download': ai.models.forEach((model) => { if (model.status === 'downloading') model.status = 'cancelled'; }); return null;
          case 'ai_register': ai.models.push({ ...model('external-model'), catalog_id: null as any, ownership: 'external', path: args.path, name: '외부 로컬 모델' }); return null;
          case 'ai_select': ai.preferences.active_model_id = args.id; ai.runtime.model = null; return null;
          case 'ai_load': await operation('load', options.aiLoadDelay ?? 0); ai.runtime.model = ai.models.find((value) => value.id === ai.preferences.active_model_id)?.path ?? null; return null;
          case 'ai_cancel_operation': if (ai.operation) ai.operation.cancelling = true; return null;
          case 'ai_unload':
          case 'ai_restart': ai.runtime.model = null; return null;
          case 'ai_preferences': if (args.preferences.setup_completed && !ai.preferences.setup_completed) await operation('setup', options.aiSetupDelay ?? 0); Object.assign(ai.preferences, args.preferences); return null;
          case 'ai_remove': ai.models = ai.models.filter((model) => model.id !== args.id); if (ai.preferences.active_model_id === args.id) ai.preferences.active_model_id = null; return null;
          case 'ai_analyze': {
            const cached = analyses.find((result) => result.document_id === args.documentId && result.revision_id === args.revisionId);
            if (cached && !args.regenerate) return { cached: copy(cached), job: null };
            const generated = analysis(analyses.length + ai.jobs.length + 1, args.documentId, args.revisionId);
            const job = { ...generated, model_identity: { name: 'Qwen3.5 4B' }, priority: 100, status: 'running', error: null as string | null, updated_at: '2026-10-24 12:00:00' };
            ai.jobs.unshift(job);
            analysisStarted = Date.now();
            ai.progress = { job_id: job.id, phase: 'inference', elapsed_ms: 0, stages_ms: {}, attempts: [{}] };
            setTimeout(() => { if (job.status !== 'running') return; job.status = options.aiInferenceError ? 'failed' : 'completed'; job.error = options.aiInferenceError ? '로컬 추론에 실패했습니다. 다시 요청해 주세요.' : null; ai.progress = null; if (!options.aiInferenceError) analyses.push(generated); }, options.aiDelay ?? 300);
            return { cached: null, job: copy(job) };
          }
          case 'ai_cancel': { const job = [...ai.jobs, ...fixture.documentJobs].find((job) => job.id === args.id); if (job) job.status = 'cancelled'; if (ai.progress?.job_id === args.id) ai.progress = null; return null; }
          case 'archive_sources': { const result = copy(sources); await wait(fixture.sourceDelay); return result; }
          case 'archive_summary': return { documents: entries.length, unread: entries.filter((x) => !x.is_read).length,
            saved: entries.filter((x) => x.saved).length, new_since_sync: 0, resource_bytes: 0 };
          case 'archive_status': return copy(sync);
          case 'archive_query': {
            const query = args.query;
            if (options.queryError) throw '로컬 검색을 완료하지 못했습니다.';
            const filtered = copy(entries.filter((entry) => (!query.saved_only || entry.saved) && (!query.unread_only || !entry.is_read)
              && (!query.source_id || entry.source_id === query.source_id) && (!query.category || entry.category === query.category)
              && query.query.trim().split(/\s+/).filter(Boolean).every((term: string) => `${entry.title} ${entry.snippet}`.toLowerCase().includes(term.toLowerCase()))));
            if (options.queryGate) {
              const { promise, resolve } = Promise.withResolvers<void>();
              fixture.queryResolvers.push(resolve);
              await promise;
            } else await wait(options.queryDelay ?? 0);
            return { items: filtered.slice(query.offset, query.offset + query.limit), total: filtered.length, categories: ['장학', '학사'] };
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
          case 'archive_subscribe':
            if (fixture.subscribeError) throw '구독 설정을 저장하지 못했습니다.';
            sources = sources.map((source) => source.id === args.id ? { ...source, subscribed: args.subscribed } : source);
            return copy(sources);
          case 'archive_start': sync = { running: true, source_id: 'computing-updates', mode: args.mode, page: 1, message: '공개 공지를 가져오는 중입니다.' }; return copy(sync);
          case 'archive_cancel': sync = { ...sync, running: false, message: '가져오기를 중단했습니다.' }; return copy(sync);
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
          default: throw `테스트에서 예상하지 않은 명령: ${command}`;
        }
      },
    } });
  }, { options, contract: analysisContract });
}

export async function callCount(page: Page, command: string) {
  return page.evaluate((command) => (window as any).__fixture.calls.filter((call: any) => call.command === command).length, command);
}
