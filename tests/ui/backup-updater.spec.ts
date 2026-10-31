import { test, expect } from '@playwright/test';
import type { Page } from '@playwright/test';
import { mockDesktop, callCount } from './fixture';

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
  expect(await callCount(page, 'backup_inspect')).toBe(1);

  const confirmations: string[] = [];
  page.once('dialog', async (dialog) => {
    confirmations.push(dialog.message());
    await dialog.dismiss();
  });
  await backup.getByRole('button', { name: '이 백업으로 복원', exact: true }).click();
  await expect.poll(() => confirmations.length).toBe(1);
  expect(confirmations[0]).toContain('현재 Rhythm 데이터 전체');
  expect(await callCount(page, 'backup_restore')).toBe(0);
  await expect(backup.getByRole('heading', { name: '복원할 백업 확인', exact: true })).toBeVisible();

  page.once('dialog', (dialog) => dialog.accept());
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
  page.once('dialog', (dialog) => dialog.accept());
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

test('updater reports a missing signing key instead of showing a successful no-update result', async ({ page }) => {
  await mockDesktop(page, { updaterCheckError: '업데이트 공개 키가 설정되지 않았습니다.' });
  await page.goto('/');
  const updater = await openUpdater(page);
  await updater.getByRole('button', { name: '업데이트 확인', exact: true }).click();

  await expect(updater.getByRole('alert')).toContainText('업데이트 공개 키가 설정되지 않았습니다.');
  await expect(updater.getByText('현재 최신 버전을 사용하고 있습니다.', { exact: true })).toHaveCount(0);
  expect(await callCount(page, 'updater_check')).toBe(1);
});

test('updater installation errors remain recoverable', async ({ page }) => {
  await mockDesktop(page, { updaterInstallError: true });
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
  await mockDesktop(page);
  await page.goto('/');
  const updater = await openUpdater(page);
  await updater.getByRole('button', { name: '업데이트 확인', exact: true }).click();

  page.once('dialog', (dialog) => dialog.dismiss());
  await updater.getByRole('button', { name: '다운로드 및 설치', exact: true }).click();
  await expect(updater.getByText('서명된 업데이트를 준비했습니다.', { exact: true })).toBeVisible();
  await expect(updater.getByRole('button', { name: '설치 및 다시 시작', exact: true })).toBeVisible();
  expect(await callCount(page, 'updater_restart')).toBe(0);

  const confirmations: string[] = [];
  page.once('dialog', async (dialog) => {
    confirmations.push(dialog.message());
    await dialog.dismiss();
  });
  await updater.getByRole('button', { name: '설치 및 다시 시작', exact: true }).click();
  await expect.poll(() => confirmations.length).toBe(1);
  expect(confirmations[0]).toContain('서명 검증이 끝난 업데이트');
  expect(await callCount(page, 'updater_restart')).toBe(0);

  page.once('dialog', (dialog) => dialog.accept());
  await updater.getByRole('button', { name: '설치 및 다시 시작', exact: true }).click();
  await expect.poll(() => callCount(page, 'updater_restart')).toBe(1);
  const restart = await commandArgs(page, 'updater_restart');
  expect(restart).toEqual({ confirmed: true });
});

test('restart failure is surfaced after explicit confirmation', async ({ page }) => {
  await mockDesktop(page, { updaterRestartError: true });
  await page.goto('/');
  const updater = await openUpdater(page);
  await updater.getByRole('button', { name: '업데이트 확인', exact: true }).click();
  page.once('dialog', (dialog) => dialog.accept());
  await updater.getByRole('button', { name: '다운로드 및 설치', exact: true }).click();

  await expect(updater.getByRole('alert')).toContainText('업데이트를 적용하며 다시 시작하지 못했습니다.');
  expect(await callCount(page, 'updater_restart')).toBe(1);
});
