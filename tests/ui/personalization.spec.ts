import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { callCount, mockDesktop, answerDialogs, openSettings } from './fixture';

interface FixtureCall { command: string; args: Record<string, unknown> }
type FixtureWindow = Window & { __fixture: { calls: FixtureCall[] } };
async function navigate(page: Page, name: string) {
  await page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name, exact: true }).click();
}
async function fixtureCalls(page: Page): Promise<FixtureCall[]> {
  return page.evaluate(() => (window as unknown as FixtureWindow).__fixture.calls);
}

const explicitInterest = (id: number, name: string) => ({ id, name, aliases: [] as string[], priority: 3, polarity: 'positive' as const, origin: 'explicit' as const, evidence_count: 0, evidence_summary: null, updated_at: '2026-10-27' });
function personal(page: Page) {
  return page.locator('#personalization');
}
async function addTopic(page: Page, name: string, direction: '관심' | '관심 없음' = '관심') {
  const field = personal(page).getByLabel(`${direction} 주제 추가`, { exact: true });
  await field.fill(name);
  await field.press('Enter');
}
function editor(page: Page, name: string) {
  return page.getByRole('dialog', { name: `${name} 수정`, exact: true });
}

test('profile and structured interests persist through personalization IPC', async ({ page }) => {
  await mockDesktop(page); await page.goto('/'); await openSettings(page, '개인화');
  await personal(page).getByLabel('대학교').fill('서강대학교');
  await personal(page).getByLabel('학과').fill('컴퓨터공학과');
  await personal(page).getByLabel('학업 단계').fill('학부 3학년');
  await personal(page).getByRole('button', { name: '프로필 저장' }).click();
  await expect(personal(page).getByRole('status')).toContainText('개인화 설정을 저장했습니다.');
  await addTopic(page, '인공지능');
  await expect(personal(page).getByRole('list', { name: '관심', exact: true })).toContainText('인공지능');
  await expect(personal(page).getByLabel('관심 주제 추가', { exact: true })).toHaveValue('');
  await personal(page).getByRole('button', { name: '인공지능 관심 수정', exact: true }).click();
  await editor(page, '인공지능').getByLabel('별칭').fill('AI, machine learning');
  await editor(page, '인공지능').getByRole('button', { name: '저장', exact: true }).click();
  await expect(editor(page, '인공지능')).toBeHidden();
  const calls = await fixtureCalls(page);
  expect(calls.find((call) => call.command === 'personalization_save_profile')?.args.profile).toEqual({ university: '서강대학교', department: '컴퓨터공학과', study_level: '학부 3학년' });
  expect(calls.filter((call) => call.command === 'personalization_save_interest').map((call) => call.args.interest)).toMatchObject([
    { id: null, name: '인공지능', aliases: [], priority: 3, polarity: 'positive' },
    { id: 1, name: '인공지능', aliases: ['AI', 'machine learning'], polarity: 'positive' },
  ]);
});

test('typing during a save keeps the new draft in the add field and in the open editor', async ({ page }) => {
  await mockDesktop(page, { interestSaveDelay: 700, initialInterests: [explicitInterest(1, '분산 시스템')] }); await page.goto('/'); await openSettings(page, '개인화');
  const add = personal(page).getByLabel('관심 주제 추가', { exact: true });
  await add.fill('네트워크');
  await add.press('Enter');
  await add.fill('인턴십');
  await expect(personal(page).getByRole('status')).toContainText('개인화 설정을 저장했습니다.');
  await expect(add).toHaveValue('인턴십');
  await add.press('Enter');
  await expect(add).toHaveValue('');

  await personal(page).getByRole('button', { name: '분산 시스템 관심 수정', exact: true }).click();
  await editor(page, '분산 시스템').getByLabel('이름').fill('분산 컴퓨팅');
  await editor(page, '분산 시스템').getByRole('button', { name: '저장', exact: true }).click();
  await editor(page, '분산 시스템').getByRole('group', { name: '우선순위' }).getByRole('button', { name: '5', exact: true }).click();
  await expect(personal(page).getByRole('status')).toContainText('개인화 설정을 저장했습니다.');
  const edit = editor(page, '분산 컴퓨팅');
  // The edit made during the save stays open instead of being discarded.
  await expect(edit.getByRole('group', { name: '우선순위' }).getByRole('button', { name: '5', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await edit.getByRole('button', { name: '저장', exact: true }).click();
  await expect(edit).toBeHidden();
  const interests = (await fixtureCalls(page)).filter((call) => call.command === 'personalization_save_interest');
  expect(interests.map((call) => call.args.interest)).toMatchObject([
    { id: null, name: '네트워크' },
    { id: null, name: '인턴십' },
    { id: 1, name: '분산 컴퓨팅', priority: 3 },
    { id: 1, name: '분산 컴퓨팅', priority: 5 },
  ]);
});

test('profile edits made during a save remain in the draft and can be saved afterward', async ({ page }) => {
  await mockDesktop(page, { profileSaveDelay: 350 }); await page.goto('/'); await openSettings(page, '개인화');
  const university = personal(page).getByLabel('대학교');
  const department = personal(page).getByLabel('학과');
  await university.fill('서강대학교');
  await personal(page).getByRole('button', { name: '프로필 저장' }).click();
  await department.fill('컴퓨터공학과');
  await expect(personal(page).getByRole('status')).toContainText('개인화 설정을 저장했습니다.');
  await expect(department).toHaveValue('컴퓨터공학과');
  await personal(page).getByRole('button', { name: '프로필 저장' }).click();
  await expect(personal(page).getByRole('status')).toContainText('개인화 설정을 저장했습니다.');
  const profiles = (await fixtureCalls(page)).filter((call) => call.command === 'personalization_save_profile');
  expect(profiles.map((call) => call.args.profile)).toEqual([
    { university: '서강대학교', department: '', study_level: '' },
    { university: '서강대학교', department: '컴퓨터공학과', study_level: '' },
  ]);
});

test('interest alias limit is explained before invalid input reaches the backend', async ({ page }) => {
  await mockDesktop(page, { initialInterests: [explicitInterest(1, '분산 시스템')] }); await page.goto('/'); await openSettings(page, '개인화');
  await personal(page).getByRole('button', { name: '분산 시스템 관심 수정', exact: true }).click();
  const edit = editor(page, '분산 시스템');
  await edit.getByLabel('별칭').fill('one,two,three,four,five,six,seven,eight,nine,ten,eleven,twelve,thirteen');
  await expect(edit.getByRole('alert')).toContainText('최대 12개');
  await expect(edit.getByRole('button', { name: '저장', exact: true })).toBeDisabled();
  expect(await callCount(page, 'personalization_save_interest')).toBe(0);
});

test('explicit interests are edited in place and their direction can change', async ({ page }) => {
  await mockDesktop(page); await page.goto('/'); await openSettings(page, '개인화');
  await addTopic(page, '분산 시스템');
  await personal(page).getByRole('button', { name: '분산 시스템 관심 수정', exact: true }).click();
  const edit = editor(page, '분산 시스템');
  await expect(edit.getByLabel('이름')).toBeFocused();
  await edit.getByLabel('이름').fill('분산 컴퓨팅');
  await edit.getByRole('group', { name: '관심 방향' }).getByRole('button', { name: '관심 없음', exact: true }).click();
  await edit.getByRole('button', { name: '저장', exact: true }).click();
  await expect(personal(page).getByRole('list', { name: '관심 없음', exact: true })).toContainText('분산 컴퓨팅');
  const calls = (await fixtureCalls(page)).filter((call) => call.command === 'personalization_save_interest');
  expect(calls).toHaveLength(2);
  expect(calls[1].args.interest).toMatchObject({ id: 1, name: '분산 컴퓨팅', polarity: 'negative' });
});

test('an interest is deleted from its editor only after confirmation', async ({ page }) => {
  await mockDesktop(page, { initialInterests: [explicitInterest(1, '분산 시스템')] });
  await page.goto('/'); await openSettings(page, '개인화');
  const chip = personal(page).getByRole('button', { name: '분산 시스템 관심 수정', exact: true });
  await chip.click();
  await answerDialogs(page, 'cancel');
  await editor(page, '분산 시스템').getByRole('button', { name: '분산 시스템 삭제', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.dialogs.length)).toBe(1);
  expect(await callCount(page, 'personalization_remove_interest')).toBe(0);
  await answerDialogs(page, 'ok');
  await editor(page, '분산 시스템').getByRole('button', { name: '분산 시스템 삭제', exact: true }).click();
  await expect(chip).toHaveCount(0);
  expect(await callCount(page, 'personalization_remove_interest')).toBe(1);
});

test('learned interest can be corrected into an explicit preference', async ({ page }) => {
  await mockDesktop(page, { initialInterests: [{ id: 1, name: '인턴', aliases: ['internship'], priority: 2, polarity: 'positive', origin: 'learned', evidence_count: 2, evidence_summary: '관심 표시한 공지 2건', updated_at: '2026-10-27' }] });
  await page.goto('/'); await openSettings(page, '개인화');
  await expect(personal(page).getByRole('list', { name: '학습된 관심사', exact: true })).toContainText('인턴');
  await personal(page).getByRole('button', { name: '인턴 학습된 관심사 수정', exact: true }).click();
  const edit = editor(page, '인턴');
  await expect(edit).toContainText('관심 표시한 공지 2건');
  await edit.getByLabel('이름').fill('연구 인턴십');
  await edit.getByRole('button', { name: '직접 입력으로 저장', exact: true }).click();
  await expect(personal(page).getByRole('list', { name: '관심', exact: true })).toContainText('연구 인턴십');
  await expect(personal(page).getByRole('list', { name: '학습된 관심사', exact: true })).toHaveCount(0);
  const calls = (await fixtureCalls(page)).filter((call) => call.command === 'personalization_save_interest');
  expect(calls.at(-1)?.args.interest).toMatchObject({ id: 1, name: '연구 인턴십', polarity: 'positive' });
});

test('learning toggle and reset call the reversible personalization commands', async ({ page }) => {
  await mockDesktop(page); await page.goto('/'); await openSettings(page, '개인화');
  const learning = personal(page).getByRole('switch', { name: '관심사 학습' });
  await expect(learning).toHaveAttribute('aria-checked', 'true');
  await expect(learning).toHaveClass(/\bon\b/);
  await learning.click();
  await expect(learning).toHaveAttribute('aria-checked', 'false');
  // The visible switch must agree with the announced state.
  await expect(learning).not.toHaveClass(/\bon\b/);
  await answerDialogs(page, 'cancel');
  await personal(page).getByRole('button', { name: '개인화 데이터 초기화' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.dialogs.length)).toBe(1);
  expect(await callCount(page, 'personalization_reset')).toBe(0);
  await answerDialogs(page, 'ok');
  await personal(page).getByRole('button', { name: '개인화 데이터 초기화' }).click();
  expect(await callCount(page, 'personalization_set_learning')).toBe(1);
  await expect.poll(() => callCount(page, 'personalization_reset')).toBe(1);
});

test('reset disables profile and interest editing until all personalization data is cleared', async ({ page }) => {
  await mockDesktop(page, { personalizationResetDelay: 700 });
  await page.goto('/'); await openSettings(page, '개인화');
  const university = personal(page).getByLabel('대학교');
  const interest = personal(page).getByLabel('관심 주제 추가', { exact: true });
  await university.fill('서강대학교');
  await interest.fill('장학');
  await answerDialogs(page, 'ok');
  await personal(page).getByRole('button', { name: '개인화 데이터 초기화' }).click();
  await expect(university).toBeDisabled();
  await expect(interest).toBeDisabled();
  await expect(personal(page).getByRole('status')).toContainText('개인화 데이터를 초기화하는 중입니다.');
  await expect(personal(page).getByRole('status')).toContainText('개인화 설정을 저장했습니다.');
  await expect(university).toHaveValue('');
  await expect(interest).toHaveValue('');
});

test('Today renders the explanation returned for an explicit interest', async ({ page }) => {
  await mockDesktop(page); await page.goto('/'); await openSettings(page, '개인화');
  await addTopic(page, '장학');
  await expect(personal(page).getByRole('list', { name: '관심', exact: true })).toContainText('장학');
  await navigate(page, '오늘');
  await expect(page.getByRole('region', { name: '오늘의 추천' })).toContainText('관심 분야 ‘장학’에 맞는 공지');
  await expect(page.getByRole('region', { name: '오늘의 추천' })).toContainText('관심 설정과 저장·피드백을 반영한 추천');
});

test('users can explicitly prioritize or de-emphasize a topic', async ({ page }) => {
  await mockDesktop(page); await page.goto('/'); await openSettings(page, '개인화');
  await addTopic(page, '장학', '관심 없음');
  await expect(personal(page).getByRole('list', { name: '관심 없음', exact: true })).toContainText('장학');
  await navigate(page, '오늘');
  const recommendations = page.getByRole('region', { name: '오늘의 추천' });
  await expect(recommendations).toContainText('덜 보고 싶은 분야 ‘장학’ 관련 공지');
  const saved = (await fixtureCalls(page)).find((call) => call.command === 'personalization_save_interest');
  expect(saved?.args.interest).toMatchObject({ name: '장학', polarity: 'negative' });
});

test('learned memory can be removed from the settings review list', async ({ page }) => {
  await mockDesktop(page, { initialInterests: [{ id: 1, name: '인턴십', aliases: ['internship'], priority: 1, polarity: 'positive', origin: 'learned', evidence_count: 2, evidence_summary: '서로 다른 공지 2건에 저장 또는 관심 표시', updated_at: '2026-10-27' }] });
  await page.goto('/'); await openSettings(page, '개인화');
  const chip = personal(page).getByRole('button', { name: '인턴십 학습된 관심사 수정', exact: true });
  await chip.click();
  const edit = editor(page, '인턴십');
  await expect(edit).toContainText('학습된 관심사 · 근거 2건');

  // Cancelling keeps the memory; the dialog says it cannot be recreated from past activity.
  await edit.getByRole('button', { name: '인턴십 삭제', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__fixture.dialogs.length)).toBe(1);
  expect(await page.evaluate(() => (window as any).__fixture.dialogs[0])).toContain('다시 만들어지지 않습니다');
  await expect(chip).toBeVisible();
  expect(await callCount(page, 'personalization_remove_interest')).toBe(0);

  await answerDialogs(page, 'ok');
  await edit.getByRole('button', { name: '인턴십 삭제', exact: true }).click();
  await expect(chip).toHaveCount(0);
  expect((await fixtureCalls(page)).some((call) => call.command === 'personalization_remove_interest' && call.args.id === 1)).toBeTruthy();
});

test('Today keeps a useful cold-start list and offers a direct path to personalization', async ({ page }) => {
  await mockDesktop(page); await page.goto('/');
  const today = page.getByRole('region', { name: '오늘의 추천' });
  await expect(today).toContainText('최근 공지 · 개인화 시작 전');
  await expect(today.getByRole('button', { name: '관심 있어요' }).first()).toHaveAttribute('aria-pressed', 'false');
  await today.getByRole('button', { name: '관심 분야 설정' }).click();
  await expect(page.getByRole('heading', { level: 2, name: '개인화', exact: true })).toBeFocused();
});

test('Today shows recommendation explanations while inbox remains chronological archive workflow', async ({ page }) => {
  await mockDesktop(page); await page.goto('/');
  await expect(page.getByRole('region', { name: '오늘의 추천' })).toContainText('최근 공지');
  await page.getByRole('button', { name: /한국어 공지 01/ }).first().click();
  await expect(page.getByRole('button', { name: '관심 있어요' })).toBeVisible();
  await expect(page.locator('.reader-header')).toBeVisible();
  await page.getByRole('button', { name: /목록으로 돌아가기/ }).click();
  await navigate(page, '받은 소식');
  await expect(page.getByRole('heading', { name: /보관된 공지/ })).toBeVisible();
  const calls = await fixtureCalls(page);
  expect(calls.map((call) => call.command)).toContain('archive_query');
});

test('document feedback supports reversal and per-document commands', async ({ page }) => {
  await mockDesktop(page); await page.goto('/');
  await page.getByRole('button', { name: /한국어 공지 01/ }).first().click();
  await page.getByRole('button', { name: '관심 있어요' }).click();
  await expect(page.getByRole('button', { name: '관심 있어요' })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: '관심 없어요' }).click();
  await expect(page.getByRole('button', { name: '관심 없어요' })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: '피드백 취소' }).click();
  const calls = (await fixtureCalls(page)).filter((call) => call.command === 'personalization_set_feedback');
  expect(calls.map((call) => call.args.feedback)).toEqual(['interested', 'not_interested', null]);
  expect(calls.every((call) => call.args.documentId === 1)).toBeTruthy();
});

test('feedback completion after a revision change releases reader controls', async ({ page }) => {
  await mockDesktop(page, { revisions: true, feedbackWriteDelay: 500 }); await page.goto('/');
  await page.getByRole('button', { name: /한국어 공지 01/ }).first().click();
  const interested = page.getByRole('button', { name: '관심 있어요' });
  await interested.click();
  await expect(interested).toBeDisabled();
  await page.getByLabel('보관 이력').selectOption('2');
  await expect(interested).toBeEnabled();
  await expect(interested).toHaveAttribute('aria-pressed', 'true');
});
test('late initial feedback read cannot overwrite a newer user correction', async ({ page }) => {
  await mockDesktop(page, { feedbackReadDelay: 700 }); await page.goto('/');
  await page.getByRole('button', { name: /한국어 공지 01/ }).first().click();
  const interested = page.getByRole('button', { name: '관심 있어요' });
  await interested.click();
  await expect(interested).toHaveAttribute('aria-pressed', 'true');
  await page.waitForTimeout(850);
  await expect(interested).toHaveAttribute('aria-pressed', 'true');
  await interested.click();
  const feedback = (await fixtureCalls(page)).filter((call) => call.command === 'personalization_set_feedback');
  expect(feedback.map((call) => call.args.feedback)).toEqual(['interested', null]);
});

test('closing while reader feedback is pending refreshes Today after the write commits', async ({ page }) => {
  await page.clock.install({ time: new Date('2026-11-02T12:00:00Z') });
  await mockDesktop(page, { feedbackWriteDelay: 350 }); await page.goto('/');
  await page.getByRole('button', { name: /한국어 공지 01/ }).first().click();
  const interested = page.getByRole('button', { name: '관심 있어요' });
  await expect(interested).toBeEnabled();
  await page.clock.pauseAt(new Date('2026-11-02T13:00:00Z'));
  await interested.click();
  await expect(interested).toBeDisabled();
  await page.getByRole('button', { name: /목록으로 돌아가기/ }).click();
  await expect(page.locator('.archive-reader')).toHaveCount(0);
  await page.clock.runFor(400);
  const recommendation = page.getByRole('region', { name: '오늘의 추천' })
    .getByRole('group', { name: /한국어 공지 01 추천 피드백/ })
    .getByRole('button', { name: '관심 있어요' });
  await expect(recommendation).toHaveAttribute('aria-pressed', 'true');
});

test('Today reloads recommendations when an active sync completes', async ({ page }) => {
  await mockDesktop(page); await page.goto('/');
  const today = page.getByRole('region', { name: '오늘의 추천' });
  await expect(today.getByRole('button', { name: /한국어 공지 01/ })).toBeVisible();
  const before = await callCount(page, 'personalization_recommendations');
  await page.evaluate(async () => {
    await (window as any).__TAURI_INTERNALS__.invoke('archive_start', { mode: 'incremental' });
  });
  await expect(page.getByText('공개 공지를 가져오는 중입니다.', { exact: false })).toBeVisible();
  await page.evaluate(() => {
    const fixture = (window as any).__fixture;
    fixture.entries[0].title = '새로 갱신된 추천 공지';
    fixture.completeSync();
  });
  await expect(today.getByRole('button', { name: /새로 갱신된 추천 공지/ })).toBeVisible();
  expect(await callCount(page, 'personalization_recommendations')).toBeGreaterThan(before);
});

test('Today recommendation feedback can be reversed without opening the document', async ({ page }) => {
  await mockDesktop(page); await page.goto('/');
  const actions = page.getByRole('group', { name: /한국어 공지 01 추천 피드백/ });
  const interested = actions.getByRole('button', { name: '관심 있어요' });
  await interested.click();
  await expect(interested).toHaveAttribute('aria-pressed', 'true');
  await interested.click();
  await expect(interested).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('.reader-header')).toHaveCount(0);
  const feedback = (await fixtureCalls(page)).filter((call) => call.command === 'personalization_set_feedback');
  expect(feedback.map((call) => call.args.feedback)).toEqual(['interested', null]);
});

test('current production analysis contract does not show a stale warning', async ({ page }) => {
  await mockDesktop(page, { aiResult: true, aiInstalled: true }); await page.goto('/');
  await expect(page.getByText('저장된 AI 요약')).toBeVisible();
  await page.getByRole('button', { name: /한국어 공지 01/ }).first().click();
  await expect(page.getByText('이 결과는 현재 모델 또는 분석 방식·설정과 다릅니다.', { exact: false })).toHaveCount(0);
});

test('explicit prompt-v2 fixture warns and is not treated as a current Today summary', async ({ page }) => {
  await mockDesktop(page, { aiResult: true, aiInstalled: true, analysisContract: 'prompt-v2' }); await page.goto('/');
  await expect(page.getByText('저장된 AI 요약')).toHaveCount(0);
  await page.getByRole('button', { name: /한국어 공지 01/ }).first().click();
  await expect(page.getByText('이 결과는 현재 모델 또는 분석 방식·설정과 다릅니다.', { exact: false })).toBeVisible();
});
