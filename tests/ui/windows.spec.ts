import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { callCount, mockDesktop, openSettings } from './fixture';

// The frontend chooses Windows-specific controls and instructions from the
// WebView user agent; WebView2 reports "Windows NT". Only the IPC boundary is mocked.
test.use({ userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0' });

// Opens the AI pane with its 고급 section expanded, where the device and folder controls live.
async function openAiAdvanced(page: Page) {
  await openSettings(page, 'AI');
  await page.getByText('고급', { exact: true }).click();
}
async function modelInfo(page: Page, name: string) {
  await page.getByRole('button', { name: `${name} 정보`, exact: true }).click();
  return page.getByRole('dialog', { name: `${name} 정보`, exact: true });
}
function engine(page: Page) {
  return page.locator('.settings-row').filter({ hasText: 'AI 엔진' }).first();
}
function permission(page: Page) {
  return page.locator('.settings-row').filter({ hasText: '운영체제 알림 권한' });
}

test('Windows build offers manual installer updates without invoking updater commands', async ({ page }) => {
  await mockDesktop(page, { windows: true });
  await page.goto('/');
  await openSettings(page, '데이터 및 정보');
  const updater = page.locator('#about');
  await expect(updater).toContainText('최신 Windows 설치 파일을 내려받습니다.');
  await expect(updater).toContainText('트레이 메뉴에서 완전히 종료');
  await expect(updater).not.toContainText('DMG');
  await expect(updater.getByRole('button', { name: '업데이트 확인', exact: true })).toHaveCount(0);
  expect(await callCount(page, 'updater_check')).toBe(0);
});

test('execution device selection persists across settings remount and unloads the old model', async ({ page }) => {
  await mockDesktop(page, { windows: true, aiInstalled: true }); await page.goto('/');
  await openAiAdvanced(page);
  const mode = page.getByLabel('AI 실행 장치', { exact: true });
  await expect(mode).toHaveValue('auto');
  await page.getByRole('button', { name: '선택한 모델 불러오기', exact: true }).click();
  await expect(engine(page)).toContainText('모델 준비됨');
  await mode.selectOption('cpu');
  await expect(mode).toHaveValue('cpu');
  await expect(engine(page)).toContainText('모델 없이 대기 중');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await openAiAdvanced(page);
  await expect(mode).toHaveValue('cpu');
  await mode.selectOption('gpu');
  await expect(mode).toHaveValue('gpu');
  expect(await callCount(page, 'ai_preferences')).toBe(2);
});

test('execution device save failure restores the persisted choice and shows fallback status', async ({ page }) => {
  await mockDesktop(page, { windows: true, aiExecutionMode: 'auto', aiExecutionError: true, aiRuntimeDetail: 'GPU 메모리가 부족해 CPU로 전환했습니다.' });
  await page.goto('/');
  await openAiAdvanced(page);
  await expect(engine(page)).toContainText('GPU 메모리가 부족해 CPU로 전환했습니다.');
  const mode = page.getByLabel('AI 실행 장치', { exact: true });
  await mode.selectOption('gpu');
  await expect(page.getByText('실행 장치를 저장하지 못했습니다. 다시 시도해 주세요.', { exact: true })).toBeVisible();
  await expect(mode).toHaveValue('auto');
  expect(await callCount(page, 'ai_install')).toBe(0);
});

test('model installation folder changes persist and only new downloads use the chosen location', async ({ page }) => {
  const directory = 'D:\\로컬 모델\\Rhythm';
  await mockDesktop(page, { windows: true, aiInstalled: true, aiModelDirectoryChoice: directory, aiDelay: 10000 });
  await page.goto('/');
  await openAiAdvanced(page);
  const storage = page.getByRole('group', { name: '모델 설치 폴더', exact: true });
  await expect(storage.getByText('C:\\Rhythm-test\\models', { exact: true })).toBeVisible();
  await expect(storage.getByRole('button', { name: '기본 위치로 되돌리기', exact: true })).toBeDisabled();
  await storage.getByRole('button', { name: '설치 폴더 선택', exact: true }).click();
  await expect(storage.getByText(directory, { exact: true })).toBeVisible();
  const existing = await modelInfo(page, 'Qwen3.5 4B');
  await expect(existing.getByText('models/qwen35-4b.gguf', { exact: true })).toBeVisible();
  await expect(existing.getByText(directory, { exact: true })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await openAiAdvanced(page);
  await expect(storage.getByText(directory, { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Qwen3.5 2B 다운로드 및 설치', exact: true }).click();
  await expect(storage.getByRole('button', { name: '설치 폴더 선택', exact: true })).toBeDisabled();
  await expect(storage.getByRole('button', { name: '기본 위치로 되돌리기', exact: true })).toBeDisabled();
  let added = await modelInfo(page, 'Qwen3.5 2B');
  await expect(added.getByText(directory, { exact: true })).toBeVisible();
  await expect(added.getByText(`${directory}\\qwen35-2b.gguf`, { exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: '다운로드 중단', exact: true }).click();
  await storage.getByRole('button', { name: '기본 위치로 되돌리기', exact: true }).click();
  await expect(storage.getByText('C:\\Rhythm-test\\models', { exact: true })).toBeVisible();
  added = await modelInfo(page, 'Qwen3.5 2B');
  await expect(added.getByText(directory, { exact: true })).toBeVisible();
  expect(await callCount(page, 'ai_preferences')).toBe(0);
  expect(await callCount(page, 'ai_reset_model_directory')).toBe(1);
});

test('cancelling the model installation folder picker preserves the selected folder', async ({ page }) => {
  const directory = 'D:\\현재 모델';
  await mockDesktop(page, { windows: true, aiModelDirectory: directory, aiModelDirectoryChoice: null });
  await page.goto('/');
  await openAiAdvanced(page);
  const storage = page.getByRole('group', { name: '모델 설치 폴더', exact: true });
  await storage.getByRole('button', { name: '설치 폴더 선택', exact: true }).click();
  await expect(storage.getByRole('button', { name: '설치 폴더 선택', exact: true })).toBeEnabled();
  await expect(storage.getByText(directory, { exact: true })).toBeVisible();
  await expect(page.getByText('새 모델을 설치할 폴더를 바꿨습니다.', { exact: false })).toHaveCount(0);
  expect(await callCount(page, 'ai_choose_model_directory')).toBe(1);
  expect(await callCount(page, 'ai_preferences')).toBe(0);
  expect(await callCount(page, 'ai_install')).toBe(0);
});

test('model installation folder save and reset errors leave the previous location visible', async ({ page }) => {
  const directory = 'D:\\현재 모델';
  await mockDesktop(page, { windows: true, aiModelDirectory: directory, aiModelDirectoryError: true });
  await page.goto('/');
  await openAiAdvanced(page);
  const storage = page.getByRole('group', { name: '모델 설치 폴더', exact: true });
  await storage.getByRole('button', { name: '설치 폴더 선택', exact: true }).click();
  await expect(page.getByRole('alert').filter({ hasText: '모델 설치 폴더를 저장하지 못했습니다. 기존 위치를 유지합니다.' })).toBeVisible();
  await expect(storage.getByText(directory, { exact: true })).toBeVisible();
  await storage.getByRole('button', { name: '기본 위치로 되돌리기', exact: true }).click();
  await expect(storage.getByText(directory, { exact: true })).toBeVisible();
  await expect(storage.getByRole('button', { name: '기본 위치로 되돌리기', exact: true })).toBeEnabled();
  expect(await callCount(page, 'ai_reset_model_directory')).toBe(1);
  expect(await callCount(page, 'ai_preferences')).toBe(0);
});

test('model installation folder changes explain and preserve a cancelled download location when resuming', async ({ page }) => {
  await mockDesktop(page, { windows: true, aiDelay: 10000 });
  await page.goto('/');
  await openAiAdvanced(page);
  await page.getByRole('button', { name: 'Qwen3.5 4B 다운로드 및 설치', exact: true }).click();
  await page.getByRole('button', { name: '다운로드 중단', exact: true }).click();
  const storage = page.getByRole('group', { name: '모델 설치 폴더', exact: true });
  await storage.getByRole('button', { name: '설치 폴더 선택', exact: true }).click();
  await expect(storage.getByText('D:\\로컬 모델\\Rhythm', { exact: true })).toBeVisible();
  await storage.getByRole('button', { name: '모델 설치 폴더 설명', exact: true }).click();
  await expect(page.getByText('중단한 다운로드는 원래 폴더에서 이어받습니다.', { exact: false })).toBeVisible();
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Qwen3.5 4B 설치 다시 시도 / 이어받기', exact: true }).click();
  // Wait for the mocked install to finish: the tile's layout change can scroll the page, which closes an open popover.
  await expect(page.getByRole('button', { name: 'Qwen3.5 4B 선택', exact: true })).toBeVisible({ timeout: 15000 });
  const installed = await modelInfo(page, 'Qwen3.5 4B');
  await expect(installed.getByText('C:\\Rhythm-test\\models', { exact: true })).toBeVisible();
  await expect(installed.getByText('C:\\Rhythm-test\\models\\qwen35-4b.gguf', { exact: true })).toBeVisible();
  await expect(installed.getByText('D:\\로컬 모델\\Rhythm', { exact: true })).toHaveCount(0);
  expect(await callCount(page, 'ai_install')).toBe(2);
});

test('model installation folder picker locks its controls while pending and reports unavailable storage', async ({ page }) => {
  await mockDesktop(page, { windows: true, aiModelDirectory: 'D:\\현재 모델', aiModelDirectoryGate: true, aiModelStorageAvailable: null, aiModelStorageError: '설치 폴더에 접근할 수 없습니다. 다른 폴더를 선택해 주세요.' });
  await page.goto('/');
  await openSettings(page, 'AI');
  // A storage error opens 고급 by itself so the problem is not hidden.
  const storage = page.getByRole('group', { name: '모델 설치 폴더', exact: true });
  await expect(storage.getByText('남은 공간 확인하지 못함', { exact: true })).toBeVisible();
  await expect(storage.getByRole('alert')).toHaveText('설치 폴더에 접근할 수 없습니다. 다른 폴더를 선택해 주세요.');
  await storage.getByRole('button', { name: '설치 폴더 선택', exact: true }).click();
  await expect(storage.getByRole('button', { name: '설치 폴더 선택', exact: true })).toBeDisabled();
  await expect(storage.getByRole('button', { name: '기본 위치로 되돌리기', exact: true })).toBeDisabled();
  await page.evaluate(() => (window as any).__fixture.releaseModelDirectory());
  await expect(storage.getByText('D:\\로컬 모델\\Rhythm', { exact: true })).toBeVisible();
  await expect(storage.getByRole('alert')).toHaveCount(0);
  await page.evaluate(() => { (window as any).__fixture.ai.model_storage.available_bytes = 0; });
  await page.getByRole('button', { name: 'AI 엔진 상태 새로고침', exact: true }).click();
  await expect(storage.getByText('남은 공간 0 B', { exact: true })).toBeVisible();
});

test('Windows notifications stay disabled until OS availability is confirmed', async ({ page }) => {
  await mockDesktop(page, { windows: true, notificationAuthorization: 'denied' });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');
  await expect(permission(page)).toContainText('거부됨');
  await expect(page.getByText(/Windows 설정 → 시스템 → 알림/)).toBeVisible();
  await expect(page.getByRole('button', { name: '시험 알림으로 연결 확인' })).toHaveCount(0);
  const toggle = page.getByRole('switch', { name: '알림 활성화', exact: true });
  await expect(toggle).toBeDisabled();
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  await page.evaluate(() => (window as any).__fixture.setNotificationAuthorization('authorized'));
  await page.getByRole('button', { name: '알림 권한 상태 새로고침', exact: true }).click();
  await expect(toggle).toBeEnabled();
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
});

test('first notification connection requires an explicit test and separate opt-in', async ({ page }) => {
  await mockDesktop(page, { windows: true, notificationAuthorization: 'not_determined' });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');
  const toggle = page.getByRole('switch', { name: '알림 활성화', exact: true });
  const testButton = page.getByRole('button', { name: '시험 알림으로 연결 확인', exact: true });
  await expect(toggle).toHaveAttribute('aria-checked', 'false');
  await expect(toggle).toBeDisabled();
  await expect(testButton).toBeVisible();
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  await testButton.click();
  await expect(page.getByText('Windows에 시험 알림 표시를 요청했습니다. 실제 화면 표시 여부는 확인하지 못했습니다.', { exact: true })).toBeVisible();
  await expect(toggle).toBeEnabled();
  await expect(toggle).toHaveAttribute('aria-checked', 'false');
  expect(await callCount(page, 'notification_request_permission')).toBe(1);
  expect(await callCount(page, 'proactive_save')).toBe(0);
});

test('unsent notification test is not reported as requested or enabled', async ({ page }) => {
  await mockDesktop(page, { windows: true, notificationAuthorization: 'not_determined', notificationRequestResult: 'not_determined', notificationTestRequested: false });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');
  await page.getByRole('button', { name: '시험 알림으로 연결 확인', exact: true }).click();
  await expect(page.getByText('시험 알림을 요청하지 못했습니다. Windows 알림 상태를 확인해 주세요.', { exact: true })).toBeVisible();
  await expect(page.getByRole('switch', { name: '알림 활성화', exact: true })).toBeDisabled();
  expect(await callCount(page, 'proactive_save')).toBe(0);
});

test('unavailable notification APIs are reported without claiming delivery', async ({ page }) => {
  await mockDesktop(page, { windows: true, notificationAuthorization: 'unavailable' });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');
  await expect(permission(page)).toContainText('이 환경에서 사용할 수 없음');
  await expect(page.getByText(/Windows 알림 상태를 확인할 수 없습니다/)).toBeVisible();
  await expect(page.getByRole('switch', { name: '알림 활성화', exact: true })).toBeDisabled();
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
});

test('an existing enabled setting can be switched off when Windows notifications become unavailable', async ({ page }) => {
  await mockDesktop(page, { windows: true, notificationAuthorization: 'denied', initialNotificationsEnabled: true });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');
  const toggle = page.getByRole('switch', { name: '알림 활성화', exact: true });
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'false');
  await expect(toggle).toBeDisabled();
});

test('an unavailable capability check does not enable notifications', async ({ page }) => {
  await mockDesktop(page, { windows: true, capabilitiesError: true });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');
  await expect(page.getByText('알림 사용 가능 여부를 확인하지 못했습니다.', { exact: true })).toBeVisible();
  await expect(page.getByRole('switch', { name: '알림 활성화', exact: true })).toBeDisabled();
});

test('setup uses the Windows notification wording', async ({ page }) => {
  await mockDesktop(page, { windows: true, onboardingNeeded: true });
  await page.goto('/');
  for (let step = 0; step < 5; step++) await page.getByRole('button', { name: '다음', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1, name: '알림', exact: true })).toBeFocused();
  await expect(page.getByRole('button', { name: '시험 알림으로 연결 확인' })).toBeVisible();
  await expect(page.getByRole('button', { name: '운영체제 알림 권한 요청' })).toHaveCount(0);
});
