import { test, expect, type Page } from '@playwright/test';
import { cybercampusTask, mockDesktop, openSettings, type FixtureOptions } from './fixture';
import type { CybercampusTask } from '../../src/lib/types';

// Regenerates the README screenshots in docs/screenshots/ from the mocked IPC
// fixture (synthetic data only). Run with `npm run screenshots`; skipped otherwise.
test.skip(process.env.RHYTHM_SCREENSHOTS !== '1', 'set RHYTHM_SCREENSHOTS=1 (npm run screenshots) to regenerate README screenshots');
test.skip(({ browserName }) => browserName !== 'chromium', 'screenshots use Chromium only');
test.use({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1, colorScheme: 'light', reducedMotion: 'reduce' });

const OUT = 'docs/screenshots';
// Title, the board's own category, body, and the AI category the classifier would give.
const titles: [string, string, string, string][] = [
  ['2026학년도 2학기 교내 장학금 신청 안내', '장학', '장학금 신청 기간과 제출 서류를 안내합니다. 장학 신청서를 제출하세요.', '장학'],
  ['졸업 요건 사전 점검 신청 안내', '학사', '졸업 예정자는 졸업 요건 사전 점검을 신청하세요.', '학사'],
  ['소프트웨어융합대학 현장실습 참여 학생 모집', '취업', '현장 실습 참여 학생을 모집합니다.', '인턴·채용'],
  ['2026학년도 겨울 계절학기 수강신청 일정', '학사', '계절학기 수강신청 일정을 안내합니다.', '수업'],
  ['교내 AI 해커톤 참가팀 모집', '행사', '해커톤 참가팀을 모집합니다. 장학 혜택이 있습니다.', '공모전·대회'],
  ['학생증 재발급 및 모바일 학생증 이용 안내', '일반', '학생증 재발급 절차를 안내합니다.', '생활·시설'],
];
const day = (offset: number) => new Date(Date.now() + offset * 86_400_000).toISOString().slice(0, 10);
const kst = (days: number, hour = 23) => new Date(Date.now() + days * 86_400_000 + 9 * 3_600_000).toISOString().slice(0, 11) + String(hour).padStart(2, '0') + ':59';
const task = (overrides: Partial<CybercampusTask>) => cybercampusTask({ course_name: '자료구조', ...overrides });
const tasks = [
  task({ item_id: '1', title: '연결 리스트 구현 과제', due_at: kst(1), status: '미제출' }),
  task({ item_id: '2', course_key: 'B2', course_name: '운영체제', title: '프로세스 스케줄링 보고서', due_at: kst(3, 18), status: '미제출' }),
  task({ item_id: '3', course_key: 'C3', course_name: '선형대수학', category: 'quiz', label: '퀴즈', title: '5주차 온라인 퀴즈', due_at: kst(5, 12), pending: true }),
  task({ item_id: '4', course_key: 'D4', course_name: '데이터베이스', title: 'ER 다이어그램 설계', due_at: kst(8), status: '미제출' }),
  task({ item_id: '5', course_key: 'A1', title: '이진 트리 실습', due_at: kst(-2), status: '제출 완료', submitted_at: kst(-3, 14) }),
];
// The sidebar shows one row per settings group, so the boards sit in the catalog's groups.
const groups = [
  { id: 'sogang', name: '서강대학교', parent: null },
  { id: 'sogang-eng', name: '공과대학', parent: 'sogang' },
  { id: 'sogang-computing', name: '소프트웨어융합대학', parent: 'sogang' },
];
const board = (id: string, name: string, group: string, family: string, organization: string) => ({ id, name, organization, supported: true, subscribed: true, group, family });
const boards = [
  board('sogang-academic', '학부 학사공지', 'sogang', 'main', '서강대학교'),
  board('eng-general', '일반공지', 'sogang-eng', 'engineering', '서강대학교 · 공과대학'),
  ...([['computing-academic', '학사 공지'], ['computing-graduate', '대학원 공지'], ['computing-updates', '소식'], ['computing-career', '취업·인턴십']] as const)
    .map(([id, name]) => board(id, name, 'sogang-computing', 'computing', '서강대학교 · 소프트웨어융합대학')),
];
const cyber = { id: 'cybercampus', name: '강의 공지', organization: '서강대학교 · 사이버캠퍼스', supported: true, subscribed: true, family: 'cybercampus' };
// One board per notice in `titles`.
const noticeSources = ['sogang-academic', 'sogang-academic', 'computing-career', 'sogang-academic', 'computing-updates', 'eng-general'];

async function open(page: Page, options: FixtureOptions = {}) {
  await mockDesktop(page, { count: titles.length, sourceList: [...boards, cyber], sourceGroups: groups, noticeSources, aiEmbeddingInstalled: true, aiSemanticSearch: true,
    initialInterests: [{ id: 1, name: '장학금', aliases: ['장학'], priority: 3, polarity: 'positive', origin: 'explicit', evidence_count: 0, evidence_summary: null, updated_at: '2026-10-27' }],
    cybercampusId: '20990001', cybercampusTasks: tasks, ...options });
  // Runs after the fixture's own init script and edits its entries in place.
  await page.addInitScript(({ titles, dates }) => {
    const fixture = (window as any).__fixture;
    fixture.entries.forEach((entry: any, index: number) => {
      const [title, category, snippet, label] = titles[index];
      Object.assign(entry, { title, category, snippet, published_at: dates[index], collected_at: `${dates[index]} 09:00:00`, historical: false, is_read: index > 3, saved: index === 0 });
      fixture.classifications[index].category = label;
    });
  }, { titles, dates: titles.map((_, index) => day(-index)) });
  await page.goto('/');
  await page.evaluate(() => document.fonts.ready);
}
const nav = (page: Page, name: string) => page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name, exact: true }).click();
async function shot(page: Page, name: string) {
  await page.waitForTimeout(600);
  await page.screenshot({ path: `${OUT}/${name}.png` });
}

test('today', async ({ page }) => {
  await open(page);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await shot(page, 'today');
});
test('inbox', async ({ page }) => {
  await open(page);
  await nav(page, '받은 소식');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('받은 소식');
  await expect(page.locator('#notice-1')).toBeVisible();
  await shot(page, 'inbox');
});
test('explore search', async ({ page }) => {
  await open(page);
  await nav(page, '탐색');
  await page.getByRole('searchbox').first().fill('장학');
  await expect(page.locator('#notice-1')).toBeVisible();
  await shot(page, 'explore');
});
test('reader with notice dates', async ({ page }) => {
  await open(page, { noticeDates: [{ document_id: 1, ordinal: 0, title: titles[0][0], day: day(5), time: '18:00', label: '신청 마감', kind: 'deadline' }] });
  await nav(page, '받은 소식');
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-header h2')).toHaveText(titles[0][0]);
  await expect(page.getByRole('region', { name: /공지 속 날짜/ })).toContainText('신청 마감');
  await shot(page, 'reader');
});
test('deadlines', async ({ page }) => {
  await open(page);
  await nav(page, '과제·마감');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('과제·마감');
  await expect(page.getByText('연결 리스트 구현 과제').first()).toBeVisible();
  await shot(page, 'deadlines');
});
test('settings AI', async ({ page }) => {
  await open(page);
  await openSettings(page, 'AI');
  await expect(page.getByRole('group', { name: '공지 분류' })).toContainText('Qwen3-Embedding 0.6B');
  await shot(page, 'settings-ai');
});
