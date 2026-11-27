import { test, expect, type Page } from '@playwright/test';
import { mockDesktop, openSettings } from './fixture';
import type { CalendarEvent, CalendarTodo, CybercampusTask, NoticeDate } from '../../src/lib/types';

test.use({ timezoneId: 'Asia/Seoul' });
const board = { id: 'cybercampus', name: '강의 공지', organization: '서강대학교 · 사이버캠퍼스', supported: true, family: 'cybercampus', subscribed: true };
const deadline: CybercampusTask = { course_key: 'A1', course_name: '자료구조실습', category: 'report', item_id: '1', label: '과제', title: '실습 과제 2', week: 9,
  opens_at: null, due_at: '2026-10-26T23:59', late_due_at: null, status: null, submitted_at: null, pending: true, updated_at: '2026-10-20T00:00:00Z' };
const noticeDate = (overrides: Partial<NoticeDate & { saved: boolean }>): NoticeDate & { saved?: boolean } => ({
  document_id: 1, ordinal: 0, title: '한국어 공지 01', day: '2026-10-30', time: '18:00', end_day: null, end_time: null, label: '신청 마감', kind: 'deadline', ...overrides });
const academic = (uid: string, title: string, start_day: string, end_day = start_day): CalendarEvent =>
  ({ calendar_id: 'sogang-academic-calendar', uid, title, start_day, end_day });
const academicEvents = [academic('exam', '중간시험', '2026-10-26', '2026-10-28'), academic('fest', '개교기념일', '2026-10-30')];
const todo = (overrides: Partial<CalendarTodo>): CalendarTodo => ({ id: 1, title: '장학금 서류', day: '2026-10-25', time: null, memo: '', done: false, document_id: null, reminder_minutes: null, ...overrides });

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
  await panel.getByRole('combobox', { name: '시' }).selectOption('9');
  await panel.getByRole('combobox', { name: '분' }).selectOption('30');
  await panel.getByRole('textbox', { name: '메모 (선택)' }).fill('학과 사무실');
  await add.click();
  await expect(panel.getByRole('status')).toHaveText('할 일을 추가했습니다.');
  const list = panel.getByRole('list', { name: '10월 24일 일정' });
  await expect(list).toContainText('오전 9:30 보고서 제출');
  await expect(list).toContainText('학과 사무실');
  const today = page.locator('[data-day="2026-10-24"]');
  await expect(today).toHaveAccessibleName('10월 24일 토요일, 오늘, 할 일 1개');

  await list.getByRole('checkbox', { name: '완료 표시' }).check();
  await expect(list.getByRole('checkbox', { name: '완료함' })).toBeChecked();
  await expect(today).toHaveAccessibleName('10월 24일 토요일, 오늘');

  // Moving a todo to another day selects that day, so it stays in view.
  await list.getByRole('button', { name: '보고서 제출 수정' }).click();
  const am = panel.getByRole('button', { name: '오전', exact: true });
  const pm = panel.getByRole('button', { name: '오후', exact: true });
  await expect(am).toHaveAttribute('aria-pressed', 'true');
  await expect(panel.getByRole('combobox', { name: '시' })).toHaveValue('9');
  await expect(panel.getByRole('combobox', { name: '분' })).toHaveValue('30');
  await pm.click();
  await expect(pm).toHaveAttribute('aria-pressed', 'true');
  await expect(am).toHaveAttribute('aria-pressed', 'false');
  await panel.getByLabel('날짜').fill('2026-11-02');
  await panel.getByRole('button', { name: '저장' }).click();
  await expect(page.getByRole('heading', { name: '2026년 11월' })).toBeVisible();
  const moved = page.getByRole('region', { name: /11월 2일 월요일/ });
  await expect(moved.getByRole('list')).toContainText('오후 9:30 보고서 제출');

  await page.evaluate(() => (window as any).__fixture.dialogAnswers.push('cancel'));
  await moved.getByRole('button', { name: '보고서 제출 삭제' }).click();
  await expect(moved.getByRole('list')).toContainText('보고서 제출');
  await page.evaluate(() => (window as any).__fixture.dialogAnswers.push('ok'));
  await moved.getByRole('button', { name: '보고서 제출 삭제' }).click();
  await expect(moved.getByRole('status')).toHaveText('할 일을 삭제했습니다.');
  await expect(moved).toContainText('일정 없음');
});

test('the 24-hour setting shows and edits todo times without 오전/오후', async ({ page }) => {
  await mockDesktop(page, { calendarTodos: [todo({ day: '2026-10-24', time: '21:05', title: '보고서 제출' })] });
  await page.goto('/');
  await openSettings(page, '일반');
  const formats = page.getByRole('group', { name: '시간 표시', exact: true });
  await expect(formats.getByRole('button', { name: '12시간' })).toHaveAttribute('aria-pressed', 'true');
  await formats.getByRole('button', { name: '24시간' }).click();
  await expect(formats.getByRole('button', { name: '24시간' })).toHaveAttribute('aria-pressed', 'true');
  await expect(formats.getByRole('button', { name: '12시간' })).toHaveAttribute('aria-pressed', 'false');
  await expect.poll(() => page.evaluate(() => {
    const calls = (window as any).__fixture.calls.filter((call: any) => call.command === 'save_settings');
    return calls[calls.length - 1]?.args.settings.clock_format;
  })).toBe('24h');

  await openCalendar(page);
  const panel = page.getByRole('region', { name: /10월 24일 토요일/ });
  const list = panel.getByRole('list', { name: '10월 24일 일정' });
  await expect(list).toContainText('21:05 보고서 제출');
  await list.getByRole('button', { name: '보고서 제출 수정' }).click();
  await expect(panel.getByRole('group', { name: '오전 또는 오후' })).toHaveCount(0);
  const hour = panel.getByRole('combobox', { name: '시' });
  await expect(hour).toHaveValue('21');
  await expect(hour.locator('option')).toHaveCount(25);
  await hour.selectOption('0');
  await panel.getByRole('button', { name: '저장' }).click();
  await expect(list).toContainText('00:05 보고서 제출');
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

test('the calendar shows open eClass deadlines and saved notices\' dates', async ({ page }) => {
  await mockDesktop(page, { sourceList: [board], cybercampusId: '20990001', cybercampusTasks: [deadline], noticeDates: [noticeDate({})] });
  await page.goto('/');
  await openCalendar(page);
  await page.locator('[data-day="2026-10-26"]').click();
  const deadlineDay = page.getByRole('region', { name: /10월 26일 월요일/ });
  await expect(deadlineDay).toContainText('마감 오후 11:59 실습 과제 2');
  await page.locator('[data-day="2026-10-30"]').click();
  const noticeDay = page.getByRole('region', { name: /10월 30일 금요일/ });
  // The notice title leads; the label found before the date explains it.
  await expect(noticeDay).toContainText('마감 오후 6:00 한국어 공지 01');
  await expect(noticeDay).toContainText('신청 마감 · 원문 확인');
  await expect(page.locator('[data-day="2026-10-30"]')).toHaveAccessibleName('10월 30일 금요일, 공지 날짜 1개');
});

test('a multi-day notice date appears on every day it covers', async ({ page }) => {
  await mockDesktop(page, { noticeDates: [noticeDate({ day: '2026-10-27', time: '13:00', end_day: '2026-10-29', end_time: '17:00', label: '행사 일시', kind: 'date' })] });
  await page.goto('/');
  await openCalendar(page);
  for (const day of ['2026-10-27', '2026-10-28', '2026-10-29']) await expect(page.locator(`[data-day="${day}"]`)).toHaveAccessibleName(/공지 날짜 1개/);
  await expect(page.locator('[data-day="2026-10-26"]')).toHaveAccessibleName('10월 26일 월요일');
  await expect(page.locator('[data-day="2026-10-30"]')).toHaveAccessibleName('10월 30일 금요일');
  await page.locator('[data-day="2026-10-28"]').click();
  const panel = page.getByRole('region', { name: /10월 28일 수요일/ });
  await expect(panel).toContainText('일정 한국어 공지 01');
  await expect(panel).toContainText('행사 일시 · 10월 27일 오후 1:00 ~ 29일 오후 5:00 · 원문 확인');
});

test('the week pane shows a multi-day notice date that started before today', async ({ page }) => {
  await page.clock.setFixedTime(new Date('2026-10-28T10:00:00+09:00'));
  await mockDesktop(page, { noticeDates: [noticeDate({ day: '2026-10-27', time: '13:00', end_day: '2026-10-29', end_time: '17:00', label: '행사 일시', kind: 'date' })] });
  await page.goto('/');
  const week = page.getByRole('complementary', { name: '앞으로 7일', exact: true });
  await expect(week.locator('.week-day').nth(0).locator('.event-notice')).toContainText('일정 한국어 공지 01');
  await expect(week.locator('.week-day').nth(1).locator('.event-notice')).toContainText('29일 오후 5:00');
  await expect(week.locator('.week-day').nth(2).locator('.event-notice')).toHaveCount(0);
});

test('a todo reminder offers choices that fit its time and is saved with it', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await openCalendar(page);
  const panel = page.getByRole('region', { name: /10월 24일 토요일/ });
  await panel.getByRole('button', { name: '할 일 추가' }).click();
  await panel.getByRole('textbox', { name: '제목' }).fill('보고서 제출');
  const reminder = panel.getByRole('combobox', { name: '알림' });
  await expect(reminder.getByRole('option')).toHaveText(['알림 없음', '당일 오전 9:00', '전날 오전 9:00']);
  await expect(panel.getByRole('note')).toHaveCount(0);
  await panel.getByRole('combobox', { name: '시' }).selectOption('9');
  await expect(reminder.getByRole('option')).toHaveText(['알림 없음', '제시간', '10분 전', '30분 전', '1시간 전', '하루 전']);
  await reminder.selectOption('10분 전');
  await expect(panel.getByText('알림·백그라운드에서 알림을 켜 두고 Rhythm이 열려 있는 동안 보냅니다.', { exact: false })).toBeVisible();
  // Clearing the time drops a choice an untimed todo cannot have.
  await panel.getByRole('combobox', { name: '시' }).selectOption('');
  await expect(reminder).toHaveValue(await reminder.locator('option', { hasText: '알림 없음' }).getAttribute('value') ?? '');
  await panel.getByRole('combobox', { name: '시' }).selectOption('9');
  await reminder.selectOption('10분 전');
  await panel.getByRole('button', { name: '추가', exact: true }).click();
  await expect(panel.getByRole('status')).toHaveText('할 일을 추가했습니다.');
  await expect(panel.getByRole('list', { name: '10월 24일 일정' })).toContainText('알림 10분 전');
  const created = await page.evaluate(() => (window as any).__fixture.calls.find((call: any) => call.command === 'calendar_create_todo').args.todo);
  expect(created).toMatchObject({ title: '보고서 제출', time: '09:00', reminder_minutes: 10 });

  await panel.getByRole('button', { name: '보고서 제출 수정' }).click();
  await expect(panel.getByRole('combobox', { name: '알림' })).toHaveValue(await panel.getByRole('combobox', { name: '알림' }).locator('option', { hasText: '10분 전' }).getAttribute('value') ?? '');
});

test('a clicked reminder opens the calendar on the todo day, also before the window listened', async ({ page }) => {
  await mockDesktop(page, { pendingReminderDay: '2026-11-03', calendarTodos: [todo({ id: 4, day: '2026-11-03', time: '10:00', title: '면담', reminder_minutes: 30 })] });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: '2026년 11월' })).toBeVisible();
  const day = page.getByRole('region', { name: /11월 3일 화요일/ });
  await expect(day).toContainText('오전 10:00 면담');
  await expect(day).toContainText('알림 30분 전');
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '오늘', exact: true }).click();
  await page.evaluate(() => (window as any).__fixture.emitReminder('2026-10-30'));
  await expect(page.getByRole('region', { name: /10월 30일 금요일/ })).toBeVisible();
});

test('export saves todos and open deadlines to a chosen .ics file', async ({ page }) => {
  await mockDesktop(page, { sourceList: [board], cybercampusId: '20990001', cybercampusTasks: [deadline],
    calendarTodos: [todo({}), todo({ id: 2, title: '끝낸 일', done: true })] });
  await page.goto('/');
  await openCalendar(page);
  await page.getByRole('button', { name: '내보내기' }).click();
  const panel = page.getByRole('region', { name: /10월 24일 토요일/ });
  await expect(panel.getByRole('status')).toHaveText('할 일 1개와 마감 1개를 내보냈습니다. 내보낸 파일은 Rhythm이 관리하지 않습니다. 일정이 바뀌면 다시 내보내 주세요.');
  const calls = await page.evaluate(() => (window as any).__fixture.calls
    .filter((call: any) => call.command === 'plugin:dialog|save' || call.command === 'calendar_export').map((call: any) => call.args));
  expect(calls[0].options).toMatchObject({ defaultPath: 'Rhythm-calendar.ics', filters: [{ name: 'iCalendar', extensions: ['ics'] }] });
  expect(calls[1]).toEqual({ destination: '/fixtures/Rhythm-backup.rhythm-backup' });
});

test('export with nothing to save reports it', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await openCalendar(page);
  await page.getByRole('button', { name: '내보내기' }).click();
  await expect(page.getByRole('alert')).toHaveText('내보낼 할 일이나 마감이 없습니다.');
});

test('the calendar leaves out deadlines whose extension is off', async ({ page }) => {
  await mockDesktop(page, { sourceList: [board], cybercampusId: '20990001', cybercampusTasks: [deadline],
    extensions: { cybercampus: false } });
  await page.goto('/');
  await openCalendar(page);
  await expect(page.locator('[data-day="2026-10-26"]')).toHaveAccessibleName('10월 26일 월요일');
});

test('every notice\'s deadlines are added only after the user turns them on', async ({ page }) => {
  await mockDesktop(page, { noticeDates: [noticeDate({}),
    noticeDate({ document_id: 2, title: '한국어 공지 02', day: '2026-10-28', time: null, label: '접수 마감', saved: false }),
    noticeDate({ document_id: 2, ordinal: 1, title: '한국어 공지 02', day: '2026-10-28', time: null, label: '설명회', kind: 'date', saved: false })] });
  await page.goto('/');
  await openCalendar(page);
  const day = page.locator('[data-day="2026-10-28"]');
  await expect(day).toHaveAccessibleName('10월 28일 수요일');
  const toggle = page.getByRole('checkbox', { name: '모든 공지의 마감 표시' });
  await expect(toggle).not.toBeChecked();
  await toggle.check();
  await expect(page.locator('.calendar-feedback')).toHaveText('모든 공지의 마감을 표시합니다.');
  // Only the unsaved notice's deadline appears, not its other dates.
  await expect(day).toHaveAccessibleName('10월 28일 수요일, 공지 날짜 1개');
  await day.click();
  await expect(page.getByRole('region', { name: /10월 28일 수요일/ })).toContainText('마감 한국어 공지 02');
  const calls = await page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'calendar_set_all_deadlines').map((call: any) => call.args));
  expect(calls).toEqual([{ enabled: true }]);
  // The choice is read back from the backend when the page mounts again.
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await openCalendar(page);
  await expect(page.getByRole('checkbox', { name: '모든 공지의 마감 표시' })).toBeChecked();
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

test('a notice date becomes a linked todo after the user checks the prefilled form', async ({ page }) => {
  await mockDesktop(page, { noticeDates: [noticeDate({})] });
  await page.goto('/');
  await openCalendar(page);
  await page.locator('[data-day="2026-10-30"]').click();
  const panel = page.getByRole('region', { name: /10월 30일 금요일/ });
  await panel.getByRole('button', { name: '한국어 공지 01 신청 마감 할 일로 추가' }).click();
  await expect(panel.getByRole('note')).toContainText('공지 본문에서 찾은 날짜입니다');
  const title = panel.getByRole('textbox', { name: '제목' });
  await expect(title).toBeFocused();
  await expect(title).toHaveValue(/^신청 마감 · /);
  await expect(panel.getByLabel('날짜')).toHaveValue('2026-10-30');
  await expect(panel.getByRole('combobox', { name: '시' })).toHaveValue('6');
  // A found date may be wrong, so the form lets the user move it before saving.
  await title.fill('장학금 신청');
  await panel.getByLabel('날짜').fill('2026-10-29');
  await panel.getByRole('button', { name: '추가', exact: true }).click();
  const moved = page.getByRole('region', { name: /10월 29일 목요일/ });
  await expect(moved.getByRole('status')).toHaveText('할 일을 추가했습니다.');
  const created = await page.evaluate(() => (window as any).__fixture.calls.find((call: any) => call.command === 'calendar_create_todo').args.todo);
  expect(created).toMatchObject({ title: '장학금 신청', day: '2026-10-29', time: '18:00', document_id: 1 });

  await moved.getByRole('button', { name: '장학금 신청 원문 보기' }).click();
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 01');
  await page.getByRole('button', { name: /목록으로 돌아가기/ }).click();
  await expect(page.getByRole('button', { name: '장학금 신청 원문 보기' })).toBeFocused();
  await expect(page.getByRole('region', { name: /10월 29일 목요일/ }).getByRole('textbox', { name: '제목' })).toHaveCount(0);
});

test('a notice date in the reader opens the calendar with a prefilled todo', async ({ page }) => {
  await mockDesktop(page, { noticeDates: [noticeDate({ time: '24:00' })] });
  await page.goto('/');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await page.locator('#notice-1').click();
  const dates = page.getByRole('region', { name: /공지 속 날짜/ });
  await expect(dates).toContainText('마감 밤 12:00 신청 마감');
  await dates.getByRole('button', { name: '신청 마감 할 일로 추가' }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('캘린더');
  const panel = page.getByRole('region', { name: /10월 30일 금요일/ });
  await expect(panel.getByRole('textbox', { name: '제목' })).toHaveValue('신청 마감 · 한국어 공지 01');
  // A todo time is below 24:00, so the end of the day stays a date without a time.
  await expect(panel.getByRole('combobox', { name: '시' })).toHaveValue('');
  await expect(panel.getByRole('note')).toContainText('원문에서 날짜와 시간을 확인한 뒤 저장하세요');
});

test('an open eClass deadline is copied into a todo with its due time and course', async ({ page }) => {
  await mockDesktop(page, { sourceList: [board], cybercampusId: '20990001', cybercampusTasks: [deadline] });
  await page.goto('/');
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '과제·마감' }).click();
  await page.getByRole('button', { name: '실습 과제 2 캘린더에 할 일로 추가' }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('캘린더');
  const panel = page.getByRole('region', { name: /10월 26일 월요일/ });
  await expect(panel.getByRole('textbox', { name: '제목' })).toHaveValue('실습 과제 2');
  await expect(panel.getByRole('textbox', { name: '메모 (선택)' })).toHaveValue('자료구조실습');
  await expect(panel.getByRole('combobox', { name: '시' })).toHaveValue('11');
  await expect(panel.getByRole('combobox', { name: '분' })).toHaveValue('59');
  await expect(panel.getByRole('button', { name: '오후', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(panel.getByRole('note')).toContainText('마감이 바뀌어도 이 할 일은 바뀌지 않으니');
  await panel.getByRole('button', { name: '추가', exact: true }).click();
  await expect(panel.getByRole('list')).toContainText('오후 11:59 실습 과제 2');
  // eClass deadlines are replaced on every sync, so the todo keeps no link.
  await expect(panel.getByRole('button', { name: '실습 과제 2 원문 보기' })).toHaveCount(0);

  // The calendar's own deadline row offers the same copy.
  await panel.getByRole('button', { name: '실습 과제 2 할 일로 추가' }).click();
  await expect(panel.getByRole('textbox', { name: '제목' })).toHaveValue('실습 과제 2');
});

test('without the calendar extension no page offers to add a todo', async ({ page }) => {
  await mockDesktop(page, { sourceList: [board], cybercampusId: '20990001', cybercampusTasks: [deadline], extensions: { calendar: false },
    noticeDates: [noticeDate({})] });
  await page.goto('/');
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '과제·마감' }).click();
  await expect(page.getByText('실습 과제 2')).toBeVisible();
  await expect(page.getByRole('button', { name: /캘린더에 할 일로 추가/ })).toHaveCount(0);
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await page.locator('#notice-1').click();
  // The reader still lists the notice's dates, without a todo button.
  const dates = page.getByRole('region', { name: /공지 속 날짜/ });
  await expect(dates).toContainText('신청 마감');
  await expect(dates.getByRole('button')).toHaveCount(0);
});

test('the academic calendar shows school events on every day they cover until the user hides it', async ({ page }) => {
  await mockDesktop(page, { calendarEvents: academicEvents });
  await page.goto('/');
  await openCalendar(page);
  for (const day of ['2026-10-26', '2026-10-27', '2026-10-28']) await expect(page.locator(`[data-day="${day}"]`)).toHaveAccessibleName(/학사 일정 1개/);
  await expect(page.locator('[data-day="2026-10-29"]')).toHaveAccessibleName('10월 29일 목요일');
  await page.locator('[data-day="2026-10-27"]').click();
  const panel = page.getByRole('region', { name: /10월 27일 화요일/ });
  await expect(panel).toContainText('학사 중간시험');
  await expect(panel).toContainText('10월 26일 ~ 28일 · 학교 학사 일정');
  await expect(page.locator('.calendar-options').nth(1)).toContainText('학교가 게시한 학사 일정을 표시합니다.');

  const toggle = page.getByRole('checkbox', { name: '학사 일정 표시' });
  await expect(toggle).toBeChecked();
  await toggle.uncheck();
  await expect(page.locator('.calendar-feedback')).toHaveText('학사 일정을 숨깁니다.');
  await expect(page.locator('[data-day="2026-10-27"]')).toHaveAccessibleName('10월 27일 화요일');
  await expect(panel).toContainText('일정 없음');
  const calls = await page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'calendar_set_academic').map((call: any) => call.args));
  expect(calls).toEqual([{ enabled: false }]);
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await openCalendar(page);
  await expect(page.getByRole('checkbox', { name: '학사 일정 표시' })).not.toBeChecked();
});

test('the week pane shows school events and opens the calendar on their day', async ({ page }) => {
  await mockDesktop(page, { calendarEvents: academicEvents });
  await page.goto('/');
  const week = page.getByRole('complementary', { name: '앞으로 7일', exact: true });
  // The window runs from 10월 24일; the exam covers its third to fifth days.
  for (const index of [2, 3, 4]) await expect(week.locator('.week-day').nth(index).locator('.event-academic')).toContainText('학사 중간시험');
  await expect(week.locator('.week-day').nth(6).locator('.event-academic')).toContainText('학사 개교기념일');
  await expect(week.locator('.event-academic')).toHaveCount(4);
  await expect(week).toContainText('학교 학사 일정');
  await week.locator('.week-day').nth(3).locator('.event-academic').click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('캘린더');
  await expect(page.getByRole('region', { name: /10월 27일 화요일/ })).toContainText('학사 중간시험');
});

test('the academic calendar says it needs 서강 공지, and the week leaves it out without the calendar', async ({ page }) => {
  await mockDesktop(page, { enabled: false, calendarEvents: academicEvents });
  await page.goto('/');
  await openCalendar(page);
  await expect(page.locator('.calendar-options').nth(1)).toContainText('학사 일정은 서강 공지 확장 기능이 켜져 있을 때 받습니다.');
  // What was received before stays visible.
  await expect(page.locator('[data-day="2026-10-27"]')).toHaveAccessibleName(/학사 일정 1개/);

  await mockDesktop(page, { extensions: { calendar: false }, calendarEvents: academicEvents });
  await page.goto('/');
  const week = page.getByRole('complementary', { name: '앞으로 7일', exact: true });
  await expect(week.locator('.week-day')).toHaveCount(7);
  await expect(week.locator('.event-academic')).toHaveCount(0);
  await expect(week).not.toContainText('학사 일정');
});

test('a school event becomes a todo after the user checks the prefilled form', async ({ page }) => {
  await mockDesktop(page, { calendarEvents: academicEvents });
  await page.goto('/');
  await openCalendar(page);
  await page.locator('[data-day="2026-10-28"]').click();
  await page.getByRole('button', { name: '중간시험 할 일로 추가' }).click();
  await expect(page.getByRole('note')).toContainText('학사 일정을 옮겨 만든 할 일입니다.');
  await expect(page.getByRole('textbox', { name: '제목' })).toHaveValue('중간시험');
  await page.getByRole('button', { name: '추가', exact: true }).click();
  await expect(page.locator('.calendar-feedback')).toHaveText('할 일을 추가했습니다.');
  // The todo starts on the event's first day.
  await expect(page.locator('[data-day="2026-10-26"]')).toHaveAccessibleName(/할 일 1개/);
});
