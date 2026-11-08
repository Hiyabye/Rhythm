import { test, expect } from '@playwright/test';
import type { Page } from '@playwright/test';
import { mockDesktop, callCount, answerDialogs, dialogMessages, openSettings } from './fixture';

function permission(page: Page) {
  return page.locator('.settings-row').filter({ hasText: '운영체제 알림 권한' });
}
function history(page: Page) {
  return page.getByRole('list', { name: '알림 기록', exact: true });
}

test('sync interval distinguishes the energy-profile default from an explicit one-hour override', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');

  const enabled = page.getByRole('switch', { name: '자동 동기화', exact: true });
  await enabled.click();
  const interval = page.getByLabel('확인 간격', { exact: true });
  await expect(interval).toHaveValue('0');
  await page.getByRole('button', { name: '자동 동기화 설명', exact: true }).click();
  await expect(page.getByText('전원 모드 기본값은 절전 60분, 균형 30분, 성능 15분', { exact: false })).toBeVisible();
  await page.keyboard.press('Escape');

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
  await openSettings(page, '알림·백그라운드');

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
  const schedulerStatus = page.locator('#background .scheduler-status');
  await expect(schedulerStatus).toContainText('대기 중인 알림 1개');
  await expect(schedulerStatus).toContainText('최근 실행');
  await expect(schedulerStatus).toContainText('다음 확인 예약 없음');
  await expect(schedulerStatus).toContainText('실패');
  await expect(schedulerStatus).toContainText('최근 오류: 자동 동기화에 실패했습니다.');
  await expect(schedulerStatus).toContainText('다음 확인 때 다시 시도합니다');
  // A stored scheduler error is status, not a fresh announcement on every visit.
  await expect(schedulerStatus.getByRole('alert')).toHaveCount(0);

  await page.getByRole('navigation', { name: '설정 항목', exact: true }).getByRole('button', { name: '알림·백그라운드', exact: true }).click();
  const notifications = page.getByRole('switch', { name: '알림 활성화', exact: true });
  await notifications.click();
  await expect(notifications).toHaveAttribute('aria-checked', 'true');
  await page.getByLabel('알림 민감도', { exact: true }).selectOption('interested');
  await page.getByRole('switch', { name: '방해 금지 시간', exact: true }).click();
  const quietStart = page.getByLabel('시작 시간', { exact: true });
  const quietEnd = page.getByLabel('종료 시간', { exact: true });
  await quietStart.fill('23:30');
  // WebKit commits native time inputs on blur; finish each edit as a user would
  // before inspecting the persisted settings or editing the next field.
  await quietStart.press('Tab');
  await expect(quietStart).toBeEnabled();
  await quietEnd.fill('07:30');
  await quietEnd.press('Tab');
  await expect(quietEnd).toBeEnabled();
  await page.getByRole('button', { name: '알림 설명', exact: true }).click();
  await expect(page.getByText('권한을 허용해도 전달을 보장하지는 않습니다', { exact: false })).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const calls = (window as any).__fixture.calls.filter((call: any) => call.command === 'proactive_save');
    return calls[calls.length - 1]?.args.settings.quiet_hours;
  })).toEqual({ enabled: true, start: '23:30', end: '07:30' });
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
});

test('notification permission is queried without prompting and granted only after an explicit request', async ({ page }) => {
  await mockDesktop(page, { notificationAuthorization: 'not_determined', notificationRequestResult: 'authorized' });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');

  await expect(permission(page)).toContainText('아직 결정되지 않음');
  expect(await callCount(page, 'notification_authorization')).toBe(1);
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  await page.getByRole('button', { name: '운영체제 알림 권한 요청', exact: true }).click();
  await expect(permission(page)).toContainText('허용됨');
  expect(await callCount(page, 'notification_request_permission')).toBe(1);
});

test('denied permission remains denied after an explicit request', async ({ page }) => {
  await mockDesktop(page, { notificationAuthorization: 'denied', notificationRequestResult: 'denied' });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');

  await expect(permission(page)).toContainText('거부됨');
  await expect(permission(page)).toContainText('시스템 설정의 알림에서 Rhythm을 허용한 뒤 새로고침하세요.');
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  await page.getByRole('button', { name: '운영체제 알림 권한 요청', exact: true }).click();
  await expect(permission(page)).toContainText('거부됨');
  expect(await callCount(page, 'notification_request_permission')).toBe(1);
});

test('unavailable permission request is reported without claiming delivery', async ({ page }) => {
  await mockDesktop(page, { notificationAuthorization: 'not_determined', notificationRequestResult: 'unavailable' });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');

  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  await page.getByRole('button', { name: '운영체제 알림 권한 요청', exact: true }).click();
  await expect(permission(page)).toContainText('이 환경에서 사용할 수 없음');
  await expect(permission(page)).not.toContainText('허용됨');
  expect(await callCount(page, 'notification_request_permission')).toBe(1);
});

test('unsupported notification permission APIs are reported as unavailable and cannot be requested', async ({ page }) => {
  await mockDesktop(page, { notificationAuthorization: 'unavailable' });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');

  await expect(permission(page)).toContainText('이 환경에서 사용할 수 없음');
  await expect(permission(page)).toContainText('이 환경에서는 권한을 확인하거나 요청할 수 없습니다.');
  expect(await callCount(page, 'notification_authorization')).toBe(1);
  expect(await callCount(page, 'notification_request_permission')).toBe(0);
  await expect(page.getByRole('button', { name: '운영체제 알림 권한 요청', exact: true })).toBeDisabled();
});

test('failed proactive save retains the saved value and exposes the backend error', async ({ page }) => {
  await mockDesktop(page, { proactiveSaveError: true });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');
  const enabled = page.getByRole('switch', { name: '자동 동기화', exact: true });
  await enabled.click();
  await expect(enabled).toHaveAttribute('aria-checked', 'false');
  await expect(page.getByRole('alert')).toContainText('백그라운드 설정을 저장하지 못했습니다.');
});

test('history can be cleared and opens its stable document destination in the archive reader', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');
  await expect(history(page)).toContainText('한국어 공지 01');
  expect(await callCount(page, 'proactive_history')).toBeGreaterThan(0);
  await page.getByRole('button', { name: '한국어 공지 01 공지 열기', exact: true }).click();
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 01');
  await page.keyboard.press('Escape');
  // Settings reopens on the pane it last showed.
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '설정', exact: true }).click();

  // Cancelling the confirmation leaves the history untouched.
  await page.getByRole('button', { name: '알림 기록 비우기', exact: true }).click();
  await expect.poll(() => dialogMessages(page).then((messages) => messages.length)).toBe(1);
  const [confirmation] = await dialogMessages(page);
  expect(confirmation).toContain('되돌릴 수 없습니다');
  expect(confirmation).toContain('중복 방지 정보');
  await expect(history(page)).toContainText('한국어 공지 01');
  expect(await callCount(page, 'proactive_clear_history')).toBe(0);

  await answerDialogs(page, 'ok');
  await page.getByRole('button', { name: '알림 기록 비우기', exact: true }).click();
  await expect(page.getByText('저장된 알림 기록이 없습니다.', { exact: true })).toBeVisible();
  expect(await callCount(page, 'proactive_clear_history')).toBe(1);
});

test('notification activation opens a document by stable identity and missing destinations remain recoverable', async ({ page }) => {
  await mockDesktop(page, { revisions: true });
  await page.goto('/');
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.calls
    .filter((call: { command: string; args: { event?: string } }) => call.command === 'plugin:event|listen' && call.args.event === 'proactive://notification-clicked').length)).toBe(1);
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
  // Without the desktop backend no extension is on, so only the core panes appear.
  await openSettings(page, '확장 기능');
  await expect(page.getByText('데스크톱 앱에서 확장 기능을 켤 수 있습니다.', { exact: true })).toBeVisible();
  await expect(page.getByRole('navigation', { name: '설정 항목', exact: true }).getByRole('button', { name: '알림·백그라운드', exact: true })).toHaveCount(0);
  await expect(page.getByRole('switch')).toHaveCount(0);
});

test('OS lifecycle and history-clear failures retain authoritative values and disclose errors', async ({ page }) => {
  await mockDesktop(page, { lifecycleError: true, proactiveClearError: true });
  await page.goto('/');
  await openSettings(page, '알림·백그라운드');
  const closeToBackground = page.getByRole('switch', { name: '창을 닫아도 백그라운드에서 실행', exact: true });
  await closeToBackground.click();
  await expect(closeToBackground).toHaveAttribute('aria-checked', 'false');
  // Both parts share the pane, so each reports its own failure.
  await expect(page.locator('#background').getByRole('alert')).toContainText('운영체제 시작 설정을 바꾸지 못했습니다.');
  await answerDialogs(page, 'ok');
  await page.getByRole('button', { name: '알림 기록 비우기', exact: true }).click();
  await expect(page.locator('#notifications').getByRole('alert')).toContainText('알림 기록을 비우지 못했습니다.');
  await expect(history(page)).toContainText('한국어 공지 01');
});

test('cold-start notification activation is recovered from the one-shot proactive snapshot', async ({ page }) => {
  await mockDesktop(page, { pendingActivation: { event_id: 22, document_id: 2, revision_id: 1, attempt_count: 1, event_ids: [22] } });
  await page.goto('/');
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 02');
  expect(await callCount(page, 'proactive_snapshot')).toBe(1);
});

test('notification event and snapshot share deduplication without blocking a later attempt', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await expect.poll(() => callCount(page, 'proactive_snapshot')).toBe(1);
  await page.evaluate(() => (window as any).__fixture.emitNotification({ event_id: 12, document_id: 1, revision_id: 1 }, true));
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 01');
  await page.keyboard.press('Escape');
  await expect(page.locator('.reader-header h2')).toHaveCount(0);
  await openSettings(page, '알림·백그라운드');
  await expect(page.getByRole('heading', { name: '알림 기록', exact: true })).toBeVisible();
  // Each mounted settings pane reads the snapshot again; none may reopen the handled activation.
  await expect.poll(() => callCount(page, 'proactive_snapshot')).toBeGreaterThanOrEqual(2);
  await expect(page.getByRole('button', { name: '알림 권한 상태 새로고침', exact: true })).toBeVisible();
  await expect(page.locator('.reader-header h2')).toHaveCount(0);

  // A separate attempt for the same event and document remains actionable.
  await page.evaluate(() => (window as any).__fixture.emitNotification({ event_id: 12, document_id: 1, revision_id: 1, attempt_count: 2 }));
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 01');
  await page.keyboard.press('Escape');
  await page.evaluate(() => (window as any).__fixture.emitNotification({ event_id: 13, document_id: 1, revision_id: 1 }));
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 01');
});

test('snapshot-first activation ignores a late event but manual history can reopen it', async ({ page }) => {
  await mockDesktop(page, { pendingActivation: { event_id: 12, document_id: 1, revision_id: 1, attempt_count: 1, event_ids: [12] } });
  await page.goto('/');
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 01');
  await page.keyboard.press('Escape');
  await openSettings(page, '알림·백그라운드');
  await expect(page.getByRole('button', { name: '한국어 공지 01 공지 열기', exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).__fixture.emitNotification({ event_id: 12, document_id: 1, revision_id: 1 }));
  await expect(page.locator('.reader-header h2')).toHaveCount(0);
  await page.getByRole('button', { name: '한국어 공지 01 공지 열기', exact: true }).click();
  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 01');
});
