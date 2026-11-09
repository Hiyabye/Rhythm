import { test, expect, type Page } from '@playwright/test';
import { mockDesktop, callCount, openSettings } from './fixture';
import type { StudiumRoomState } from '../../src/lib/types';

const preferences = { room_ids: ['5', '6'], view_room: '5' };
const now = () => new Date().toISOString();
const row = (roomId: string, remaining: number, numbers: number[] | null, status: StudiumRoomState['status'] = 'ready'): StudiumRoomState => ({
  room_id: roomId, status, result: { total: roomId === '6' ? 311 : 102, used: (roomId === '6' ? 311 : 102) - remaining, remaining, seat_numbers: numbers, source_time: now(), source_url: 'https://libseat.sogang.ac.kr:444/SEAT/roomview5.asp?room_no=' + roomId },
  error: status === 'count_only' ? '좌석 번호를 확인하지 못했습니다.' : null, checked_at: now(),
});
const nav = (page: Page) => page.getByRole('navigation', { name: '기본 탐색', exact: true });
async function openStudium(page: Page) {
  await page.goto('/');
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect(page.getByRole('heading', { name: '확인할 열람실', exact: true })).toBeVisible();
}

test('Studium is opt-in and opening or selecting rooms never starts a lookup', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await expect(nav(page).getByRole('button', { name: '열람실', exact: true })).toHaveCount(0);
  await openSettings(page, '확장 기능');
  await page.getByRole('switch', { name: 'Studium 사용 설정', exact: true }).click();
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect(page.getByRole('region', { name: '로욜라 인벤툼 II 좌석 정보' })).toContainText('아직 확인하지 않았습니다');
  await expect.poll(() => callCount(page, 'studium_snapshot')).toBe(1);
  await page.getByRole('checkbox', { name: /K관 열람실/ }).check();
  await page.getByRole('button', { name: '선택 저장', exact: true }).click();
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('6');
  await expect(page.getByRole('region', { name: 'K관 열람실 좌석 정보' })).toBeVisible();
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_status')).toBe(0);
});

test('saved targets and viewed room are separate from unsaved target edits', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences } });
  await openStudium(page);
  await page.getByRole('checkbox', { name: /PA관 열람실/ }).check();
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('6');
  await expect(page.getByRole('checkbox', { name: /PA관 열람실/ })).toBeChecked();
  await expect(page.getByText('아직 저장하지 않은 선택입니다.')).toBeVisible();
  await expect(page.getByRole('combobox', { name: '표시할 열람실' }).getByRole('option')).toHaveCount(2);
  await page.getByRole('button', { name: '선택 저장', exact: true }).click();
  await expect(page.getByRole('combobox', { name: '표시할 열람실' }).getByRole('option')).toHaveCount(3);
  expect(await callCount(page, 'studium_start')).toBe(0);
  for (const label of [/로욜라 인벤툼 II/, /K관 열람실/, /PA관 열람실/]) await page.getByRole('checkbox', { name: label }).uncheck();
  await expect(page.getByRole('button', { name: '선택 저장', exact: true })).toBeDisabled();
  await expect(page.getByText('열람실을 하나 이상 선택해 주세요.')).toBeVisible();
});

test('manual lookup clears old values and cancellation keeps only completed rooms', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [row('5', 1, [84]), row('6', 2, [1, 2])] } });
  await openStudium(page);
  await expect(page.locator('.seat-numbers')).toContainText('84번');
  await page.getByRole('button', { name: '지금 확인', exact: true }).click();
  await expect(page.locator('.seat-numbers')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeDisabled();
  await expect(page.getByRole('checkbox', { name: /K관 열람실/ })).toBeDisabled();
  await page.evaluate((completed) => { const state = (window as any).__fixture.studium; state.results[0] = completed; }, row('5', 0, []));
  await expect(page.locator('.seat-numbers')).toContainText('빈 좌석 없음');
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('6');
  await page.getByRole('button', { name: '확인 취소', exact: true }).click();
  await expect(page.getByRole('region', { name: 'K관 열람실 좌석 정보' })).toContainText('확인 취소됨');
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('5');
  await expect(page.locator('.seat-numbers')).toContainText('빈 좌석 없음');
  expect(await callCount(page, 'studium_start')).toBe(1);
});

test('count-only, failed and zero results remain distinct and one room cannot overwrite another', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences: { room_ids: ['5', '6', '11'], view_room: '5' }, results: [row('5', 0, []), row('6', 3, null, 'count_only'), { room_id: '11', status: 'failed', result: null, error: '학교 서버가 접근을 거절했습니다.', checked_at: now() }] } });
  await openStudium(page);
  await expect(page.locator('.seat-numbers')).toContainText('빈 좌석 없음');
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('6');
  await expect(page.locator('.seat-count')).toContainText('3석');
  await expect(page.locator('.seat-numbers')).toContainText('확인 불가 · 빈 좌석 수만 확인됨');
  await expect(page.locator('.times')).toContainText('원본 기준 시각');
  await expect(page.locator('.times')).toContainText('조회 시각');
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('11');
  await expect(page.locator('.seat-count')).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'PA관 열람실 좌석 정보' })).toContainText('학교 서버가 접근을 거절했습니다.');
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('save failure preserves the stored view and target draft and reports recovery', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences }, studiumSaveError: true });
  await openStudium(page);
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('6');
  await expect(page.getByRole('alert')).toContainText('표시할 열람실을 저장하지 못했습니다.');
  await expect(page.getByRole('combobox', { name: '표시할 열람실' })).toHaveValue('5');
  await page.getByRole('checkbox', { name: /PA관 열람실/ }).check();
  await page.getByRole('button', { name: '선택 저장', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('열람실 설정을 저장하지 못했습니다.');
  await expect(page.getByRole('checkbox', { name: /PA관 열람실/ })).toBeChecked();
  await expect(page.getByRole('combobox', { name: '표시할 열람실' }).getByRole('option')).toHaveCount(2);
});

test('manual start failure preserves the previous result and allows another attempt', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { results: [row('5', 1, [84])] }, studiumStartError: true });
  await openStudium(page);
  await page.getByRole('button', { name: '지금 확인', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('좌석 확인을 시작하지 못했습니다.');
  await expect(page.locator('.seat-numbers')).toContainText('84번');
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeEnabled();
  expect(await callCount(page, 'studium_status')).toBe(0);
});

test('the live status announces the completed cycle including partial failure', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences: { room_ids: ['5', '6', '11'], view_room: '5' } } });
  await openStudium(page);
  await page.getByRole('button', { name: '지금 확인', exact: true }).click();
  const status = page.getByRole('status').filter({ hasText: '선택한 열람실을 확인하고 있습니다.' });
  await expect(status).toBeVisible();
  await page.evaluate((results) => {
    const state = (window as any).__fixture.studium;
    state.results = results;
    state.running = false;
  }, [row('5', 0, []), row('6', 3, null, 'count_only'), { room_id: '11', status: 'failed', result: null, error: '학교 서버 응답 오류', checked_at: now() }]);
  await expect(page.getByRole('status').filter({ hasText: '열람실 확인이 끝났습니다.' })).toHaveText('열람실 확인이 끝났습니다. 수와 번호 확인 1곳, 수만 확인 1곳, 확인 실패 1곳.');
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeEnabled();
});

test('keyboard focus follows the lookup actions and polling stops once the cycle ends', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences } });
  await openStudium(page);
  await page.getByRole('button', { name: '지금 확인', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: '확인 취소', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: '확인 취소', exact: true })).toBeFocused();
  await expect.poll(() => callCount(page, 'studium_status')).toBeGreaterThan(0);
  await page.evaluate((results) => {
    const state = (window as any).__fixture.studium;
    state.results = results;
    state.running = false;
  }, [row('5', 1, [84]), row('6', 2, [1, 2])]);
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeFocused();
  const polls = await callCount(page, 'studium_status');
  await page.waitForTimeout(1_200);
  expect(await callCount(page, 'studium_status')).toBe(polls);
});

test('late status cannot revert a saved view and reopening resumes memory status without another start', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, running: true, job_id: 7, results: [{ room_id: '5', status: 'loading', result: null, error: null, checked_at: null }, { room_id: '6', status: 'loading', result: null, error: null, checked_at: null }] } });
  await openStudium(page);
  await page.evaluate(() => { (window as any).__fixture.studiumStatusDelay = 700; });
  await expect.poll(() => callCount(page, 'studium_status')).toBeGreaterThan(0);
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('6');
  await expect(page.getByRole('region', { name: 'K관 열람실 좌석 정보' })).toBeVisible();
  await expect.poll(() => callCount(page, 'studium_status')).toBeGreaterThan(1);
  await expect(page.getByRole('combobox', { name: '표시할 열람실' })).toHaveValue('6');
  await nav(page).getByRole('button', { name: '오늘', exact: true }).click();
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect(page.getByRole('button', { name: '확인 취소', exact: true })).toBeVisible();
  await expect(page.getByRole('combobox', { name: '표시할 열람실' })).toHaveValue('6');
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('old source time stays visibly stale without a background lookup at narrow width', async ({ page }) => {
  const old = row('5', 1, [84]);
  old.result!.source_time = '2026-01-01T00:00:00Z';
  await mockDesktop(page, { extensions: { studium: true }, studium: { results: [old] } });
  await page.setViewportSize({ width: 640, height: 900 });
  await openStudium(page);
  await expect(page.getByRole('status').filter({ hasText: '오래되었거나 PC 시계와 맞지 않는 정보입니다.' })).toBeVisible();
  await expect(page.getByRole('checkbox', { name: /로욜라 인벤툼 II/ })).toBeChecked();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true);
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('source freshness expires with the local clock without starting or polling a lookup', async ({ page }) => {
  const clock = new Date('2026-11-09T10:00:00Z');
  const nearExpiry = row('5', 1, [84]);
  nearExpiry.result!.source_time = new Date(clock.getTime() - 299_000).toISOString();
  // A frozen clock keeps the one-second margin independent of runner speed.
  await page.clock.setFixedTime(clock);
  await mockDesktop(page, { extensions: { studium: true }, studium: { results: [nearExpiry] } });
  await openStudium(page);
  await expect(page.locator('.stale')).toHaveCount(0);
  await page.clock.setFixedTime(new Date(clock.getTime() + 2_000));
  await expect(page.locator('.stale')).toBeVisible();
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_status')).toBe(0);
});

test('turning Studium off clears memory and re-enabling never queries automatically', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { results: [row('5', 1, [84])] } });
  await openStudium(page);
  await expect(page.locator('.seat-numbers')).toContainText('84번');
  await openSettings(page, '확장 기능');
  await page.getByRole('switch', { name: 'Studium 사용 설정', exact: true }).click();
  await expect(nav(page).getByRole('button', { name: '열람실', exact: true })).toHaveCount(0);
  await page.getByRole('switch', { name: 'Studium 사용 설정', exact: true }).click();
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect(page.getByRole('region', { name: '로욜라 인벤툼 II 좌석 정보' })).toContainText('아직 확인하지 않았습니다');
  expect(await callCount(page, 'studium_start')).toBe(0);
});
