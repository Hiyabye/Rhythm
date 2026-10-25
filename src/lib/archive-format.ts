export function archiveError(reason: unknown): string {
  if (typeof reason !== 'string') return '작업을 완료하지 못했습니다. 다시 시도해 주세요.';
  if (/database or disk is full|no space left on device/i.test(reason)) {
    return '저장 공간이 부족해 작업을 완료하지 못했습니다. 공간을 확보한 뒤 다시 시도해 주세요.';
  }
  if (/readonly database|read-only file system|permission denied/i.test(reason)) {
    return '저장 위치에 쓸 수 없습니다. 폴더 권한을 확인한 뒤 다시 시도해 주세요.';
  }
  if (/database is (?:locked|busy)/i.test(reason)) {
    return '저장소가 다른 작업을 처리하고 있습니다. 잠시 후 다시 시도해 주세요.';
  }
  return /[가-힣]/.test(reason) ? reason : '작업을 완료하지 못했습니다. 다시 시도해 주세요.';
}

export function archiveDate(value: string | null, includeTime = false): string {
  if (!value) return '기록 없음';
  const normalized = /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(value)
    ? `${value.replace(' ', 'T')}Z` : /^\d{4}-\d{2}-\d{2}$/.test(value) ? `${value}T00:00:00` : value;
  const date = new Date(normalized);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat('ko-KR', {
    year: 'numeric', month: 'long', day: 'numeric',
    ...(includeTime ? { hour: '2-digit', minute: '2-digit' } as const : {}),
  }).format(date);
}

export function archiveSize(bytes: number | null): string {
  if (bytes === null) return '크기 미상';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function sourceStatus(status: string): string {
  return ({ idle: '대기', running: '수집 중', completed: '완료', failed: '오류', cancelled: '취소됨',
    interrupted: '중단됨 · 이어받기 가능', limited: '일부 가져옴 · 이어받기 가능' } as Record<string, string>)[status] ?? '확인 필요';
}

export function resourceStatus(status: string): string {
  return ({ downloaded: '로컬 저장됨', pending: '다운로드 대기', failed: '다운로드 실패',
    too_large: '용량 제한 초과', unsupported: '다운로드 미지원' } as Record<string, string>)[status] ?? '확인 필요';
}
