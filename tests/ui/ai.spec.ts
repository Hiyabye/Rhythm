import { expect, test } from '@playwright/test';
import { callCount, mockDesktop } from './fixture';

async function reader(page: import('@playwright/test').Page) {
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await page.locator('#notice-1').click();
  await expect(page.getByRole('region', { name: '공지 AI 분석' })).toBeVisible();
}

test('first-run AI setup never blocks archived notices or starts a download', async ({ page }) => {
  await mockDesktop(page, { aiSetupNeeded: true }); await page.goto('/');
  await expect(page.getByRole('button', { name: '로컬 AI 설정하기' })).toBeVisible();
  expect(await callCount(page, 'ai_install')).toBe(0);
  await page.getByRole('button', { name: '나중에', exact: true }).click();
  await reader(page);
  await expect(page.getByText('보관된 한국어 본문입니다.', { exact: true })).toBeVisible();
  await expect(page.getByText('분석하려면 로컬 모델을 설치하고 선택해 주세요.', { exact: false })).toBeVisible();
  await page.getByRole('button', { name: 'AI 모델 설정', exact: true }).click();
  await expect(page.getByRole('heading', { name: '로컬 AI 시작하기' })).toBeVisible();
});

test('catalog installation progress, cancellation, selection, loading and setup completion', async ({ page }) => {
  await mockDesktop(page, { aiSetupNeeded: true, aiDelay: 2300 }); await page.goto('/');
  await page.getByRole('button', { name: '로컬 AI 설정하기' }).click();
  await page.getByRole('button', { name: 'Qwen3.5 4B 다운로드 및 설치', exact: true }).click();
  await expect(page.getByRole('progressbar')).toBeVisible();
  await page.getByRole('button', { name: '다운로드 중단', exact: true }).click();
  await expect(page.getByText('중단됨 · 이어받기 가능', { exact: false })).toBeVisible();
  await page.getByRole('button', { name: 'Qwen3.5 4B 설치 다시 시도 / 이어받기', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Qwen3.5 4B 선택', exact: true })).toBeVisible({ timeout: 10000 });
  await page.getByRole('button', { name: 'Qwen3.5 4B 선택', exact: true }).click();
  await page.getByRole('button', { name: '선택한 모델 불러오기', exact: true }).click();
  await expect(page.getByText('모델 준비됨', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '실제 추론 확인 후 AI 설정 완료' }).click();
  await expect(page.getByRole('heading', { name: '로컬 AI 시작하기' })).toHaveCount(0);
  await page.getByRole('button', { name: '모델 메모리 해제', exact: true }).click();
  expect(await callCount(page, 'ai_unload')).toBe(1);
});

test('installation failure exposes retry without hiding local archives', async ({ page }) => {
  await mockDesktop(page, { aiInstallError: true }); await page.goto('/');
  await page.getByRole('button', { name: '설정', exact: true }).click();
  await page.getByRole('button', { name: 'Qwen3.5 4B 다운로드 및 설치', exact: true }).click();
  await expect(page.getByText('모델 다운로드에 실패했습니다. 연결을 확인하고 이어받아 주세요.', { exact: true })).toBeVisible({ timeout: 10000 });
  await expect(page.getByRole('button', { name: 'Qwen3.5 4B 설치 다시 시도 / 이어받기', exact: true })).toBeEnabled();
  await reader(page); await expect(page.getByText('보관된 한국어 본문입니다.', { exact: true })).toBeVisible();
});

test('real reader UI submits once, shows grounded output, then reuses stored analysis', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true }); await page.goto('/'); await reader(page);
  const panel = page.getByRole('region', { name: '공지 AI 분석' });
  await panel.getByRole('button', { name: '이 공지 분석하기', exact: true }).click();
  await expect(panel.getByText('이 기기에서 공지를 분석하고 있습니다…', { exact: true })).toBeVisible();
  await expect(panel.getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
  await panel.getByText('원문 근거', { exact: true }).click();
  await expect(panel.getByText('2026년 10월 30일까지 신청서를 제출하세요.', { exact: true })).toBeVisible();
  await expect(panel.getByText('AI 분류 · 장학', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: /목록으로 돌아가기/ }).click();
  await page.locator('#notice-1').click();
  await expect(panel.getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
  expect(await callCount(page, 'ai_analyze')).toBe(1);
  await panel.getByRole('button', { name: '이 공지 분석 다시 생성', exact: true }).click();
  expect(await callCount(page, 'ai_analyze')).toBe(2);
});

test('a completion status cannot strand an earlier result snapshot in the empty state', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiResultLag: true }); await page.goto('/'); await reader(page);
  await page.getByRole('button', { name: '이 공지 분석하기', exact: true }).click();
  await expect(page.getByRole('region', { name: '공지 AI 분석' }).getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
  expect(await callCount(page, 'ai_analyze')).toBe(1);
  expect(await callCount(page, 'ai_results')).toBeGreaterThanOrEqual(3);
});

test('running analysis can be cancelled while original remains available', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiDelay: 10000 }); await page.goto('/'); await reader(page);
  await page.getByRole('button', { name: '이 공지 분석하기', exact: true }).click();
  await expect(page.getByText('보관된 한국어 본문입니다.', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '이 공지 분석 취소', exact: true }).click();
  await expect(page.getByText('분석을 취소했습니다. 기존 결과는 유지됩니다.', { exact: true })).toBeVisible();
  expect(await callCount(page, 'ai_cancel')).toBe(1);
  await expect(page.getByRole('button', { name: '이 공지 분석하기', exact: true })).toBeEnabled();
});

test('failed regeneration keeps good cached analysis and offers retry', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiResult: true, aiInferenceError: true }); await page.goto('/'); await reader(page);
  const panel = page.getByRole('region', { name: '공지 AI 분석' });
  await panel.getByRole('button', { name: '이 공지 분석 다시 생성', exact: true }).click();
  await expect(panel.getByRole('alert')).toHaveText('로컬 추론에 실패했습니다. 다시 요청해 주세요.');
  await expect(panel.getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
  await expect(panel.getByRole('button', { name: '이 공지 분석 다시 생성', exact: true })).toBeEnabled();
});

test('cached output is escaped, partial processing is honest, disabled connector retains AI', async ({ page }) => {
  await mockDesktop(page, { enabled: false, aiResult: true, aiPartial: true, aiText: '<img src=x onerror="window.__injected=true">' }); await page.goto('/'); await reader(page);
  const panel = page.getByRole('region', { name: '공지 AI 분석' });
  await expect(panel.getByText('<img src=x onerror="window.__injected=true">', { exact: true })).toBeVisible();
  await expect(panel.locator('img')).toHaveCount(0);
  await expect(panel.getByText('긴 공지의 일부만 분석했습니다', { exact: false })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__injected)).toBeUndefined();
  expect(await callCount(page, 'ai_analyze')).toBe(0);
});

test('model switching and confirmed external unregister preserve results', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiResult: true }); await page.goto('/');
  await page.getByRole('button', { name: '설정', exact: true }).click();
  await page.getByText('이미 가진 로컬 GGUF 등록', { exact: true }).click();
  await page.getByLabel('로컬 GGUF 전체 경로').fill('/existing/my-model.gguf');
  await page.getByRole('button', { name: '외부 파일 참조로 등록', exact: true }).click();
  await page.getByRole('button', { name: '외부 로컬 모델 선택', exact: true }).click();
  await expect(page.getByText('선택한 모델 · 설치 완료', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '참조 해제', exact: true }).click();
  expect(await callCount(page, 'ai_remove')).toBe(0);
  await expect(page.getByText('등록된 참조만 해제합니다. 외부 원본 파일은 삭제하지 않습니다.', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '확인하고 참조 해제', exact: true }).click();
  expect(await callCount(page, 'ai_remove')).toBe(1);
  await reader(page); await expect(page.getByRole('region', { name: '공지 AI 분석' }).getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
});

test('offline cached reading and regeneration do not require a browser network connection', async ({ page, context }) => {
  await mockDesktop(page, { aiInstalled: true, aiResult: true }); await page.goto('/');
  await context.setOffline(true);
  await reader(page);
  const panel = page.getByRole('region', { name: '공지 AI 분석' });
  await expect(panel.getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
  expect(await callCount(page, 'ai_analyze')).toBe(0);
  await panel.getByRole('button', { name: '이 공지 분석 다시 생성', exact: true }).click();
  await expect(panel.getByText('분석을 저장했습니다. 다음에는 추론 없이 바로 표시합니다.', { exact: true })).toBeVisible();
  expect(await callCount(page, 'ai_analyze')).toBe(1);
});

test('switching document revision never presents another revision cached result as current', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiResult: true, revisions: true }); await page.goto('/'); await reader(page);
  const panel = page.getByRole('region', { name: '공지 AI 분석' });
  await expect(panel.getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
  await page.getByLabel('보관 이력').selectOption('2');
  await expect(panel.getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toHaveCount(0);
  await expect(panel.getByRole('button', { name: '이 공지 분석하기', exact: true })).toBeVisible();
  await page.getByLabel('보관 이력').selectOption('1');
  await expect(panel.getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
  expect(await callCount(page, 'ai_analyze')).toBe(0);
});

test('today uses existing summaries and dates without scheduling inference', async ({ page }) => {
  await mockDesktop(page, { aiResult: true }); await page.goto('/');
  await expect(page.getByRole('region', { name: '최근 분석한 소식' })).toBeVisible();
  await expect(page.getByText('신청 마감 · 2026-10-30 · 원문에서 추출한 참고 일정', { exact: true })).toBeVisible();
  expect(await callCount(page, 'ai_analyze')).toBe(0);
});
