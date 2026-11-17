import type { ClockFormat, CybercampusTask, NoticeDate, StudiumRoomState, TodoPrefill } from './types';

export function archiveError(reason: unknown): string {
  if (typeof reason !== 'string') return '작업을 완료하지 못했습니다. 다시 시도해 주세요.';
  if (/database or disk is full|no space left on device/i.test(reason)) {
    return '저장 공간이 부족해 작업을 완료하지 못했습니다. 공간을 확보한 뒤 다시 시도해 주세요.';
  }
  if (/readonly database|read-only file system|permission denied/i.test(reason)) {
    return '저장 위치에 쓸 수 없습니다. 폴더 권한을 확인한 뒤 다시 시도해 주세요.';
  }
  if (/database is (?:locked|busy)/i.test(reason)) {
    return '저장소가 다른 작업을 처리하고 있습니다. 잠시 후 다시 시도해 주세요.';
  }
  return /[가-힣]/.test(reason) ? reason : '작업을 완료하지 못했습니다. 다시 시도해 주세요.';
}

/** With a clock format, the time is included in that format. */
export function archiveDate(value: string | null, time: ClockFormat | null = null): string {
  if (!value) return '기록 없음';
  const normalized = /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(value)
    ? `${value.replace(' ', 'T')}Z` : /^\d{4}-\d{2}-\d{2}$/.test(value) ? `${value}T00:00:00` : value;
  const date = new Date(normalized);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat('ko-KR', {
    year: 'numeric', month: 'long', day: 'numeric',
    ...(time ? timeOptions(time) : {}),
  }).format(date);
}

/** Intl hour/minute options for a clock format: `오후 9:30` or `21:30`. */
export function timeOptions(format: ClockFormat, seconds = false): Intl.DateTimeFormatOptions {
  const twelve = format === '12h';
  return { hour: twelve ? 'numeric' : '2-digit', minute: '2-digit', ...(seconds ? { second: '2-digit' } : {}), hourCycle: twelve ? 'h12' : 'h23' };
}

/** A local wall-clock `HH:MM` in a clock format. */
export function wallTime(value: string, format: ClockFormat): string {
  if (format === '24h') return value.slice(0, 5);
  const hours = Number(value.slice(0, 2));
  return `${hours < 12 ? '오전' : '오후'} ${hours % 12 || 12}:${value.slice(3, 5)}`;
}

/** Local calendar day (`YYYY-MM-DD`) of an archive timestamp, or null when unparseable. */
export function archiveDay(value: string | null): string | null {
  if (!value) return null;
  if (/^\d{4}-\d{2}-\d{2}$/.test(value)) return value;
  const normalized = /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(value) ? `${value.replace(' ', 'T')}Z` : value;
  const date = new Date(normalized);
  return Number.isNaN(date.getTime()) ? null : localDayKey(date);
}

/** eClass shows Korean local time without an offset (`YYYY-MM-DDTHH:MM` or `YYYY-MM-DD`); read it as KST on any device. */
export function kstInstant(value: string): number {
  return new Date(value.length === 10 ? `${value}T23:59:59+09:00` : `${value}:00+09:00`).getTime();
}

/** Still to do: unsubmitted, and pending or not past its (late) deadline. The 과제·마감 page and its sidebar count share this. */
export function openTask(task: CybercampusTask, now: number): boolean {
  return !task.submitted_at && (task.pending || (task.due_at !== null && kstInstant(task.late_due_at ?? task.due_at) >= now));
}

// storage/calendar `MAX_TITLE`, counted in characters as Rust counts them.
const todoTitleLimit = 200;
function todoTitle(text: string): string {
  const characters = [...text.trim()];
  return characters.length > todoTitleLimit ? `${characters.slice(0, todoTitleLimit - 1).join('')}…` : characters.join('');
}

const noticeDateKinds: Record<NoticeDate['kind'], string> = { deadline: '마감', start: '시작', date: '일정' };
/** The mark shown before a notice date, such as "마감 오후 6:00". A multi-day date shows its times in its span instead. */
export function noticeDateMark(date: NoticeDate, format: ClockFormat): string {
  const kind = noticeDateKinds[date.kind];
  if (!date.time || date.end_day) return kind;
  // The end of the day ("밤 12시"); wallTime would read 24:00 as noon.
  return `${kind} ${date.time === '24:00' && format === '12h' ? '밤 12:00' : wallTime(date.time, format)}`;
}

/** "11월 15일 ~ 16일" for a multi-day notice date, with times when the notice gives them; empty for one day. */
export function noticeDateSpan(date: NoticeDate, format: ClockFormat): string {
  if (!date.end_day) return '';
  const at = (time: string | null) => (time ? ` ${time === '24:00' && format === '12h' ? '밤 12:00' : wallTime(time, format)}` : '');
  const [startYear, startMonth, startDay] = date.day.split('-').map(Number);
  const [endYear, endMonth, endDay] = date.end_day.split('-').map(Number);
  const end = endYear !== startYear ? `${endYear}년 ${endMonth}월 ${endDay}일` : endMonth !== startMonth ? `${endMonth}월 ${endDay}일` : `${endDay}일`;
  return `${startMonth}월 ${startDay}일${at(date.time)} ~ ${end}${at(date.end_time)}`;
}

/** The line under a notice date in the calendar and week: its label and span, then a reminder to check the notice. */
export function noticeDateMeta(date: NoticeDate, format: ClockFormat): string {
  return [date.label, noticeDateSpan(date, format), '원문 확인'].filter(Boolean).join(' · ');
}

/** Whether a notice date falls on `day`, counting every day of a multi-day date. */
export function noticeDateOn(date: NoticeDate, day: string): boolean {
  return date.day <= day && day <= (date.end_day ?? date.day);
}

/** A todo form for a date found in a notice's text. Matching is lexical, so the user checks it first. */
export function noticeDateTodo(date: NoticeDate): TodoPrefill {
  // A todo time is below 24:00; "밤 12시" stays a date without a time.
  const time = date.time && date.time < '24:00' ? date.time : null;
  return {
    todo: { title: todoTitle(`${date.label || noticeDateKinds[date.kind]} · ${date.title}`), day: date.day, time, memo: '', document_id: date.document_id },
    caution: '공지 본문에서 찾은 날짜입니다. 연도나 의미가 틀릴 수 있으니 원문에서 날짜와 시간을 확인한 뒤 저장하세요.',
  };
}

/** A todo form for an eClass deadline. Its due time is Korean local time, copied as the todo's wall-clock time. */
export function taskTodo(task: CybercampusTask & { due_at: string }): TodoPrefill {
  return {
    todo: { title: todoTitle(task.title), day: task.due_at.slice(0, 10), time: task.due_at.length > 10 ? task.due_at.slice(11, 16) : null, memo: task.course_name, document_id: null },
    caution: '사이버캠퍼스 마감을 옮겨 적은 할 일입니다. 마감이 바뀌어도 이 할 일은 그대로이니 과제·마감에서 확인하세요.',
  };
}

export function localDayKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}

export function archiveSize(bytes: number | null): string {
  if (bytes === null) return '크기 미상';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function sourceStatus(status: string): string {
  return ({ idle: '대기', running: '수집 중', completed: '완료', failed: '오류', cancelled: '취소됨',
    interrupted: '중단됨 · 이어받기 가능', limited: '일부 가져옴 · 이어받기 가능' } as Record<string, string>)[status] ?? '확인 필요';
}

export function resourceStatus(status: string): string {
  return ({ downloaded: '로컬 저장됨', pending: '다운로드 대기', failed: '다운로드 실패',
    too_large: '용량 제한 초과', unsupported: '다운로드 미지원' } as Record<string, string>)[status] ?? '확인 필요';
}

// A list scope is '' (every source), one board ID, or `group:<id>` for the
// boards placed directly in that settings group. Board IDs never contain ':'.
const groupPrefix = 'group:';
export function groupScope(id: string): string {
  return groupPrefix + id;
}
export function scopeGroup(scope: string): string | null {
  return scope.startsWith(groupPrefix) ? scope.slice(groupPrefix.length) : null;
}

export function organizationLabel(organization: string): string {
  return organization.replace(/^서강대학교\s*·\s*/, '') || organization;
}

export function studiumStateLabel(value: StudiumRoomState | undefined): string {
  if (value?.status === 'ready') return value.result?.remaining === 0 ? '빈 좌석 없음' : `빈 좌석 ${value.result?.remaining ?? '—'}석`;
  if (value?.status === 'count_only') return `빈 좌석 ${value.result?.remaining ?? '—'}석 · 번호 확인 불가`;
  if (value?.status === 'loading') return '확인 중';
  if (value?.status === 'failed') return '확인 실패';
  if (value?.status === 'cancelled') return '확인 취소됨';
  return '아직 확인하지 않았습니다';
}

/** A seat observation older than five minutes, or more than a minute ahead of the PC clock. */
export function isStaleObservation(value: string, now: number): boolean {
  const age = now - Date.parse(value);
  return !Number.isFinite(age) || age > 300_000 || age < -60_000;
}
