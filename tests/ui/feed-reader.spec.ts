import { expect, test } from '@playwright/test';
import { callCount, mockDesktop } from './fixture';

// Opening a feed notice (#177): the excerpt shows at once and the body is
// fetched behind it. Only the IPC boundary is mocked: nothing here fetches
// from the school or checks the stored revision.

test('a feed notice shows its excerpt first and then the fetched body', async ({ page }) => {
  await mockDesktop(page, { feedBody: { delay: 400 } });
  await page.goto('/');
  await page.locator('#notice-1').click();

  await expect(page.locator('.reader-header h2')).toHaveText('한국어 공지 01');
  const summary = page.getByRole('region', { name: '공지 요약', exact: true });
  await expect(summary).toContainText('피드 요약: 2026년 10월 30일까지');
  await expect(summary.getByRole('status')).toHaveText('학교 사이트에서 본문을 가져오는 중입니다…');
  await expect(page.locator('.archive-local-status')).toContainText('아직 공지 요약만 있습니다');
  await expect(page.locator('.reader-header h2')).toBeFocused();

  await expect(page.locator('.reader-body')).toContainText('보관된 한국어 본문입니다.');
  await expect(summary).toHaveCount(0);
  await expect(page.locator('.reader-header h2')).toBeFocused();
  await expect(page.getByRole('note').filter({ hasText: '원문이 수정됨' })).toHaveCount(0);
  expect(await callCount(page, 'archive_fetch_body')).toBe(1);
});

test('a failed body fetch keeps the excerpt and retries on request', async ({ page }) => {
  await mockDesktop(page, { feedBody: { result: 'fail' } });
  await page.goto('/');
  await page.locator('#notice-1').click();

  const summary = page.getByRole('region', { name: '공지 요약', exact: true });
  await expect(summary.getByRole('alert')).toContainText('본문을 가져오지 못했습니다. 학교 서버에 연결하지 못했습니다.');
  await expect(summary).toContainText('피드 요약');
  await summary.getByRole('button', { name: '다시 시도', exact: true }).click();
  await expect(page.locator('.reader-body')).toContainText('보관된 한국어 본문입니다.');
  expect(await callCount(page, 'archive_fetch_body')).toBe(2);
});

test('a revised school text is labeled', async ({ page }) => {
  await mockDesktop(page, { feedBody: { result: 'revised' } });
  await page.goto('/');
  await page.locator('#notice-1').click();
  await expect(page.locator('.reader-body')).toContainText('보관된 한국어 본문입니다.');
  await expect(page.getByRole('note').filter({ hasText: '원문이 수정됨' })).toContainText('현재 본문을 보관했습니다');
});

test('a notice deleted at the school shows what was kept without fetching', async ({ page }) => {
  await mockDesktop(page, { feedBody: { removed: true } });
  await page.goto('/');
  await page.locator('#notice-1').click();
  await expect(page.getByRole('note').filter({ hasText: '원문 삭제됨' })).toContainText('학교 사이트에서 삭제된 공지입니다');
  const summary = page.getByRole('region', { name: '공지 요약', exact: true });
  await expect(summary).toContainText('피드 요약');
  await expect(summary).toContainText('삭제되어 본문을 가져올 수 없습니다');
  expect(await callCount(page, 'archive_fetch_body')).toBe(0);
});

const pendingImage = { id: 7, url: 'https://www.sogang.ac.kr/file-fe-prd/editor/a.png', filename: '안내.png', kind: 'image',
  status: 'pending', mime: null, bytes: null, local_path: null, sha256: null, error: null };

test('files of an opened notice download after its body and then show', async ({ page }) => {
  await mockDesktop(page, { feedBody: {}, feedFiles: { delay: 400 }, resources: [pendingImage],
    body: `<p>보관된 한국어 본문입니다.</p><img data-resource-url="${pendingImage.url}" alt="안내 이미지">` });
  await page.goto('/');
  await page.locator('#notice-1').click();

  const files = page.locator('.reader-resources').filter({ hasText: '첨부 자료 및 이미지' });
  await expect(files.getByRole('status')).toHaveText('학교 사이트에서 이미지와 첨부 파일을 내려받는 중입니다…');
  await expect(files.getByRole('button', { name: '안내.png 다시 내려받기', exact: true })).toBeDisabled();
  await expect(page.locator('.reader-body img')).toHaveAttribute('src', /^data:image\/png;base64,/);
  await expect(files).toContainText('로컬 저장됨');
  await expect(files.getByRole('status')).toHaveCount(0);
  await expect(page.locator('.reader-header h2')).toBeFocused();
  expect(await callCount(page, 'archive_fetch_resources')).toBe(1);
});

test('a failed file download is shown and left for a retry', async ({ page }) => {
  await mockDesktop(page, { feedFiles: { fail: true }, resources: [pendingImage] });
  await page.goto('/');
  await page.locator('#notice-1').click();

  const files = page.locator('.reader-resources').filter({ hasText: '첨부 자료 및 이미지' });
  await expect(files.getByRole('alert')).toContainText('자료를 내려받지 못했습니다. 학교 서버가 요청을 제한했습니다.');
  await expect(files).toContainText('다운로드 실패');
  await expect(files.getByRole('button', { name: '안내.png 다시 내려받기', exact: true })).toBeEnabled();
  expect(await callCount(page, 'archive_fetch_resources')).toBe(1);
});
