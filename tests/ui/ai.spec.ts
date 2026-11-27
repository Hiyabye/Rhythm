import { expect, test } from '@playwright/test';
import { callCount, mockDesktop, openSettings } from './fixture';
import { localDayKey } from '../../src/lib/archive-format';

async function advanced(page: import('@playwright/test').Page) {
  await page.getByText('고급', { exact: true }).click();
}

const removals = (page: import('@playwright/test').Page) =>
  page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'ai_remove').map((call: any) => call.args));

test('the reader and list show no summaries, and AI settings offer only the classification model', async ({ page }) => {
  await mockDesktop(page, { aiEmbeddingInstalled: true }); await page.goto('/');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await expect(page.locator('#notice-1')).not.toContainText('AI 요약');
  await page.locator('#notice-1').click();
  await expect(page.getByRole('region', { name: '보관된 공지 읽기' }).getByRole('heading', { level: 2 })).toHaveText('한국어 공지 01');
  await expect(page.getByRole('region', { name: '공지 AI 분석' })).toHaveCount(0);
  await page.getByRole('button', { name: '목록으로 돌아가기' }).click();
  await openSettings(page, 'AI');
  await expect(page.getByRole('group', { name: '공지 분류' })).toContainText('Qwen3-Embedding 0.6B');
  await expect(page.getByText('더 이상 제공하지 않는 모델', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /다운로드 및 설치/ })).toHaveCount(0);
  for (const command of ['ai_analyze', 'ai_results', 'ai_recent', 'ai_load']) expect(await callCount(page, command)).toBe(0);
});

test('installing the classification model shows progress and selects it', async ({ page }) => {
  await mockDesktop(page, { aiDelay: 400 }); await page.goto('/');
  await openSettings(page, 'AI');
  const section = page.getByRole('group', { name: '공지 분류' });
  await expect(section).toContainText('설치하지 않음');
  await expect(section).toContainText('AI 요약과 원격 AI는 더 이상 제공하지 않습니다.');
  await section.getByRole('button', { name: '분류 모델 설치', exact: true }).click();
  await expect(section.getByText('분류 모델 다운로드')).toBeAttached();
  await expect(section.locator('#ai-classification-enabled')).toBeChecked();
  await expect(section).not.toContainText('더 이상 제공하지 않습니다.');
  expect(await page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'ai_install').map((call: any) => call.args))).toEqual([{ catalogId: 'qwen3-embedding-0-6b' }]);
});

test('installation failure exposes retry without hiding local archives', async ({ page }) => {
  await mockDesktop(page, { aiInstallError: true, aiDelay: 100 }); await page.goto('/');
  await openSettings(page, 'AI');
  const section = page.getByRole('group', { name: '공지 분류' });
  await section.getByRole('button', { name: '분류 모델 설치', exact: true }).click();
  await expect(section.getByRole('alert')).toHaveText('모델 다운로드에 실패했습니다. 연결을 확인하고 이어받아 주세요.');
  await expect(section.getByRole('button', { name: '다시 설치', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await expect(page.locator('#notice-1')).toBeVisible();
});

test('embedding classification is shown independently and can be toggled', async ({ page }) => {
  await mockDesktop(page, { aiEmbeddingInstalled: true }); await page.goto('/');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  // The list and reader show the stored classification.
  await expect(page.locator('#notice-1')).toContainText('분류 · 장학');
  await page.locator('#notice-1').click();
  await expect(page.getByRole('region', { name: '보관된 공지 읽기' }).getByText('분류 · 장학', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '목록으로 돌아가기' }).click();
  await openSettings(page, 'AI');
  const section = page.getByRole('group', { name: '공지 분류' });
  await expect(section).toContainText('Qwen3-Embedding 0.6B');
  const toggle = page.locator('#ai-classification-enabled');
  await expect(toggle).toBeChecked();
  await toggle.uncheck();
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'ai_classification_preferences').map((call: any) => call.args.enabled))).toEqual([false]);
});

test('low-confidence classification is marked without leaking the raw score', async ({ page }) => {
  await mockDesktop(page, { aiEmbeddingInstalled: true, aiClassificationLow: true }); await page.goto('/');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await expect(page.locator('#notice-1')).toContainText('분류 · 장학 · 확신 낮음');
  await expect(page.locator('#notice-1')).not.toContainText('0.');
});

test('AI category chips filter the list, count each category and note unclassified notices', async ({ page }) => {
  await mockDesktop(page, { count: 4, aiEmbeddingInstalled: true, aiCategoryOf: { 2: '학사', 4: null } }); await page.goto('/');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await expect(page.getByLabel('게시판 분류')).toBeVisible();
  const chips = page.getByRole('group', { name: 'AI 분류' });
  // Only categories with notices are offered, in the classifier's order.
  await expect(chips.getByRole('button')).toHaveText(['전체', '학사 1', '장학 2']);
  await expect(chips.getByRole('button', { name: '전체' })).toHaveAttribute('aria-pressed', 'true');
  await chips.getByRole('button', { name: '장학 2' }).click();
  await expect(chips.getByRole('button', { name: '장학 2' })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.notice-list #notice-1')).toBeVisible();
  await expect(page.locator('.notice-list #notice-3')).toBeVisible();
  await expect(page.locator('.notice-list #notice-2')).toHaveCount(0);
  await expect(page.locator('.notice-list #notice-4')).toHaveCount(0);
  await expect(page.getByText('아직 분류되지 않은 공지 1건은 이 목록에 없습니다.')).toBeVisible();
  const lastQuery = () => page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'archive_query').at(-1).args.query);
  expect(await lastQuery()).toMatchObject({ ai_category: '장학', offset: 0 });
  await chips.getByRole('button', { name: '전체' }).click();
  await expect(page.locator('.notice-list #notice-4')).toBeVisible();
  await expect(page.getByText(/아직 분류되지 않은 공지/)).toHaveCount(0);
  expect(await lastQuery()).toMatchObject({ ai_category: null });
  // Removing the model while a category is chosen brings the whole list back.
  await chips.getByRole('button', { name: '학사 1' }).click();
  await expect(page.locator('.notice-list #notice-1')).toHaveCount(0);
  await page.evaluate(() => { (window as any).__fixture.ai.preferences.embedding_model_id = null; });
  await page.getByRole('button', { name: '목록 새로고침' }).click();
  await expect(chips).toHaveCount(0);
  await expect(page.locator('.notice-list #notice-1')).toBeVisible();
  await expect.poll(lastQuery).toMatchObject({ ai_category: null });
});

test('AI category chips stay hidden without an installed embedding model', async ({ page }) => {
  await mockDesktop(page, { count: 2 }); await page.goto('/');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await expect(page.locator('#notice-1')).toBeVisible();
  await expect(page.getByRole('group', { name: 'AI 분류' })).toHaveCount(0);
});

test('reader lists similar notices and opens one', async ({ page }) => {
  await mockDesktop(page, { count: 3, aiEmbeddingInstalled: true, aiSimilar: true }); await page.goto('/');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await page.locator('.notice-list #notice-1').click();
  const reader = page.getByRole('region', { name: '보관된 공지 읽기' });
  const section = reader.getByRole('region', { name: /비슷한 공지/ });
  await expect(section).toContainText('공과대학 공지');
  await expect(section).toContainText('같은 공지');
  await expect(section).toContainText('한국어 공지 03');
  await section.getByRole('button', { name: '한국어 공지 03 읽기' }).click();
  await expect(reader.getByRole('heading', { level: 2 })).toHaveText('한국어 공지 03');
  await page.getByRole('button', { name: '목록으로 돌아가기' }).click();
  await expect(page.locator('.notice-list #notice-3')).toBeFocused();
});

test('reader shows no similar section without an embedding model', async ({ page }) => {
  await mockDesktop(page, { count: 3, aiSimilar: true }); await page.goto('/');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await page.locator('.notice-list #notice-1').click();
  const reader = page.getByRole('region', { name: '보관된 공지 읽기' });
  await expect(reader.getByRole('heading', { level: 2 })).toHaveText('한국어 공지 01');
  await expect(reader.getByRole('region', { name: /비슷한 공지/ })).toHaveCount(0);
});

test('cross-posted notices fold under one row and expand on request', async ({ page }) => {
  await mockDesktop(page, { count: 3, aiEmbeddingInstalled: true, aiSimilar: true }); await page.goto('/');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  const list = page.locator('.notice-list');
  await expect(list.locator('#notice-1')).toContainText('같은 공지 1건 · 공과대학 공지');
  // The copy is folded, the merely similar notice is not.
  await expect(list.locator('#notice-2')).toHaveCount(0);
  await expect(list.locator('#notice-3')).toBeVisible();
  await expect(list.locator('#notice-3')).not.toContainText('같은 공지');
  const toggle = list.getByRole('button', { name: '같은 공지 1건 펼치기' });
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await toggle.click();
  await expect(list.locator('#notice-2')).toBeVisible();
  await expect(list.locator('li').nth(1)).toHaveClass(/notice-duplicate-member/);
  await list.getByRole('button', { name: '같은 공지 1건 접기' }).click();
  await expect(list.locator('#notice-2')).toHaveCount(0);
});

test('a read leader carries the unread state of its folded copy', async ({ page }) => {
  await mockDesktop(page, { count: 3, aiEmbeddingInstalled: true, aiSimilar: true }); await page.goto('/');
  await page.getByRole('button', { name: '받은 소식', exact: true }).click();
  await page.locator('.notice-list #notice-1').click();
  await page.getByRole('button', { name: '목록으로 돌아가기' }).click();
  const list = page.locator('.notice-list');
  const leader = list.locator('li').first();
  await expect(leader).toHaveClass(/notice-unread/);
  await expect(leader.locator('.notice-title .sr-only')).toHaveText('읽음, 접힌 같은 공지 1건 읽지 않음');
  const toggle = list.getByRole('button', { name: '같은 공지 1건 펼치기 · 읽지 않음 1건' });
  await toggle.click();
  // Expanded, each row shows its own state again.
  await expect(leader).not.toHaveClass(/notice-unread/);
  await expect(leader.locator('.notice-title .sr-only')).toHaveText('읽음');
  await expect(list.locator('li').nth(1)).toHaveClass(/notice-unread/);
  await expect(list.getByRole('button', { name: '같은 공지 1건 접기' })).toBeVisible();
});

test('retired summary models are listed only for removal', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiRetiredModel: true }); await page.goto('/');
  await openSettings(page, 'AI');
  const retired = page.getByRole('list', { name: '더 이상 제공하지 않는 모델' });
  await expect(retired.getByRole('listitem')).toHaveCount(2);
  await expect(retired).toContainText('Qwen3.5 4B');
  await expect(retired).toContainText('Qwen3.5 9B');
  await expect(retired.getByRole('button', { name: /설치|이어받기|선택|불러오기/ })).toHaveCount(0);
  const item = (name: string) => retired.getByRole('listitem').filter({ hasText: name });
  await item('Qwen3.5 9B').getByRole('button', { name: '모델 삭제', exact: true }).click();
  // Opening another confirmation closes the first one.
  await item('Qwen3.5 4B').getByRole('button', { name: '모델 삭제', exact: true }).click();
  await expect(page.getByRole('group', { name: 'Qwen3.5 9B 삭제 확인', exact: true })).toHaveCount(0);
  await page.getByRole('group', { name: 'Qwen3.5 4B 삭제 확인', exact: true }).getByRole('button', { name: '확인하고 삭제', exact: true }).click();
  await expect(item('Qwen3.5 4B')).toHaveCount(0);
  await item('Qwen3.5 9B').getByRole('button', { name: '모델 삭제', exact: true }).click();
  await page.getByRole('group', { name: 'Qwen3.5 9B 삭제 확인', exact: true }).getByRole('button', { name: '확인하고 삭제', exact: true }).click();
  await expect(retired).toHaveCount(0);
  expect(await removals(page)).toEqual([{ id: 'qwen35-4b', confirmed: true }, { id: 'qwen35-9b', confirmed: true }]);
});

test('managed model removal can keep files after an inaccessible folder error and still requires confirmation', async ({ page }) => {
  await mockDesktop(page, { aiInstalled: true, aiRemoveError: true }); await page.goto('/');
  await openSettings(page, 'AI');
  await page.getByRole('button', { name: '모델 삭제', exact: true }).click();
  const confirmation = page.getByRole('group', { name: 'Qwen3.5 4B 삭제 확인', exact: true });
  await expect(confirmation).toContainText('폴더에 접근할 수 없으면 파일은 두고 목록에서만 제거하세요.');
  await confirmation.getByRole('button', { name: '취소', exact: true }).click();
  await expect(confirmation).toHaveCount(0);
  expect(await callCount(page, 'ai_remove')).toBe(0);
  await page.getByRole('button', { name: '모델 삭제', exact: true }).click();
  await confirmation.getByRole('button', { name: '확인하고 삭제', exact: true }).click();
  await expect(page.getByRole('alert').filter({ hasText: '모델 폴더에 접근할 수 없습니다.' })).toBeVisible();
  await expect(page.getByRole('list', { name: '더 이상 제공하지 않는 모델' })).toContainText('Qwen3.5 4B');
  await page.getByRole('button', { name: '모델 삭제', exact: true }).click();
  await confirmation.getByRole('button', { name: '파일을 남기고 목록에서 제거', exact: true }).click();
  await expect(page.getByRole('list', { name: '더 이상 제공하지 않는 모델' })).toHaveCount(0);
  await expect(page.getByText('파일을 남기고 모델을 목록에서 제거했습니다.', { exact: true })).toBeVisible();
  expect(await removals(page)).toEqual([{ id: 'qwen35-4b', confirmed: true }, { id: 'qwen35-4b', confirmed: true, keepFiles: true }]);
});

test('a chosen model folder applies to new downloads', async ({ page }) => {
  await mockDesktop(page, {}); await page.goto('/');
  await openSettings(page, 'AI');
  await advanced(page);
  await expect(page.getByPlaceholder('/전체/경로/model.gguf')).toHaveCount(0);
  const storage = page.getByRole('group', { name: '모델 설치 폴더', exact: true });
  await expect(storage.getByText('/isolated/models', { exact: true })).toBeVisible();
  await storage.getByRole('button', { name: '설치 폴더 선택', exact: true }).click();
  await expect(storage.getByText('/Volumes/로컬 모델/Rhythm', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '분류 모델 설치', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.ai.models[0]?.path)).toBe('/Volumes/로컬 모델/Rhythm/qwen3-embedding-0-6b.gguf');
});

test('기회 lists opportunity categories by nearest open deadline and can show closed ones', async ({ page }) => {
  const day = (offset: number) => { const now = new Date(); return localDayKey(new Date(now.getFullYear(), now.getMonth(), now.getDate() + offset)); };
  const deadline = (document_id: number, offset: number, time: string | null = null) => ({ document_id, ordinal: 0, title: `한국어 공지 0${document_id}`, day: day(offset), time, label: '신청 마감', kind: 'deadline' as const });
  await mockDesktop(page, { count: 6, aiEmbeddingInstalled: true,
    aiCategoryOf: { 1: '공모전·대회', 2: '인턴·채용', 3: '교육·프로그램', 4: '행사·활동', 5: '공모전·대회', 6: '장학' },
    noticeDates: [deadline(1, 5), deadline(2, 1, '18:00'), deadline(3, 0), deadline(5, -1), deadline(6, 2)] });
  await page.goto('/');
  // The badge counts what the page labels 오늘 마감, not every opportunity.
  await expect(page.getByRole('button', { name: '기회', exact: true })).toHaveAccessibleDescription('오늘 마감 1개');
  await page.getByRole('button', { name: '기회', exact: true }).click();
  const rows = page.locator('.notice-list > li');
  // Nearest open deadline first, then the notice without one; closed and other categories stay out.
  await expect(rows.locator('.notice-title')).toHaveText([/한국어 공지 03/, /한국어 공지 02/, /한국어 공지 01/, /한국어 공지 04/]);
  await expect(page.locator('#notice-3 .notice-deadline')).toHaveText('오늘 마감');
  await expect(page.locator('#notice-2 .notice-deadline')).toHaveText(/^마감 .+ 오후 6:00 · D-1$/);
  await expect(page.locator('#notice-2 .notice-deadline')).toHaveClass(/soon/);
  await expect(page.locator('#notice-1 .notice-deadline')).toHaveText(/ · D-5$/);
  await expect(page.locator('#notice-1 .notice-deadline')).not.toHaveClass(/soon/);
  await expect(page.locator('#notice-4 .notice-deadline')).toHaveCount(0);
  await expect(page.getByLabel('정렬')).toHaveCount(0);
  const lastQuery = () => page.evaluate(() => (window as any).__fixture.calls.filter((call: any) => call.command === 'archive_query').at(-1).args.query);
  expect(await lastQuery()).toMatchObject({ opportunities: true, include_closed: false, offset: 0 });
  // The chips narrow within the four categories only.
  const chips = page.getByRole('group', { name: 'AI 분류' });
  await expect(chips.getByRole('button')).toHaveText(['전체', '공모전·대회 1', '인턴·채용 1', '교육·프로그램 1', '행사·활동 1']);
  await chips.getByRole('button', { name: '인턴·채용 1' }).click();
  await expect(rows).toHaveCount(1);
  await chips.getByRole('button', { name: '전체' }).click();

  await page.getByLabel('마감 지난 공지도 보기').check();
  await expect(rows).toHaveCount(5);
  await expect(page.locator('#notice-5 .notice-deadline')).toHaveText(/^마감 지남 · /);
  await expect(page.locator('#notice-5 .notice-deadline')).toHaveClass(/closed/);
  expect(await lastQuery()).toMatchObject({ opportunities: true, include_closed: true });
});

test('기회 is offered only with the AI extension', async ({ page }) => {
  await mockDesktop(page, { extensions: { ai: false } }); await page.goto('/');
  await expect(page.getByRole('button', { name: '받은 소식', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: '기회', exact: true })).toHaveCount(0);
});
