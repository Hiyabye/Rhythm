import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import type { Locator, Page } from '@playwright/test';
import { mockDesktop, openSettings } from './fixture';
import { interestSuggestions, type InterestSuggestion } from '../../src/lib/interest-suggestions';

interface FixtureCall { command: string; args: Record<string, any> }
type FixtureWindow = Window & { __fixture: { calls: FixtureCall[] } };
async function calls(page: Page, command: string): Promise<FixtureCall[]> {
  return (await page.evaluate(() => (window as unknown as FixtureWindow).__fixture.calls)).filter((call) => call.command === command);
}
const interest = (id: number, name: string, polarity: 'positive' | 'negative' = 'positive', origin: 'explicit' | 'learned' = 'explicit') =>
  ({ id, name, aliases: [] as string[], priority: 3, polarity, origin, evidence_count: 0, evidence_summary: null, updated_at: '2026-10-27' });
const picker = (page: Page) => page.getByRole('list', { name: '추천 관심 주제', exact: true });
const chip = (page: Page, name: string) => picker(page).getByRole('button', { name, exact: true });
const labels = (page: Page) => picker(page).getByRole('button').allTextContents();

test('the built-in suggestions are unique, short and avoid the backend sensitive-term markers', () => {
  const all: string[] = [];
  const walk = (nodes: InterestSuggestion[]) => nodes.forEach((node) => { all.push(node.label); walk(node.children ?? []); });
  walk(interestSuggestions);
  expect(new Set(all).size).toBe(all.length);
  const topics = readFileSync(new URL('../../crates/core/src/storage/personalization/topics.rs', import.meta.url), 'utf8');
  const block = topics.match(/SENSITIVE_TOPIC_MARKERS[^=]*=\s*&\[([\s\S]*?)\];/)![1];
  const markers = [...block.matchAll(/"([^"]+)"/g)].map((match) => match[1].toLowerCase());
  expect(markers.length).toBeGreaterThan(20);
  for (const label of all) {
    expect(Array.from(label).length).toBeLessThanOrEqual(80);
    for (const marker of markers) expect(label.toLowerCase(), `${label} contains ${marker}`).not.toContain(marker);
  }
});

test('categories start unselected; choosing one saves it and opens related chips right after it', async ({ page }) => {
  await mockDesktop(page); await page.goto('/'); await openSettings(page, '개인화');
  const roots = interestSuggestions.map((node) => node.label);
  expect(await labels(page)).toEqual(roots);
  for (const root of roots) await expect(chip(page, root)).toHaveAttribute('aria-pressed', 'false');
  await expect(chip(page, '인턴십')).toHaveAttribute('aria-expanded', 'false');

  await chip(page, '인턴십').click();
  await expect(chip(page, '인턴십')).toHaveAttribute('aria-pressed', 'true');
  await expect(chip(page, '인턴십')).toHaveAttribute('aria-expanded', 'true');
  const expected = [...roots];
  expected.splice(roots.indexOf('인턴십') + 1, 0, '체험형 인턴', '기업 인턴', '해외 인턴');
  await expect.poll(() => labels(page)).toEqual(expected);
  await expect(chip(page, '체험형 인턴')).toHaveAttribute('aria-pressed', 'false');
  expect((await calls(page, 'personalization_save_interest')).map((call) => call.args.interest)).toEqual([
    { id: null, name: '인턴십', aliases: [], priority: 3, polarity: 'positive' },
  ]);
  await expect(page.locator('#personalization').getByRole('list', { name: '관심', exact: true })).toContainText('인턴십');

  // A related chip opens its own level when it has children.
  await chip(page, '연구').click();
  await chip(page, '인공지능').click();
  await expect(chip(page, '머신러닝')).toBeVisible();
  const order = await labels(page);
  expect(order.indexOf('인공지능')).toBe(order.indexOf('학부연구생') + 2);
  expect(order.slice(order.indexOf('인공지능') + 1, order.indexOf('인공지능') + 4)).toEqual(['머신러닝', '데이터 분석', 'AI 인프라']);
});

test('interests the user already has show as selected; learned and not-interested ones do not', async ({ page }) => {
  await mockDesktop(page, { initialInterests: [interest(1, '장학금'), interest(2, '공모전', 'negative'), interest(3, '봉사활동', 'positive', 'learned')] });
  await page.goto('/'); await openSettings(page, '개인화');
  await expect(chip(page, '장학금')).toHaveAttribute('aria-pressed', 'true');
  await expect(chip(page, '교내 장학금')).toBeVisible();
  await expect(chip(page, '공모전')).toHaveAttribute('aria-pressed', 'false');
  await expect(chip(page, '공모전')).toHaveAttribute('aria-disabled', 'true');
  await expect(chip(page, '봉사활동')).toHaveAttribute('aria-pressed', 'false');
  await expect(chip(page, '해커톤')).toHaveCount(0);
  // A not-interested topic is never flipped by a click.
  await chip(page, '공모전').click({ force: true });
  expect(await calls(page, 'personalization_save_interest')).toHaveLength(0);
});

test('choosing a chosen chip again removes the interest and folds its related chips', async ({ page }) => {
  await mockDesktop(page, { initialInterests: [interest(7, '창업')] });
  await page.goto('/'); await openSettings(page, '개인화');
  await expect(chip(page, '창업 동아리')).toBeVisible();
  await chip(page, '창업').click();
  await expect(chip(page, '창업')).toHaveAttribute('aria-pressed', 'false');
  await expect(chip(page, '창업 동아리')).toHaveCount(0);
  expect((await calls(page, 'personalization_remove_interest')).map((call) => call.args)).toEqual([{ id: 7 }]);
});

test('a rejected save shows the error and leaves the chip unselected', async ({ page }) => {
  await mockDesktop(page);
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__;
    const invoke = internals.invoke;
    internals.invoke = (command: string, args: any) => command === 'personalization_save_interest' && args.interest.name === '학사'
      ? Promise.reject('직접 설정할 수 있는 관심 분야는 최대 50개입니다.')
      : invoke(command, args);
  });
  await page.goto('/'); await openSettings(page, '개인화');
  await chip(page, '학사').click();
  await expect(page.locator('#personalization').getByRole('alert')).toContainText('최대 50개');
  await expect(chip(page, '학사')).toHaveAttribute('aria-pressed', 'false');
  await expect(chip(page, '수강신청')).toHaveCount(0);
  await chip(page, '행사').click();
  await expect(chip(page, '행사')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('#personalization').getByRole('alert')).toHaveCount(0);
});

test('chips work from the keyboard and focus stays on the pressed chip', async ({ page, browserName }) => {
  await mockDesktop(page); await page.goto('/'); await openSettings(page, '개인화');
  const first = chip(page, '장학금');
  await first.focus();
  await page.keyboard.press('Enter');
  await expect(first).toHaveAttribute('aria-pressed', 'true');
  await expect(first).toBeFocused();
  const child = chip(page, '교내 장학금');
  // WebKit's Tab skips buttons unless the OS option is on, so only Chromium checks the tab order.
  if (browserName === 'chromium') await page.keyboard.press('Tab'); else await child.focus();
  await expect(child).toBeFocused();
  await page.keyboard.press('Space');
  await expect(child).toHaveAttribute('aria-pressed', 'true');
  await expect(child).toBeFocused();
  await first.focus();
  await page.keyboard.press('Space');
  await expect(first).toHaveAttribute('aria-pressed', 'false');
  await expect(first).toBeFocused();
  // The folded parent keeps its chosen child visible so it can still be unselected.
  await expect(child).toHaveAttribute('aria-pressed', 'true');
});

test('the setup flow shows the picker in its compact interests step', async ({ page }) => {
  await mockDesktop(page, { onboardingNeeded: true, extensions: { sogang_notices: false, cybercampus: false, search: false, saved: false, recommendations: true, week: false, calendar: false, ai: false, proactive: false } });
  await page.goto('/');
  await page.getByRole('button', { name: '다음', exact: true }).click();
  await page.getByRole('button', { name: '다음', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1, name: '관심사', exact: true })).toBeFocused();
  const list: Locator = picker(page);
  await expect(list).toBeVisible();
  await chip(page, '교환학생').click();
  await expect(chip(page, '방문학생')).toBeVisible();
  await expect(page.getByRole('textbox', { name: '관심 주제 추가' })).toBeVisible();
});

test('picking a chip keeps its width, so picking alone never reflows the row', async ({ page }) => {
  // Reduced motion zeroes the entry scale, so sizes are measured at rest.
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await mockDesktop(page); await page.goto('/'); await openSettings(page, '개인화');
  const target = chip(page, '창업');
  const width = (await target.boundingBox())!.width;
  const weight = () => target.evaluate((node) => getComputedStyle(node).fontWeight);
  const before = await weight();
  await target.click();
  await expect(target).toHaveAttribute('aria-pressed', 'true');
  expect((await target.boundingBox())!.width).toBe(width);
  // Test browsers may fake bold without widening glyphs, so the weight is checked directly.
  expect(await weight()).toBe(before);
  // Unpicking removes its children at once, with no fade that holds their space.
  await expect(chip(page, '창업 동아리')).toBeVisible();
  await target.click();
  await expect(chip(page, '창업 동아리')).toHaveCount(0);
  expect((await target.boundingBox())!.width).toBe(width);
});
