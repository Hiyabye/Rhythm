import { test, expect } from '@playwright/test';
import { mockDesktop, openSettings } from './fixture';
import type { CybercampusTask } from '../../src/lib/types';

const board = { id: 'cybercampus', name: '강의 공지', organization: '서강대학교 · 사이버캠퍼스', supported: true, family: 'cybercampus' };

/** A KST wall-clock value `days` from now, as the connector stores it. */
function kst(days: number) {
  return new Date(Date.now() + days * 86_400_000 + 9 * 3_600_000).toISOString().slice(0, 16);
}
function task(overrides: Partial<CybercampusTask>): CybercampusTask {
  return { course_key: 'A1', course_name: '자료구조실습', category: 'report', item_id: '1', label: '과제', title: '실습 과제', week: 9,
    opens_at: null, due_at: null, late_due_at: null, status: null, submitted_at: null, pending: false, updated_at: '2026-11-06T00:00:00Z', ...overrides };
}

test('the Cybercampus login is checked before it is saved and is never shown again', async ({ page }) => {
  await mockDesktop(page, { sourceList: [board] });
  await page.goto('/');
  await openSettings(page, '사이버캠퍼스');
  const login = page.getByRole('group', { name: '사이버캠퍼스 로그인' });
  await expect(login).toContainText('저장된 로그인 정보 없음');
  const id = login.getByRole('textbox', { name: '사이버캠퍼스 아이디(학번)' });
  const password = login.getByLabel('사이버캠퍼스 비밀번호');
  const save = login.getByRole('button', { name: '확인 후 저장' });
  await expect(save).toBeDisabled();
  await id.fill('20990001');
  await password.fill('wrong');
  await save.click();
  await expect(login.getByRole('alert')).toContainText('로그인에 실패했습니다');
  // The password field never keeps a rejected or accepted password.
  await expect(password).toHaveValue('');
  await password.fill('right');
  await password.press('Enter');
  await expect(login).toContainText('20990001 계정으로 로그인 정보 저장됨 · 강좌 1개');
  await expect(login).toContainText('자료구조실습');
  await expect(password).toHaveValue('');
  await expect(login.getByRole('status')).toContainText('운영체제 보안 저장소에 저장했습니다');
  await login.getByRole('button', { name: '저장된 로그인 정보 삭제' }).click();
  await expect(login).toContainText('저장된 로그인 정보 없음');
  await expect(login.getByRole('status')).toContainText('보관한 공지와 과제는 남습니다');
});

test('deadlines list what is due first and fold submitted or past items', async ({ page }) => {
  await mockDesktop(page, { sourceList: [{ ...board, subscribed: true }], cybercampusId: '20990001', cybercampusTasks: [
    task({ item_id: '2', title: '실습 과제 2', due_at: kst(3), pending: true, status: '미제출' }),
    task({ item_id: '3', category: 'test', label: '퀴즈', title: '2주차 퀴즈', course_name: '확률과통계', due_at: kst(0.25), pending: true, week: null }),
    task({ item_id: '1', title: '실습 과제 1', due_at: kst(-20), status: '정상제출', submitted_at: kst(-21) }),
  ] });
  await page.goto('/');
  const navigation = page.getByRole('navigation', { name: '기본 탐색', exact: true });
  // The sidebar counts what is still to do, matching the page's list below.
  await expect(navigation.getByRole('button', { name: '과제·마감' })).toHaveAccessibleDescription('남은 과제 2개');
  await navigation.getByRole('button', { name: '과제·마감' }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('과제·마감');
  await expect(page.getByRole('heading', { name: '해야 할 일 2개' })).toBeVisible();
  const due = page.getByRole('list', { name: '해야 할 일' }).getByRole('listitem');
  await expect(due).toHaveCount(2);
  await expect(due.nth(0)).toContainText('퀴즈2주차 퀴즈');
  await expect(due.nth(0)).toContainText('시간 남음');
  await expect(due.nth(1)).toContainText('D-3');
  await expect(due.nth(1)).toContainText('9주차');
  const history = page.getByText('제출했거나 마감이 지난 항목 1개');
  await expect(page.getByRole('list', { name: '제출했거나 마감이 지난 항목' })).toBeHidden();
  await history.click();
  await expect(page.getByRole('list', { name: '제출했거나 마감이 지난 항목' })).toContainText('정상제출');
});

test('D-day counts calendar days and rolls over at midnight without refetching', async ({ page }) => {
  await page.clock.install({ time: new Date('2026-10-24T22:00:00+09:00') });
  await mockDesktop(page, { sourceList: [{ ...board, subscribed: true }], cybercampusId: '20990001', cybercampusTasks: [
    task({ item_id: '2', title: '실습 과제 2', due_at: '2026-10-27T23:59', pending: true }),
    task({ item_id: '3', title: '실습 과제 3', due_at: '2026-10-26T01:00', pending: true }),
  ] });
  await page.goto('/');
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '과제·마감' }).click();
  const due = page.getByRole('list', { name: '해야 할 일' }).getByRole('listitem');
  await expect(due.nth(0)).toContainText('D-2');
  await expect(due.nth(1)).toContainText('D-3');
  // Past midnight KST the count drops by one; the page reads only the clock.
  await page.clock.runFor('02:01:00');
  await expect(due.nth(0)).toContainText('D-1');
  await expect(due.nth(1)).toContainText('D-2');
});

test('the deadline page appears only with the Cybercampus extension on', async ({ page }) => {
  await mockDesktop(page, { sourceList: [{ ...board, subscribed: false }], extensions: { cybercampus: false } });
  await page.goto('/');
  const sidebar = page.getByRole('navigation', { name: '기본 탐색', exact: true });
  await expect(sidebar.getByRole('button', { name: '받은 소식' })).toBeVisible();
  await expect(sidebar.getByRole('button', { name: '과제·마감' })).toHaveCount(0);
  await openSettings(page, '확장 기능');
  await page.getByRole('switch', { name: '사이버캠퍼스 사용 설정', exact: true }).click();
  await expect(sidebar.getByRole('button', { name: '과제·마감' })).toBeVisible();
});

test.describe('Today week pane', () => {
  test.use({ timezoneId: 'Asia/Seoul' });
  const tasks = [
    task({ item_id: '2', title: '실습 과제 2', due_at: '2026-10-26T23:59', pending: true }),
    task({ item_id: '3', title: '실습 과제 3', due_at: '2026-10-24T09:00', late_due_at: '2026-10-25T23:59', pending: true }),
    task({ item_id: '4', title: '제출한 과제', due_at: '2026-10-27T23:59', submitted_at: '2026-10-23T12:00' }),
    task({ item_id: '5', title: '다음 달 과제', due_at: '2026-11-05T23:59', pending: true }),
    task({ item_id: '6', title: '지난 과제', due_at: '2026-10-24T08:00', pending: true }),
  ];

  test('lists open Cybercampus deadlines on their day and opens the deadline page', async ({ page }) => {
    await page.clock.setFixedTime(new Date('2026-10-24T10:00:00+09:00'));
    await mockDesktop(page, { sourceList: [{ ...board, subscribed: true }], cybercampusId: '20990001', cybercampusTasks: tasks });
    await page.goto('/');
    const week = page.getByRole('complementary', { name: '앞으로 7일', exact: true });
    const days = week.locator('.week-day');
    await expect(days.nth(2).locator('.event-task')).toHaveText(/마감 23:59 실습 과제 2\s*자료구조실습/);
    await expect(days.first().locator('.event-task')).toContainText('실습 과제 3');
    await expect(days.first().locator('.event-task')).toContainText('지각 제출 가능');
    await expect(week.locator('.event-task')).toHaveCount(2);
    await expect(days.nth(1)).toContainText('일정 없음');
    await expect(week).toContainText('사이버캠퍼스에서 마지막으로 가져온 과제 마감');
    await days.nth(2).locator('.event-task').click();
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('과제·마감');
  });

  test('hides stored deadlines while the Cybercampus board is unsubscribed', async ({ page }) => {
    await page.clock.setFixedTime(new Date('2026-10-24T10:00:00+09:00'));
    await mockDesktop(page, { sourceList: [{ ...board, subscribed: false }], cybercampusTasks: tasks });
    await page.goto('/');
    const week = page.getByRole('complementary', { name: '앞으로 7일', exact: true });
    await expect(week.locator('.week-day')).toHaveCount(7);
    await expect(week.locator('.event-task')).toHaveCount(0);
    await expect(week).not.toContainText('사이버캠퍼스');
  });
});
