import { test, expect, type Page } from '@playwright/test';
import { mockDesktop } from './fixture';
import type { Meal, MealWeek } from '../../src/lib/types';

const meal = (items: string[], status: Meal['status'] = items.length ? 'available' : 'unknown', time: string | null = null): Meal => ({ status, items, time });

/** A readable week from `monday` whose days are the given 7 consecutive dates. */
function week(monday: string, korean: string, low = false): MealWeek {
  const start = Date.parse(`${monday}T00:00:00Z`);
  return {
    week: monday, source_id: 'dorm-menu', external_id: monday, low_resolution: low, status: 'ok',
    image_url: `https://new-cms.sogang.ac.kr/CrossEditor/binary/images/000047/${monday}.jpg`,
    days: Array.from({ length: 7 }, (_, index) => ({
      day: new Date(start + index * 86_400_000).toISOString().slice(0, 10),
      breakfast_korean: meal([`${korean} ${index + 1}`, '배추김치']),
      breakfast_simple: meal([]),
      cup_rice: index === 1 ? meal(['돈까스덮밥'], 'available', '11:40') : meal([]),
      dinner: index === 5 ? meal([], 'closed') : meal(['제육볶음']),
    })),
    breakfast_common: meal(['시리얼', '토스트']),
    drinks: meal(['우유']),
  };
}

async function openMeals(page: Page) {
  await page.goto('/');
  await page.getByRole('button', { name: '식단', exact: true }).click();
}

test('the 식단 page is opt-in and opens on today with each kind of cell', async ({ page }) => {
  await mockDesktop(page, { meals: { today: '2026-11-25', weeks: [week('2026-11-23', '된장국'), week('2026-11-30', '북엇국', true)] } });
  await page.goto('/');
  await expect(page.getByRole('button', { name: '식단', exact: true })).toHaveCount(0);
  await mockDesktop(page, { extensions: { meals: true }, meals: { today: '2026-11-25', weeks: [week('2026-11-23', '된장국'), week('2026-11-30', '북엇국', true)] } });
  await openMeals(page);
  await expect(page.getByRole('heading', { name: '식단', level: 1 })).toBeVisible();
  await expect(page.getByRole('heading', { name: '11/23 – 11/29' })).toBeVisible();
  const days = page.getByRole('tablist', { name: '요일' });
  // Wednesday 11-25 is today.
  const wednesday = days.getByRole('tab', { name: /수.*오늘/ });
  await expect(wednesday).toHaveAttribute('aria-selected', 'true');
  const panel = page.getByRole('tabpanel');
  await expect(panel).toContainText('된장국 3');
  await expect(panel).toContainText('시리얼');
  await expect(panel).toContainText('우유');
  await expect(panel.locator('.meal-row').filter({ hasText: '조식 · 간편식' })).toContainText('메뉴표에 없음');
  await expect(page.getByText('작은 이미지에서 읽어')).toHaveCount(0);

  // Arrow keys move between days and focus follows.
  await wednesday.focus();
  await page.keyboard.press('ArrowLeft');
  const tuesday = days.getByRole('tab', { name: /화/ });
  await expect(tuesday).toBeFocused();
  await expect(tuesday).toHaveAttribute('aria-selected', 'true');
  await expect(panel.locator('.meal-row').filter({ hasText: '컵밥' })).toContainText('돈까스덮밥');
  await expect(panel.locator('.meal-row').filter({ hasText: '컵밥' })).toContainText('오전 11:40');
  await page.keyboard.press('End');
  await page.keyboard.press('ArrowLeft');
  await expect(days.getByRole('tab', { name: /토/ })).toBeFocused();
  await expect(panel.locator('.meal-row').filter({ hasText: '석식' })).toContainText('운영하지 않음');

  // Next week opens on its Monday and notes the small image.
  await page.getByRole('button', { name: '다음 주' }).click();
  await expect(page.getByRole('button', { name: '다음 주' })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('heading', { name: '11/30 – 12/6' })).toBeVisible();
  await expect(days.getByRole('tab', { name: /월/ })).toHaveAttribute('aria-selected', 'true');
  await expect(panel).toContainText('북엇국 1');
  await expect(page.getByText('작은 이미지에서 읽어 메뉴 이름이 원본과 다를 수 있습니다.')).toBeVisible();
  await page.getByRole('button', { name: '메뉴표 원본 보기' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'archive_open_original').map((call: any) => call.args.url)))
    .toEqual(['https://new-cms.sogang.ac.kr/CrossEditor/binary/images/000047/2026-11-30.jpg']);
  await page.getByRole('button', { name: '이번 주' }).click();
  await expect(days.getByRole('tab', { name: /수/ })).toHaveAttribute('aria-selected', 'true');
});

test('an unreadable or missing week shows no other week, and a note when 공지 is off', async ({ page }) => {
  const unreadable: MealWeek = { ...week('2026-11-23', 'x'), status: 'unreadable', days: [], breakfast_common: null, drinks: null };
  await mockDesktop(page, { extensions: { meals: true }, meals: { today: '2026-11-29', weeks: [unreadable] } });
  await openMeals(page);
  await expect(page.getByRole('heading', { name: '메뉴표를 글자로 읽지 못했습니다' })).toBeVisible();
  await expect(page.getByRole('tablist')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '메뉴표 원본 보기' })).toBeVisible();
  // No next week is published, so there is no week switch.
  await expect(page.getByRole('button', { name: '다음 주' })).toHaveCount(0);
  await expect(page.getByRole('note')).toHaveCount(0);

  // Today's week is not published yet: the page says so and starts no work.
  await mockDesktop(page, { enabled: false, extensions: { meals: true }, meals: { today: '2026-12-01', weeks: [] } });
  await openMeals(page);
  await expect(page.getByRole('heading', { name: '이번 주 식단이 아직 없습니다' })).toBeVisible();
  await expect(page.getByRole('heading', { name: '11/30 – 12/6' })).toBeVisible();
  const note = page.getByRole('note');
  await expect(note).toContainText('서강대학교 공지가 꺼져 있는 동안에는 새 메뉴표를 받지 않습니다.');
  await note.getByRole('button', { name: '확장 기능 설정' }).click();
  await expect(page.getByRole('heading', { name: '설정', level: 1 })).toBeVisible();
});
