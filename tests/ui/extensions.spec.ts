import { test, expect } from '@playwright/test';
import { mockDesktop, openSettings } from './fixture';

const none = { sogang_notices: false, cybercampus: false, search: false, saved: false, recommendations: false, week: false, ai: false, proactive: false };

test('a blank profile shows only Today and the core settings', async ({ page }) => {
  await mockDesktop(page, { extensions: none });
  await page.goto('/');
  const sidebar = page.getByRole('navigation', { name: '기본 탐색', exact: true });
  await expect(sidebar.locator('.source-list').first().getByRole('button')).toHaveText(['오늘']);
  await expect(sidebar.locator('.source-group-heading')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: '오늘을 채울 확장 기능을 켜 보세요', exact: true })).toBeVisible();
  await expect(page.getByRole('region', { name: '오늘의 추천' })).toHaveCount(0);
  await expect(page.getByRole('complementary', { name: '앞으로 7일', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: '확장 기능 열기', exact: true }).click();
  await expect(page.getByRole('heading', { level: 2, name: '확장 기능', exact: true })).toBeFocused();
  const panes = page.getByRole('navigation', { name: '설정 항목', exact: true }).getByRole('button');
  await expect(panes).toHaveText(['확장 기능', '일반', '데이터 및 정보']);
  await expect(page.getByRole('switch', { checked: true })).toHaveCount(0);
});

test('turning an extension on adds its page, widget and settings, and off removes them', async ({ page }) => {
  await mockDesktop(page, { extensions: none });
  await page.goto('/');
  const sidebar = page.getByRole('navigation', { name: '기본 탐색', exact: true });
  const panes = page.getByRole('navigation', { name: '설정 항목', exact: true });
  await openSettings(page, '확장 기능');
  const toggle = (name: string) => page.getByRole('switch', { name: `${name} 사용 설정`, exact: true }).click();
  for (const [extension, nav] of [['서강대학교 공지', '받은 소식'], ['탐색', '탐색'], ['보관함', '보관함'], ['사이버캠퍼스', '과제·마감']]) {
    await toggle(extension);
    await expect(sidebar.getByRole('button', { name: nav, exact: true })).toBeVisible();
  }
  await expect(panes.getByRole('button', { name: '게시판', exact: true })).toBeVisible();
  await expect(panes.getByRole('button', { name: '사이버캠퍼스', exact: true })).toBeVisible();
  await expect(sidebar.locator('.source-group-heading')).toHaveText(['서강대학교 공지', '사이버캠퍼스']);
  for (const [extension, pane] of [['나를 위한 소식', '개인화'], ['AI 분석', 'AI'], ['알림·백그라운드', '알림·백그라운드']]) {
    await toggle(extension);
    await expect(panes.getByRole('button', { name: pane, exact: true })).toBeVisible();
  }
  await toggle('앞으로 7일');
  await sidebar.getByRole('button', { name: '오늘', exact: true }).click();
  await expect(page.getByRole('heading', { level: 2, name: '나를 위한 소식', exact: true })).toBeVisible();
  await expect(page.getByRole('complementary', { name: '앞으로 7일', exact: true })).toBeVisible();

  // Turning 탐색 off while on its page falls back to Today.
  await sidebar.getByRole('button', { name: '탐색', exact: true }).click();
  await openSettings(page, '확장 기능');
  await toggle('탐색');
  await expect(sidebar.getByRole('button', { name: '탐색', exact: true })).toHaveCount(0);
  await toggle('AI 분석');
  await expect(panes.getByRole('button', { name: 'AI', exact: true })).toHaveCount(0);
});

test('without the AI extension the reader hides analysis and Today hides AI dates', async ({ page }) => {
  await mockDesktop(page, { aiResult: true, extensions: { ai: false } });
  await page.goto('/');
  const week = page.getByRole('complementary', { name: '앞으로 7일', exact: true });
  await expect(week).toBeVisible();
  await expect(week.locator('.event-ai')).toHaveCount(0);
  await expect(week).not.toContainText('추천 순위와 알림에 쓰지 않으니');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-body')).toBeVisible();
  await expect(page.getByRole('region', { name: '공지 AI 분석' })).toHaveCount(0);
});
