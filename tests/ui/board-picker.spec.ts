import { test, expect, type Locator, type Page } from '@playwright/test';
import { mockDesktop, callCount, openSettings } from './fixture';

const board = (id: string, name: string, group: string, subscribed = false) => ({ id, name, organization: '서강대학교', supported: true, subscribed, group });
const groups = [
  { id: 'sogang', name: '서강대학교', parent: null },
  { id: 'sogang-colleges', name: '단과대학', parent: 'sogang' },
  { id: 'sogang-liberalarts', name: '인문대학', parent: 'sogang-colleges' },
  { id: 'sogang-history', name: '사학과', parent: 'sogang-liberalarts' },
  { id: 'sogang-offices', name: '행정·지원', parent: 'sogang' },
  { id: 'sogang-dorm', name: '벨라르미노 학사', parent: 'sogang-offices' },
];
const sourceList = [
  board('sogang-academic', '학부 학사공지', 'sogang', true),
  board('liberalarts-notice', '공지사항', 'sogang-liberalarts'),
  board('liberalarts-event', '행사', 'sogang-liberalarts'),
  board('history-notice', '공지사항', 'sogang-history'),
  board('dorm-menu', '주간 메뉴', 'sogang-dorm', true),
];

async function open(page: Page, sourceGroups = groups) {
  await mockDesktop(page, { sourceGroups, sourceList });
  await page.goto('/');
  await openSettings(page, '게시판');
}
const chips = (page: Page) => page.getByRole('list', { name: '서강대학교 게시판 고르기' });
const category = (page: Page, name: string) => chips(page).getByRole('button', { name: new RegExp(`^${name},`) });
const pick = (page: Page, name: string) => page.getByRole('button', { name: `${name} 구독`, exact: true });
// The chip sequence as visible labels, in DOM order.
const order = (page: Page) => chips(page).locator('.chip').evaluateAll((nodes) => nodes.map((node) => node.querySelector('span')?.textContent ?? ''));

test('a category is open exactly while its group is on, and picked boards start in view', async ({ page }) => {
  await open(page);
  // 행정·지원 holds a picked board, so it starts open down to that board; 단과대학 has none.
  await expect(category(page, '단과대학')).toHaveAttribute('aria-expanded', 'false');
  await expect(category(page, '행정·지원')).toHaveAttribute('aria-expanded', 'true');
  await expect(category(page, '인문대학')).toHaveCount(0);
  await expect(category(page, '행정·지원')).toHaveAccessibleName('행정·지원, 구독 1/1');
  expect(await order(page)).toEqual(['단과대학', '행정·지원', '벨라르미노 학사', '주간 메뉴', '학부 학사공지']);
  await expect(chips(page)).not.toContainText('꺼짐');
  await expect(chips(page)).not.toContainText('그룹');

  // Opening a group that is already on needs no save; its children follow it in place.
  await category(page, '단과대학').click();
  await expect(category(page, '단과대학')).toHaveAttribute('aria-expanded', 'true');
  expect(await order(page)).toEqual(['단과대학', '인문대학', '행정·지원', '벨라르미노 학사', '주간 메뉴', '학부 학사공지']);
  await category(page, '인문대학').click();
  expect(await order(page)).toEqual(['단과대학', '인문대학', '사학과', '공지사항', '행사', '행정·지원', '벨라르미노 학사', '주간 메뉴', '학부 학사공지']);
  expect(await callCount(page, 'archive_set_group')).toBe(0);

  // Closing a category turns its group off and takes its subtree with it; reopening turns it on.
  await category(page, '단과대학').click();
  await expect.poll(() => order(page)).toEqual(['단과대학', '행정·지원', '벨라르미노 학사', '주간 메뉴', '학부 학사공지']);
  await category(page, '단과대학').click();
  await expect(category(page, '사학과')).toBeVisible();
  const calls = await page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'archive_set_group').map((call: any) => call.args));
  expect(calls).toEqual([{ id: 'sogang-colleges', enabled: false }, { id: 'sogang-colleges', enabled: true }]);
  expect(await callCount(page, 'archive_subscribe')).toBe(0);
});

test('a board chip toggles its subscription and reflects the stored result', async ({ page }) => {
  await open(page);
  await category(page, '단과대학').click();
  await category(page, '인문대학').click();
  const notice = pick(page, '인문대학 공지사항');
  await expect(notice).toHaveAttribute('aria-pressed', 'false');
  await notice.click();
  await expect(notice).toHaveAttribute('aria-pressed', 'true');
  await expect(category(page, '인문대학')).toHaveAccessibleName('인문대학, 구독 1/3');
  await expect(category(page, '단과대학')).toHaveAccessibleName('단과대학, 구독 1/3');
  await expect(page.locator('.source-manager .archive-feedback')).toContainText('이 게시판 공지를 추천과 알림에 포함합니다');
  const calls = await page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'archive_subscribe').map((call: any) => call.args));
  expect(calls).toEqual([{ id: 'liberalarts-notice', subscribed: true }]);
  await notice.click();
  await expect(notice).toHaveAttribute('aria-pressed', 'false');
});

test('a failed subscribe shows the error and leaves the chip as it was', async ({ page }) => {
  await open(page);
  const menu = pick(page, '벨라르미노 학사 주간 메뉴');
  await expect(menu).toHaveAttribute('aria-pressed', 'true');
  await page.evaluate(() => { (window as any).__fixture.subscribeError = true; });
  await menu.click();
  await expect(page.getByRole('alert')).toContainText('구독 설정을 저장하지 못했습니다');
  await expect(menu).toHaveAttribute('aria-pressed', 'true');
  await expect(category(page, '벨라르미노 학사')).toHaveAccessibleName('벨라르미노 학사, 구독 1/1');
});

test('chips work from the keyboard and keep focus on the pressed chip', async ({ page, browserName }) => {
  await open(page);
  // WebKit's Tab skips buttons unless the OS setting says otherwise, so only Chromium walks the tab order.
  const step = async (chip: Locator) => { if (browserName === 'chromium') await page.keyboard.press('Tab'); else await chip.focus(); };
  await category(page, '단과대학').focus();
  await page.keyboard.press('Enter');
  await expect(category(page, '단과대학')).toBeFocused();
  await expect(category(page, '단과대학')).toHaveAttribute('aria-expanded', 'true');
  // The inserted chip is the next stop in tab order.
  await step(category(page, '인문대학'));
  await expect(category(page, '인문대학')).toBeFocused();
  await page.keyboard.press('Space');
  await expect(category(page, '인문대학')).toHaveAttribute('aria-expanded', 'true');
  await expect(category(page, '인문대학')).toBeFocused();
  await step(category(page, '사학과'));
  await expect(category(page, '사학과')).toBeFocused();
  await page.keyboard.press('Space');
  await step(pick(page, '사학과 공지사항'));
  await expect(pick(page, '사학과 공지사항')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(pick(page, '사학과 공지사항')).toHaveAttribute('aria-pressed', 'true');
  await expect(pick(page, '사학과 공지사항')).toBeFocused();
});

test('a group that is off shows closed and opening it turns it on; the top group switch hides everything', async ({ page }) => {
  await open(page, groups.map((group) => group.id === 'sogang-offices' ? { ...group, enabled: false } : group));
  // Its picked board is kept but not subscribed, and nothing below it is shown.
  await expect(category(page, '행정·지원')).toHaveAttribute('aria-expanded', 'false');
  await expect(category(page, '벨라르미노 학사')).toHaveCount(0);
  await category(page, '행정·지원').click();
  await expect(category(page, '행정·지원')).toHaveAttribute('aria-expanded', 'true');
  await expect(category(page, '행정·지원')).toHaveAccessibleName('행정·지원, 구독 1/1');
  await expect(pick(page, '벨라르미노 학사 주간 메뉴')).toHaveAttribute('aria-pressed', 'true');
  await category(page, '행정·지원').click();
  await expect(category(page, '행정·지원')).toHaveAttribute('aria-expanded', 'false');
  await expect(category(page, '행정·지원')).toHaveAccessibleName('행정·지원, 구독 0/1');
  expect(await callCount(page, 'archive_set_group')).toBe(2);

  await page.getByRole('switch', { name: '서강대학교 전체', exact: true }).click();
  await expect(chips(page)).toHaveCount(0);
  await expect(page.getByRole('group', { name: '서강대학교', exact: true })).toContainText('꺼짐 · 게시판 5개');
});
