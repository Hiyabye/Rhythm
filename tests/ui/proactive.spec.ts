import { test, expect } from '@playwright/test';
import { mockDesktop, callCount } from './fixture';

async function openSettings(page: import('@playwright/test').Page) {
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '설정', exact: true }).click();
}

test('sync interval distinguishes the energy-profile default from an explicit one-hour override', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await openSettings(page);

  const enabled = page.getByRole('switch', { name: '자동 동기화', exact: true });
  await enabled.click();
  const interval = page.getByLabel('확인 간격', { exact: true });
  await expect(interval).toHaveValue('0');
  await expect(page.getByText('전원 모드 기본값은 절전 60분, 균형 30분, 성능 15분입니다.', { exact: false })).toBeVisible();

  await interval.selectOption('60');
  await expect.poll(() => page.evaluate(() => {
    const calls = (window as any).__fixture.calls.filter((call: any) => call.command === 'proactive_save');
    return calls[calls.length - 1]?.args.settings.scheduler_interval_minutes;
  })).toBe(60);
  await expect(interval).toHaveValue('60');

  await interval.selectOption('0');
  await expect.poll(() => page.evaluate(() => {
    const calls = (window as any).__fixture.calls.filter((call: any) => call.command === 'proactive_save');
    return calls[calls.length - 1]?.args.settings.scheduler_interval_minutes;
  })).toBe(0);
  await expect(interval).toHaveValue('0');
});

test('proactive settings save OS lifecycle, sync cadence, notifications and quiet hours', async ({ page }) => {
  await mockDesktop(page, { schedulerError: true });
  await page.goto('/');
  await openSettings(page);

  const closeToBackground = page.getByRole('switch', { name: '창을 닫아도 백그라운드에서 실행', exact: true });
  await closeToBackground.click();
  await expect(closeToBackground).toHaveAttribute('aria-checked', 'true');
  expect(await callCount(page, 'lifecycle_set_close_to_background')).toBe(1);

  const autostart = page.getByRole('switch', { name: '로그인 시 자동 시작', exact: true });
  await autostart.click();
  await expect(autostart).toHaveAttribute('aria-checked', 'true');
  const hidden = page.getByRole('switch', { name: '로그인 후 창 숨기기', exact: true });
  await hidden.click();
  await expect(hidden).toHaveAttribute('aria-checked', 'true');

  const enabled = page.getByRole('switch', { name: '자동 동기화', exact: true });
  await enabled.click();
  await expect(enabled).toHaveAttribute('aria-checked', 'true');
  await page.getByLabel('확인 간격', { exact: true }).selectOption('120');
  await expect.poll(() => page.evaluate(() => {
    const calls = (window as any).__fixture.calls.filter((call: any) => call.command === 'proactive_save');
    return calls[calls.length - 1]?.args.settings.scheduler_interval_minutes;
  })).toBe(120);
  const paused = page.getByRole('switch', { name: '자동 동기화 일시 중지', exact: true });
  await paused.click();
  await expect(paused).toHaveAttribute('aria-checked', 'true');
  const schedulerStatus = page.locator('#background .privacy-note').filter({ hasText: '대기 중인 알림' });
  await expect(schedulerStatus).toContainText('최근 실행:');
  await expect(schedulerStatus).toContainText('다음 확인 예정: 예약된 작업 없음');
  await expect(schedulerStatus).toContainText('실패');
  await expect(page.getByText('최근 오류: 자동 동기화에 실패했습니다.', { exact: true })).toBeVisible();

  const notifications = page.getByRole('switch', { name: '알림 활성화', exact: true });
  await notifications.click();
  await expect(notifications).toHaveAttribute('aria-checked', 'true');
  await page.getByLabel('알림 민감도', { exact: true }).selectOption('interested');
  await page.getByRole('switch', { name: '방해 금지 시간', exact: true }).click();
  const quietStart = page.getByLabel('시작 시간', { exact: true });
  const quietEnd = page.getByLabel('종료 시간', { exact: true });
  await quietStart.fill('23:30');
  await expect(quietStart).toBeEnabled();
  await quietEnd.fill('07:30');
  await expect(quietEnd).toBeEnabled();
  await expect(page.getByText('권한 허용은 알림 전달을 보장하지 않습니다.', { exact: false })).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const calls = (window as any).__fixture.calls.filter((call: any) => call.command === 'proactive_save');
    return calls[calls.length - 1]?.args.settings.quiet_hours;
  })).toEqual({ enabled: true, start: '23:30', end: '07:30' });
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
});

test('notification permission is queried without prompting and granted only after an explicit request', async ({ page }) => {
  await mockDesktop(page, { notificationAuthorization: 'not_determined', notificationRequestResult: 'authorized' });
  await page.goto('/');
  await openSettings(page);

  await expect(page.getByText('운영체제 알림 권한: 아직 결정되지 않음', { exact: true })).toBeVisible();
  expect(await callCount(page, 'notification_authorization')).toBe(1);
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  await page.getByRole('button', { name: '운영체제 알림 권한 요청', exact: true }).click();
  await expect(page.getByText('운영체제 알림 권한: 허용됨', { exact: true })).toBeVisible();
  expect(await callCount(page, 'notification_request_permission')).toBe(1);
});

test('denied permission remains denied after an explicit request', async ({ page }) => {
  await mockDesktop(page, { notificationAuthorization: 'denied', notificationRequestResult: 'denied' });
  await page.goto('/');
  await openSettings(page);

  await expect(page.getByText('운영체제 알림 권한: 거부됨', { exact: true })).toBeVisible();
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  await page.getByRole('button', { name: '운영체제 알림 권한 요청', exact: true }).click();
  await expect(page.getByText('운영체제 알림 권한: 거부됨', { exact: true })).toBeVisible();
  expect(await callCount(page, 'notification_request_permission')).toBe(1);
});

test('unavailable permission request is reported without claiming delivery', async ({ page }) => {
  await mockDesktop(page, { notificationAuthorization: 'not_determined', notificationRequestResult: 'unavailable' });
  await page.goto('/');
  await openSettings(page);

  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  await page.getByRole('button', { name: '운영체제 알림 권한 요청', exact: true }).click();
  await expect(page.getByText('운영체제 알림 권한: 이 환경에서 사용할 수 없음', { exact: true })).toBeVisible();
  await expect(page.getByText(/권한 허용은 알림 전달을 보장하지 않습니다/)).toBeVisible();
  expect(await callCount(page, 'notification_request_permission')).toBe(1);
});

test('unsupported notification permission APIs are reported as unavailable and cannot be requested', async ({ page }) => {
  await mockDesktop(page, { notificationAuthorization: 'unavailable' });
  await page.goto('/');
  await openSettings(page);

  await expect(page.getByText('운영체제 알림 권한: 이 환경에서 사용할 수 없음', { exact: true })).toBeVisible();
  await expect(page.getByText('이 환경에서는 운영체제 알림 권한 상태를 확인하거나 요청할 수 없습니다. 알림 전달 가능 여부는 운영체제 설정에서 확인하세요.', { exact: true })).toBeVisible();
  expect(await callCount(page, 'notification_authorization')).toBe(1);
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  await expect(page.getByRole('button', { name: '운영체제 알림 권한 요청', exact: true })).toBeDisabled();
});

test('failed proactive save retains the saved value and exposes the backend error', async ({ page }) => {
  await mockDesktop(page, { proactiveSaveError: true });
  await page.goto('/');
  await openSettings(page);
  const enabled = page.getByRole('switch', { name: '자동 동기화', exact: true });
  await enabled.click();
  await expect(enabled).toHaveAttribute('aria-checked', 'false');
  await expect(page.getByRole('alert')).toContainText('백그라운드 설정을 저장하지 못했습니다.');
});

test('history can be cleared and opens its stable document destination in the archive reader', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await openSettings(page);
  await expect(page.getByRole('heading', { name: '한국어 공지 01', exact: true })).toBeVisible();
  expect(await callCount(page, 'proactive_history')).toBeGreaterThan(0);
  await page.getByRole('button', { name: '공지 열기', exact: true }).click();
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 01');
  await page.keyboard.press('Escape');
  await openSettings(page);
  await page.getByRole('button', { name: '알림 기록 비우기', exact: true }).click();
  await expect(page.getByText('저장된 알림 기록이 없습니다.', { exact: true })).toBeVisible();
  expect(await callCount(page, 'proactive_clear_history')).toBe(1);
});

test('notification activation opens a document by stable identity and missing destinations remain recoverable', async ({ page }) => {
  await mockDesktop(page, { revisions: true });
  await page.goto('/');
  await expect.poll(() => callCount(page, 'plugin:event|listen')).toBe(1);
  await page.evaluate(() => (window as any).__fixture.emitNotification({ event_id: 12, document_id: 2, revision_id: 2 }));
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 02');
  await expect(page.getByLabel('보관 이력')).toHaveValue('2');
  await page.keyboard.press('Escape');
  await page.evaluate(() => (window as any).__fixture.emitNotification({ event_id: 13, document_id: 999, revision_id: 99 }));
  await expect(page.getByRole('alert')).toContainText('공지 없음');
  await expect(page.getByRole('button', { name: '← 목록으로 돌아가기 Esc', exact: true })).toBeVisible();
});

test('browser preview explains that proactive settings and OS delivery are unavailable', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByText('브라우저에서는 화면만 둘러볼 수 있습니다.', { exact: false })).toBeVisible();
  await openSettings(page);
  await expect(page.getByText('브라우저 미리보기에서는 설정을 저장하거나 운영체제 기능을 확인할 수 없습니다.', { exact: true })).toBeVisible();
  await expect(page.getByRole('switch')).toHaveCount(0);
});

test('OS lifecycle and history-clear failures retain authoritative values and disclose errors', async ({ page }) => {
  await mockDesktop(page, { lifecycleError: true, proactiveClearError: true });
  await page.goto('/');
  await openSettings(page);
  const closeToBackground = page.getByRole('switch', { name: '창을 닫아도 백그라운드에서 실행', exact: true });
  await closeToBackground.click();
  await expect(closeToBackground).toHaveAttribute('aria-checked', 'false');
  await expect(page.getByRole('alert')).toContainText('운영체제 시작 설정을 바꾸지 못했습니다.');
  await page.getByRole('button', { name: '알림 기록 비우기', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('알림 기록을 비우지 못했습니다.');
  await expect(page.getByRole('heading', { name: '한국어 공지 01', exact: true })).toBeVisible();
});

test('cold-start notification activation is recovered from the one-shot proactive snapshot', async ({ page }) => {
  await mockDesktop(page, { pendingActivation: { event_id: 22, document_id: 2, revision_id: 1, attempt_count: 1, event_ids: [22] } });
  await page.goto('/');
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 02');
  expect(await callCount(page, 'proactive_snapshot')).toBe(1);
});
