import { test, expect, type Page } from '@playwright/test';
import { mockDesktop, callCount, openSettings, syntheticStudiumLayout } from './fixture';
import type { StudiumRoomState } from '../../src/lib/types';

const preferences = { room_ids: ['5', '6'], view_room: '5' };
const now = () => new Date().toISOString();
const row = (roomId: string, remaining: number, numbers: number[] | null, status: StudiumRoomState['status'] = 'ready'): StudiumRoomState => ({
  room_id: roomId, status, result: { total: roomId === '6' ? 311 : 102, used: (roomId === '6' ? 311 : 102) - remaining, remaining, seat_numbers: numbers, source_time: now(), source_url: 'https://libseat.sogang.ac.kr:444/SEAT/roomview5.asp?room_no=' + roomId, layout: numbers === null ? null : syntheticStudiumLayout(roomId === '6' ? 311 : 102), layout_error: null },
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
  await page.getByRole('switch', { name: '열람실 사용 설정', exact: true }).click();
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

test('automatic checks stay off while interval edits validate, save and cancel without a lookup', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true } });
  await openStudium(page);
  const automatic = page.getByRole('region', { name: '자동 확인', exact: true });
  const interval = automatic.getByRole('spinbutton', { name: '자동 확인 간격 (분)', exact: true });
  const save = automatic.getByRole('button', { name: '간격 저장', exact: true });
  await expect(interval).toHaveValue('5');
  await expect(automatic.getByRole('status')).toHaveText('자동 확인이 꺼져 있습니다.');
  await interval.fill('1');
  await expect(interval).toHaveAttribute('aria-invalid', 'true');
  await expect(save).toBeDisabled();
  await interval.fill('2.5');
  await expect(save).toBeDisabled();
  await interval.fill('60');
  await save.click();
  await expect(automatic).toContainText('저장한 간격 60분');
  await expect(save).toBeDisabled();
  await interval.fill('2');
  await automatic.getByRole('button', { name: '간격 변경 취소', exact: true }).click();
  await expect(interval).toHaveValue('60');
  expect(await callCount(page, 'studium_save_interval')).toBe(1);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
  expect(await callCount(page, 'studium_status')).toBe(0);
});

test('waiting automatic status preserves interval and room drafts through state refresh and save failure', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studiumSaveError: true, studium: {
    preferences, results: [row('5', 1, [84]), row('6', 0, [])],
    automatic: { active: true, interval_minutes: 5, next_check_at: '2026-11-10T10:00:00Z', error: null },
  } });
  await openStudium(page);
  const automatic = page.getByRole('region', { name: '자동 확인', exact: true });
  const interval = automatic.getByRole('spinbutton', { name: '자동 확인 간격 (분)', exact: true });
  await interval.fill('7');
  await page.getByRole('checkbox', { name: /PA관 열람실/ }).check();
  // A waiting session does not poll; it refreshes on the shared state event.
  const polls = await callCount(page, 'studium_status');
  await page.waitForTimeout(1_200);
  expect(await callCount(page, 'studium_status')).toBe(polls);
  const refreshes = await callCount(page, 'studium_snapshot');
  await page.evaluate(() => (window as any).__fixture.emitStudiumState());
  await expect.poll(() => callCount(page, 'studium_snapshot')).toBeGreaterThan(refreshes);
  await expect(interval).toHaveValue('7');
  await expect(page.getByRole('checkbox', { name: /PA관 열람실/ })).toBeChecked();
  await automatic.getByRole('button', { name: '간격 저장', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('자동 확인 간격을 저장하지 못했습니다.');
  await expect(interval).toHaveValue('7');
  await expect(automatic).toContainText('저장한 간격 5분');
  await page.evaluate(() => { (window as any).__fixture.studiumSaveError = false; });
  await automatic.getByRole('button', { name: '간격 저장', exact: true }).click();
  await expect(automatic).toContainText('저장한 간격 7분');
  expect(await page.evaluate(() => (window as any).__fixture.studium.automatic.next_check_at)).toBe('2026-11-10T10:00:00Z');
  await expect(page.getByRole('checkbox', { name: /PA관 열람실/ })).toBeChecked();
  await expect(automatic.getByRole('status')).toContainText('다음 자동 확인:');
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
});

test('automatic start immediately checks once, keeps waiting status on reopen, and pause preserves a manual cycle', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [row('5', 1, [84]), row('6', 0, [])] } });
  await openStudium(page);
  const automatic = page.getByRole('region', { name: '자동 확인', exact: true });
  await automatic.getByRole('button', { name: '자동 확인 시작', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(automatic.getByRole('button', { name: '자동 확인 일시정지', exact: true })).toBeFocused();
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeDisabled();
  await expect(page.locator('.seat-numbers')).toHaveCount(0);
  await page.evaluate((results) => (window as any).__fixture.finishStudiumCycle(results), [row('5', 2, [1, 84]), row('6', 0, [])]);
  await expect(automatic.getByRole('status')).toContainText('다음 자동 확인:');
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeEnabled();
  const polls = await callCount(page, 'studium_status');
  await page.waitForTimeout(1_200);
  expect(await callCount(page, 'studium_status')).toBe(polls);
  await nav(page).getByRole('button', { name: '오늘', exact: true }).click();
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect(automatic.getByRole('status')).toContainText('다음 자동 확인:');
  await page.getByRole('button', { name: '지금 확인', exact: true }).click();
  await expect(automatic.getByRole('status')).toContainText('진행 중인 확인을 마친 뒤 5분 후');
  await automatic.getByRole('button', { name: '자동 확인 일시정지', exact: true }).click();
  await expect(automatic.getByRole('button', { name: '자동 확인 시작', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: '확인 취소', exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__fixture.studiumCancelRequested)).toBeNull();
  await page.evaluate((results) => (window as any).__fixture.finishStudiumCycle(results), [row('5', 0, []), row('6', 0, [])]);
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeEnabled();
  const stoppedPolls = await callCount(page, 'studium_status');
  await page.waitForTimeout(1_200);
  expect(await callCount(page, 'studium_status')).toBe(stoppedPolls);
  expect(await callCount(page, 'studium_automatic_start')).toBe(1);
  expect(await callCount(page, 'studium_automatic_pause')).toBe(1);
  expect(await callCount(page, 'studium_start')).toBe(1);
});

test('automatic start adopts an existing manual cycle and pausing an automatic cycle cancels only that cycle', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true } });
  await openStudium(page);
  const automatic = page.getByRole('region', { name: '자동 확인', exact: true });
  await page.getByRole('button', { name: '지금 확인', exact: true }).click();
  const manualId = await page.evaluate(() => (window as any).__fixture.studium.job_id);
  await automatic.getByRole('button', { name: '자동 확인 시작', exact: true }).click();
  expect(await page.evaluate(() => (window as any).__fixture.studium.job_id)).toBe(manualId);
  await automatic.getByRole('button', { name: '자동 확인 일시정지', exact: true }).click();
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeDisabled();
  await page.evaluate((results) => (window as any).__fixture.finishStudiumCycle(results), [row('5', 0, [])]);
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeEnabled();
  await automatic.getByRole('button', { name: '자동 확인 시작', exact: true }).click();
  await automatic.getByRole('button', { name: '자동 확인 일시정지', exact: true }).click();
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeEnabled();
  await expect(page.getByRole('region', { name: '로욜라 인벤툼 II 좌석 정보' })).toContainText('확인 취소됨');
  expect(await callCount(page, 'studium_start')).toBe(1);
});

test('server refusal stops automatic polling and stays stopped after navigation or extension off and on', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: {
    automatic: { active: true, interval_minutes: 7, next_check_at: null, error: null }, running: true, job_id: 1,
    results: [{ room_id: '5', status: 'loading', result: null, error: null, checked_at: null }],
  } });
  await openStudium(page);
  const automatic = page.getByRole('region', { name: '자동 확인', exact: true });
  await page.evaluate(() => {
    const state = (window as any).__fixture.studium;
    state.running = false;
    state.results = [{ room_id: '5', status: 'failed', result: null, error: '학교 서버가 요청을 제한했습니다.', checked_at: new Date().toISOString() }];
    state.automatic = { ...state.automatic, active: false, next_check_at: null, error: '학교 서버가 요청을 제한하여 자동 확인을 중단했습니다.' };
  });
  await expect(automatic.getByRole('alert')).toContainText('자동 확인을 중단했습니다.');
  await expect(automatic.getByRole('status')).toHaveText('자동 확인이 중단되었습니다.');
  const polls = await callCount(page, 'studium_status');
  await page.waitForTimeout(1_200);
  expect(await callCount(page, 'studium_status')).toBe(polls);
  await nav(page).getByRole('button', { name: '오늘', exact: true }).click();
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect(automatic.getByRole('button', { name: '자동 확인 시작', exact: true })).toBeVisible();
  await openSettings(page, '확장 기능');
  await page.getByRole('switch', { name: '열람실 사용 설정', exact: true }).click();
  await page.getByRole('switch', { name: '열람실 사용 설정', exact: true }).click();
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect(automatic.getByRole('status')).toHaveText('자동 확인이 꺼져 있습니다.');
  await expect(automatic.getByRole('spinbutton', { name: '자동 확인 간격 (분)', exact: true })).toHaveValue('7');
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('favorite numbers validate, save independently and exact search keeps the total count', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, favorites: { '5': [84], '6': [1] }, results: [row('5', 2, [1, 84]), row('6', 0, [])] } });
  await openStudium(page);
  const input = page.getByRole('textbox', { name: '선호 좌석 번호', exact: true });
  const save = page.getByRole('button', { name: '선호 좌석 저장', exact: true });
  await expect(page.locator('.preferred')).toContainText('84');
  await input.fill('1, 1');
  await expect(page.getByText('같은 좌석 번호를 중복해서 입력할 수 없습니다.')).toBeVisible();
  await expect(save).toBeDisabled();
  await input.fill('103');
  await expect(input).toHaveAttribute('aria-invalid', 'true');
  await expect(save).toBeDisabled();
  await input.fill('2.5');
  await expect(save).toBeDisabled();
  await input.fill('102, 84 1');
  await save.click();
  await expect(input).toHaveValue('1, 84, 102');
  await expect(input).toBeFocused();
  await expect(page.locator('.preferred')).toHaveCount(2);
  await expect(page.locator('.favorite-availability')).toHaveText('이번 조회에서 빈 선호 좌석: 1, 84번');
  const search = page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true });
  await search.fill('84');
  await expect(page.locator('.search-count')).toHaveText('전체 빈 좌석 2석 · 검색 결과 1개');
  await expect(page.locator('.seat-count')).toContainText('2석');
  await expect(page.locator('.preferred')).toHaveCount(1);
  await expect(page.locator('.preferred')).toContainText('84');
  await search.fill('4');
  await expect(page.locator('.search-count')).toHaveText('전체 빈 좌석 2석 · 검색 결과 0개');
  await expect(page.locator('.seat-numbers')).toContainText('검색한 번호가 빈 좌석 목록에 없습니다.');
  await search.fill('0');
  await expect(search).toHaveAttribute('aria-invalid', 'true');
  await expect(page.locator('.search-count')).toHaveCount(0);
  await page.getByRole('button', { name: '검색 해제', exact: true }).click();
  await expect(search).toHaveValue('');
  await expect(search).toBeFocused();
  await expect(page.locator('.preferred')).toHaveCount(2);
  await input.fill('');
  await save.click();
  await expect(page.locator('.saved-favorites')).toHaveText('저장한 선호 좌석: 없음');
  await expect(page.locator('.preferred')).toHaveCount(0);
  expect(await callCount(page, 'studium_save_favorites')).toBe(2);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
  expect(await callCount(page, 'studium_status')).toBe(0);
});

test('favorite drafts survive state refresh and remain separate across viewed and deselected rooms', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: {
    preferences, favorites: { '5': [84], '6': [1] }, results: [row('5', 1, [84]), row('6', 1, [1])],
    automatic: { active: true, interval_minutes: 5, next_check_at: '2026-11-10T10:00:00Z', error: null },
  } });
  await openStudium(page);
  const input = page.getByRole('textbox', { name: '선호 좌석 번호', exact: true });
  const search = page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true });
  await input.fill('1 42');
  await search.fill('84');
  const refreshes = await callCount(page, 'studium_snapshot');
  await page.evaluate(() => (window as any).__fixture.emitStudiumState());
  await expect.poll(() => callCount(page, 'studium_snapshot')).toBeGreaterThan(refreshes);
  await expect(input).toHaveValue('1 42');
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('6');
  await expect(input).toHaveValue('1');
  await expect(search).toHaveValue('');
  await expect(page.locator('.favorite-availability')).toHaveText('이번 조회에서 빈 선호 좌석: 1번');
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('5');
  await expect(input).toHaveValue('84');
  await input.fill('1');
  await page.getByRole('button', { name: '선호 좌석 변경 취소', exact: true }).click();
  await expect(input).toHaveValue('84');
  await expect(input).toBeFocused();
  await page.getByRole('checkbox', { name: /K관 열람실/ }).uncheck();
  await page.getByRole('button', { name: '선택 저장', exact: true }).click();
  await page.getByRole('checkbox', { name: /K관 열람실/ }).check();
  await page.getByRole('button', { name: '선택 저장', exact: true }).click();
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('6');
  await expect(input).toHaveValue('1');
  expect(await callCount(page, 'studium_save_favorites')).toBe(0);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
});

test('favorite save failure keeps the draft and allows saving during a running check', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studiumSaveError: true, studium: {
    running: true, job_id: 7, results: [{ room_id: '5', status: 'loading', result: null, error: null, checked_at: null }],
  } });
  await openStudium(page);
  const input = page.getByRole('textbox', { name: '선호 좌석 번호', exact: true });
  await input.fill('84');
  await page.getByRole('button', { name: '선호 좌석 저장', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('선호 좌석을 저장하지 못했습니다.');
  await expect(input).toHaveValue('84');
  await expect(page.locator('.saved-favorites')).toHaveText('저장한 선호 좌석: 없음');
  await page.evaluate(() => { (window as any).__fixture.studiumSaveError = false; });
  await page.getByRole('button', { name: '선호 좌석 저장', exact: true }).click();
  await expect(page.locator('.saved-favorites')).toHaveText('저장한 선호 좌석: 84번');
  await expect(page.locator('.favorite-availability')).toHaveText('현재 조회로 선호 좌석의 빈자리 여부를 확인하지 못했습니다.');
  await page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true }).fill('84');
  await expect(page.getByText('좌석 번호를 확인한 뒤 검색할 수 있습니다.')).toBeVisible();
  await expect(page.locator('.search-count')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeDisabled();
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('favorite availability and search distinguish unverified counts, failure, unchecked and verified zero', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: {
    preferences: { room_ids: ['5', '6', '11', '15'], view_room: '5' }, favorites: { '5': [84], '6': [84], '11': [84], '15': [84] },
    results: [row('5', 3, null, 'count_only'), { room_id: '6', status: 'failed', result: null, error: '학교 응답 오류', checked_at: now() }, { room_id: '11', status: 'not_checked', result: null, error: null, checked_at: null }, row('15', 0, [])],
  } });
  await openStudium(page);
  const search = page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true });
  await search.fill('84');
  await expect(page.locator('.favorite-availability')).toHaveText('번호를 확인하지 못해 선호 좌석의 빈자리 여부를 알 수 없습니다.');
  await expect(page.getByText('좌석 번호를 검증하지 못해 검색할 수 없습니다.')).toBeVisible();
  await expect(page.locator('.seat-count')).toContainText('3석');
  await expect(page.locator('.search-count')).toHaveCount(0);
  for (const id of ['6', '11']) {
    await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption(id);
    await search.fill('84');
    await expect(page.locator('.favorite-availability')).toHaveText('현재 조회로 선호 좌석의 빈자리 여부를 확인하지 못했습니다.');
    await expect(page.locator('.seat-count')).toHaveCount(0);
    await expect(page.locator('.search-count')).toHaveCount(0);
    await expect(page.locator('.preferred')).toHaveCount(0);
  }
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('15');
  await search.fill('84');
  await expect(page.locator('.favorite-availability')).toHaveText('이번 조회에서 빈 선호 좌석이 없습니다.');
  await expect(page.locator('.search-count')).toHaveText('전체 빈 좌석 0석 · 검색 결과 0개');
  await expect(page.locator('.seat-numbers')).toContainText('빈 좌석 없음');
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('stale favorite highlights and search explicitly describe the earlier observation', async ({ page }) => {
  const old = row('5', 1, [84]);
  old.result!.source_time = '2026-01-01T00:00:00Z';
  await mockDesktop(page, { extensions: { studium: true }, studium: { favorites: { '5': [84] }, results: [old] } });
  await openStudium(page);
  await expect(page.locator('.preferred')).toContainText('84');
  await expect(page.locator('.favorite-availability')).toHaveText('이전 조회 기준 빈 선호 좌석: 84번. 현재 빈자리 여부는 확인되지 않았습니다.');
  await page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true }).fill('84');
  await expect(page.locator('.search-count')).toHaveText('전체 빈 좌석 1석 · 검색 결과 1개 · 이전 조회 기준');
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('favorite editing and search support keyboard use in dark theme at 200% content zoom', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { results: [row('5', 2, [1, 84])] } });
  await page.setViewportSize({ width: 1280, height: 900 });
  await openStudium(page);
  await openSettings(page, '일반');
  await page.getByRole('button', { name: '다크', exact: true }).click();
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await page.evaluate(() => { document.documentElement.style.zoom = '2'; });
  const input = page.getByRole('textbox', { name: '선호 좌석 번호', exact: true });
  await input.fill('84, 1');
  await input.press('Enter');
  await expect(input).toHaveValue('1, 84');
  await expect(input).toBeFocused();
  await expect(page.locator('.preferred')).toHaveCount(2);
  const search = page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true });
  await search.fill('84');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: '검색 해제', exact: true })).toBeFocused();
  await page.keyboard.press('Space');
  await expect(search).toHaveValue('');
  await expect(search).toBeFocused();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('seat alert rules start off and saving a valid condition never enables global notifications or checks', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true, proactive: false } });
  await openStudium(page);
  const alert = page.getByRole('region', { name: '좌석 알림 조건', exact: true });
  const toggle = alert.getByRole('checkbox', { name: '이 열람실 좌석 알림', exact: true });
  const minimum = alert.getByRole('spinbutton', { name: '알림을 보낼 최소 빈 좌석 수', exact: true });
  const save = alert.getByRole('button', { name: '알림 조건 저장', exact: true });
  await expect(toggle).not.toBeChecked();
  await expect(alert.locator('.saved-alert')).toHaveText('저장한 조건: 꺼짐');
  await expect(alert.locator('.notification-policy')).toContainText('확장 기능이 꺼져 있어');
  await toggle.check();
  await minimum.fill('103');
  await expect(save).toBeDisabled();
  await minimum.fill('2.5');
  await expect(save).toBeDisabled();
  await minimum.fill('3');
  await save.click();
  await expect(alert.locator('.saved-alert')).toHaveText('저장한 조건: 켜짐 · 빈 좌석 3석 이상');
  await expect(toggle).toBeFocused();
  await alert.getByRole('button', { name: '확장 기능 설정 열기', exact: true }).click();
  await expect(page.getByRole('heading', { name: '확장 기능', exact: true })).toBeVisible();
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  expect(await callCount(page, 'proactive_save')).toBe(0);
});

test('seat alerts report denied permission, quiet hours, global off and uncertain sends without prompting', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, initialNotificationsEnabled: true, notificationAuthorization: 'denied', studium: { alert_error: '전송 결과를 확인하지 못했습니다. 자동으로 다시 보내지 않습니다.' } });
  await openStudium(page);
  const alert = page.getByRole('region', { name: '좌석 알림 조건', exact: true });
  await expect(alert.locator('.notification-policy')).toContainText('운영체제 알림 권한이 거부되어 있습니다.');
  await expect(alert.getByRole('alert')).toContainText('좌석 알림 전달 상태: 전송 결과를 확인하지 못했습니다.');
  await page.evaluate(() => {
    const fixture = (window as any).__fixture;
    fixture.setNotificationAuthorization('authorized');
    fixture.setNotificationSettings({ quiet_hours: { enabled: true, start: '22:00', end: '07:00' } });
  });
  await alert.getByRole('button', { name: '알림 상태 새로 확인', exact: true }).click();
  await expect(alert.locator('.notification-policy')).toHaveText('운영체제 알림 권한이 허용되어 있습니다.');
  await expect(alert).toContainText('조용한 시간 오후 10:00–오전 7:00(PC 현지 시각)에는 알리지 않습니다.');
  await expect(alert).toContainText('10분당 최대 3회');
  await page.evaluate(() => (window as any).__fixture.setNotificationSettings({ notifications_enabled: false }));
  await alert.getByRole('button', { name: '알림 상태 새로 확인', exact: true }).click();
  await expect(alert.locator('.notification-policy')).toContainText('전체 운영체제 알림이 꺼져 있어');
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_save_alert')).toBe(0);
});

test.describe('clock preferences remain global with seat alerts', () => {
  test.use({ timezoneId: 'Asia/Seoul' });

  test('settings, quiet-hours text and notification navigation retain the selected clock without a lookup', async ({ page }) => {
    const observed = row('5', 1, [84]);
    observed.result!.source_time = '2026-11-09T12:34:56Z';
    observed.checked_at = observed.result!.source_time;
    await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [observed] } });
    await openStudium(page);
    await expect(page.locator('.times time').first()).toContainText('오후 9:34:56');
    await page.evaluate(() => (window as any).__fixture.setNotificationSettings({ quiet_hours: { enabled: true, start: '22:00', end: '07:00' } }));
    await page.getByRole('button', { name: '알림 상태 새로 확인', exact: true }).click();
    await expect(page.getByRole('region', { name: '좌석 알림 조건', exact: true })).toContainText('오후 10:00–오전 7:00');
    await expect(nav(page).getByRole('button', { name: '휴지통', exact: true })).toBeVisible();

    await openSettings(page, '일반');
    const formats = page.getByRole('group', { name: '시간 표시', exact: true });
    await formats.getByRole('button', { name: '24시간', exact: true }).click();
    await expect(formats.getByRole('button', { name: '24시간', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
    await expect(page.locator('.times time').first()).toContainText('21:34:56');
    await expect(page.locator('.times time').first()).not.toContainText('오후');
    await expect(page.getByRole('region', { name: '좌석 알림 조건', exact: true })).toContainText('22:00–07:00');
    await page.evaluate(() => (window as any).__fixture.emitStudiumNotification('5'));
    await expect(page.locator('.times time').first()).toContainText('21:34:56');
    expect(await callCount(page, 'studium_start')).toBe(0);
    expect(await callCount(page, 'studium_automatic_start')).toBe(0);
  });
});

test('preferred-seat alerts require saved favorites and remain separate for each viewed room', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [row('5', 3, null, 'count_only'), row('6', 0, [])] } });
  await openStudium(page);
  const alert = page.getByRole('region', { name: '좌석 알림 조건', exact: true });
  const kind = alert.getByRole('combobox', { name: '알림 조건', exact: true });
  const toggle = alert.getByRole('checkbox', { name: '이 열람실 좌석 알림', exact: true });
  const save = alert.getByRole('button', { name: '알림 조건 저장', exact: true });
  await kind.selectOption('preferred');
  await toggle.check();
  await expect(alert.getByText('선호 좌석 알림을 켜려면 이 방의 선호 번호를 먼저 저장해 주세요.')).toBeVisible();
  await expect(save).toBeDisabled();
  await page.getByRole('textbox', { name: '선호 좌석 번호', exact: true }).fill('84');
  await expect(save).toBeDisabled();
  await page.getByRole('button', { name: '선호 좌석 저장', exact: true }).click();
  await expect(save).toBeEnabled();
  await save.click();
  await expect(alert.locator('.saved-alert')).toHaveText('저장한 조건: 켜짐 · 선호 좌석 중 빈자리');
  await expect(page.locator('.favorite-availability')).toContainText('빈자리 여부를 알 수 없습니다.');
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('6');
  await expect(toggle).not.toBeChecked();
  await expect(kind).toHaveValue('remaining');
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('5');
  await expect(toggle).toBeChecked();
  await expect(kind).toHaveValue('preferred');
  expect(await page.evaluate(() => (window as any).__fixture.studium.alert_rules['5'].minimum)).toBe(1);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
});

test('alert drafts survive polling and failed saves while another editor and a check are active', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studiumAlertSaveError: true, studium: {
    preferences, running: true, job_id: 7, results: [{ room_id: '5', status: 'loading', result: null, error: null, checked_at: null }],
  } });
  await openStudium(page);
  const alert = page.getByRole('region', { name: '좌석 알림 조건', exact: true });
  const toggle = alert.getByRole('checkbox', { name: '이 열람실 좌석 알림', exact: true });
  const minimum = alert.getByRole('spinbutton', { name: '알림을 보낼 최소 빈 좌석 수', exact: true });
  await toggle.check();
  await minimum.fill('3');
  await page.getByRole('textbox', { name: '선호 좌석 번호', exact: true }).fill('42');
  const polls = await callCount(page, 'studium_status');
  await expect.poll(() => callCount(page, 'studium_status')).toBeGreaterThan(polls);
  await expect(toggle).toBeChecked();
  await expect(minimum).toHaveValue('3');
  await alert.getByRole('button', { name: '알림 조건 저장', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('좌석 알림 조건을 저장하지 못했습니다.');
  await expect(alert.locator('.saved-alert')).toHaveText('저장한 조건: 꺼짐');
  await expect(minimum).toHaveValue('3');
  await page.evaluate(() => { (window as any).__fixture.studiumAlertSaveError = false; });
  await alert.getByRole('button', { name: '알림 조건 저장', exact: true }).click();
  await expect(alert.locator('.saved-alert')).toHaveText('저장한 조건: 켜짐 · 빈 좌석 3석 이상');
  await expect(page.getByRole('textbox', { name: '선호 좌석 번호', exact: true })).toHaveValue('42');
  await minimum.fill('4');
  await alert.getByRole('button', { name: '알림 조건 변경 취소', exact: true }).click();
  await expect(minimum).toHaveValue('3');
  await expect(toggle).toBeFocused();
  await page.getByRole('combobox', { name: '표시할 열람실' }).selectOption('6');
  await expect(toggle).not.toBeChecked();
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('seat notification clicks refresh the idle view, including repeated clicks for the same room', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [row('5', 1, [84]), row('6', 0, [])] } });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: '오늘', exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).__fixture.emitStudiumNotification('6'));
  await expect(page.getByRole('region', { name: 'K관 열람실 좌석 정보', exact: true })).toBeVisible();
  await expect(page.getByRole('combobox', { name: '표시할 열람실' })).toHaveValue('6');
  await page.evaluate(() => (window as any).__fixture.emitStudiumNotification('5'));
  await expect(page.getByRole('combobox', { name: '표시할 열람실' })).toHaveValue('5');
  await page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true }).fill('84');
  await page.evaluate(() => (window as any).__fixture.emitStudiumNotification('5'));
  await expect(page.getByRole('searchbox', { name: '빈 좌석 번호 검색', exact: true })).toHaveValue('');
  const reads = await callCount(page, 'studium_snapshot');
  await page.evaluate(() => (window as any).__fixture.emitStudiumNotification('999'));
  expect(await callCount(page, 'studium_snapshot')).toBe(reads);
  expect(await callCount(page, 'studium_set_view_room')).toBe(0);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
});

test('seat notification activation before subscription is recovered after the core snapshot loads', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, pendingStudiumActivation: '6', studium: { preferences } });
  await page.goto('/');
  await expect(page.getByRole('region', { name: 'K관 열람실 좌석 정보', exact: true })).toBeVisible();
  await expect(page.getByRole('combobox', { name: '표시할 열람실' })).toHaveValue('6');
  expect(await callCount(page, 'studium_notification_activation')).toBe(1);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_automatic_start')).toBe(0);
});

test('a delayed seat activation fallback cannot replace a newer live notification click', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: true }, pendingStudiumActivation: '5', studiumActivationGate: true, studium: { preferences } });
  await page.goto('/');
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.studiumActivationResolvers.length)).toBe(1);
  await page.evaluate(() => (window as any).__fixture.emitStudiumNotification('6'));
  await expect(page.getByRole('combobox', { name: '표시할 열람실' })).toHaveValue('6');
  await page.evaluate(() => (window as any).__fixture.releaseStudiumActivation());
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.studiumActivationCompleted)).toBe(true);
  await expect(page.getByRole('combobox', { name: '표시할 열람실' })).toHaveValue('6');
  expect(await callCount(page, 'studium_snapshot')).toBe(1);
  expect(await callCount(page, 'studium_start')).toBe(0);
});

test('seat notification events and pending activation cannot open a disabled extension', async ({ page }) => {
  await mockDesktop(page, { extensions: { studium: false }, pendingStudiumActivation: '6', studium: { preferences } });
  await page.goto('/');
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.studiumActivationCompleted)).toBe(true);
  await page.evaluate(() => (window as any).__fixture.emitStudiumNotification('6'));
  await expect(page.getByRole('heading', { name: '오늘', exact: true })).toBeVisible();
  await expect(nav(page).getByRole('button', { name: '열람실', exact: true })).toHaveCount(0);
  expect(await callCount(page, 'studium_snapshot')).toBe(0);
  expect(await callCount(page, 'studium_start')).toBe(0);
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
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeDisabled();
  await expect(page.getByRole('checkbox', { name: /K관 열람실/ })).toBeDisabled();
  await expect(page.getByRole('region', { name: 'K관 열람실 좌석 정보' })).toContainText('확인 중');
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.studiumCancelRequested)).toBe(1);
  const pendingPolls = await callCount(page, 'studium_status');
  await expect.poll(() => callCount(page, 'studium_status')).toBeGreaterThan(pendingPolls);
  await page.evaluate(() => (window as any).__fixture.finishStudiumCancel());
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
  await expect(page.getByRole('button', { name: '지금 확인', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: '확인 취소', exact: true })).toBeFocused();
  await page.evaluate(() => (window as any).__fixture.finishStudiumCancel());
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

test('dark Studium remains readable and keyboard usable at 200% content zoom', async ({ page }) => {
  const old = row('5', 3, [1, 42, 102]);
  old.result!.source_time = '2026-01-01T00:00:00Z';
  await mockDesktop(page, { extensions: { studium: true }, studium: { preferences, results: [old, row('6', 0, [])] } });
  await page.setViewportSize({ width: 1280, height: 900 });
  await openStudium(page);
  await openSettings(page, '일반');
  await page.getByRole('button', { name: '다크', exact: true }).click();
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.evaluate(() => { document.documentElement.style.zoom = '2'; });
  const detail = page.getByRole('region', { name: '로욜라 인벤툼 II 좌석 정보' });
  await expect(detail.locator('.seat-count')).toContainText('3석');
  await expect(detail.locator('.seat-numbers')).toContainText('1, 42, 102번');
  await expect(detail.locator('.stale')).toBeVisible();
  await expect(detail.locator('time')).toHaveCount(2);
  const contrast = await detail.evaluate((node) => {
    const luminance = (color: string) => {
      const [r, g, b] = color.match(/[\d.]+/g)!.slice(0, 3).map(Number).map((value) => {
        const channel = value / 255;
        return channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4;
      });
      return r * .2126 + g * .7152 + b * .0722;
    };
    const background = luminance(getComputedStyle(node).backgroundColor);
    return ['.seat-count', '.times', '.stale'].map((selector) => {
      const text = luminance(getComputedStyle(node.querySelector(selector)!).color);
      return (Math.max(text, background) + .05) / (Math.min(text, background) + .05);
    });
  });
  for (const ratio of contrast) expect(ratio).toBeGreaterThanOrEqual(4.5);
  const target = page.getByRole('checkbox', { name: /K관 열람실/ });
  await target.focus();
  await page.keyboard.press('Space');
  await expect(target).not.toBeChecked();
  await expect(target).toBeFocused();
  await page.keyboard.press('Space');
  await expect(target).toBeChecked();
  const view = page.getByRole('combobox', { name: '표시할 열람실' });
  await view.selectOption('6');
  await view.focus();
  await expect(view).toBeFocused();
  await expect(page.getByRole('region', { name: 'K관 열람실 좌석 정보' })).toContainText('빈 좌석 없음');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  expect(await callCount(page, 'studium_start')).toBe(0);
  expect(await callCount(page, 'studium_status')).toBe(0);
});

for (const reducedMotion of ['no-preference', 'reduce'] as const) {
  test(`Studium entry ${reducedMotion === 'reduce' ? 'skips' : 'uses'} motion while results stay available`, async ({ page }) => {
    await page.addInitScript(() => {
      const animate = Element.prototype.animate;
      (window as any).__studiumAnimations = 0;
      Element.prototype.animate = function (...args) {
        if (this.matches('.studium, .studium *')) (window as any).__studiumAnimations++;
        return animate.apply(this, args);
      };
    });
    await page.emulateMedia({ reducedMotion });
    await mockDesktop(page, { extensions: { studium: true }, studium: { results: [row('5', 1, [84])] } });
    await openStudium(page);
    await expect(page.locator('.seat-numbers')).toContainText('84번');
    // The results can resolve before the browser starts a scheduled intro.
    if (reducedMotion === 'reduce') {
      await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
      expect(await page.evaluate(() => (window as any).__studiumAnimations)).toBe(0);
    } else await expect.poll(() => page.evaluate(() => (window as any).__studiumAnimations)).toBeGreaterThan(0);
    expect(await callCount(page, 'studium_start')).toBe(0);
  });
}

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
  await page.getByRole('switch', { name: '열람실 사용 설정', exact: true }).click();
  await expect(nav(page).getByRole('button', { name: '열람실', exact: true })).toHaveCount(0);
  await page.getByRole('switch', { name: '열람실 사용 설정', exact: true }).click();
  await nav(page).getByRole('button', { name: '열람실', exact: true }).click();
  await expect(page.getByRole('region', { name: '로욜라 인벤툼 II 좌석 정보' })).toContainText('아직 확인하지 않았습니다');
  expect(await callCount(page, 'studium_start')).toBe(0);
});
