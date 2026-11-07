import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { callCount, mockDesktop, openSettings } from './fixture';

function title(page: Page, name: string) {
  return page.getByRole('heading', { level: 1, name, exact: true });
}
async function next(page: Page, name: string) {
  await page.getByRole('button', { name: '다음', exact: true }).click();
  await expect(title(page, name)).toBeFocused();
}

test('a fresh profile walks through setup, saving each choice without starting network work', async ({ page }) => {
  await mockDesktop(page, { onboardingNeeded: true, enabled: false });
  await page.goto('/');
  await expect(title(page, 'Rhythm에 오신 것을 환영합니다')).toBeFocused();
  await expect(page.getByRole('navigation', { name: '기본 탐색' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: '이전', exact: true })).toHaveCount(0);

  await page.getByRole('group', { name: '화면 테마' }).getByRole('button', { name: '다크' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

  await next(page, '받아 볼 게시판');
  await page.getByRole('switch', { name: '서강대학교 공지 사용 설정' }).click();
  await page.getByRole('switch', { name: '소프트웨어융합대학 소식 구독' }).click();
  await expect(page.getByRole('switch', { name: '소프트웨어융합대학 소식 구독' })).toHaveAttribute('aria-checked', 'true');

  await next(page, '관심사');
  await expect(page.getByRole('textbox', { name: '관심 주제 추가' })).toBeVisible();
  await expect(page.getByRole('switch', { name: '관심사 학습' })).toHaveCount(0);

  await next(page, 'AI 요약');
  await page.getByRole('button', { name: 'AI 이 기기에서', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Qwen3.5 4B 다운로드 및 설치', exact: true })).toBeVisible();
  expect(await callCount(page, 'ai_install')).toBe(0);
  await page.getByRole('button', { name: 'Qwen3.5 4B 다운로드 및 설치', exact: true }).click();
  await expect(page.getByRole('progressbar', { name: 'Qwen3.5 4B 다운로드' })).toBeVisible();
  expect(await callCount(page, 'ai_install')).toBe(1);

  await next(page, '백그라운드 실행');
  await expect(page.getByRole('switch', { name: '자동 동기화', exact: true })).toBeVisible();
  await expect(page.getByRole('switch', { name: '자동 동기화 일시 중지' })).toHaveCount(0);
  await expect(page.getByRole('group', { name: '전원 모드' })).toBeVisible();

  await next(page, '알림');
  await expect(page.getByRole('switch', { name: '알림 활성화' })).toBeVisible();
  await expect(page.getByRole('heading', { name: '알림 기록' })).toHaveCount(0);
  expect(await callCount(page, 'proactive_history')).toBe(0);

  await next(page, '준비가 끝났습니다');
  const summary = page.getByRole('group', { name: '선택한 설정' });
  await expect(summary.locator('.settings-row').filter({ hasText: '구독한 게시판' }).first()).toContainText('1개');
  await expect(summary.locator('.settings-row').filter({ hasText: 'AI' }).first()).toContainText('이 기기에서 실행');
  expect(await callCount(page, 'archive_start')).toBe(0);
  expect(await callCount(page, 'complete_onboarding')).toBe(0);

  await page.getByRole('button', { name: '시작하기', exact: true }).click();
  await expect(page.getByRole('navigation', { name: '기본 탐색' })).toBeVisible();
  await expect(page.getByRole('heading', { level: 1, name: '오늘' })).toBeVisible();
  expect(await callCount(page, 'complete_onboarding')).toBe(1);
  expect(await callCount(page, 'archive_start')).toBe(0);
});

test('the last step fetches notices only when asked', async ({ page }) => {
  await mockDesktop(page, { onboardingNeeded: true, sourceList: [{ id: 'computing-updates', name: '소식', organization: '소프트웨어융합대학', subscribed: true }] });
  await page.goto('/');
  for (const name of ['받아 볼 게시판', '관심사', 'AI 요약', '백그라운드 실행', '알림', '준비가 끝났습니다']) await next(page, name);
  await page.getByRole('button', { name: '가져오기', exact: true }).click();
  await expect(page.getByText('공개 공지를 가져오는 중입니다.')).toBeVisible();
  expect(await callCount(page, 'archive_start')).toBe(1);
  await expect(page.getByRole('button', { name: '가져오기', exact: true })).toBeDisabled();
});

test('skipping ends setup without changing settings', async ({ page }) => {
  await mockDesktop(page, { onboardingNeeded: true });
  await page.goto('/');
  await next(page, '받아 볼 게시판');
  await page.getByRole('button', { name: '건너뛰기', exact: true }).click();
  await expect(page.getByRole('navigation', { name: '기본 탐색' })).toBeVisible();
  expect(await callCount(page, 'complete_onboarding')).toBe(1);
  expect(await callCount(page, 'save_settings')).toBe(0);
  expect(await callCount(page, 'set_extension')).toBe(0);
});

test('choosing the remote engine saves it and leaves the details to the AI pane', async ({ page }) => {
  await mockDesktop(page, { onboardingNeeded: true });
  await page.goto('/');
  for (const name of ['받아 볼 게시판', '관심사', 'AI 요약']) await next(page, name);
  await page.getByRole('button', { name: 'AI OpenAI 호환 API', exact: true }).click();
  await expect(page.getByText('API 주소, 키, 전송 동의는 설정 → AI에서 입력합니다.', { exact: false })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__fixture.ai.preferences.engine)).toBe('openai-compatible');
  await page.getByRole('button', { name: 'AI 나중에', exact: true }).click();
  expect(await page.evaluate(() => (window as any).__fixture.ai.preferences.engine)).toBe('openai-compatible');
});

test('setup can be reopened from Settings and from the empty Today guide', async ({ page }) => {
  await mockDesktop(page, { count: 0 });
  await page.goto('/');
  await expect(page.getByRole('navigation', { name: '기본 탐색' })).toBeVisible();
  await page.getByRole('button', { name: '초기 설정 열기', exact: true }).click();
  await expect(title(page, 'Rhythm에 오신 것을 환영합니다')).toBeFocused();
  await expect(page.getByRole('group', { name: '화면 테마' }).getByRole('button', { name: '라이트' })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: '건너뛰기', exact: true }).click();

  await openSettings(page, '일반');
  await page.getByRole('button', { name: '다시 실행', exact: true }).click();
  await expect(title(page, 'Rhythm에 오신 것을 환영합니다')).toBeFocused();
  await page.getByRole('button', { name: '건너뛰기', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1, name: '오늘' })).toBeVisible();
});
