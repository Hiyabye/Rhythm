import { test, expect, type Page } from '@playwright/test';
import { mockDesktop, callCount, syntheticStudiumLayout } from './fixture';
import type { StudiumRoomState } from '../../src/lib/types';

const preferences = { room_ids: ['5', '6'], view_room: '5' };
const row = (id: string, remaining: number, status: 'ready' | 'count_only' = 'ready'): StudiumRoomState => ({
  room_id: id, status, error: status === 'count_only' ? '좌석 번호를 확인하지 못했습니다.' : null, checked_at: new Date().toISOString(),
  result: { total: id === '6' ? 311 : 102, used: (id === '6' ? 311 : 102) - remaining, remaining, source_time: new Date().toISOString(), seat_numbers: status === 'count_only' ? null : Array.from({ length: remaining }, (_, index) => index + 1), source_url: status === 'count_only' ? 'https://libseat.sogang.ac.kr:444/SEAT/domian5.asp' : `https://libseat.sogang.ac.kr:444/SEAT/roomview5.asp?room_no=${id}`, layout: status === 'count_only' ? null : syntheticStudiumLayout(id === '6' ? 311 : 102), layout_error: null },
});
const nav = (page: Page) => page.getByRole('navigation', { name: '기본 탐색', exact: true });
async function openMini(page: Page) {
  await page.goto('/?studium-mini=1');
  await expect(page.getByRole('heading', { name: 'Studium 미니 창', exact: true })).toBeVisible();
}
async function openMainStudium(page: Page) {
  await page.goto('/');
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect(page.getByRole('heading', { name: '확인할 열람실', exact: true })).toBeVisible();
}

test('main opens the mini window explicitly and retains edits after a window error', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, miniOpenError: true });
  await openMainStudium(page);
  await page.getByRole('textbox', { name: '선호 좌석 번호', exact: true }).fill('42');
  await page.getByRole('button', { name: '미니 창 열기', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('미니 창을 열지 못했습니다.');
  await expect(page.getByRole('textbox', { name: '선호 좌석 번호', exact: true })).toHaveValue('42');
  await page.evaluate(() => { (window as any).__fixture.options.miniOpenError = false; });
  await page.getByRole('button', { name: '미니 창 열기', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: '미니 창을 열었습니다.' })).toBeVisible();
  await page.getByRole('button', { name: '미니 창 열기', exact: true }).click();
  expect(await page.evaluate(() => (window as any).__fixture.mini.open)).toBe(true);
  expect(await callCount(page, 'studium_open_mini')).toBe(3);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
});

test('mini opening, room selection and always-on-top only display the shared snapshot', async ({ page }) => {
  const ready = row('5', 2);
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [ready, row('6', 3, 'count_only')] } });
  await page.setViewportSize({ width: 300, height: 480 });
  await openMini(page);
  await expect(page.getByRole('main', { name: 'Studium 미니 창' })).toBeVisible();
  await expect(page.getByRole('checkbox', { name: '항상 위에 표시', exact: true })).not.toBeChecked();
  await expect(page.locator('.seat-count')).toContainText('2석');
  await expect(page.getByRole('region', { name: '로욜라 인벤툼 II 좌석 정보', exact: true }).locator('time')).toHaveAttribute('datetime', ready.result!.source_time);
  await expect(page.getByRole('navigation')).toHaveCount(0);
  const helper = await page.locator('.mini-hint').evaluate((node) => {
    const box = node.getBoundingClientRect();
    return { width: box.width, height: box.height, lineHeight: Number.parseFloat(getComputedStyle(node).lineHeight) };
  });
  expect(helper.width).toBeGreaterThanOrEqual(240);
  expect(helper.lineHeight).toBeGreaterThanOrEqual(18);
  expect(helper.height).toBeLessThan(150);
  await page.getByRole('combobox', { name: '표시할 열람실', exact: true }).selectOption('6');
  await expect(page.getByRole('region', { name: 'K관 열람실 좌석 정보', exact: true })).toContainText('빈 좌석 3석 · 번호 확인 불가');
  await page.getByRole('checkbox', { name: '항상 위에 표시', exact: true }).check();
  expect(await page.evaluate(() => (window as any).__fixture.mini.always_on_top)).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
  expect(await callCount(page, 'studium_status')).toBe(0);
  expect(await callCount(page, 'ai_snapshot')).toBe(0);
});

test('mini receives idle changes from another view and polls only the shared active job', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [row('5', 1), row('6', 0)] } });
  await openMini(page);
  await expect(page.getByRole('combobox', { name: '표시할 열람실', exact: true })).toHaveValue('5');
  await page.evaluate(() => {
    const fixture = (window as any).__fixture;
    fixture.studium.preferences.view_room = '6'; fixture.studium.running = true; fixture.studium.job_id = 9;
    fixture.studium.results[1] = { room_id: '6', status: 'loading', result: null, error: null, checked_at: null };
    fixture.emitStudiumState();
  });
  await expect(page.getByRole('combobox', { name: '표시할 열람실', exact: true })).toHaveValue('6');
  await expect(page.getByRole('button', { name: '확인 취소', exact: true })).toBeVisible();
  await expect.poll(() => callCount(page, 'studium_status')).toBeGreaterThan(0);
  const globalReads = await callCount(page, 'get_snapshot');
  await page.evaluate((results) => (window as any).__fixture.finishStudiumCycle(results), [row('5', 0), row('6', 4)]);
  await expect(page.getByRole('region', { name: 'K관 열람실 좌석 정보', exact: true })).toContainText('빈 좌석 4석');
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeEnabled();
  const polls = await callCount(page, 'studium_status');
  await page.waitForTimeout(1_200);
  expect(await callCount(page, 'studium_status')).toBe(polls);
  expect(await callCount(page, 'get_snapshot')).toBe(globalReads);
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('mini keyboard manual start and cancellation reuse the owning controller job', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences } });
  await openMini(page);
  await page.getByRole('button', { name: '지금 확인', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: '확인 취소', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeDisabled();
  await page.evaluate(() => (window as any).__fixture.finishStudiumCancel());
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeFocused();
  await expect(page.getByRole('region', { name: '로욜라 인벤툼 II 좌석 정보', exact: true })).toContainText('확인 취소됨');
  expect(await callCount(page, 'studium_start')).toBe(1);
  expect(await callCount(page, 'studium_cancel')).toBe(1);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
});

test('closing mini removes subscriptions, resets always-on-top and leaves a running check alone', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { running: true, job_id: 9, results: [{ room_id: '5', status: 'loading', result: null, error: null, checked_at: null }] } });
  await openMini(page);
  await expect(page.getByRole('button', { name: '확인 취소', exact: true })).toBeVisible();
  await page.getByRole('checkbox', { name: '항상 위에 표시', exact: true }).check();
  await page.getByRole('button', { name: '미니 창 닫기', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('미니 창을 닫았습니다.');
  expect(await page.evaluate(() => (window as any).__fixture.studium.running)).toBe(true);
  expect(await page.evaluate(() => (window as any).__fixture.mini)).toEqual({ open: false, always_on_top: false });
  expect(await page.evaluate(() => (window as any).__fixture.studiumStateListenerCount())).toBe(0);
  expect(await page.evaluate(() => (window as any).__fixture.themeListenerCount())).toBe(0);
  expect(await page.evaluate(() => (window as any).__fixture.clockListenerCount())).toBe(0);
  const reads = await callCount(page, 'studium_status');
  await page.waitForTimeout(1_200);
  expect(await callCount(page, 'studium_status')).toBe(reads);
  expect(await callCount(page, 'studium_cancel')).toBe(0);
});

test('mini window and lookup errors preserve saved controls and previous results', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, miniOnTopError: true, miniCloseError: true, miniShowMainError: true, studiumSaveError: true, studiumStartError: true, studium: { preferences, results: [row('5', 2), row('6', 0)] } });
  await openMini(page);
  await page.getByRole('checkbox', { name: '항상 위에 표시', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('항상 위 표시를 바꾸지 못했습니다.');
  await expect(page.getByRole('checkbox', { name: '항상 위에 표시', exact: true })).not.toBeChecked();
  await page.getByRole('button', { name: '기본 창 열기', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('기본 창을 열지 못했습니다.');
  await page.getByRole('button', { name: '미니 창 닫기', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('미니 창을 닫지 못했습니다.');
  await page.getByRole('combobox', { name: '표시할 열람실', exact: true }).selectOption('6');
  await expect(page.getByRole('alert')).toContainText('표시할 열람실을 저장하지 못했습니다.');
  await expect(page.getByRole('combobox', { name: '표시할 열람실', exact: true })).toHaveValue('5');
  await page.getByRole('button', { name: '지금 확인', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('좌석 확인을 시작하지 못했습니다.');
  await expect(page.locator('.seat-count')).toContainText('2석');
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeEnabled();
});

test('mini can recover a failed local snapshot read without starting a check', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studiumSnapshotError: true });
  await openMini(page);
  await expect(page.getByRole('alert')).toContainText('좌석 현황을 불러오지 못했습니다.');
  await page.evaluate(() => { (window as any).__fixture.options.studiumSnapshotError = false; });
  await page.getByRole('button', { name: '다시 불러오기', exact: true }).click();
  await expect(page.getByRole('region', { name: '로욜라 인벤툼 II 좌석 정보', exact: true })).toContainText('아직 확인하지 않았습니다');
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('mini live theme, clock and queued state changes survive an older initial settings response', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, miniSettingsGate: true, studium: { preferences, results: [row('5', 1), row('6', 0)] } });
  await openMini(page);
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.miniSettingsResolvers.length)).toBe(1);
  await page.evaluate(() => {
    const fixture = (window as any).__fixture;
    fixture.emitTheme('dark'); fixture.emitClockFormat('24h'); fixture.studium.preferences.view_room = '6'; fixture.emitStudiumState();
    fixture.releaseMiniSettings();
  });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.getByRole('combobox', { name: '표시할 열람실', exact: true })).toHaveValue('6');
  const sourceTime = page.getByRole('region', { name: 'K관 열람실 좌석 정보', exact: true }).locator('time').first();
  await expect(sourceTime).not.toContainText(/오전|오후/);
  await page.evaluate(() => (window as any).__fixture.emitClockFormat('12h'));
  await expect(sourceTime).toContainText(/오전|오후/);
  await page.evaluate(() => (window as any).__fixture.emitTheme('system'));
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(sourceTime).toContainText(/오전|오후/);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
});

test('mini stale status follows its clock in dark theme at 200% content zoom without requests', async ({ page }) => {
  const time = new Date('2026-11-09T15:30:00Z');
  await page.clock.setFixedTime(time);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const nearExpiry = row('5', 0);
  nearExpiry.result!.source_time = new Date(time.getTime() - 299_000).toISOString();
  await mockDesktop(page, { extensions: { studium: true }, studium: { results: [nearExpiry] } });
  await page.setViewportSize({ width: 600, height: 960 });
  await openMini(page);
  await page.evaluate(() => { (window as any).__fixture.emitTheme('dark'); document.documentElement.style.zoom = '2'; });
  const detail = page.getByRole('region', { name: '로욜라 인벤툼 II 좌석 정보', exact: true });
  await expect(detail.locator('.stale')).toHaveCount(0);
  await expect(page.getByRole('region', { name: '로욜라 인벤툼 II 좌석 정보', exact: true })).toContainText('빈 좌석 없음');
  const reads = await callCount(page, 'studium_snapshot');
  await page.clock.setFixedTime(new Date(time.getTime() + 2_000));
  await expect(detail.locator('.stale')).toBeVisible();
  await page.getByRole('checkbox', { name: '항상 위에 표시', exact: true }).focus();
  await page.keyboard.press('Space');
  await expect(page.getByRole('checkbox', { name: '항상 위에 표시', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: '항상 위에 표시', exact: true })).toBeFocused();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  expect(await callCount(page, 'studium_snapshot')).toBe(reads);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_status')).toBe(0);
});

test('main idle state events preserve drafts and ignore a delayed snapshot after a view change', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [row('5', 1), row('6', 0)] } });
  await openMainStudium(page);
  const interval = page.getByRole('spinbutton', { name: '자동 확인 간격 (분)', exact: true });
  await interval.fill('7');
  await page.getByRole('textbox', { name: '선호 좌석 번호', exact: true }).fill('42');
  await page.getByRole('checkbox', { name: '이 열람실 좌석 알림', exact: true }).check();
  await page.getByRole('checkbox', { name: /PA관 열람실/ }).check();
  await page.evaluate((value) => { const fixture = (window as any).__fixture; fixture.studium.results[0] = value; fixture.emitStudiumState(); }, row('5', 4));
  await expect(page.locator('.seat-count')).toContainText('4석');
  await expect(interval).toHaveValue('7');
  await expect(page.getByRole('textbox', { name: '선호 좌석 번호', exact: true })).toHaveValue('42');
  await expect(page.getByRole('checkbox', { name: '이 열람실 좌석 알림', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: /PA관 열람실/ })).toBeChecked();
  await page.evaluate(() => { const fixture = (window as any).__fixture; fixture.studiumSnapshotGate = true; fixture.emitStudiumState(); });
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.studiumSnapshotResolvers.length)).toBe(1);
  await page.getByRole('combobox', { name: '표시할 열람실', exact: true }).selectOption('6');
  await page.evaluate(() => { const fixture = (window as any).__fixture; fixture.studiumSnapshotGate = false; fixture.releaseStudiumSnapshots(); });
  await expect(page.getByRole('combobox', { name: '표시할 열람실', exact: true })).toHaveValue('6');
  await expect(interval).toHaveValue('7');
  await nav(page).getByRole('button', { name: '오늘', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.studiumStateListenerCount())).toBe(0);
  const reads = await callCount(page, 'studium_snapshot');
  await page.evaluate(() => (window as any).__fixture.emitStudiumState());
  expect(await callCount(page, 'studium_snapshot')).toBe(reads);
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.studiumStateListenerCount())).toBe(1);
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('browser mini preview states its desktop limit without simulating native success', async ({ page }) => {
  await openMini(page);
  await expect(page.getByText('데스크톱 앱에서 미니 창을 사용할 수 있습니다.')).toBeVisible();
  await expect(page.getByRole('button')).toHaveCount(0);
});

test.describe('source clocks use Korean time independently of the device zone', () => {
  test.use({ timezoneId: 'America/New_York' });

  test('every mini summary room shows its own source time and stale state', async ({ page }) => {
    await page.clock.setFixedTime(new Date('2026-11-10T01:02:05Z'));
    const fresh = row('5', 1), old = row('6', 2, 'count_only');
    fresh.result!.source_time = '2026-11-10T01:02:03Z';
    old.result!.source_time = '2026-11-10T00:00:00Z';
    await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [fresh, old] } });
    await page.setViewportSize({ width: 300, height: 480 });
    await openMini(page);
    const summaries = page.getByRole('list', { name: '열람실별 확인 상태', exact: true });
    const freshRoom = summaries.getByRole('listitem').filter({ hasText: '로욜라 인벤툼 II' });
    const oldRoom = summaries.getByRole('listitem').filter({ hasText: 'K관 열람실' });
    await expect(freshRoom.locator('time')).toContainText('오전 10:02:03');
    await expect(freshRoom).toContainText('한국시간');
    await expect(freshRoom.locator('.stale')).toHaveCount(0);
    await expect(oldRoom.locator('time')).toContainText('오전 9:00:00');
    await expect(oldRoom).toContainText('번호 확인 불가');
    await expect(oldRoom.locator('.stale')).toHaveText('오래됨 · 이전 조회 기준');
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    expect(await callCount(page, 'studium_start')).toBe(0);
    expect(await callCount(page, 'studium_status')).toBe(0);
  });

  test('the main room detail also labels and formats source time in Korea', async ({ page }) => {
    await page.clock.setFixedTime(new Date('2026-11-10T01:02:05Z'));
    const fresh = row('5', 1);
    fresh.result!.source_time = '2026-11-10T01:02:03Z';
    await mockDesktop(page, { extensions: { studium: true }, studium: { results: [fresh] } });
    await openMainStudium(page);
    await expect(page.locator('.times')).toContainText('원본 기준 시각 · 한국시간');
    await expect(page.locator('.times time').first()).toContainText('오전 10:02:03');
    expect(await callCount(page, 'studium_start')).toBe(0);
  });

  test('mini initializes its saved clock and follows live format changes without reading or querying again', async ({ page }) => {
    const fresh = row('5', 1);
    fresh.result!.source_time = '2026-11-10T01:02:03Z';
    await mockDesktop(page, { clockFormat: '24h', extensions: { studium: true }, studium: { results: [fresh] } });
    await openMini(page);
    const sourceTime = page.getByRole('region', { name: '로욜라 인벤툼 II 좌석 정보', exact: true }).locator('time').first();
    await expect(sourceTime).toContainText('10:02:03');
    await expect(sourceTime).not.toContainText(/오전|오후/);
    const reads = await callCount(page, 'get_snapshot');
    expect(await page.evaluate(() => (window as any).__fixture.clockListenerCount())).toBe(1);
    await page.evaluate(() => (window as any).__fixture.emitClockFormat('12h'));
    await expect(sourceTime).toContainText('오전 10:02:03');
    await page.evaluate(() => (window as any).__fixture.emitClockFormat('24h'));
    await expect(sourceTime).not.toContainText(/오전|오후/);
    expect(await callCount(page, 'get_snapshot')).toBe(reads);
    expect(await callCount(page, 'studium_start')).toBe(0);
    expect(await callCount(page, 'studium_automatic_start')).toBe(0);
    expect(await callCount(page, 'studium_status')).toBe(0);
  });
});
