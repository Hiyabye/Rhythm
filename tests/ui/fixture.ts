import type { Page } from '@playwright/test';
import type { DocumentItem, ResourceRecord } from '../../src/lib/types';

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
}

// Only the IPC boundary is faked: tests exercise the production Svelte build,
// DOM reconstruction, reactivity and browser keyboard/layout behavior.
export async function mockDesktop(page: Page, options: FixtureOptions = {}) {
  await page.addInitScript((options) => {
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
      storage: { path: '/isolated/rhythm.sqlite3', bytes: 4096, schema_version: 2 },
      worker: { state: 'ready', runtime_available: true, model: null, detail: null } };
    let sync = { running: false, source_id: null as string | null, mode: 'historical', page: 0, message: '수집 대기 중입니다.' };
    const fixture = { options, entries, calls: [] as { command: string; args: any }[], sourceDelay: 0,
      stateDelay: 0, subscribeError: false, retryMessage: '자료를 내려받지 못했습니다. 연결을 확인해 주세요.' };
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
              body_html: options.body ?? '<p>보관된 한국어 본문입니다.</p>', body_text: '보관된 한국어 본문입니다.',
              revision_id: 1, revisions: [{ id: 1, content_hash: 'abc', collected_at: '2026-10-24 12:00:00' }], resources: copy(options.resources ?? []) };
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
  }, options);
}

export async function callCount(page: Page, command: string) {
  return page.evaluate((command) => (window as any).__fixture.calls.filter((call: any) => call.command === command).length, command);
}
