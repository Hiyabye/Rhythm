import type { Page } from '@playwright/test';
import analysisContract from '../../src/lib/analysis-contract.json' with { type: 'json' };
import type { AiOperation, DocumentItem, PersonalInterest, ResourceRecord } from '../../src/lib/types';

export interface FixtureOptions {
  count?: number;
  saved?: boolean;
  queryError?: boolean;
  readError?: boolean;
  settingsError?: boolean;
  enabled?: boolean;
  body?: string;
  resources?: ResourceRecord[];
  imageError?: boolean;
  imageDelay?: number;
  queryDelay?: number;
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
    const snapshot = { settings: { theme: 'light', energy_profile: 'eco', interests: '' },
      extensions: [{ id: 'sogang_notices', display_name: '서강대학교 공지', description: '학교 소식 보관', available: true, enabled: options.enabled ?? true }],
      storage: { path: '/isolated/rhythm.sqlite3', bytes: 4096, schema_version: 4 },
      worker: { state: 'ready', runtime_available: true, model: null, detail: null } };
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
    const fixture = { ai, analyses, options, entries, calls: [] as { command: string; args: any }[], sourceDelay: 0,
      stateDelay: 0, subscribeError: false, snapshotFailures: 0, resultsFailures: 0, documentJobs: [] as any[], retryMessage: '자료를 내려받지 못했습니다. 연결을 확인해 주세요.',
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
    Object.assign(window, { isTauri: true, __fixture: fixture, __TAURI_INTERNALS__: {
      invoke: async (command: string, args: any = {}) => {
        fixture.calls.push({ command, args: copy(args) });
        switch (command) {
          case 'get_snapshot': return copy(snapshot);
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
            await wait(options.queryDelay ?? 0);
            return { items: filtered.slice(query.offset, query.offset + query.limit), total: filtered.length, categories: ['장학', '학사'] };
          }
          case 'archive_document': {
            const item = entries.find((entry) => entry.id === args.id);
            if (!item) throw '공지 없음';
            return { item: copy(item), canonical_url: 'https://computing.sogang.ac.kr/ko/community/news/detail/282',
              body_html: options.body ?? '<p>보관된 한국어 본문입니다.</p>', body_text: '보관된 한국어 본문입니다. 2026년 10월 30일까지 신청서를 제출하세요.',
              revision_id: args.revisionId ?? 1, revisions: options.revisions ? [{ id: 1, content_hash: 'abc', collected_at: '2026-10-24 12:00:00' }, { id: 2, content_hash: 'def', collected_at: '2026-10-23 12:00:00' }] : [{ id: 1, content_hash: 'abc', collected_at: '2026-10-24 12:00:00' }], resources: copy(options.resources ?? []) };
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
