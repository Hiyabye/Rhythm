import { test, expect, type Page } from '@playwright/test';
import { mockDesktop, openSettings, type FixtureOptions } from './fixture';
import type { CybercampusTask } from '../../src/lib/types';

// Regenerates the README screenshots in docs/screenshots/ from the mocked IPC
// fixture (synthetic data only). Run with `npm run screenshots`; skipped otherwise.
test.skip(process.env.RHYTHM_SCREENSHOTS !== '1', 'set RHYTHM_SCREENSHOTS=1 (npm run screenshots) to regenerate README screenshots');
test.skip(({ browserName }) => browserName !== 'chromium', 'screenshots use Chromium only');
test.use({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1, colorScheme: 'light', reducedMotion: 'reduce' });

const OUT = 'docs/screenshots';
const titles: [string, string, string][] = [
  ['2026학년도 2학기 교내 장학금 신청 안내', '장학', '장학금 신청 기간과 제출 서류를 안내합니다. 장학 신청서를 제출하세요.'],
  ['졸업 요건 사전 점검 신청 안내', '학사', '졸업 예정자는 졸업 요건 사전 점검을 신청하세요.'],
  ['소프트웨어융합대학 현장실습 참여 학생 모집', '취업', '현장 실습 참여 학생을 모집합니다.'],
  ['2026학년도 겨울 계절학기 수강신청 일정', '학사', '계절학기 수강신청 일정을 안내합니다.'],
  ['교내 AI 해커톤 참가팀 모집', '행사', '해커톤 참가팀을 모집합니다. 장학 혜택이 있습니다.'],
  ['학생증 재발급 및 모바일 학생증 이용 안내', '일반', '학생증 재발급 절차를 안내합니다.'],
];
const day = (offset: number) => new Date(Date.now() + offset * 86_400_000).toISOString().slice(0, 10);
const kst = (days: number, hour = 23) => new Date(Date.now() + days * 86_400_000 + 9 * 3_600_000).toISOString().slice(0, 11) + String(hour).padStart(2, '0') + ':59';
function task(overrides: Partial<CybercampusTask>): CybercampusTask {
  return { course_key: 'A1', course_name: '자료구조', category: 'report', item_id: '1', label: '과제', title: '실습 과제', week: 9,
    opens_at: null, due_at: null, late_due_at: null, status: null, submitted_at: null, pending: false, updated_at: '2026-11-06T00:00:00Z', ...overrides };
}
const tasks = [
  task({ item_id: '1', title: '연결 리스트 구현 과제', due_at: kst(1), status: '미제출' }),
  task({ item_id: '2', course_key: 'B2', course_name: '운영체제', title: '프로세스 스케줄링 보고서', due_at: kst(3, 18), status: '미제출' }),
  task({ item_id: '3', course_key: 'C3', course_name: '선형대수학', category: 'quiz', label: '퀴즈', title: '5주차 온라인 퀴즈', due_at: kst(5, 12), pending: true }),
  task({ item_id: '4', course_key: 'D4', course_name: '데이터베이스', title: 'ER 다이어그램 설계', due_at: kst(8), status: '미제출' }),
  task({ item_id: '5', course_key: 'A1', title: '이진 트리 실습', due_at: kst(-2), status: '제출 완료', submitted_at: kst(-3, 14) }),
];
const board = { id: 'computing-updates', name: '소식', organization: '소프트웨어융합대학', supported: true, subscribed: true };
const cyber = { id: 'cybercampus', name: '강의 공지', organization: '서강대학교 · 사이버캠퍼스', supported: true, subscribed: true, family: 'cybercampus' };

async function open(page: Page, options: FixtureOptions = {}) {
  await mockDesktop(page, { count: titles.length, sourceList: [board, cyber], aiInstalled: true, aiSemanticSearch: true, aiResult: true,
    initialInterests: [{ id: 1, name: '장학금', aliases: ['장학'], priority: 3, polarity: 'positive', origin: 'explicit', evidence_count: 0, evidence_summary: null, updated_at: '2026-10-27' }],
    cybercampusId: '20990001', cybercampusTasks: tasks, ...options });
  // Runs after the fixture's own init script and edits its entries in place.
  await page.addInitScript(({ titles, dates }) => {
    const fixture = (window as any).__fixture;
    fixture.entries.forEach((entry: any, index: number) => {
      const [title, category, snippet] = titles[index];
      Object.assign(entry, { title, category, snippet, published_at: dates[index], collected_at: `${dates[index]} 09:00:00`, historical: false, is_read: index > 3, saved: index === 0 });
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
  await open(page, { aiEmbeddingInstalled: true });
  await nav(page, '탐색');
  await page.getByRole('searchbox').first().fill('장학');
  await expect(page.locator('#notice-1')).toBeVisible();
  await shot(page, 'explore');
});
test('reader with AI analysis', async ({ page }) => {
  await open(page);
  // The canned analysis names a 30 October deadline, so publish the notice before it.
  await page.evaluate(() => { (window as any).__fixture.entries[0].published_at = '2026-10-24'; });
  await nav(page, '받은 소식');
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-header h2')).toHaveText(titles[0][0]);
  await expect(page.getByRole('region', { name: '공지 AI 분석' }).getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
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
  await expect(page.getByText('Qwen3.5 4B').first()).toBeVisible();
  await shot(page, 'settings-ai');
});
