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
  // Limits are stated next to the result, not only in fine print.
  const limits = panel.getByRole('note').filter({ hasText: 'AI가 만든 참고용 정리입니다.' });
  await expect(limits).toContainText('틀리거나 빠질 수 있습니다');
  await expect(limits).toContainText('첨부 파일과 이미지의 내용은 분석하지 않았습니다');
  await expect(limits).toContainText('원문에서 직접 확인하세요');
  await expect(panel).toContainText('근거가 있어도 AI의 해석은 틀릴 수 있습니다');
  await expect(panel).not.toContainText('(원문에서 추출)');
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

test('load phases survive settings remount and remain cancellable without blocking reading', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiLoadDelay: 20000 }); await page.goto('/');
  await page.getByRole('button', { name: '설정', exact: true }).click();
  await page.getByRole('button', { name: '선택한 모델 불러오기', exact: true }).click();
  await expect(page.getByText('모델 파일 무결성을 확인하고 있습니다 · 경과', { exact: false })).toBeVisible();
  await reader(page);
  await expect(page.getByText('보관된 한국어 본문입니다.', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: '이 공지 분석하기', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: '설정', exact: true }).click();
  await expect(page.getByText('모델 파일 무결성을 확인하고 있습니다 · 경과', { exact: false })).toBeVisible();
  await expect(page.getByRole('button', { name: '선택한 모델 불러오기', exact: true })).toBeDisabled();
  const cancel = page.getByRole('button', { name: '현재 AI 작업 취소', exact: true });
  await cancel.focus(); await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: '선택한 모델 불러오기', exact: true })).toBeEnabled();
  expect(await callCount(page, 'ai_cancel_operation')).toBe(1);
  expect(await callCount(page, 'ai_analyze')).toBe(0);
  await page.evaluate(() => { (window as any).__fixture.options.aiLoadDelay = 0; });
  await page.getByRole('button', { name: '선택한 모델 불러오기', exact: true }).click();
  await expect(page.getByText('모델 준비됨', { exact: true })).toBeVisible();
});

test('pending job cancellation bypasses a long command-owned model operation', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiLoadDelay: 20000 }); await page.goto('/');
  await page.getByRole('button', { name: '설정', exact: true }).click();
  await page.getByRole('button', { name: '선택한 모델 불러오기', exact: true }).click();
  await page.evaluate(() => {
    (window as any).__fixture.ai.jobs.push({ id: 101, document_id: 1, revision_id: 1, status: 'pending', error: null });
  });
  await page.getByRole('button', { name: '분석 취소', exact: true }).click();
  expect(await callCount(page, 'ai_cancel')).toBe(1);
  expect(await page.evaluate(() => (window as any).__fixture.ai.jobs[0].status)).toBe('cancelled');
  await expect(page.getByRole('button', { name: '선택한 모델 불러오기', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: '현재 AI 작업 취소', exact: true }).click();
  await expect(page.getByRole('button', { name: '선택한 모델 불러오기', exact: true })).toBeEnabled();
});

test('setup reports real inference phase and cancellation never marks setup complete', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiSetupNeeded: true, aiSetupDelay: 20000 }); await page.goto('/');
  await page.getByRole('button', { name: '로컬 AI 설정하기', exact: true }).click();
  await page.getByRole('button', { name: '선택한 모델 불러오기', exact: true }).click();
  await page.getByRole('button', { name: '실제 추론 확인 후 AI 설정 완료' }).click();
  await expect(page.getByText('짧은 로컬 추론으로 설치를 확인하고 있습니다 · 경과', { exact: false })).toBeVisible();
  await page.getByRole('button', { name: '현재 AI 작업 취소', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveText('AI 작업을 취소했습니다.');
  await expect(page.getByRole('heading', { name: '로컬 AI 시작하기' })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__fixture.ai.preferences.setup_completed)).toBe(false);
  await page.evaluate(() => { (window as any).__fixture.options.aiSetupDelay = 0; });
  await page.getByRole('button', { name: '실제 추론 확인 후 AI 설정 완료' }).click();
  await expect(page.getByRole('heading', { name: '로컬 AI 시작하기' })).toHaveCount(0);
});

test('initial analysis read failure has explicit retry and clears after successful reading', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiResult: true }); await page.goto('/');
  await page.evaluate(() => { (window as any).__fixture.resultsFailures = 1; });
  await reader(page);
  const panel = page.getByRole('region', { name: '공지 AI 분석' });
  await expect(panel.getByRole('alert')).toHaveText('분석 정보를 일시적으로 확인하지 못했습니다.');
  await expect(panel.getByText('아직 분석하지 않은 공지입니다.', { exact: false })).toHaveCount(0);
  await panel.getByRole('button', { name: '분석 정보 다시 확인', exact: true }).click();
  await expect(panel.getByRole('alert')).toHaveCount(0);
  await expect(panel.getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
});

test('successful model refresh clears a transient read error', async ({ page }) => {
  // Pause before mounting so there is no in-flight periodic refresh to race
  // this test's explicit retry.
  await page.clock.install({ time: new Date('2026-11-02T12:00:00Z') });
  await page.clock.pauseAt(new Date('2026-11-02T13:00:00Z'));
  await mockDesktop(page, { aiInstalled: true }); await page.goto('/');
  await page.getByRole('button', { name: '설정', exact: true }).click();
  await expect(page.getByRole('button', { name: '선택한 모델 불러오기', exact: true })).toBeVisible();
  await page.evaluate(() => { (window as any).__fixture.snapshotFailures = 1; });
  await page.getByRole('button', { name: 'AI 엔진 상태 새로고침', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveText('모델 정보를 일시적으로 확인하지 못했습니다.');
  await page.getByRole('button', { name: '모델 정보 다시 확인', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('polling recovers from a transient read error without hiding previous output', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiResult: true, aiDelay: 2500 }); await page.goto('/'); await reader(page);
  const panel = page.getByRole('region', { name: '공지 AI 분석' });
  await panel.getByRole('button', { name: '이 공지 분석 다시 생성', exact: true }).click();
  await page.evaluate(() => { (window as any).__fixture.resultsFailures = 1; });
  await expect(panel.getByRole('alert')).toHaveText('분석 정보를 일시적으로 확인하지 못했습니다.');
  await expect(panel.getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
  await expect(panel.getByText('분석을 저장했습니다. 다음에는 추론 없이 바로 표시합니다.', { exact: true })).toBeVisible();
  await expect(panel.getByRole('alert')).toHaveCount(0);
});

for (const change of ['prompt', 'schema', 'configuration']) {
  test(`backend ${change} contract marks preserved analysis outdated without reanalysis`, async ({ page }) => {
    await mockDesktop(page, { aiInstalled: true, aiResult: true }); await page.goto('/');
    await page.evaluate((change) => {
      const contract = (window as any).__fixture.ai.analysis_contract;
      if (change === 'configuration') contract.configuration_hash = 'c'.repeat(64);
      else if (change === 'schema') contract.schema_version += 1;
      else contract.prompt_version += 1;
    }, change);
    await reader(page);
    const panel = page.getByRole('region', { name: '공지 AI 분석' });
    await expect(panel.getByText('이 결과는 현재 모델 또는 분석 방식·설정과 다릅니다.', { exact: false })).toBeVisible();
    await expect(panel.getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
    expect(await callCount(page, 'ai_analyze')).toBe(0);
  });
}

test('reader uses revision-local failure history outside the global history window', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiResult: true }); await page.goto('/');
  await page.evaluate(() => {
    const fixture = (window as any).__fixture;
    fixture.documentJobs.push({ ...fixture.analyses[0], id: 1, status: 'failed', error: '이 공지의 이전 분석에 실패했습니다.' });
    fixture.ai.jobs = Array.from({ length: 101 }, (_, i) => ({ ...fixture.analyses[0], id: i + 2, document_id: 2, status: 'completed', error: null }));
  });
  await reader(page);
  await expect(page.getByRole('region', { name: '공지 AI 분석' }).getByRole('alert')).toHaveText('이 공지의 이전 분석에 실패했습니다.');
  expect(await callCount(page, 'ai_document_jobs')).toBeGreaterThan(0);
});

test('reader prefers active work over newer terminal history and shows actual phase', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiResult: true }); await page.goto('/');
  await page.evaluate(() => {
    const fixture = (window as any).__fixture;
    fixture.documentJobs = [{ ...fixture.analyses[0], id: 1, status: 'running', error: null }, { ...fixture.analyses[0], id: 9, status: 'failed', error: '다른 설정의 이전 실패' }];
    fixture.ai.jobs = [fixture.documentJobs[1]];
    fixture.ai.progress = { job_id: 1, phase: 'integrity', elapsed_ms: 12000, stages_ms: {}, attempts: [] };
  });
  await reader(page);
  const panel = page.getByRole('region', { name: '공지 AI 분석' });
  await expect(panel.getByText('모델 파일 무결성을 확인하고 있습니다 · 경과 12초', { exact: true })).toBeVisible();
  await expect(panel.getByRole('alert')).toHaveCount(0);
  await panel.getByRole('button', { name: '이 공지 분석 취소', exact: true }).click();
  const calls = await page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'ai_cancel'));
  expect(calls[0].args.id).toBe(1);
});

test('resource guidance is honest at narrow widths and operation cancellation is keyboard accessible', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiLowResources: true, aiLoadDelay: 20000 }); await page.goto('/');
  await page.setViewportSize({ width: 640, height: 900 });
  await page.getByRole('button', { name: '설정', exact: true }).click();
  await expect(page.getByText('현재 남은 저장 공간 확인하지 못함', { exact: false })).toBeVisible();
  await expect(page.getByText('이 기기의 메모리와 저장 공간을 확인했습니다', { exact: false })).toHaveCount(0);
  await expect(page.getByText('물리 메모리는 지금 사용할 수 있는 메모리와 다릅니다.', { exact: false })).toBeVisible();
  for (const theme of ['다크', '라이트']) {
    await page.getByRole('button', { name: theme, exact: true }).click();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  }
  await page.getByRole('button', { name: '선택한 모델 불러오기', exact: true }).click();
  const cancel = page.getByRole('button', { name: '현재 AI 작업 취소', exact: true });
  await expect(cancel).toBeVisible(); await cancel.focus(); await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: '선택한 모델 불러오기', exact: true })).toBeEnabled();
});

test('today uses existing summaries and dates without scheduling inference', async ({ page }) => {
  await mockDesktop(page, { aiResult: true }); await page.goto('/');
  await expect(page.getByRole('region', { name: '최근 분석한 소식' })).toBeVisible();
  await expect(page.getByText('신청 마감 · 2026-10-30 · 원문에서 추출한 참고 일정', { exact: true })).toBeVisible();
  expect(await callCount(page, 'ai_analyze')).toBe(0);
});

test('managed model removal can keep files after an inaccessible folder error and still requires confirmation', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiResult: true, aiRemoveError: true });
  await page.goto('/');
  await page.getByRole('button', { name: '설정', exact: true }).click();
  await page.getByRole('button', { name: '모델 삭제', exact: true }).click();
  const confirmation = page.getByRole('group', { name: 'Qwen3.5 4B 삭제 확인', exact: true });
  await expect(confirmation).toContainText('폴더에 접근할 수 없거나 다른 위치에 다시 설치하려면 파일을 남기고 목록에서 제거하세요.');
  await confirmation.getByRole('button', { name: '취소', exact: true }).click();
  await expect(confirmation).toHaveCount(0);
  expect(await callCount(page, 'ai_remove')).toBe(0);
  await page.getByRole('button', { name: '모델 삭제', exact: true }).click();
  await confirmation.getByRole('button', { name: '확인하고 삭제', exact: true }).click();
  await expect(page.getByRole('alert').filter({ hasText: '모델 폴더에 접근할 수 없습니다.' })).toBeVisible();
  await expect(page.locator('.ai-installed')).toHaveCount(1);
  await page.getByRole('button', { name: '모델 삭제', exact: true }).click();
  await page.evaluate(() => { (window as any).__fixture.ai.operation = { kind: 'load', phase: 'verifying', elapsed_ms: 0, cancelling: false }; });
  await page.getByRole('button', { name: 'AI 엔진 상태 새로고침', exact: true }).click();
  await expect(confirmation.getByRole('button', { name: '파일을 남기고 목록에서 제거', exact: true })).toBeDisabled();
  await page.evaluate(() => { (window as any).__fixture.ai.operation = null; });
  await page.getByRole('button', { name: 'AI 엔진 상태 새로고침', exact: true }).click();
  await confirmation.getByRole('button', { name: '파일을 남기고 목록에서 제거', exact: true }).click();
  await expect(page.locator('.ai-installed')).toHaveCount(0);
  await expect(page.getByText('파일을 남기고 모델을 목록에서 제거했습니다. 기존 분석은 유지됩니다.', { exact: true })).toBeVisible();
  const calls = await page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'ai_remove'));
  expect(calls).toHaveLength(2);
  expect(calls[0].args).toEqual({ id: 'qwen35-4b', confirmed: true });
  expect(calls[1].args).toEqual({ id: 'qwen35-4b', confirmed: true, keepFiles: true });
  await reader(page);
  await expect(page.getByRole('region', { name: '공지 AI 분석' }).getByText('장학금 신청 일정과 제출 서류를 확인하세요.', { exact: true })).toBeVisible();
});

test('AI setup stays visible after a delayed long source list and settings links reach their sections', async ({ page }) => {
  await mockDesktop(page, { aiSetupNeeded: true, sourceCount: 32, sourceDelay: 1200 });
  await page.goto('/');
  await page.getByRole('button', { name: '로컬 AI 설정하기', exact: true }).click();
  const heading = page.locator('#ai h2');
  await expect(heading).toBeFocused();
  await expect(page.getByLabel('서강대학교 공지 정보원 관리')).toHaveAttribute('aria-busy', 'false');
  await expect(heading).toBeInViewport();
  const nav = page.getByRole('navigation', { name: '설정 항목' });
  for (const [label, id] of [['일반', 'general'], ['확장 기능', 'extensions'], ['AI 모델', 'ai'], ['AI 처리', 'ai-processing'], ['개인화', 'personalization'], ['백그라운드', 'background'], ['알림', 'notifications']]) {
    await expect(nav.getByRole('link', { name: label, exact: true })).toHaveAttribute('href', `#${id}`);
  }
  await nav.getByRole('link', { name: '일반', exact: true }).click();
  await nav.getByRole('link', { name: 'AI 모델', exact: true }).click();
  await expect(heading).toBeInViewport();
});

test('macOS hides the Windows execution device and keeps a chosen model folder for new downloads', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true });
  await page.goto('/');
  await page.getByRole('button', { name: '설정', exact: true }).click();
  await expect(page.getByLabel('AI 실행 장치', { exact: true })).toHaveCount(0);
  await expect(page.getByPlaceholder('/전체/경로/model.gguf')).toHaveCount(1);
  const storage = page.getByRole('group', { name: '모델 설치 폴더', exact: true });
  await expect(storage.getByText('/isolated/models', { exact: true })).toBeVisible();
  await storage.getByRole('button', { name: '설치 폴더 선택', exact: true }).click();
  await expect(storage.getByText('/Volumes/로컬 모델/Rhythm', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Qwen3.5 2B 다운로드 및 설치', exact: true }).click();
  const added = page.locator('.ai-installed').filter({ has: page.getByRole('heading', { name: 'Qwen3.5 2B', exact: true }) });
  await expect(added.getByText('/Volumes/로컬 모델/Rhythm/qwen35-2b.gguf', { exact: true })).toBeVisible();
  expect(await callCount(page, 'ai_preferences')).toBe(0);
});
