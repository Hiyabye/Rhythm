import { test, expect, type Page } from '@playwright/test';
import { mockDesktop } from './fixture';
import type { CalendarTodo, CybercampusTask } from '../../src/lib/types';

test.use({ timezoneId: 'Asia/Seoul' });
const board = { id: 'cybercampus', name: '강의 공지', organization: '서강대학교 · 사이버캠퍼스', supported: true, family: 'cybercampus', subscribed: true };
const deadline: CybercampusTask = { course_key: 'A1', course_name: '자료구조실습', category: 'report', item_id: '1', label: '과제', title: '실습 과제 2', week: 9,
  opens_at: null, due_at: '2026-10-26T23:59', late_due_at: null, status: null, submitted_at: null, pending: true, updated_at: '2026-10-20T00:00:00Z' };
const todo = (overrides: Partial<CalendarTodo>): CalendarTodo => ({ id: 1, title: '장학금 서류', day: '2026-10-25', time: null, memo: '', done: false, ...overrides });

async function openCalendar(page: Page) {
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '캘린더' }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('캘린더');
}

test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(new Date('2026-10-24T10:00:00+09:00'));
});

test('todos are added, completed, moved and deleted from the selected day', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await openCalendar(page);
  await expect(page.getByRole('heading', { name: '2026년 10월' })).toBeVisible();
  const panel = page.getByRole('region', { name: /10월 24일 토요일/ });
  await expect(panel).toContainText('일정 없음');

  await panel.getByRole('button', { name: '할 일 추가' }).click();
  const add = panel.getByRole('button', { name: '추가', exact: true });
  await expect(add).toBeDisabled();
  await panel.getByRole('textbox', { name: '제목' }).fill('보고서 제출');
  await panel.getByLabel('시간 (선택)').fill('09:30');
  await panel.getByRole('textbox', { name: '메모 (선택)' }).fill('학과 사무실');
  await add.click();
  await expect(panel.getByRole('status')).toHaveText('할 일을 추가했습니다.');
  const list = panel.getByRole('list', { name: '10월 24일 일정' });
  await expect(list).toContainText('09:30 보고서 제출');
  await expect(list).toContainText('학과 사무실');
  const today = page.locator('[data-day="2026-10-24"]');
  await expect(today).toHaveAccessibleName('10월 24일 토요일, 오늘, 할 일 1개');

  await list.getByRole('checkbox', { name: '완료 표시' }).check();
  await expect(list.getByRole('checkbox', { name: '완료함' })).toBeChecked();
  await expect(today).toHaveAccessibleName('10월 24일 토요일, 오늘');

  // Moving a todo to another day selects that day, so it stays in view.
  await list.getByRole('button', { name: '보고서 제출 수정' }).click();
  await panel.getByLabel('날짜').fill('2026-11-02');
  await panel.getByRole('button', { name: '저장' }).click();
  await expect(page.getByRole('heading', { name: '2026년 11월' })).toBeVisible();
  const moved = page.getByRole('region', { name: /11월 2일 월요일/ });
  await expect(moved.getByRole('list')).toContainText('보고서 제출');

  await page.evaluate(() => (window as any).__fixture.dialogAnswers.push('cancel'));
  await moved.getByRole('button', { name: '보고서 제출 삭제' }).click();
  await expect(moved.getByRole('list')).toContainText('보고서 제출');
  await page.evaluate(() => (window as any).__fixture.dialogAnswers.push('ok'));
  await moved.getByRole('button', { name: '보고서 제출 삭제' }).click();
  await expect(moved.getByRole('status')).toHaveText('할 일을 삭제했습니다.');
  await expect(moved).toContainText('일정 없음');
});

test('arrow and page keys move the selected day across months', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await openCalendar(page);
  const today = page.locator('[data-day="2026-10-24"]');
  await today.focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.locator('[data-day="2026-10-25"]')).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(page.locator('[data-day="2026-11-01"]')).toBeFocused();
  await expect(page.getByRole('heading', { name: '2026년 11월' })).toBeVisible();
  await page.keyboard.press('PageUp');
  await expect(page.locator('[data-day="2026-10-01"]')).toBeFocused();
  await expect(page.locator('[data-day="2026-10-01"]')).toHaveAttribute('tabindex', '0');
  await page.getByRole('button', { name: '오늘로 이동' }).click();
  await expect(today).toBeFocused();
});

test('the calendar shows open eClass deadlines and AI reference dates', async ({ page }) => {
  await mockDesktop(page, { sourceList: [board], cybercampusId: '20990001', cybercampusTasks: [deadline], aiResult: true, aiInstalled: true });
  await page.goto('/');
  await openCalendar(page);
  await page.locator('[data-day="2026-10-26"]').click();
  const deadlineDay = page.getByRole('region', { name: /10월 26일 월요일/ });
  await expect(deadlineDay).toContainText('마감 23:59 실습 과제 2');
  await page.locator('[data-day="2026-10-30"]').click();
  const aiDay = page.getByRole('region', { name: /10월 30일 금요일/ });
  await expect(aiDay).toContainText('AI 참고 일정 신청 마감');
});

test('the calendar leaves out deadlines and AI dates whose extensions are off', async ({ page }) => {
  await mockDesktop(page, { sourceList: [board], cybercampusId: '20990001', cybercampusTasks: [deadline], aiResult: true, aiInstalled: true,
    extensions: { cybercampus: false, ai: false } });
  await page.goto('/');
  await openCalendar(page);
  await expect(page.locator('[data-day="2026-10-26"]')).toHaveAccessibleName('10월 26일 월요일');
  await expect(page.locator('[data-day="2026-10-30"]')).toHaveAccessibleName('10월 30일 금요일');
});

test('the week pane shows open todos and opens the calendar on their day', async ({ page }) => {
  await mockDesktop(page, { calendarTodos: [todo({}), todo({ id: 2, title: '끝낸 일', done: true }), todo({ id: 3, title: '다음 주 일', day: '2026-11-03' })] });
  await page.goto('/');
  const week = page.getByRole('complementary', { name: '앞으로 7일', exact: true });
  await expect(week.locator('.event-todo')).toHaveCount(1);
  await expect(week.locator('.week-day').nth(1).locator('.event-todo')).toContainText('할 일 장학금 서류');
  await expect(week).toContainText('캘린더에 직접 만든 할 일');
  await week.locator('.event-todo').click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('캘린더');
  await expect(page.getByRole('region', { name: /10월 25일 일요일/ })).toContainText('장학금 서류');
  await expect(page.locator('[data-day="2026-10-25"]')).toHaveAttribute('tabindex', '0');
});

test('turning the calendar off hides its page and the todos in the week', async ({ page }) => {
  await mockDesktop(page, { extensions: { calendar: false }, calendarTodos: [todo({})] });
  await page.goto('/');
  await expect(page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '캘린더' })).toHaveCount(0);
  const week = page.getByRole('complementary', { name: '앞으로 7일', exact: true });
  await expect(week.locator('.week-day')).toHaveCount(7);
  await expect(week.locator('.event-todo')).toHaveCount(0);
  await expect(week).not.toContainText('캘린더');
});
