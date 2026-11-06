import { test, expect } from '@playwright/test';
import { mockDesktop, openSettings } from './fixture';
import type { CybercampusTask } from '../../src/lib/types';

const board = { id: 'cybercampus', name: '강의 공지', organization: '서강대학교 · 사이버캠퍼스', supported: true, family: 'cybercampus' };

/** A KST wall-clock value `days` from now, as the connector stores it. */
function kst(days: number) {
  return new Date(Date.now() + days * 86_400_000 + 9 * 3_600_000).toISOString().slice(0, 16);
}
function task(overrides: Partial<CybercampusTask>): CybercampusTask {
  return { course_key: 'A1', course_name: '자료구조실습', category: 'report', item_id: '1', label: '과제', title: '실습 과제', week: 9,
    opens_at: null, due_at: null, late_due_at: null, status: null, submitted_at: null, pending: false, updated_at: '2026-11-06T00:00:00Z', ...overrides };
}

test('the Cybercampus login is checked before it is saved and is never shown again', async ({ page }) => {
  await mockDesktop(page, { sourceList: [board] });
  await page.goto('/');
  await openSettings(page, '게시판');
  const login = page.getByRole('group', { name: '사이버캠퍼스 로그인' });
  await expect(login).toContainText('저장된 로그인 정보 없음');
  const id = login.getByRole('textbox', { name: '사이버캠퍼스 아이디(학번)' });
  const password = login.getByLabel('사이버캠퍼스 비밀번호');
  const save = login.getByRole('button', { name: '확인 후 저장' });
  await expect(save).toBeDisabled();
  await id.fill('20990001');
  await password.fill('wrong');
  await save.click();
  await expect(login.getByRole('alert')).toContainText('로그인에 실패했습니다');
  // The password field never keeps a rejected or accepted password.
  await expect(password).toHaveValue('');
  await password.fill('right');
  await password.press('Enter');
  await expect(login).toContainText('20990001 계정으로 로그인 정보 저장됨 · 강좌 1개');
  await expect(login).toContainText('자료구조실습');
  await expect(password).toHaveValue('');
  await expect(login.getByRole('status')).toContainText('운영체제 보안 저장소에 저장했습니다');
  await login.getByRole('button', { name: '저장된 로그인 정보 삭제' }).click();
  await expect(login).toContainText('저장된 로그인 정보 없음');
  await expect(login.getByRole('status')).toContainText('보관한 공지와 과제는 남습니다');
});

test('deadlines list what is due first and fold submitted or past items', async ({ page }) => {
  await mockDesktop(page, { sourceList: [{ ...board, subscribed: true }], cybercampusId: '20990001', cybercampusTasks: [
    task({ item_id: '2', title: '실습 과제 2', due_at: kst(3), pending: true, status: '미제출' }),
    task({ item_id: '3', category: 'test', label: '퀴즈', title: '2주차 퀴즈', course_name: '확률과통계', due_at: kst(0.25), pending: true, week: null }),
    task({ item_id: '1', title: '실습 과제 1', due_at: kst(-20), status: '정상제출', submitted_at: kst(-21) }),
  ] });
  await page.goto('/');
  const navigation = page.getByRole('navigation', { name: '기본 탐색', exact: true });
  await navigation.getByRole('button', { name: '과제·마감' }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('과제·마감');
  await expect(page.getByRole('heading', { name: '해야 할 일 2개' })).toBeVisible();
  const due = page.getByRole('list', { name: '해야 할 일' }).getByRole('listitem');
  await expect(due).toHaveCount(2);
  await expect(due.nth(0)).toContainText('퀴즈2주차 퀴즈');
  await expect(due.nth(0)).toContainText('시간 남음');
  await expect(due.nth(1)).toContainText('D-3');
  await expect(due.nth(1)).toContainText('9주차');
  const history = page.getByText('제출했거나 마감이 지난 항목 1개');
  await expect(page.getByRole('list', { name: '제출했거나 마감이 지난 항목' })).toBeHidden();
  await history.click();
  await expect(page.getByRole('list', { name: '제출했거나 마감이 지난 항목' })).toContainText('정상제출');
});

test('the deadline page appears only with the Cybercampus board subscribed', async ({ page }) => {
  await mockDesktop(page, { sourceList: [{ ...board, subscribed: false }] });
  await page.goto('/');
  await expect(page.getByRole('navigation', { name: '기본 탐색', exact: true }).getByRole('button', { name: '받은 소식' })).toBeVisible();
  await expect(page.getByRole('button', { name: '과제·마감' })).toHaveCount(0);
});
