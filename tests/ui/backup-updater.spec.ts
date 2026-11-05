import { test, expect } from '@playwright/test';
import type { Page } from '@playwright/test';
import { mockDesktop, callCount, answerDialogs, dialogMessages, openSettings } from './fixture';

type FixtureWindow = Window & { __fixture: { calls: { command: string; args: Record<string, unknown> }[] } };

async function openBackup(page: Page) {
  await openSettings(page, '데이터 및 정보');
  return page.locator('#data-backup');
}

async function openUpdater(page: Page) {
  await openSettings(page, '데이터 및 정보');
  return page.locator('#about');
}

async function commandArgs(page: Page, command: string) {
  return page.evaluate((name) => (window as FixtureWindow).__fixture.calls.find((call) => call.command === name)?.args, command);
}

test('backup restore inspects selected file before confirmation and only invokes restore after acceptance', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  const backup = await openBackup(page);

  await backup.getByRole('button', { name: '백업 파일 선택', exact: true }).click();
  await expect(backup.getByRole('heading', { name: '복원할 백업 확인', exact: true })).toBeVisible();
  await expect(backup).toContainText('공지 12건 · 첨부 자료 3개');
  await expect(backup).toContainText('형식 v1 · 스키마 6');
  await expect(backup).toContainText('모델 가중치는 포함되지 않습니다.');
  await expect(backup).toContainText('자동으로 다시 열리지 않습니다. 직접 다시 열면 복원이 마무리됩니다.');
  expect(await callCount(page, 'backup_inspect')).toBe(1);

  await answerDialogs(page, 'cancel');
  await backup.getByRole('button', { name: '이 백업으로 복원', exact: true }).click();
  await expect.poll(() => dialogMessages(page).then((messages) => messages.length)).toBe(1);
  const confirmations = await dialogMessages(page);
  expect(confirmations[0]).toContain('현재 Rhythm 데이터 전체');
  expect(confirmations[0]).toContain('복구용으로 보관');
  expect(confirmations[0]).toContain('Rhythm이 종료됩니다');
  expect(confirmations[0]).toContain('직접 다시 열면 복원이 마무리됩니다');
  expect(confirmations[0]).not.toContain('다시 시작');
  expect(await callCount(page, 'backup_restore')).toBe(0);
  await expect(backup.getByRole('heading', { name: '복원할 백업 확인', exact: true })).toBeVisible();

  await answerDialogs(page, 'ok');
  await backup.getByRole('button', { name: '이 백업으로 복원', exact: true }).click();
  await expect(backup.getByRole('status').filter({ hasText: '복원을 준비했습니다.' })).toBeVisible();
  expect(await callCount(page, 'backup_restore')).toBe(1);
  const restore = await commandArgs(page, 'backup_restore');
  expect(restore).toMatchObject({ source: '/fixtures/Rhythm-backup.rhythm-backup', confirmed: true });
});

test('backup inspection failures explain the error and do not expose a restore action', async ({ page }) => {
  await mockDesktop(page, { backupInspectError: true });
  await page.goto('/');
  const backup = await openBackup(page);

  await backup.getByRole('button', { name: '백업 파일 선택', exact: true }).click();
  await expect(backup.getByRole('alert')).toContainText('백업 파일의 형식이 올바르지 않습니다.');
  await expect(backup.getByRole('button', { name: '이 백업으로 복원', exact: true })).toHaveCount(0);
  expect(await callCount(page, 'backup_restore')).toBe(0);
});

test('restore failure remains visible and releases the pending operation state', async ({ page }) => {
  await mockDesktop(page, { backupRestoreError: true });
  await page.goto('/');
  const backup = await openBackup(page);
  await backup.getByRole('button', { name: '백업 파일 선택', exact: true }).click();
  await answerDialogs(page, 'ok');
  await backup.getByRole('button', { name: '이 백업으로 복원', exact: true }).click();

  await expect(backup.getByRole('alert')).toContainText('백업 데이터를 복원하지 못했습니다.');
  await expect(backup.getByRole('button', { name: '이 백업으로 복원', exact: true })).toBeEnabled();
});

test('backup preview can be cancelled without invoking restore', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  const backup = await openBackup(page);
  await backup.getByRole('button', { name: '백업 파일 선택', exact: true }).click();
  await expect(backup.getByRole('heading', { name: '복원할 백업 확인', exact: true })).toBeVisible();

  await backup.getByRole('button', { name: '취소', exact: true }).click();
  await expect(backup.getByRole('heading', { name: '복원할 백업 확인', exact: true })).toHaveCount(0);
  expect(await callCount(page, 'backup_restore')).toBe(0);
});

test('unconfigured updater shows manual update guidance instead of a failing check', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  const updater = await openUpdater(page);

  // Static guidance, not a live announcement on every visit to Settings.
  await expect(updater).toContainText('앱 안 업데이트를 지원하지 않습니다');
  await expect(updater.getByRole('status')).toHaveCount(0);
  await expect(updater.getByRole('listitem')).toHaveCount(0);
  await updater.getByText('설치 방법', { exact: true }).click();
  await expect(updater.getByRole('listitem')).toHaveCount(6);
  await expect(updater).toContainText('Rhythm을 완전히 종료합니다.');
  await expect(updater).toContainText('shasum -a 256');
  await expect(updater).toContainText('github.com/Hiyabye/Rhythm');
  await expect(updater).toContainText('Apple 공증을 받지 않았습니다');
  await expect(updater).not.toContainText('공증된');
  await expect(updater).toContainText('공지, 설정, AI 모델은 앱과 별도로 저장되어 앱을 바꿔도 지워지지 않습니다.');
  await expect(updater.getByRole('button', { name: '업데이트 확인', exact: true })).toHaveCount(0);
  await expect(updater.getByRole('alert')).toHaveCount(0);
  await expect(updater).toContainText('1.3.0');
  expect(await callCount(page, 'updater_configured')).toBe(1);
  expect(await callCount(page, 'updater_check')).toBe(0);
});

test('configured updater reports genuine check failures', async ({ page }) => {
  await mockDesktop(page, { updaterConfigured: true, updaterCheckError: '업데이트 정보를 확인하지 못했습니다: 네트워크 오류' });
  await page.goto('/');
  const updater = await openUpdater(page);
  await updater.getByRole('button', { name: '업데이트 확인', exact: true }).click();

  await expect(updater.getByRole('alert')).toContainText('업데이트 정보를 확인하지 못했습니다');
  await expect(updater.getByText('최신 버전입니다.', { exact: true })).toHaveCount(0);
  await expect(updater.getByRole('listitem')).toHaveCount(0);
  expect(await callCount(page, 'updater_check')).toBe(1);
});

test('updater installation errors remain recoverable', async ({ page }) => {
  await mockDesktop(page, { updaterConfigured: true, updaterInstallError: true });
  await page.goto('/');
  const updater = await openUpdater(page);
  await updater.getByRole('button', { name: '업데이트 확인', exact: true }).click();
  await updater.getByRole('button', { name: '다운로드 및 설치', exact: true }).click();

  await expect(updater.getByRole('alert')).toContainText('서명된 업데이트를 설치하지 못했습니다.');
  await expect(updater.getByRole('button', { name: '다시 시도', exact: true })).toBeEnabled();
  expect(await callCount(page, 'updater_install')).toBe(1);
  expect(await callCount(page, 'updater_restart')).toBe(0);
});

test('updater asks before restart both after staging and when restarting is chosen', async ({ page }) => {
  await mockDesktop(page, { updaterConfigured: true });
  await page.goto('/');
  const updater = await openUpdater(page);
  await updater.getByRole('button', { name: '업데이트 확인', exact: true }).click();

  await answerDialogs(page, 'cancel');
  await updater.getByRole('button', { name: '다운로드 및 설치', exact: true }).click();
  await expect(updater.getByText('서명된 업데이트를 준비했습니다.', { exact: true })).toBeVisible();
  await expect(updater.getByRole('button', { name: '설치 및 다시 시작', exact: true })).toBeVisible();
  expect(await callCount(page, 'updater_restart')).toBe(0);

  await answerDialogs(page, 'cancel');
  await updater.getByRole('button', { name: '설치 및 다시 시작', exact: true }).click();
  await expect.poll(() => dialogMessages(page).then((messages) => messages.length)).toBe(2);
  expect((await dialogMessages(page))[1]).toContain('서명 검증이 끝난 업데이트');
  await expect(updater.getByRole('button', { name: '설치 및 다시 시작', exact: true })).toBeEnabled();
  expect(await callCount(page, 'updater_restart')).toBe(0);

  await answerDialogs(page, 'ok');
  await updater.getByRole('button', { name: '설치 및 다시 시작', exact: true }).click();
  await expect.poll(() => callCount(page, 'updater_restart')).toBe(1);
  const restart = await commandArgs(page, 'updater_restart');
  expect(restart).toEqual({ confirmed: true });
});

test('restart failure is surfaced after explicit confirmation', async ({ page }) => {
  await mockDesktop(page, { updaterConfigured: true, updaterRestartError: true });
  await page.goto('/');
  const updater = await openUpdater(page);
  await updater.getByRole('button', { name: '업데이트 확인', exact: true }).click();
  await answerDialogs(page, 'ok');
  await updater.getByRole('button', { name: '다운로드 및 설치', exact: true }).click();

  await expect(updater.getByRole('alert')).toContainText('업데이트를 적용하며 다시 시작하지 못했습니다.');
  expect(await callCount(page, 'updater_restart')).toBe(1);

  // The verified package stays staged in the backend: retrying installs it
  // instead of checking and downloading again.
  await expect(updater.getByRole('button', { name: '설치 및 다시 시작', exact: true })).toBeEnabled();
  await answerDialogs(page, 'ok');
  await updater.getByRole('button', { name: '다시 시도', exact: true }).click();
  await expect.poll(() => callCount(page, 'updater_restart')).toBe(2);
  expect(await callCount(page, 'updater_install')).toBe(1);
  expect(await callCount(page, 'updater_check')).toBe(1);
});

test('a finalized restore is reported once the reopened window loads', async ({ page }) => {
  await mockDesktop(page, { restoreOutcome: { documents: 1240, resources: 37, unavailable_resources: 2, models_needing_reinstall: 1 } });
  await page.goto('/');
  const banner = page.getByRole('status').filter({ hasText: '백업 복원을 마쳤습니다' });
  await expect(banner).toContainText('공지 1,240건과 첨부 자료 37개를 복원하고 무결성을 확인했습니다.');
  await expect(banner).toContainText('내려받지 못한 첨부 자료 2개는 포함되지 않았습니다.');
  await expect(banner).toContainText('사용할 수 없는 AI 모델 1개는 다시 등록하거나 설치해 주세요.');
  await banner.getByRole('button', { name: '닫기', exact: true }).click();
  await expect(banner).toHaveCount(0);
});

test('a complete restore without gaps reports no missing items', async ({ page }) => {
  await mockDesktop(page, { restoreOutcome: { documents: 3, resources: 0, unavailable_resources: 0, models_needing_reinstall: 0 } });
  await page.goto('/');
  const banner = page.getByRole('status').filter({ hasText: '백업 복원을 마쳤습니다' });
  await expect(banner).toContainText('공지 3건과 첨부 자료 0개를 복원하고 무결성을 확인했습니다.');
  await expect(banner).not.toContainText('다시 설치');
  await expect(banner).not.toContainText('포함되지 않았습니다');
});

test('ordinary launches show no restore report', async ({ page }) => {
  await mockDesktop(page);
  await page.goto('/');
  await expect(page.getByRole('heading', { level: 1 }).first()).toBeVisible();
  expect(await callCount(page, 'backup_restore_outcome')).toBe(1);
  await expect(page.getByText('백업 복원을 마쳤습니다', { exact: true })).toHaveCount(0);
});

test('a native confirmation that cannot open never starts a restore', async ({ page }) => {
  await mockDesktop(page, { dialogError: true });
  await page.goto('/');
  const backup = await openBackup(page);
  await backup.getByRole('button', { name: '백업 파일 선택', exact: true }).click();
  await backup.getByRole('button', { name: '이 백업으로 복원', exact: true }).click();

  await expect(backup.getByRole('alert')).toContainText('확인 창을 열지 못해 복원을 시작하지 않았습니다.');
  expect(await callCount(page, 'backup_restore')).toBe(0);
  await expect(backup.getByRole('button', { name: '이 백업으로 복원', exact: true })).toBeEnabled();
});

test('the frontend never relies on synchronous browser dialogs', async () => {
  const { readdirSync, readFileSync } = await import('node:fs');
  const { join } = await import('node:path');
  const files = (dir: string): string[] => readdirSync(dir, { withFileTypes: true })
    .flatMap((item) => item.isDirectory() ? files(join(dir, item.name)) : [join(dir, item.name)]);
  const offenders = files('src')
    .filter((file) => /\.(svelte|ts)$/.test(file) && !file.endsWith('confirm.ts'))
    .filter((file) => /(^|[^.\w])(window\.)?(confirm|alert|prompt)\(/m.test(readFileSync(file, 'utf8')));
  expect(offenders).toEqual([]);
});

test('unavailable restore shows the actual reason while backup creation remains enabled', async ({ page }) => {
  await mockDesktop(page, { restoreSupportError: '이 저장소는 복원을 지원하지 않습니다.' });
  await page.goto('/');
  const backup = await openBackup(page);
  await expect(backup).toContainText('이 저장소는 복원을 지원하지 않습니다.');
  await expect(backup.getByRole('button', { name: '백업 파일 선택', exact: true })).toBeDisabled();
  await expect(backup.getByRole('button', { name: '백업 파일 저장', exact: true })).toBeEnabled();
  expect(await callCount(page, 'backup_inspect')).toBe(0);
});

test('an update found before the window opened is announced and opens About without checking again', async ({ page }) => {
  await mockDesktop(page, { updaterConfigured: true, autoUpdateFound: true });
  await page.goto('/');
  const banner = page.getByRole('status').filter({ hasText: 'Rhythm 1.1.1 버전을 설치할 수 있습니다' });
  await expect(banner).toBeVisible();
  await expect(banner).toContainText('그 전에는 아무것도 바뀌지 않습니다');

  await banner.getByRole('button', { name: '업데이트 보기', exact: true }).click();
  await expect(page.locator('#about-title')).toBeFocused();
  await expect(page.locator('#about')).toContainText('새 버전 1.1.1');
  await expect(page.locator('#about').getByRole('button', { name: '다운로드 및 설치', exact: true })).toBeEnabled();
  // The banner stays out of Settings, where the About section shows the update.
  await expect(banner).toHaveCount(0);
  expect(await callCount(page, 'updater_check')).toBe(0);
  expect(await callCount(page, 'updater_install')).toBe(0);
});

test('a live update announcement can be dismissed without downloading', async ({ page }) => {
  await mockDesktop(page, { updaterConfigured: true });
  await page.goto('/');
  const banner = page.getByRole('status').filter({ hasText: '버전을 설치할 수 있습니다' });
  await expect(page.getByRole('heading', { name: '오늘', exact: true })).toBeVisible();
  await expect(banner).toHaveCount(0);
  await page.evaluate(() => (window as unknown as { __fixture: { emitAvailable: (payload: unknown) => void } }).__fixture
    .emitAvailable({ version: '1.1.1', currentVersion: '1.1.0', date: null, body: null }));
  await expect(banner).toContainText('Rhythm 1.1.1 버전을 설치할 수 있습니다');

  await banner.getByRole('button', { name: '나중에', exact: true }).click();
  await expect(banner).toHaveCount(0);
  expect(await callCount(page, 'updater_install')).toBe(0);
});

test('automatic update checks are on by default and can be turned off', async ({ page }) => {
  await mockDesktop(page, { updaterConfigured: true });
  await page.goto('/');
  const updater = await openUpdater(page);
  const toggle = updater.getByRole('switch', { name: '자동으로 업데이트 확인', exact: true });
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  await expect(updater).toContainText('내려받기와 설치는 직접 고를 때만 진행합니다');

  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'false');
  expect(await commandArgs(page, 'updater_set_auto_check')).toEqual({ enabled: false });
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  expect(await callCount(page, 'updater_check')).toBe(0);
});

test('builds without an update key show no automatic check or announcement', async ({ page }) => {
  await mockDesktop(page, { autoUpdateFound: true });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: '오늘', exact: true })).toBeVisible();
  await expect(page.getByText('버전을 설치할 수 있습니다')).toHaveCount(0);
  const updater = await openUpdater(page);
  await expect(updater.getByRole('switch', { name: '자동으로 업데이트 확인', exact: true })).toHaveCount(0);
});
