import { test, expect } from '@playwright/test';
import type { Page } from '@playwright/test';
import { mockDesktop, callCount, answerDialogs, dialogMessages } from './fixture';

type FixtureWindow = Window & { __fixture: { calls: { command: string; args: Record<string, unknown> }[] } };

async function openSettings(page: Page) {
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '설정', exact: true }).click();
}

async function openBackup(page: Page) {
  await openSettings(page);
  return page.locator('#data-backup');
}

async function openUpdater(page: Page) {
  await openSettings(page);
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
  await expect(backup).toContainText('형식 v1 · 데이터베이스 스키마 6 · 공지 12건 · 첨부 자료 3개');
  await expect(backup).toContainText('모델 가중치는 포함되지 않습니다.');
  await expect(backup).toContainText('자동으로 다시 열리지 않으니 Rhythm을 직접 다시 열어 주세요.');
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
  await expect(updater).toContainText('자동 업데이트는 제공하지 않습니다');
  await expect(updater.getByRole('status')).toHaveCount(0);
  await expect(updater.getByRole('listitem')).toHaveCount(6);
  await expect(updater).toContainText('Rhythm을 완전히 종료합니다.');
  await expect(updater).toContainText('shasum -a 256');
  await expect(updater).toContainText('github.com/Hiyabye/Rhythm');
  await expect(updater).toContainText('Apple 공증을 받지 않았습니다');
  await expect(updater).not.toContainText('공증된');
  await expect(updater).toContainText('공지, 설정, AI 모델은 앱과 별도로 저장되어 앱을 바꿔도 지워지지 않습니다.');
  await expect(updater.getByRole('button', { name: '업데이트 확인', exact: true })).toHaveCount(0);
  await expect(updater.getByRole('alert')).toHaveCount(0);
  await expect(updater).toContainText('1.0.0');
  expect(await callCount(page, 'updater_configured')).toBe(1);
  expect(await callCount(page, 'updater_check')).toBe(0);
});

test('configured updater reports genuine check failures', async ({ page }) => {
  await mockDesktop(page, { updaterConfigured: true, updaterCheckError: '업데이트 정보를 확인하지 못했습니다: 네트워크 오류' });
  await page.goto('/');
  const updater = await openUpdater(page);
  await updater.getByRole('button', { name: '업데이트 확인', exact: true }).click();

  await expect(updater.getByRole('alert')).toContainText('업데이트 정보를 확인하지 못했습니다');
  await expect(updater.getByText('현재 최신 버전을 사용하고 있습니다.', { exact: true })).toHaveCount(0);
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
});

test('a finalized restore is reported once the reopened window loads', async ({ page }) => {
  await mockDesktop(page, { restoreOutcome: { documents: 1240, resources: 37, unavailable_resources: 2, models_needing_reinstall: 1 } });
  await page.goto('/');
  const banner = page.getByRole('status').filter({ hasText: '백업 복원을 마쳤습니다' });
  await expect(banner).toContainText('공지 1,240건과 첨부 자료 37개를 복원하고 무결성을 확인했습니다.');
  await expect(banner).toContainText('내려받지 못한 첨부 자료 2개는 포함되지 않았습니다.');
  await expect(banner).toContainText('AI 모델 1개는 다시 설치해야 합니다.');
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
