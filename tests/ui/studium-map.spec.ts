import { test, expect, type Page } from '@playwright/test';
import { callCount, mockDesktop, openSettings, syntheticStudiumLayout } from './fixture';
import type { StudiumRoomState } from '../../src/lib/types';

const preferences = { room_ids: ['5', '6'], view_room: '5' };
function row(id = '5', numbers: number[] = [1, 84]): StudiumRoomState {
  const total = id === '6' ? 311 : 102;
  const time = new Date().toISOString();
  return { room_id: id, status: 'ready', error: null, checked_at: time, result: {
    total, used: total - numbers.length, remaining: numbers.length, seat_numbers: numbers,
    source_time: time, source_url: `https://libseat.sogang.ac.kr:444/SEAT/roomview5.asp?room_no=${id}`,
    layout: syntheticStudiumLayout(total), layout_error: null,
  } };
}
const nav = (page: Page) => page.getByRole('navigation', { name: '기본 탐색', exact: true });
const map = (page: Page) => page.locator('.seat-map');
const viewport = (page: Page) => page.getByRole('region', { name: /좌석 배치도 · 방향키로 이동/ });
async function openStudium(page: Page) {
  await page.goto('/');
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect(page.getByRole('heading', { name: '확인할 열람실', exact: true })).toBeVisible();
}
async function openMap(page: Page) {
  await page.getByRole('button', { name: '좌석 배치도 열기', exact: true }).click();
  await expect(viewport(page)).toBeVisible();
}

test('map opens locally with complete numbered symbols, padding and the same source time', async ({ page }) => {
  const ready = row();
  await mockDesktop(page, { extensions: { studium: true }, studium: { results: [ready] } });
  await openStudium(page);
  await expect(page.getByRole('button', { name: '좌석 배치도 열기', exact: true })).toHaveAttribute('aria-expanded', 'false');
  await expect(map(page).locator('svg')).toHaveCount(0);
  const calls = await page.evaluate(() => (window as any).__fixture.calls.length);
  await openMap(page);
  await expect(map(page).locator('g[data-seat-number]')).toHaveCount(102);
  await expect(map(page).locator('[data-seat-number="1"] .marker')).toHaveText('○');
  await expect(map(page).locator('[data-seat-number="2"] .marker')).toHaveText('×');
  await expect(map(page).locator('.map-summary')).toHaveText('로욜라 인벤툼 II · 전체 102석 · 빈 좌석 2석 · 사용 중 100석');
  await expect(map(page).locator('time')).toHaveAttribute('datetime', ready.result!.source_time);
  await expect(map(page)).toContainText('실제 좌석의 크기·형태나 벽·문을 나타내지 않습니다.');
  await expect(map(page).locator('svg')).toHaveAttribute('aria-hidden', 'true');
  await expect(map(page).locator('[tabindex="0"]')).toHaveCount(1);
  await page.evaluate(() => document.fonts.ready);
  const clipped = await map(page).locator('svg').evaluate((node) => {
    const svg = node as SVGSVGElement;
    const bounds = svg.viewBox.baseVal;
    return [...svg.querySelectorAll('text')].some((text) => {
      const box = text.getBBox();
      return box.x < bounds.x || box.y < bounds.y || box.x + box.width > bounds.x + bounds.width || box.y + box.height > bounds.y + bounds.height;
    });
  });
  expect(clipped).toBe(false);
  await page.getByRole('button', { name: '좌석 배치도 닫기', exact: true }).click();
  await expect(map(page).locator('svg')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__fixture.calls.length)).toBe(calls);
});

test.describe('map source clocks follow the saved format in Korea', () => {
  test.use({ timezoneId: 'America/New_York' });

  test('main detail and map retain 12/24-hour preferences across settings navigation without querying', async ({ page }) => {
    const ready = row();
    ready.result!.source_time = '2026-11-09T12:34:56Z';
    await mockDesktop(page, { extensions: { studium: true }, studium: { results: [ready] } });
    await openStudium(page);
    await openMap(page);
    await expect(map(page).locator('time')).toContainText('오후 9:34:56');
    await expect(page.locator('.times time').first()).toContainText('오후 9:34:56');
    await openSettings(page, '일반');
    const formats = page.getByRole('group', { name: '시간 표시', exact: true });
    await formats.getByRole('button', { name: '24시간', exact: true }).click();
    await expect(formats.getByRole('button', { name: '24시간', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
    await openMap(page);
    await expect(map(page).locator('time')).toContainText('21:34:56');
    await expect(map(page).locator('time')).not.toContainText('오후');
    await expect(page.locator('.times time').first()).toContainText('21:34:56');
    expect(await callCount(page, 'studium_start')).toBe(0);
    expect(await callCount(page, 'studium_automatic_start')).toBe(0);
  });
});

test('favorites and exact-number search emphasize verified empty seats without hiding other symbols', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { favorites: { '5': [2, 84] }, results: [row()] } });
  await openStudium(page);
  await openMap(page);
  await expect(map(page).locator('[data-seat-number="2"] .marker')).toHaveText('×');
  await expect(map(page).locator('[data-seat-number="2"] .favorite')).toHaveText('★');
  await expect(map(page).locator('[data-seat-number="84"] .favorite')).toHaveText('★');
  const search = page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true });
  await search.fill('84');
  await expect(map(page).locator('.search-match')).toHaveCount(1);
  await expect(map(page).locator('.search-match')).toHaveAttribute('data-seat-number', '84');
  await expect(map(page).locator('.search-match .number')).toHaveCSS('text-decoration-line', 'underline');
  await expect(map(page).locator('.map-summary')).toContainText('검색 결과 1개');
  await expect(page.locator('.search-count')).toHaveText('전체 빈 좌석 2석 · 검색 결과 1개');
  await expect(map(page).locator('[data-seat-number="2"]')).toHaveCSS('opacity', '1');
  await search.fill('2');
  await expect(map(page).locator('.search-match')).toHaveCount(0);
  await expect(map(page).locator('.map-summary')).toContainText('검색 결과 0개');
  await expect(map(page).locator('[data-seat-number="2"] .marker')).toHaveText('×');
  await page.getByRole('button', { name: '검색 해제', exact: true }).click();
  await expect(page.getByRole('button', { name: '좌석 배치도 닫기', exact: true })).toHaveAttribute('aria-expanded', 'true');
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_set_view_room')).toBe(0);
  expect(await callCount(page, 'studium_save_favorites')).toBe(0);
});

test('map zoom is bounded and its keyboard scroll region pans and resets locally', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences: { room_ids: ['6'], view_room: '6' }, results: [row('6', [1, 311])] } });
  await page.setViewportSize({ width: 640, height: 900 });
  await openStudium(page);
  await openMap(page);
  const larger = page.getByRole('button', { name: '배치도 확대', exact: true });
  const smaller = page.getByRole('button', { name: '배치도 축소', exact: true });
  await expect(map(page).locator('output')).toHaveText('100%');
  const normalWidth = await map(page).locator('svg').getAttribute('width');
  for (let step = 0; step < 4; step++) await larger.click();
  await expect(map(page).locator('output')).toHaveText('200%');
  await expect(larger).toBeDisabled();
  await viewport(page).focus();
  await page.keyboard.press('ArrowRight');
  await expect.poll(() => viewport(page).evaluate((node) => node.scrollLeft)).toBeGreaterThan(0);
  await page.keyboard.press('ArrowDown');
  await expect.poll(() => viewport(page).evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
  await expect(viewport(page)).toBeFocused();
  await expect(viewport(page)).toHaveCSS('outline-style', 'solid');
  await page.getByRole('button', { name: '배치도 보기 초기화', exact: true }).click();
  await expect(map(page).locator('output')).toHaveText('100%');
  await expect.poll(() => viewport(page).evaluate((node) => ({
    width: node.querySelector('svg')?.getAttribute('width'), left: node.scrollLeft, top: node.scrollTop,
  }))).toEqual({ width: normalWidth, left: 0, top: 0 });
  for (let step = 0; step < 2; step++) await smaller.click();
  await expect(map(page).locator('output')).toHaveText('50%');
  await expect(smaller).toBeDisabled();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
});

test('repeated resets at normal zoom preserve viewport, control and outside focus', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences: { room_ids: ['6'], view_room: '6' }, results: [row('6')] } });
  await page.setViewportSize({ width: 640, height: 900 });
  await openStudium(page);
  await openMap(page);
  const reset = page.getByRole('button', { name: '배치도 보기 초기화', exact: true });
  const search = page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true });
  for (const focus of ['viewport', 'control', 'outside']) {
    await viewport(page).focus();
    await page.keyboard.press('ArrowDown');
    await expect.poll(() => viewport(page).evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
    if (focus === 'control') {
      await reset.focus();
      await page.keyboard.press('Enter');
      await expect(reset).toBeFocused();
    } else {
      await reset.evaluate((button, moveOutside) => {
        (button as HTMLButtonElement).click();
        if (moveOutside) document.getElementById('studium-seat-search')?.focus();
      }, focus === 'outside');
      await expect(focus === 'outside' ? search : viewport(page)).toBeFocused();
    }
    await expect(map(page).locator('output')).toHaveText('100%');
    await expect.poll(() => viewport(page).evaluate((node) => [node.scrollLeft, node.scrollTop])).toEqual([0, 0]);
  }
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
});

test('a layout-only error and invalid geometry preserve the verified counts and number list', async ({ page }) => {
  const ready = row();
  ready.result!.layout = null;
  ready.result!.layout_error = '좌석의 상대 위치를 확인하지 못했습니다. 번호 목록을 이용해 주세요.';
  await mockDesktop(page, { extensions: { studium: true }, studium: { results: [ready] } });
  await openStudium(page);
  await expect(page.locator('.seat-count')).toContainText('2석');
  await expect(page.locator('.seat-numbers')).toContainText('1, 84번');
  await expect(page.getByRole('button', { name: '좌석 배치도 열기', exact: true })).toBeDisabled();
  await expect(map(page).getByRole('status')).toHaveText(ready.result!.layout_error!);
  const invalid = row();
  invalid.result!.layout!.points[1] = { ...invalid.result!.layout!.points[0] };
  await page.evaluate((value) => { const fixture = (window as any).__fixture; fixture.studium.results = [value]; fixture.emitStudiumState(); }, invalid);
  await expect(map(page).getByRole('status')).toHaveText('좌석 위치를 확인하지 못했습니다. 번호 목록을 이용해 주세요.');
  await expect(page.locator('.seat-numbers')).toContainText('1, 84번');
  await expect(map(page).locator('svg')).toHaveCount(0);
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('missing layout and count-only results never invent a map or occupied seats', async ({ page }) => {
  const missing = row(), counts = row('6', [1, 2, 3]);
  missing.result!.layout = null;
  counts.status = 'count_only'; counts.result!.layout = null; counts.result!.seat_numbers = null;
  counts.result!.source_url = 'https://libseat.sogang.ac.kr:444/SEAT/domian5.asp';
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [missing, counts] } });
  await openStudium(page);
  await expect(map(page).getByRole('status')).toHaveText('이 조회에는 좌석 위치 정보가 없습니다. 번호 목록을 이용해 주세요.');
  await expect(page.locator('.seat-numbers')).toContainText('1, 84번');
  await page.getByRole('combobox', { name: '표시할 열람실', exact: true }).selectOption('6');
  await expect(map(page).getByRole('status')).toHaveText('좌석 번호와 위치를 확인하지 못했습니다. 확인된 빈 좌석 수를 이용해 주세요.');
  await expect(page.locator('.seat-count')).toContainText('3석');
  await expect(map(page).locator('svg')).toHaveCount(0);
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('verified zero has occupied symbols and stale maps explicitly remain historical', async ({ page }) => {
  const zero = row('5', []), old = row('6', [311]);
  old.result!.source_time = '2026-01-01T00:00:00Z';
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [zero, old] } });
  await openStudium(page);
  await openMap(page);
  await expect(map(page).locator('.available')).toHaveCount(0);
  await expect(map(page).locator('.occupied')).toHaveCount(102);
  await expect(map(page).locator('.map-summary')).toContainText('빈 좌석 0석 · 사용 중 102석');
  await page.getByRole('combobox', { name: '표시할 열람실', exact: true }).selectOption('6');
  await expect(page.getByRole('button', { name: '좌석 배치도 열기', exact: true })).toHaveAttribute('aria-expanded', 'false');
  await openMap(page);
  await expect(map(page).locator('.map-warning')).toHaveText('이전 조회의 위치와 상태입니다. 현재 빈자리 여부는 확인되지 않았습니다.');
  await expect(map(page).locator('time')).toHaveAttribute('datetime', old.result!.source_time);
  await page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true }).fill('311');
  await expect(map(page).locator('.map-summary')).toContainText('검색 결과 1개 · 이전 조회 기준');
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('a same-result refresh preserves map controls and a new result resets them without stealing outside focus', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, job_id: 1, results: [row(), row('6', [311])] } });
  await openStudium(page);
  await openMap(page);
  await page.getByRole('button', { name: '배치도 확대', exact: true }).click();
  await page.evaluate(() => (window as any).__fixture.emitStudiumState());
  await expect(map(page).locator('output')).toHaveText('125%');
  await page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true }).fill('84');
  await expect(map(page).locator('output')).toHaveText('125%');
  const view = page.getByRole('combobox', { name: '표시할 열람실', exact: true });
  await view.focus();
  await view.selectOption('6');
  await expect(page.getByRole('button', { name: '좌석 배치도 열기', exact: true })).toHaveAttribute('aria-expanded', 'false');
  await expect(view).toBeFocused();
  await openMap(page);
  await expect(map(page).locator('output')).toHaveText('100%');
  await viewport(page).focus();
  const fresh = row('6', [1]);
  await page.evaluate((value) => { const fixture = (window as any).__fixture; fixture.studium.job_id = 2; fixture.studium.results = [value]; fixture.emitStudiumState(); }, fresh);
  await expect(page.getByRole('button', { name: '좌석 배치도 열기', exact: true })).toBeFocused();
  await openMap(page);
  await expect(map(page).locator('[data-seat-number="1"] .marker')).toHaveText('○');
  await expect(map(page).locator('[data-seat-number="311"] .marker')).toHaveText('×');
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('automatic clearing of a focused map restores the room heading and never retains an old map', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { job_id: 1, results: [row()] } });
  await openStudium(page);
  await openMap(page);
  await viewport(page).focus();
  await page.evaluate(() => {
    const fixture = (window as any).__fixture;
    fixture.studium.running = true; fixture.studium.job_id = 2;
    fixture.studium.results = [{ room_id: '5', status: 'loading', result: null, error: null, checked_at: null }];
    fixture.emitStudiumState();
  });
  await expect(page.locator('.room-detail h3')).toBeFocused();
  await expect(map(page)).toHaveCount(0);
  await expect(page.locator('.seat-numbers')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '확인 취소', exact: true })).toBeVisible();
  await page.evaluate(() => {
    const fixture = (window as any).__fixture;
    fixture.studium.running = false;
    fixture.studium.results = [{ room_id: '5', status: 'failed', result: null, error: '학교 응답 오류', checked_at: new Date().toISOString() }];
    fixture.emitStudiumState();
  });
  await expect(map(page)).toHaveCount(0);
  await expect(page.locator('.seat-count')).toHaveCount(0);
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('leaving a focused map respects the normal page navigation focus', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { results: [row()] } });
  await openStudium(page);
  await openMap(page);
  await viewport(page).focus();
  await nav(page).getByRole('button', { name: '오늘', exact: true }).click();
  await expect(page.getByRole('heading', { name: '오늘', exact: true })).toBeVisible();
  await expect(page.locator('#main')).toBeFocused();
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('map controls and symbols remain readable in dark theme at 200% content zoom with reduced motion', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await mockDesktop(page, { extensions: { studium: true }, studium: { results: [row()] } });
  await page.setViewportSize({ width: 1280, height: 900 });
  await openStudium(page);
  await openSettings(page, '일반');
  await page.getByRole('button', { name: '다크', exact: true }).click();
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await page.evaluate(() => { document.documentElement.style.zoom = '2'; });
  await page.getByRole('button', { name: '좌석 배치도 열기', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(viewport(page)).toBeVisible();
  const ratios = await map(page).evaluate((node) => {
    const luminance = (color: string) => {
      const [r, g, b] = color.match(/[\d.]+/g)!.slice(0, 3).map(Number).map((value) => {
        const channel = value / 255;
        return channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4;
      });
      return r * .2126 + g * .7152 + b * .0722;
    };
    const background = luminance(getComputedStyle(node.querySelector('.map-viewport')!).backgroundColor);
    return ['.available .number', '.occupied .number'].map((selector) => {
      const text = luminance(getComputedStyle(node.querySelector(selector)!).fill);
      return (Math.max(text, background) + .05) / (Math.min(text, background) + .05);
    });
  });
  for (const ratio of ratios) expect(ratio).toBeGreaterThanOrEqual(4.5);
  await expect(map(page).getByRole('list', { name: '배치도 기호', exact: true })).toContainText('○ 빈 좌석');
  await expect(map(page).getByRole('list', { name: '배치도 기호', exact: true })).toContainText('× 사용 중');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  expect(await callCount(page, 'studium_start')).toBe(0);
});
