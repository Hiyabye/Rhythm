import { test, expect } from '@playwright/test';
import { archiveError, archiveDate, archiveDay, localDayKey, noticeDateTodo, taskTodo, wallTime } from '../../src/lib/archive-format';

test('storage failures have actionable Korean feedback rather than raw database diagnostics', () => {
  expect(archiveError('database or disk is full')).toContain('저장 공간이 부족');
  expect(archiveError('attempt to write a readonly database')).toContain('폴더 권한');
  expect(archiveError('database is locked')).toContain('잠시 후');
  expect(archiveError('unknown SQLite exception')).toBe('작업을 완료하지 못했습니다. 다시 시도해 주세요.');
  expect(archiveError('공개 자료를 찾지 못했습니다.')).toBe('공개 자료를 찾지 못했습니다.');
});

test('calendar publication dates are stable across timezones and UTC collection dates parse explicitly', () => {
  const original = process.env.TZ;
  try {
    for (const zone of ['Asia/Seoul', 'America/Los_Angeles', 'Pacific/Honolulu']) {
      process.env.TZ = zone;
      expect(archiveDate('2026-01-01')).toBe('2026년 1월 1일');
      expect(archiveDate('2026-01-01 23:15:00', '12h')).toBe(archiveDate('2026-01-01T23:15:00Z', '12h'));
    }
  } finally {
    if (original === undefined) delete process.env.TZ;
    else process.env.TZ = original;
  }
});

test('archive day keys keep calendar dates and convert UTC collection times to the local day', () => {
  expect(archiveDay('2026-10-24')).toBe('2026-10-24');
  expect(archiveDay('2026-10-24 12:00:00')).toBe(localDayKey(new Date(Date.UTC(2026, 9, 24, 12))));
  expect(archiveDay('잘못된 날짜')).toBeNull();
  expect(archiveDay(null)).toBeNull();
  expect(localDayKey(new Date(2026, 0, 5))).toBe('2026-01-05');
});

test('times follow the 12/24-hour setting at midnight, noon and evening', () => {
  expect(['00:05', '12:00', '21:30'].map((time) => wallTime(time, '12h'))).toEqual(['오전 12:05', '오후 12:00', '오후 9:30']);
  expect(['00:05', '12:00', '21:30'].map((time) => wallTime(time, '24h'))).toEqual(['00:05', '12:00', '21:30']);
  const original = process.env.TZ;
  try {
    process.env.TZ = 'Asia/Seoul';
    expect(archiveDate('2026-01-01 12:05:00', '12h')).toBe('2026년 1월 1일 오후 9:05');
    expect(archiveDate('2026-01-01 15:05:00', '24h')).toBe('2026년 1월 2일 00:05');
  } finally {
    if (original === undefined) delete process.env.TZ;
    else process.env.TZ = original;
  }
});

test('todo prefills fit the stored title limit and keep only what the source supports', () => {
  const date = { document_id: 7, ordinal: 0, title: '가'.repeat(300), day: '2026-10-30', time: '24:00', label: '신청 마감', kind: 'deadline' as const };
  const long = noticeDateTodo(date);
  expect([...long.todo.title]).toHaveLength(200);
  expect(long.todo.title.endsWith('…')).toBe(true);
  expect(long.todo).toMatchObject({ day: '2026-10-30', time: null, document_id: 7 });
  const unlabeled = noticeDateTodo({ ...date, label: '', kind: 'date', time: '09:30' }).todo;
  expect(unlabeled.title.startsWith('일정 · 가')).toBe(true);
  expect([...unlabeled.title]).toHaveLength(200);
  expect(unlabeled.time).toBe('09:30');
  const task = { course_key: 'A1', course_name: '자료구조실습', category: 'report', item_id: '1', label: '과제', title: '실습 과제 2', week: 9,
    opens_at: null, late_due_at: null, status: null, submitted_at: null, pending: true, updated_at: '2026-10-20T00:00:00Z' };
  expect(taskTodo({ ...task, due_at: '2026-10-26T23:59' }).todo).toEqual({ title: '실습 과제 2', day: '2026-10-26', time: '23:59', memo: '자료구조실습', document_id: null });
  expect(taskTodo({ ...task, due_at: '2026-10-26' }).todo.time).toBeNull();
});
