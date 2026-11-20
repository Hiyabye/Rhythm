import { expect, test } from '@playwright/test';
import { callCount, mockDesktop, navigate, openSettings } from './fixture';

// The notice feed's status (#177). Only the IPC boundary is mocked: these
// tests do not fetch, verify or apply a feed.
const failing = { source_id: 'sw', name: '소프트웨어융합대학 소식', last_success_at: '2026-11-17T09:00:00.000Z', consecutive_failures: 4 };

test('the board settings show the feed age and failing boards and sync on request', async ({ page }) => {
  await mockDesktop(page, {
    feed: { failing_boards: [failing] },
    feedSyncDelay: 400,
    feedSyncResult: { generated_at: '2026-11-18T04:00:00.000Z', failing_boards: [] },
  });
  await page.goto('/');
  await openSettings(page, '게시판');
  const feed = page.getByRole('region', { name: '공지 피드', exact: true });
  await expect(feed.getByRole('status')).toContainText('2026년 11월 18일');
  await expect(feed.getByRole('list', { name: '서버가 가져오지 못하는 게시판' })).toContainText('소프트웨어융합대학 소식연속 4회 실패');
  await expect(feed.getByRole('alert')).toHaveCount(0);

  const sync = feed.getByRole('button', { name: '지금 받기', exact: true });
  await sync.click();
  await expect(sync).toBeDisabled();
  await expect(feed.getByRole('status')).toHaveText('받는 중');
  await expect(sync).toBeEnabled();
  await expect(feed.getByRole('list', { name: '서버가 가져오지 못하는 게시판' })).toHaveCount(0);
  expect(await callCount(page, 'feed_sync')).toBe(1);
});

test('an offline feed stays quiet in settings and explains itself in the list', async ({ page }) => {
  await mockDesktop(page, { feed: { result: 'offline', error: '공지 피드에 연결하지 못했습니다.', stale: true } });
  await page.goto('/');
  await navigate(page, '받은 소식');
  const banner = page.locator('.banner').filter({ hasText: '기준 공지입니다' });
  await expect(banner).toContainText('인터넷에 연결되지 않아');
  await expect(banner).toHaveAttribute('role', 'status');
  await banner.getByRole('button', { name: '상태 보기', exact: true }).click();
  const feed = page.getByRole('region', { name: '공지 피드', exact: true });
  await expect(feed).toContainText('연결되면 자동으로 다시 받습니다');
  await expect(feed.getByRole('alert')).toHaveCount(0);
});

test('a stale feed and a newer feed format are reported above the list', async ({ page }) => {
  await mockDesktop(page, { feed: { stale: true } });
  await page.goto('/');
  await navigate(page, '받은 소식');
  await expect(page.locator('.banner').filter({ hasText: '이후 갱신되지 않았습니다' })).toContainText('3시간 넘게');

  await mockDesktop(page, { feed: { result: 'update_required', error: '공지 피드 형식이 바뀌었습니다. 앱을 업데이트하세요.' } });
  await page.goto('/');
  await navigate(page, '받은 소식');
  await expect(page.locator('.banner.error')).toContainText('공지 피드를 받으려면 앱을 업데이트하세요');
  await openSettings(page, '게시판');
  await expect(page.getByRole('region', { name: '공지 피드', exact: true }).getByRole('alert')).toContainText('앱을 업데이트하세요');
});

test('without the notice extension the feed is not polled', async ({ page }) => {
  await mockDesktop(page, { enabled: false, feed: { stale: true } });
  await page.goto('/');
  // Longer than the list's two-second status poll.
  await page.waitForTimeout(2500);
  expect(await callCount(page, 'feed_status')).toBe(0);
  await expect(page.locator('.banner').filter({ hasText: '공지 피드' })).toHaveCount(0);
});

test('the sidebar dates public boards by the last feed check, not their old crawl', async ({ page }) => {
  const sourceList = [{ id: 'sogang-academic', name: '학사공지', organization: '서강대학교', supported: true, subscribed: true, last_success: '2026-10-01T00:00:00.000Z' }];
  await mockDesktop(page, { sourceList });
  await page.goto('/');
  await expect(page.locator('.sync-line')).toHaveText('마지막 확인 11월 18일');

  // A feed never checked leaves nothing to show, whatever the v1 crawl recorded.
  await mockDesktop(page, { sourceList, feed: { checked_at: null } });
  await page.goto('/');
  await expect(page.locator('.sync-line')).toHaveText('아직 가져온 적 없음');
});
