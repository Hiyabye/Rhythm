import { test, expect, type Page } from '@playwright/test';
import { mockDesktop, answerDialogs, dialogMessages, openSettings } from './fixture';

async function navigate(page: Page, name: string) {
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name, exact: true }).click();
}
const list = (page: Page) => page.getByRole('list', { name: '공지 목록' });
const trashCalls = (page: Page) => page.evaluate(() =>
  (window as Window & { __fixture: { trashCalls: { command: string; ids: number[] | null }[] } }).__fixture.trashCalls);

test('a notice moved to 휴지통 leaves the inbox and comes back when restored', async ({ page }) => {
  await mockDesktop(page, { count: 3 });
  await page.goto('/');
  await navigate(page, '받은 소식');
  await page.getByRole('button', { name: '한국어 공지 02 휴지통으로 이동' }).click();
  await expect(page.locator('.archive-feedback')).toContainText('1개 공지를 휴지통으로 옮겼습니다');
  await expect(list(page).locator('#notice-2')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '휴지통', exact: true })).toHaveAccessibleDescription('휴지통의 공지 1개');

  await navigate(page, '휴지통');
  await expect(page.getByRole('heading', { name: '휴지통 1' })).toBeVisible();
  await expect(list(page).locator('li')).toHaveCount(1);
  await page.getByRole('button', { name: '한국어 공지 02 복원' }).click();
  await expect(page.getByRole('heading', { name: '휴지통이 비어 있습니다' })).toBeVisible();
  await navigate(page, '받은 소식');
  await expect(list(page).locator('#notice-2')).toBeVisible();
  expect(await trashCalls(page)).toEqual([{ command: 'archive_trash', ids: [2] }, { command: 'archive_restore', ids: [2] }]);
});

test('bulk selection moves several notices and emptying 휴지통 asks first', async ({ page }) => {
  await mockDesktop(page, { count: 3 });
  await page.goto('/');
  await navigate(page, '받은 소식');
  await page.getByRole('button', { name: '선택', exact: true }).click();
  const move = page.getByRole('group', { name: '선택한 공지 작업' }).getByRole('button', { name: '휴지통으로 이동' });
  await expect(move).toBeDisabled();
  await page.getByRole('checkbox', { name: '한국어 공지 01 선택' }).check();
  await page.getByRole('checkbox', { name: '한국어 공지 03 선택' }).check();
  await expect(page.locator('.bulk-count')).toHaveText('2개 선택');
  await move.click();
  await expect(list(page).locator('li')).toHaveCount(1);
  // Selection mode ends once the action is done.
  await expect(page.getByRole('group', { name: '선택한 공지 작업' })).toHaveCount(0);

  await navigate(page, '휴지통');
  await expect(list(page).locator('li')).toHaveCount(2);
  const empty = page.getByRole('button', { name: '휴지통 비우기' });
  await answerDialogs(page, 'cancel');
  await empty.click();
  await expect(list(page).locator('li')).toHaveCount(2);
  await answerDialogs(page, 'ok');
  await empty.click();
  await expect(page.getByRole('heading', { name: '휴지통이 비어 있습니다' })).toBeVisible();
  expect((await dialogMessages(page)).at(-1)).toContain('휴지통의 공지 2개를 영구 삭제할까요?');
  expect((await trashCalls(page)).at(-1)).toEqual({ command: 'archive_purge', ids: null });
  await expect(page.getByRole('button', { name: '휴지통', exact: true })).not.toHaveAccessibleDescription(/휴지통의 공지/);
});

test('the reader moves a notice to 휴지통, then deletes it permanently after confirmation', async ({ page }) => {
  await mockDesktop(page, { count: 2 });
  await page.goto('/');
  await navigate(page, '받은 소식');
  await page.locator('#notice-1').click();
  await page.getByRole('button', { name: '휴지통으로 이동' }).click();
  const note = page.getByRole('note');
  await expect(note).toContainText('휴지통에 있는 공지입니다');
  await expect(page.getByRole('button', { name: '휴지통으로 이동' })).toHaveCount(0);

  await answerDialogs(page, 'ok');
  await note.getByRole('button', { name: '영구 삭제' }).click();
  await expect(page.locator('.archive-reader')).toHaveCount(0);
  await expect(list(page).locator('li')).toHaveCount(1);
  expect((await dialogMessages(page)).at(-1)).toContain('다시 가져오지 않습니다');
  expect((await trashCalls(page)).at(-1)).toEqual({ command: 'archive_purge', ids: [1] });
});

test('the old-notice period is saved with the other settings and survives reopening the pane', async ({ page }) => {
  await mockDesktop(page, { count: 1 });
  await page.goto('/');
  await openSettings(page, '데이터 및 정보');
  const period = page.getByRole('combobox', { name: '오래된 공지 정리' });
  await expect(period).toHaveValue('0');
  await period.selectOption({ label: '6개월' });
  await expect(page.getByText('설정을 저장했습니다.')).toBeVisible();
  const saved = await page.evaluate(() => (window as Window & { __fixture: { calls: { command: string; args: { settings: Record<string, unknown> } }[] } })
    .__fixture.calls.filter((call) => call.command === 'save_settings').map((call) => call.args.settings));
  expect(saved).toEqual([{ theme: 'light', energy_profile: 'eco', interests: '', clock_format: '12h', auto_trash_months: 6 }]);
  await openSettings(page, '일반');
  await openSettings(page, '데이터 및 정보');
  await expect(page.getByRole('combobox', { name: '오래된 공지 정리' })).toHaveValue('6');
});
