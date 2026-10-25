import { test, expect } from '@playwright/test';
import { archiveError, archiveDate } from '../../src/lib/archive-format';

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
      expect(archiveDate('2026-01-01 23:15:00', true)).toBe(archiveDate('2026-01-01T23:15:00Z', true));
    }
  } finally {
    if (original === undefined) delete process.env.TZ;
    else process.env.TZ = original;
  }
});
