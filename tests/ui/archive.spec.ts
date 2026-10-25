import { test, expect } from '@playwright/test';
import { mockDesktop, callCount } from './fixture';
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
  await expect(page.getByRole('button', { name: '읽음으로 표시', exact: true })).toHaveAttribute('aria-pressed', 'false');
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
  await expect(page.locator('.notice-list > li')).toHaveCount(2);
});

test('saving an item admits only one pending write and announces feedback', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await page.evaluate(() => (window as any).__fixture.stateDelay = 400);
  const save = page.getByRole('button', { name: '한국어 공지 01 보관함에 저장', exact: true });
  await save.click();
  await expect(save).toBeDisabled();
  await expect.poll(() => callCount(page, 'archive_state')).toBe(1);
  await expect(page.getByRole('button', { name: '한국어 공지 01 저장 해제', exact: true })).toHaveAttribute('aria-pressed', 'true');
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

test('successful empty archive is distinct from loading and errors', async ({ page }) => {
  await mockDesktop(page, { count: 0, queryDelay: 300 });
  await page.goto('/');
  await expect(page.locator('.archive-view .loading')).toBeVisible();
  await expect(page.getByText('아직 보관된 공지가 없습니다', { exact: true })).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('settings failure rolls back optimistic theme; system theme follows OS', async ({ page }) => {
  await mockDesktop(page, { settingsError: true });
  await page.goto('/');
  await navigate(page, '설정');
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

test('subscriptions have no automatic collection, protect refreshing state, show errors and cancel', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await navigate(page, '설정');
  const subscribe = page.getByRole('switch', { name: '소프트웨어융합대학 소식 구독', exact: true });
  await expect(subscribe).toBeEnabled();
  await expect(page.getByRole('switch', { name: '다른 학과 미검증 게시판 구독', exact: true })).toBeDisabled();
  await expect(page.getByText('일부 가져옴 · 이어받기 가능', { exact: true })).toBeVisible();
  await page.evaluate(() => { (window as any).__fixture.sourceDelay = 400; });
  await page.getByRole('button', { name: '상태 새로고침', exact: true }).click();
  await expect(subscribe).toBeDisabled();
  await expect(subscribe).toBeEnabled();
  await page.evaluate(() => { (window as any).__fixture.subscribeError = true; });
  await subscribe.click();
  await expect(page.getByRole('alert')).toContainText('구독 설정을 저장하지 못했습니다');
  await expect(subscribe).toHaveAttribute('aria-checked', 'false');
  await page.evaluate(() => { (window as any).__fixture.subscribeError = false; });
  await subscribe.click();
  await expect(subscribe).toHaveAttribute('aria-checked', 'true');
  expect(await callCount(page, 'archive_start')).toBe(0);
  await page.getByRole('button', { name: '과거 자료 가져오기', exact: true }).click();
  await expect(page.getByRole('button', { name: '가져오기 중단', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '가져오기 중단', exact: true }).click();
  await expect(page.locator('.source-manager .archive-feedback')).toContainText('이미 저장한 공지는 유지됩니다');
});

test('reader hydrates legacy attachment-role inline images once and rejects remote executable markup', async ({ page }) => {
  const foreign: string[] = [];
  await page.route('https://untrusted.invalid/**', (route) => { foreign.push(route.request().url()); return route.abort(); });
  await mockDesktop(page, { resources: [image], body: `<p onclick="window.bad=true">한국어 본문</p><p>CO<sub>2</sub>와 m<sup>2</sup></p><script>window.bad=true</script><iframe src="https://untrusted.invalid/frame"></iframe><svg onload="window.bad=true"></svg><img src="https://untrusted.invalid/tracker" data-resource-url="${image.url}" alt="첫 이미지"><img data-resource-url="${image.url}" alt="반복 이미지"><a href="javascript:window.bad=true">위험 링크</a><a href="https://example.com/page">외부 링크</a>` });
  await page.goto('/');
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-body img')).toHaveCount(2);
  await expect(page.locator('.reader-body img').first()).toHaveAttribute('src', /^data:image\/png;base64,/);
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
      const values = hex.trim().slice(1).match(/../g)!.map((x) => parseInt(x, 16) / 255).map((x) => x <= .04045 ? x / 12.92 : ((x + .055) / 1.055) ** 2.4);
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
  const resources = [image, { ...image, id: 8, url: `${image.url}?two` }, { ...image, id: 9, url: `${image.url}?three` }];
  await mockDesktop(page, { resources, imageDelay: 250, body: resources.map((item) => `<img data-resource-url="${item.url}">`).join('') });
  await page.goto('/');
  await page.locator('#notice-1').click();
  await expect.poll(() => callCount(page, 'archive_resource_image')).toBe(1);
  await page.getByRole('button', { name: /목록으로 돌아가기/ }).click();
  await expect(page.locator('.notice-list')).toBeVisible();
  await page.waitForTimeout(350); // Observe completion of the one already admitted local read.
  expect(await callCount(page, 'archive_resource_image')).toBe(1);
  await expect(page.locator('.archive-reader')).toHaveCount(0);
});

test('returning before automatic read write completes refreshes unread filters afterward', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await navigate(page, '받은 소식');
  await page.getByLabel('읽지 않은 공지만').check();
  await page.evaluate(() => { (window as any).__fixture.stateDelay = 300; });
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-header h2')).toBeVisible();
  await page.getByRole('button', { name: /목록으로 돌아가기/ }).click();
  await expect(page.locator('.notice-list > li')).toHaveCount(1);
  await expect(page.locator('#notice-2')).toBeVisible();
  await expect(page.locator('.archive-list-heading h2')).toBeFocused();
});

test('disabling extension preserves local saved items and disables resource retries', async ({ page }) => {
  await mockDesktop(page, { saved: true, resources: [image] });
  await page.goto('/');
  await navigate(page, '설정');
  await page.getByRole('switch', { name: '서강대학교 공지 사용 설정', exact: true }).click();
  await expect(page.locator('.source-manager')).toHaveCount(0);
  await navigate(page, '보관함');
  await expect(page.locator('.notice-list > li')).toHaveCount(2);
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-body')).toContainText('보관된 한국어 본문');
  await expect(page.getByRole('button', { name: '본문.png 다시 내려받기', exact: true })).toBeDisabled();
});
