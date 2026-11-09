import { test, expect } from '@playwright/test';
import { mockDesktop, callCount, openSettings, syncMode } from './fixture';
import type { ResourceRecord } from '../../src/lib/types';

const image: ResourceRecord = { id: 7, url: 'https://computing.sogang.ac.kr/web/file/editor/test.png', filename: '본문.png',
  kind: 'attachment', status: 'downloaded', mime: 'image/png', bytes: 70, local_path: 'resources/hash.png', sha256: 'abc', error: null };

async function navigate(page: import('@playwright/test').Page, name: string) {
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name, exact: true }).click();
}

// Failure regressions are intentionally asserted through user-visible content,
// not snapshots of the components' private state.
test('read-state write failure does not hide a successfully archived article', async ({ page }) => {
  await mockDesktop(page, { readError: true });
  await page.goto('/');
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 01');
  await expect(page.locator('.reader-body')).toContainText('보관된 한국어 본문입니다.');
  await expect(page.getByRole('alert')).toContainText('읽음 상태를 저장하지 못했습니다');
  // Action-named buttons must not also expose a pressed state that contradicts their label.
  await expect(page.getByRole('button', { name: '읽음으로 표시', exact: true })).not.toHaveAttribute('aria-pressed');
});
test('iframe-only archive revision explains unsupported body without embedding it and keeps earlier text', async ({ page }) => {
  const remoteRequests: string[] = [];
  page.on('request', (request) => {
    if (request.url().includes('sogang.ac.kr')) remoteRequests.push(request.url());
  });
  await mockDesktop(page, { unsupportedBody: true });
  await page.goto('/');
  await page.locator('#notice-1').click();

  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 01');
  await expect(page.getByRole('status').filter({ hasText: '본문 미지원' })).toContainText('본문은 로컬에 보관되지 않았습니다');
  await expect(page.locator('.archive-local-status')).toContainText('본문은 지원되지 않아 로컬에 보관되지 않았습니다');
  await expect(page.locator('.archive-local-status')).not.toContainText('본문은 로컬에 보관되어 있습니다');
  await expect(page.locator('.archive-local-status')).not.toContainText('연결된 자료도 모두 저장되어 있습니다');
  expect(remoteRequests).toEqual([]);
  await expect(page.locator('iframe')).toHaveCount(0);
  await expect(page.locator('.reader-body')).toHaveCount(0);

  const original = page.getByRole('button', { name: '학교 원문 열기' });
  await expect(original).toBeVisible();
  await original.click();
  await expect.poll(() => page.evaluate(() => {
    const fixture = (window as Window & { __fixture: { calls: Array<{ command: string; args: Record<string, unknown> }> } }).__fixture;
    return fixture.calls.find((call) => call.command === 'archive_open_original')?.args.url;
  })).toBe('https://www.sogang.ac.kr/notice/original/551681');

  await page.getByLabel('보관 이력').selectOption('1');
  await expect(page.locator('.reader-body')).toContainText('보관된 한국어 본문입니다.');
  await expect(page.getByRole('status').filter({ hasText: '본문 미지원' })).toHaveCount(0);
  await expect(page.locator('iframe')).toHaveCount(0);
});


test('removing last saved item from page two returns to the last valid page', async ({ page }) => {
  await mockDesktop(page, { count: 31, saved: true });
  await page.goto('/');
  await navigate(page, '보관함');
  await page.getByRole('button', { name: '다음', exact: true }).click();
  await expect(page.locator('#notice-31')).toBeVisible();
  await page.getByRole('button', { name: '한국어 공지 31 저장 해제', exact: true }).click();
  await expect(page.locator('.notice-list > li')).toHaveCount(30);
  await expect(page.locator('.archive-pagination')).toContainText('1–30 / 30');
  await expect(page.getByText('아직 저장한 공지가 없습니다', { exact: true })).toHaveCount(0);
});

test('archived notices carry no past-record label wherever they were collected', async ({ page }) => {
  await mockDesktop(page, {});
  await page.goto('/');
  await navigate(page, '받은 소식');
  await expect(page.locator('.notice-list > li')).toHaveCount(2);
  // Fixture records are marked as backfilled, but the UI must not expose that concept.
  await expect(page.getByText('과거 자료', { exact: false })).toHaveCount(0);
  await expect(page.locator('.notice-meta').filter({ hasText: '과거' })).toHaveCount(0);
});

test('marking final unread page item read clamps paging after reader closes', async ({ page }) => {
  await mockDesktop(page, { count: 61 });
  await page.goto('/');
  await navigate(page, '받은 소식');
  await page.getByLabel('읽지 않은 공지만').check();
  await page.getByRole('button', { name: '다음', exact: true }).click();
  await expect(page.locator('.archive-pagination')).toContainText('31–60 / 61');
  await page.getByRole('button', { name: '다음', exact: true }).click();
  await page.locator('#notice-61').click();
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 61');
  await page.keyboard.press('Escape');
  await expect(page.locator('.notice-list > li')).toHaveCount(30);
  await expect(page.locator('.archive-pagination')).toContainText('31–60 / 60');
});

test('initial query error is not presented as a successful empty archive and can retry', async ({ page }) => {
  await mockDesktop(page, { queryError: true });
  await page.goto('/');
  await expect(page.getByRole('alert')).toContainText('로컬 검색을 완료하지 못했습니다');
  await expect(page.getByText('아직 보관된 공지가 없습니다', { exact: true })).toHaveCount(0);
  await page.evaluate(() => (window as any).__fixture.options.queryError = false);
  await page.getByRole('button', { name: '다시 시도', exact: true }).click();
  await navigate(page, '받은 소식');
  await expect(page.locator('.notice-list > li')).toHaveCount(2);
});

test('saving an item admits only one pending write and announces feedback', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await navigate(page, '받은 소식');
  await page.evaluate(() => (window as any).__fixture.stateDelay = 400);
  const save = page.getByRole('button', { name: '한국어 공지 01 보관함에 저장', exact: true });
  await save.click();
  await expect(save).toBeDisabled();
  await expect.poll(() => callCount(page, 'archive_state')).toBe(1);
  await expect(page.getByRole('button', { name: '한국어 공지 01 저장 해제', exact: true })).not.toHaveAttribute('aria-pressed');
  await expect(page.locator('.archive-feedback')).toContainText('보관함에 저장했습니다');
});

test('navigation, local searches, filters, safe highlighting and empty-result announcement', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  for (const name of ['받은 소식', '보관함', '설정', '오늘', '탐색']) await navigate(page, name);
  await expect(page.locator('#main')).toBeFocused();
  await page.getByLabel('보관된 공지 검색').fill('장학금 Computer');
  await page.getByRole('button', { name: '검색', exact: true }).click();
  await expect(page.locator('.notice-list > li')).toHaveCount(2);
  await expect(page.locator('.notice-snippet mark').first()).toBeVisible();
  await page.getByRole('combobox', { name: '분류', exact: true }).selectOption('장학');
  await expect(page.locator('.notice-list > li')).toHaveCount(1);
  await page.getByLabel('보관된 공지 검색').fill('<script> & " % _');
  await page.getByRole('button', { name: '검색', exact: true }).click();
  await expect(page.getByText('조건에 맞는 공지가 없습니다', { exact: true })).toBeVisible();
  await expect(page.locator('.archive-result-status')).toContainText('0개');
  await expect(page.locator('.archive-view script')).toHaveCount(0);
});

test('relevance search stays text-only without an embedding model', async ({ page }) => {
  await mockDesktop(page, { count: 3, aiSemanticSearch: true });
  await page.goto('/');
  await navigate(page, '탐색');
  await expect(page.getByRole('combobox', { name: '정렬', exact: true })).toHaveValue('relevance');
  await page.getByLabel('보관된 공지 검색').fill('공지 01');
  await page.getByRole('button', { name: '검색', exact: true }).click();
  await expect(page.locator('.notice-list > li')).toHaveCount(1);
  await expect.poll(() => callCount(page, 'archive_ranked_query')).toBe(1);
  await expect(page.locator('.notice-meaning-note')).toHaveCount(0);
  await expect(page.locator('.archive-search + .footnote')).not.toContainText('뜻이 비슷한');
});

test('relevance search marks notices found only by meaning, and date order is text-only', async ({ page }) => {
  await mockDesktop(page, { count: 3, aiSemanticSearch: true, aiEmbeddingInstalled: true });
  await page.goto('/');
  await navigate(page, '탐색');
  await page.getByLabel('보관된 공지 검색').fill('공지 03');
  await page.getByRole('button', { name: '검색', exact: true }).click();
  await expect(page.locator('.notice-list > li')).toHaveCount(2);
  const meaning = page.locator('.notice-list > li').filter({ has: page.locator('.notice-meaning-note') });
  await expect(meaning).toHaveCount(1);
  await expect(meaning).toContainText('한국어 공지 01');
  await expect(meaning.locator('.notice-meaning-note')).toHaveText('검색어는 없지만 뜻이 비슷한 공지');
  await expect(page.locator('.archive-search + .footnote')).toContainText('뜻이 비슷한 공지도 함께 찾습니다');
  await page.getByRole('combobox', { name: '정렬', exact: true }).selectOption('newest');
  await expect(page.locator('.notice-list > li')).toHaveCount(1);
  await expect(page.locator('.notice-meaning-note')).toHaveCount(0);
  await expect.poll(() => callCount(page, 'archive_query')).toBeGreaterThan(0);
  await expect(page.locator('.archive-search + .footnote')).not.toContainText('뜻이 비슷한');
});

test('successful empty archive is distinct from loading and errors', async ({ page }) => {
  await mockDesktop(page, { count: 0, queryGate: true });
  await page.goto('/');
  await navigate(page, '받은 소식');
  await expect(page.locator('.archive-view .loading')).toBeVisible();
  await page.evaluate(() => (window as Window & { __fixture: { releaseQueries: () => void } }).__fixture.releaseQueries());
  await expect(page.getByText('아직 보관된 공지가 없습니다', { exact: true })).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('an empty archive explains first steps, local storage and optional AI on Today', async ({ page }) => {
  await mockDesktop(page, { count: 0 });
  await page.goto('/');
  const start = page.getByRole('region', { name: 'Rhythm 시작하기' });
  await expect(start).toContainText('이 기기에만 저장됩니다');
  await expect(start).toContainText('설정 → 확장 기능에서 서강대학교 공지나 사이버캠퍼스를 켭니다');
  await expect(start).toContainText('AI 분석, 알림·백그라운드 같은 기능도 확장 기능에서 필요할 때 켭니다');
  await expect(start).toContainText('설정 → 데이터 및 정보에서 확인할 수 있습니다');
  await start.getByRole('button', { name: '초기 설정 열기', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Rhythm에 오신 것을 환영합니다', exact: true })).toBeFocused();
});

test('a populated archive does not repeat the first-run guide', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await expect(page.getByRole('region', { name: '오늘의 추천' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Rhythm 시작하기' })).toHaveCount(0);
});

test('settings shows one pane at a time and remembers it while the app is open', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await navigate(page, '설정');
  const panes = page.getByRole('navigation', { name: '설정 항목', exact: true }).getByRole('button');
  await expect(panes).toHaveText(['확장 기능', '일반', '게시판', '사이버캠퍼스', '개인화', 'AI', '알림·백그라운드', '데이터 및 정보']);
  await expect(panes.first()).toHaveAttribute('aria-current', 'page');
  for (const name of ['일반', '게시판', '사이버캠퍼스', '개인화', 'AI', '알림·백그라운드', '데이터 및 정보', '확장 기능']) {
    await panes.filter({ hasText: name }).click();
    await expect(page.getByRole('heading', { level: 2, name, exact: true })).toBeVisible();
    await expect(panes.filter({ hasText: name })).toHaveAttribute('aria-current', 'page');
    await expect(page.getByRole('heading', { level: 2 })).toHaveCount(1);
  }
  await panes.filter({ hasText: '알림·백그라운드' }).click();
  await navigate(page, '오늘');
  await navigate(page, '설정');
  await expect(page.getByRole('heading', { level: 2, name: '알림·백그라운드', exact: true })).toBeVisible();
});

test('row explanations open from the keyboard and Escape returns focus to their button', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await openSettings(page, '일반');
  const hint = page.getByRole('button', { name: '전원 모드 설명', exact: true });
  const panel = page.getByRole('dialog', { name: '전원 모드 설명', exact: true });
  await expect(panel).toBeHidden();
  await hint.focus();
  await page.keyboard.press('Enter');
  await expect(panel).toBeVisible();
  await expect(panel).toContainText('절전은 자동 분석을 하지 않고');
  await page.keyboard.press('Escape');
  await expect(panel).toBeHidden();
  await expect(hint).toBeFocused();
});

test('settings failure rolls back optimistic theme; system theme follows OS', async ({ page }) => {
  await mockDesktop(page, { settingsError: true });
  await page.goto('/');
  await openSettings(page, '일반');
  await page.getByRole('button', { name: '다크', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('설정을 저장하지 못했습니다');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.evaluate(() => (window as any).__fixture.options.settingsError = false);
  await page.getByRole('button', { name: '시스템', exact: true }).click();
  await page.emulateMedia({ colorScheme: 'dark', reducedMotion: 'reduce' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
});

for (const reducedMotion of ['no-preference', 'reduce'] as const) {
  test(`Svelte motion ${reducedMotion === 'reduce' ? 'is skipped under' : 'runs without'} reduced motion`, async ({ page }) => {
    // Svelte transitions call Element.animate, which the reduced-motion CSS rule cannot stop.
    await page.addInitScript(() => {
      const animate = Element.prototype.animate;
      (window as any).__animations = 0;
      Element.prototype.animate = function (...args) {
        (window as any).__animations++;
        return animate.apply(this, args);
      };
    });
    await page.emulateMedia({ reducedMotion });
    await mockDesktop(page);
    await page.goto('/');
    await openSettings(page, '일반');
    await page.getByRole('button', { name: '다크', exact: true }).click();
    await expect(page.locator('.save-notice')).toContainText('설정을 저장했습니다.');
    const animations = await page.evaluate(() => (window as any).__animations);
    if (reducedMotion === 'reduce') expect(animations).toBe(0);
    else expect(animations).toBeGreaterThan(0);
  });
}

test('subscriptions have no automatic collection, protect refreshing state, show errors and cancel', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await openSettings(page, '게시판');
  const subscribe = page.getByRole('switch', { name: '소프트웨어융합대학 소식 구독', exact: true });
  await expect(subscribe).toBeEnabled();
  // Unsupported boards are listed only on request, and never subscribable.
  const unsupported = page.getByRole('switch', { name: '다른 학과 미검증 게시판 구독', exact: true });
  await expect(unsupported).toHaveCount(0);
  await page.getByText('미지원 게시판 1개', { exact: true }).click();
  await expect(unsupported).toBeDisabled();
  await page.getByRole('button', { name: '소프트웨어융합대학 소식 상태', exact: true }).click();
  const status = page.getByRole('dialog', { name: '소식 상태', exact: true });
  await expect(status).toContainText('일부 가져옴 · 이어받기 가능');
  // Per-board collection is offered only for subscribed boards.
  await expect(status.getByRole('button')).toHaveCount(0);
  await page.keyboard.press('Escape');
  await page.evaluate(() => { (window as any).__fixture.subscribeError = true; });
  await subscribe.click();
  await expect(page.getByRole('alert')).toContainText('구독 설정을 저장하지 못했습니다');
  await expect(subscribe).toHaveAttribute('aria-checked', 'false');
  await page.evaluate(() => { (window as any).__fixture.subscribeError = false; });
  // Tiles stay locked while a subscription change is saved.
  await page.evaluate(() => { (window as any).__fixture.subscribeDelay = 400; });
  await subscribe.click();
  await expect(subscribe).toBeDisabled();
  await expect(subscribe).toHaveAttribute('aria-checked', 'true');
  await expect(subscribe).toBeEnabled();
  expect(await callCount(page, 'archive_start')).toBe(0);
  const starts = () => page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'archive_start').map((call: any) => call.args));
  await page.getByRole('button', { name: '새로고침', exact: true }).click();
  // The pane refreshes the subscribed boards it manages.
  await expect.poll(starts).toEqual([{ sourceIds: ['computing-updates'], mode: 'refresh' }]);
  await expect(page.locator('.collect-status')).toContainText('새로고침 준비 중');
  // The one job reports each step, so the label follows new notices into the backlog.
  await syncMode(page, 'incremental');
  await expect(page.locator('.collect-status')).toContainText('새 소식 확인 중');
  await syncMode(page, 'historical', 2);
  await expect(page.locator('.collect-status')).toContainText('이전 공지 가져오는 중 · 소식 · 2쪽');
  await expect(page.getByRole('button', { name: '가져오기 중단', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '가져오기 중단', exact: true }).click();
  await expect(page.locator('.source-manager .archive-feedback')).toContainText('이미 저장한 공지는 유지됩니다');
  // One board can still be refreshed on its own from its status popover.
  await page.getByRole('button', { name: '소프트웨어융합대학 소식 상태', exact: true }).click();
  await expect(status.getByRole('button', { name: '새 소식 확인', exact: true })).toHaveCount(0);
  await status.getByRole('button', { name: '새로고침', exact: true }).click();
  await expect.poll(starts).toEqual([{ sourceIds: ['computing-updates'], mode: 'refresh' }, { sourceIds: ['computing-updates'], mode: 'refresh' }]);
  // Missing files of every subscribed board download through one explicit action.
  await page.keyboard.press('Escape');
  await page.evaluate(() => (window as any).__fixture.completeSync());
  await expect(page.getByRole('button', { name: '자료 모두 내려받기', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: '자료 모두 내려받기', exact: true }).click();
  await expect.poll(starts).toEqual([{ sourceIds: ['computing-updates'], mode: 'refresh' }, { sourceIds: ['computing-updates'], mode: 'refresh' }, { sourceIds: ['computing-updates'], mode: 'resources' }]);
  await expect(page.locator('.collect-status')).toContainText('첨부 자료 모두 내려받는 중');
});

test('boards can be switched while a refresh runs, and several boards show progress at once', async ({ page }) => {
  await mockDesktop(page, { sourceList: [
    { id: 'computing-updates', name: '소식', organization: '소프트웨어융합대학', supported: true, subscribed: true },
    { id: 'computing-career', name: '취업', organization: '소프트웨어융합대학', supported: true },
    { id: 'sogang-academic', name: '학부 학사공지', organization: '서강대학교', supported: true, family: 'main' },
  ] });
  await page.goto('/');
  await openSettings(page, '게시판');
  await page.getByRole('button', { name: '새로고침', exact: true }).click();
  await expect(page.getByRole('button', { name: '가져오기 중단', exact: true })).toBeVisible();
  // Starting another job waits, but subscriptions stay open.
  await expect(page.getByRole('button', { name: '새로고침', exact: true })).toBeDisabled();
  const main = page.getByRole('switch', { name: '서강대학교 학부 학사공지 구독', exact: true });
  await expect(main).toBeEnabled();
  await main.click();
  await expect(main).toHaveAttribute('aria-checked', 'true');
  await expect(page.locator('.source-manager .archive-feedback')).toContainText('진행 중인 새로고침에서 함께 가져옵니다');
  await page.evaluate(() => (window as any).__fixture.syncBoards(['computing-updates', 'sogang-academic']));
  await expect(page.locator('.collect-status')).toContainText('게시판 2개를 동시에 가져오는 중');
  await expect(page.locator('.board-state.spinning')).toHaveCount(2);
  // Unsubscribing a running board stops only that board.
  const updates = page.getByRole('switch', { name: '소프트웨어융합대학 소식 구독', exact: true });
  await updates.click();
  await expect(updates).toHaveAttribute('aria-checked', 'false');
  await expect(page.locator('.source-manager .archive-feedback')).toContainText('이 게시판의 가져오기만 멈춥니다');
  expect(await callCount(page, 'archive_cancel')).toBe(0);
});

test('skipped notices are listed per board with their originals and an explicit retry', async ({ page }) => {
  const parser = '학교 페이지 형식이 예상과 다릅니다. 기존 보관 자료는 유지됩니다.';
  await mockDesktop(page, { sourceList: [{ id: 'computing-career', name: '취업', organization: '소프트웨어융합대학', supported: true, subscribed: true,
    skipped_count: 21, skipped: [
      { external_id: '3110', url: 'https://computing.sogang.ac.kr/ko/community/career/detail/3110', reason: parser, skipped_at: '2026-11-04T00:00:00Z', archived: false },
      { external_id: '3111', url: 'https://computing.sogang.ac.kr/ko/community/career/detail/3111', reason: parser, skipped_at: '2026-11-03T00:00:00Z', archived: true },
    ] }] });
  await page.goto('/');
  await openSettings(page, '게시판');
  // A skip is not a board failure: no warning mark on the tile.
  await expect(page.locator('.board-state.alert')).toHaveCount(0);
  await page.getByRole('button', { name: '소프트웨어융합대학 취업 상태', exact: true }).click();
  const status = page.getByRole('dialog', { name: '취업 상태', exact: true });
  await expect(status).toContainText('가져오지 못한 공지 21건');
  await expect(status).toContainText('최근 2건만 표시합니다.');
  await expect(status).toContainText('이전에 보관한 내용은 그대로 읽을 수 있습니다.');
  await status.getByRole('button', { name: '공지 3110 원문 열기', exact: true }).click();
  const calls = (command: string) => page.evaluate((name) => (window as any).__fixture.calls.filter((call: any) => call.command === name).map((call: any) => call.args), command);
  await expect.poll(() => calls('archive_open_original')).toEqual([{ url: 'https://computing.sogang.ac.kr/ko/community/career/detail/3110' }]);
  await status.getByRole('button', { name: '가져오지 못한 공지 다시 시도', exact: true }).click();
  await expect.poll(() => calls('archive_start')).toEqual([{ sourceIds: ['computing-career'], mode: 'skipped' }]);
});

test('reader hydrates legacy attachment-role inline images once and rejects remote executable markup', async ({ page }) => {
  const foreign: string[] = [];
  await page.route('https://untrusted.invalid/**', (route) => { foreign.push(route.request().url()); return route.abort(); });
  await mockDesktop(page, { resources: [image], body: `<p onclick="window.bad=true">한국어 본문</p><p>CO<sub>2</sub>와 m<sup>2</sup></p><script>window.bad=true</script><iframe src="https://untrusted.invalid/frame"></iframe><svg onload="window.bad=true"></svg><img src="https://untrusted.invalid/tracker" data-resource-url="${image.url}" alt="첫 이미지"><img data-resource-url="${image.url}" alt="반복 이미지"><img data-resource-url="${image.url}" alt=""><a href="javascript:window.bad=true">위험 링크</a><a href="https://example.com/page">외부 링크</a>` });
  await page.goto('/');
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-body img')).toHaveCount(3);
  await expect(page.locator('.reader-body img').first()).toHaveAttribute('src', /^data:image\/png;base64,/);
  await expect(page.locator('.reader-body img').nth(2)).toHaveAttribute('alt', '');
  expect(await callCount(page, 'archive_resource_image')).toBe(1);
  await expect(page.locator('.reader-body sub')).toHaveText('2');
  await expect(page.locator('.reader-body sup')).toHaveText('2');
  await expect(page.locator('.reader-body script, .reader-body iframe, .reader-body svg, .reader-body [onclick]')).toHaveCount(0);
  await expect(page.getByRole('link', { name: '위험 링크', exact: true })).toHaveCount(0);
  await page.getByRole('link', { name: '외부 링크 ↗', exact: true }).click();
  expect(await callCount(page, 'archive_open_original')).toBe(1);
  expect(foreign).toEqual([]);
  expect(await page.evaluate(() => (window as any).bad)).toBeUndefined();
});

test('corrupt resources stay visible, offer repair and report final retry failure accurately', async ({ page }) => {
  await mockDesktop(page, { resources: [image], imageError: true, body: `<img data-resource-url="${image.url}">` });
  await page.goto('/');
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-missing-image')).toBeVisible();
  await expect(page.locator('.archive-local-status')).toContainText('손상');
  await page.getByRole('button', { name: '본문.png 다시 내려받기', exact: true }).click();
  await expect(page.locator('.archive-feedback')).toContainText('자료를 내려받지 못했습니다');
  await expect(page.getByRole('button', { name: '본문.png 다시 내려받기', exact: true })).toBeEnabled();
});

test('reader return restores visible focus after scrolling lower in a long list', async ({ page }) => {
  await mockDesktop(page, { count: 30 });
  await page.goto('/');
  await navigate(page, '받은 소식');
  await page.locator('#notice-28').scrollIntoViewIfNeeded();
  await page.locator('#notice-28').click();
  await expect(page.locator('.reader-header h2')).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.locator('#notice-28')).toBeFocused();
  await expect(page.locator('#notice-28')).toBeInViewport();
});

test('narrow reader contains wide tables, exposes keyboard scroll and preserves contrast', async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 800 });
  const row = '<tr>' + Array.from({ length: 20 }, (_, index) => `<td>매우긴내용${index}Computer장학금</td>`).join('') + '</tr>';
  await mockDesktop(page, { body: `<h2>긴 공지 표</h2><table><tbody>${row.repeat(4)}</tbody></table><p>${'긴 한국어 내용 '.repeat(100)}</p>` });
  await page.goto('/');
  await page.locator('#notice-1').click();
  const body = page.getByRole('region', { name: '공지 본문 · 넓은 표는 좌우로 이동할 수 있습니다', exact: true });
  await expect(body).toHaveAttribute('tabindex', '0');
  for (let step = 0; step < 8 && !await body.evaluate((node) => node === document.activeElement); step++) {
    await page.keyboard.press('Tab');
  }
  await expect(body).toBeFocused();
  await expect(body).toHaveCSS('outline-style', 'solid');
  await page.keyboard.press('ArrowRight');
  await expect.poll(() => body.evaluate((node) => node.scrollLeft)).toBeGreaterThan(0);
  const overflow = () => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth);
  expect(await overflow()).toBe(true);
  await page.evaluate(() => { document.documentElement.style.zoom = '2'; });
  expect(await overflow()).toBe(true);
  await page.evaluate(() => { document.documentElement.style.zoom = ''; });
  const ratios = await page.evaluate(() => {
    const style = getComputedStyle(document.documentElement);
    const luminance = (hex: string) => {
      // The production build minifies colors, so accept #rgb as well as #rrggbb.
      const digits = hex.trim().slice(1);
      const values = (digits.length === 3 ? digits.replace(/./g, '$&$&') : digits).match(/../g)!.map((x) => parseInt(x, 16) / 255).map((x) => x <= .04045 ? x / 12.92 : ((x + .055) / 1.055) ** 2.4);
      return values[0] * .2126 + values[1] * .7152 + values[2] * .0722;
    };
    const ratio = (a: string, b: string) => (Math.max(luminance(a), luminance(b)) + .05) / (Math.min(luminance(a), luminance(b)) + .05);
    return ['--secondary', '--muted'].flatMap((text) => ['--bg', '--surface', '--sidebar'].map((bg) => ratio(style.getPropertyValue(text), style.getPropertyValue(bg))));
  });
  for (const ratio of ratios) expect(ratio).toBeGreaterThanOrEqual(4.5);
});

test('obsolete searches cannot overwrite newer filters or clamp their page', async ({ page }) => {
  await mockDesktop(page, { count: 31 });
  await page.goto('/');
  await navigate(page, '탐색');
  await expect(page.locator('.notice-list > li')).toHaveCount(30);
  await page.evaluate(() => { (window as any).__fixture.options.queryDelay = 350; });
  await page.getByLabel('보관된 공지 검색').fill('없는검색어');
  await page.getByRole('button', { name: '검색', exact: true }).click();
  await expect(page.locator('.archive-view .loading')).toBeVisible();
  await page.evaluate(() => { (window as any).__fixture.options.queryDelay = 0; });
  await page.getByRole('combobox', { name: '분류', exact: true }).selectOption('장학');
  await expect(page.getByText('조건에 맞는 공지가 없습니다', { exact: true })).toBeVisible();
  await page.getByLabel('보관된 공지 검색').fill('Computer');
  await page.getByRole('button', { name: '검색', exact: true }).click();
  await expect(page.locator('.notice-list > li')).toHaveCount(16);
  await page.waitForTimeout(400); // Let the intentionally obsolete IPC response arrive.
  await expect(page.locator('.notice-list > li')).toHaveCount(16);
  await expect(page.locator('.archive-result-status')).toContainText('16개');
});

test('closing during image hydration stops subsequent obsolete image reads', async ({ page }) => {
  // Keep the admitted read pending until the user has actually closed the reader.
  await page.clock.install({ time: new Date('2026-11-02T12:00:00Z') });
  const resources = [image, { ...image, id: 8, url: `${image.url}?two` }, { ...image, id: 9, url: `${image.url}?three` }];
  await mockDesktop(page, { resources, imageDelay: 250, body: resources.map((item) => `<img data-resource-url="${item.url}">`).join('') });
  await page.goto('/');
  await navigate(page, '받은 소식');
  await expect(page.locator('#notice-1')).toBeVisible();
  await page.clock.pauseAt(new Date('2026-11-02T13:00:00Z'));
  await page.locator('#notice-1').click();
  await expect.poll(() => callCount(page, 'archive_resource_image')).toBe(1);
  await page.getByRole('button', { name: /목록으로 돌아가기/ }).click();
  await expect(page.locator('.archive-reader')).toHaveCount(0);
  await page.clock.runFor(350); // Complete the one already admitted local read after closing.
  await expect(page.locator('.notice-list')).toBeVisible();
  expect(await callCount(page, 'archive_resource_image')).toBe(1);
  await expect(page.locator('.archive-reader')).toHaveCount(0);
});

test('returning before automatic read write completes refreshes unread filters afterward', async ({ page }) => {
  await page.clock.install({ time: new Date('2026-11-02T12:00:00Z') });
  await mockDesktop(page);
  await page.goto('/');
  await navigate(page, '받은 소식');
  await page.getByLabel('읽지 않은 공지만').check();
  await expect(page.locator('.notice-list > li')).toHaveCount(2);
  await page.clock.pauseAt(new Date('2026-11-02T13:00:00Z'));
  await page.evaluate(() => { (window as any).__fixture.stateDelay = 300; });
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-header h2')).toBeVisible();
  await expect.poll(() => callCount(page, 'archive_state')).toBe(1);
  await page.getByRole('button', { name: /목록으로 돌아가기/ }).click();
  await expect(page.locator('.archive-reader')).toHaveCount(0);
  await page.clock.runFor(350);
  await expect(page.locator('.notice-list > li')).toHaveCount(1);
  await expect(page.locator('#notice-2')).toBeVisible();
  await expect(page.locator('.archive-list-heading h2')).toBeFocused();
});

test('disabling extension preserves local saved items and disables resource retries', async ({ page }) => {
  await mockDesktop(page, { saved: true, resources: [image] });
  await page.goto('/');
  await openSettings(page, '확장 기능');
  await page.getByRole('switch', { name: '서강대학교 공지 사용 설정', exact: true }).click();
  // Its settings pane and inbox disappear; saved items stay.
  await expect(page.getByRole('navigation', { name: '설정 항목', exact: true }).getByRole('button', { name: '게시판', exact: true })).toHaveCount(0);
  await expect(page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '받은 소식', exact: true })).toHaveCount(0);
  await navigate(page, '보관함');
  await expect(page.locator('.notice-list > li')).toHaveCount(2);
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-body')).toContainText('보관된 한국어 본문');
  await expect(page.getByRole('button', { name: '본문.png 다시 내려받기', exact: true })).toBeDisabled();
});

test('today rolls seven days forward from today and labels AI reference dates', async ({ page }) => {
  // A Saturday: the window must reach into next week instead of ending on Sunday.
  await page.clock.setFixedTime(new Date('2026-10-24T10:00:00'));
  await mockDesktop(page, { count: 7, aiResult: true });
  await page.goto('/');
  const week = page.getByRole('complementary', { name: '앞으로 7일', exact: true });
  await expect(week).toContainText('10월 24일 – 30일');
  const days = week.locator('.week-day');
  await expect(days).toHaveCount(7);
  await expect(days.first()).toHaveClass(/is-today/);
  await expect(days.first()).toContainText('오늘');
  await expect(days.first().locator('.event:not(.event-ai)')).toHaveCount(5);
  await expect(days.first()).toContainText('외 2건');
  await expect(days.last()).toContainText('금');
  await expect(days.last().locator('.event-ai')).toContainText('AI 참고 일정 신청 마감');
  await expect(week.getByRole('heading', { name: '이후 AI 참고 일정', exact: true })).toHaveCount(0);
  await expect(week).toContainText('추천 순위와 알림에 쓰지 않으니');
  await week.locator('#week-3').click();
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 03');
  await page.keyboard.press('Escape');
  await expect(page.locator('#week-3')).toBeFocused();
});

test('today lists past notices nowhere in the week and AI dates beyond it separately', async ({ page }) => {
  await page.clock.setFixedTime(new Date('2026-10-20T10:00:00'));
  await mockDesktop(page, { count: 3, aiResult: true });
  await page.goto('/');
  const week = page.getByRole('complementary', { name: '앞으로 7일', exact: true });
  await expect(week).toContainText('10월 20일 – 26일');
  // Notices published on 10월 24일 fall inside the rolling window.
  await expect(week.locator('.week-day').nth(4).locator('.event:not(.event-ai)')).toHaveCount(3);
  await expect(week.getByRole('heading', { name: '이후 AI 참고 일정', exact: true })).toBeVisible();
  await expect(week.locator('.event-ai')).toContainText('AI · 10월 30일');
});

test('today moves its highlight to the new day when the window rolls past midnight', async ({ page }) => {
  await page.clock.setFixedTime(new Date('2026-10-24T23:59:00'));
  await mockDesktop(page, { count: 3 });
  await page.goto('/');
  const week = page.getByRole('complementary', { name: '앞으로 7일', exact: true });
  await expect(week).toContainText('10월 24일 – 30일');
  await page.clock.setFixedTime(new Date('2026-10-25T00:01:00'));
  // Closing the reader refreshes the archive, which recomputes the window.
  await page.locator('#notice-1').click();
  await page.keyboard.press('Escape');
  await expect(week).toContainText('10월 25일 – 31일');
  await expect(week.locator('.week-day').first()).toHaveClass(/is-today/);
  await expect(week.locator('.is-today')).toHaveCount(1);
});

test('sidebar boards scope the inbox and badges describe their counts', async ({ page }) => {
  await mockDesktop(page, { count: 3, saved: true });
  await page.goto('/');
  const sidebar = page.getByRole('navigation', { name: '기본 탐색', exact: true });
  await expect(sidebar).toContainText('구독한 게시판 없음');
  await expect(sidebar.getByRole('button', { name: '받은 소식', exact: true })).toHaveAccessibleDescription('읽지 않은 공지 3개');
  await expect(sidebar.getByRole('button', { name: '탐색', exact: true })).toHaveAccessibleDescription('보관된 공지 3개');
  await expect(sidebar.getByRole('button', { name: '보관함', exact: true })).toHaveAccessibleDescription('저장한 공지 3개');
  // Nothing arrived with the last sync, so 오늘 shows no badge rather than a zero.
  await expect(sidebar.getByRole('button', { name: '오늘', exact: true })).not.toHaveAttribute('aria-describedby');
  await page.evaluate(() => (window as any).__TAURI_INTERNALS__.invoke('archive_subscribe', { id: 'computing-updates', subscribed: true }));
  await navigate(page, '설정');
  await navigate(page, '오늘');
  await sidebar.getByRole('button', { name: '소프트웨어융합대학 소식', exact: true }).click();
  // A scoped inbox is titled by its board, like a mailbox, with the organization beside it.
  await expect(page.getByRole('heading', { level: 1, name: '소식', exact: true })).toBeVisible();
  await expect(page.locator('.page-header p')).toHaveText('소프트웨어융합대학 · 받은 소식');
  await expect(page.getByRole('combobox', { name: '정보원', exact: true })).toHaveValue('computing-updates');
  await expect(sidebar.getByRole('button', { name: '소프트웨어융합대학 소식', exact: true })).toHaveAttribute('aria-current', 'page');
  await expect(sidebar.getByRole('button', { name: '소프트웨어융합대학 소식', exact: true })).toHaveAccessibleDescription('읽지 않은 공지 3개');
  await expect(sidebar.getByRole('button', { name: '받은 소식', exact: true })).not.toHaveAttribute('aria-current');
  await page.getByRole('combobox', { name: '정보원', exact: true }).selectOption('');
  await expect(sidebar.getByRole('button', { name: '소프트웨어융합대학 소식', exact: true })).not.toHaveAttribute('aria-current');
});

test('board groups nest by level, and a group off hides and stops its subtree but keeps each pick', async ({ page }) => {
  const board = (id: string, name: string, group: string, subscribed = false) => ({ id, name, organization: '서강대학교', supported: true, subscribed, group });
  await mockDesktop(page, {
    sourceGroups: [
      { id: 'sogang', name: '서강대학교', parent: null },
      { id: 'sogang-eng', name: '공과대학', parent: 'sogang' },
      { id: 'sogang-me', name: '기계공학과', parent: 'sogang-eng' },
      { id: 'sogang-computing', name: '소프트웨어융합대학', parent: 'sogang' },
    ],
    sourceList: [
      board('sogang-academic', '학부 학사공지', 'sogang', true),
      board('eng-research', '연구공지', 'sogang-eng', true),
      board('eng-general', '일반공지', 'sogang-eng'),
      board('me-general', '기계 일반공지', 'sogang-me', true),
      board('computing-updates', '소식', 'sogang-computing'),
    ],
  });
  await page.goto('/');
  await openSettings(page, '게시판');
  const group = (name: string) => page.getByRole('group', { name, exact: true });
  const toggle = (name: string) => page.getByRole('switch', { name: `${name} 전체`, exact: true });
  const pick = (name: string) => page.getByRole('switch', { name: `${name} 구독`, exact: true });

  // Each level sits inside its parent, with its own boards as small switches.
  await expect(group('서강대학교').getByRole('heading', { name: '공과대학', level: 4 })).toBeVisible();
  await expect(group('공과대학').getByRole('heading', { name: '기계공학과', level: 5 })).toBeVisible();
  await expect(group('공과대학').getByRole('list', { name: '공과대학 게시판' }).getByRole('switch')).toHaveCount(2);
  await expect(group('서강대학교').locator('.board-group-count').first()).toHaveText('구독 3/5');

  // Level 2 off: its boards and departments disappear, its sibling stays.
  await toggle('공과대학').press('Space');
  await expect(toggle('공과대학')).toHaveAttribute('aria-checked', 'false');
  await expect(pick('공과대학 연구공지')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: '기계공학과' })).toHaveCount(0);
  await expect(pick('소프트웨어융합대학 소식')).toBeVisible();
  await expect(group('공과대학')).toContainText('꺼짐 · 게시판 3개');
  await expect(page.locator('.source-manager .archive-feedback')).toContainText('고른 게시판과 보관한 공지는 그대로 남습니다');
  await expect(page.locator('.collect-status')).toHaveText('구독 1개');

  // Level 1 off hides everything below it.
  await toggle('서강대학교').click();
  await expect(page.locator('.board-tile')).toHaveCount(0);
  await expect(toggle('공과대학')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '새로고침', exact: true })).toBeDisabled();

  // Back on, each group returns to its own state and each board to its pick.
  await toggle('서강대학교').click();
  await expect(toggle('공과대학')).toHaveAttribute('aria-checked', 'false');
  await toggle('공과대학').click();
  await expect(pick('공과대학 연구공지')).toHaveAttribute('aria-checked', 'true');
  await expect(pick('공과대학 일반공지')).toHaveAttribute('aria-checked', 'false');
  await expect(pick('기계공학과 기계 일반공지')).toHaveAttribute('aria-checked', 'true');
  await expect(page.locator('.collect-status')).toHaveText('구독 3개');
  expect(await callCount(page, 'archive_set_group')).toBe(4);
});
