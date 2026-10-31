<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { save, open } from '@tauri-apps/plugin-dialog';
  import { archiveError, archiveSize } from './archive-format';

  let { desktop }: { desktop: boolean } = $props();
  type BackupPreview = {
    format_version: number;
    created_at: string;
    database_schema: number;
    documents: number;
    resources: number;
    resource_bytes: number;
    includes_model_weights: boolean;
  };
  let preview = $state<BackupPreview | null>(null);
  let selected = $state<string | null>(null);
  let busy = $state(false);
  let cancelling = $state(false);
  let operationId = $state<string | null>(null);
  let error = $state('');
  let notice = $state('');
  function createdAt(value: string) {
    const seconds = Number(value);
    return Number.isFinite(seconds) ? new Date(seconds * 1000).toLocaleString('ko-KR') : value;
  }
  function startOperation() {
    const id = crypto.randomUUID();
    operationId = id;
    busy = true;
    cancelling = false;
    error = '';
    notice = '';
    return id;
  }
  function finishOperation(id: string) {
    if (operationId === id) operationId = null;
    busy = false;
    cancelling = false;
  }
  async function cancelOperation() {
    const id = operationId;
    if (!id || !busy || cancelling) return;
    cancelling = true;
    try {
      const accepted = await invoke<boolean>('backup_cancel', { operationId: id });
      notice = accepted
        ? '취소를 요청했습니다. 진행 중인 작업을 안전하게 정리하고 있습니다.'
        : '이미 파일 확정 단계에 들어가 취소할 수 없습니다. 작업이 끝날 때까지 기다려 주세요.';
    } catch (reason) {
      cancelling = false;
      error = archiveError(reason);
    }
  }
  async function createBackup() {
    if (!desktop || busy) return;
    error = ''; notice = '';
    let destination: string | null;
    try {
      destination = await save({
        title: 'Rhythm 백업 파일 저장',
        defaultPath: 'Rhythm-backup.rhythm-backup',
        filters: [{ name: 'Rhythm 백업', extensions: ['rhythm-backup'] }],
      });
    } catch (reason) {
      error = `${archiveError(reason)} 백업 저장 대화상자를 열지 못했습니다. 앱의 파일 접근 권한을 확인한 뒤 다시 시도해 주세요.`;
      return;
    }
    if (!destination) return;
    const id = startOperation();
    try {
      const result = await invoke<BackupPreview>('backup_create', { destination, operationId: id });
      notice = `백업을 만들었습니다. 공지 ${result.documents.toLocaleString('ko-KR')}건, 첨부 자료 ${result.resources.toLocaleString('ko-KR')}개를 저장했습니다.`;
    } catch (reason) { error = archiveError(reason); }
    finally { finishOperation(id); }
  }
  async function chooseBackup() {
    if (!desktop || busy) return;
    error = ''; notice = '';
    let source: string | string[] | null;
    try {
      source = await open({
        title: '복원할 Rhythm 백업 선택', multiple: false, directory: false,
        filters: [{ name: 'Rhythm 백업', extensions: ['rhythm-backup'] }],
      });
    } catch (reason) {
      error = `${archiveError(reason)} 백업 파일 선택 대화상자를 열지 못했습니다. 앱의 파일 접근 권한을 확인한 뒤 다시 시도해 주세요.`;
      return;
    }
    if (typeof source !== 'string') return;
    selected = source; preview = null;
    const id = startOperation();
    try { preview = await invoke<BackupPreview>('backup_inspect', { source, operationId: id }); }
    catch (reason) { selected = null; error = archiveError(reason); }
    finally { finishOperation(id); }
  }
  async function restoreBackup() {
    if (!desktop || !selected || !preview || busy) return;
    const confirmed = window.confirm('현재 Rhythm 데이터 전체를 이 백업의 데이터로 바꾸고 앱을 다시 시작합니다. 현재 데이터는 복구용으로 보관됩니다. 계속하시겠습니까?');
    if (!confirmed) return;
    const id = startOperation();
    try {
      await invoke('backup_restore', { source: selected, confirmed: true, operationId: id });
      notice = '복원을 준비했습니다. 앱이 다시 시작되면 복원된 데이터를 불러옵니다.';
    } catch (reason) { error = archiveError(reason); finishOperation(id); }
  }
</script>

<section id="data-backup" class="settings-section" aria-busy={busy}>
  <div class="section-heading"><h2>백업 및 복원</h2><p>설정과 보관한 공지·첨부 자료를 암호화되지 않은 단일 파일로 저장하거나 복원합니다. 파일을 안전한 위치에 직접 보관해 주세요.</p></div>
  {#if !desktop}<p class="muted">백업과 복원은 데스크톱 앱에서 사용할 수 있습니다.</p>
  {:else}
    <div class="setting-row"><div><h3>로컬 백업 만들기</h3><p>현재 데이터베이스의 일관된 스냅샷과 보관된 첨부 자료를 하나의 백업 파일에 담습니다. AI 모델 파일은 포함하지 않습니다.</p></div><button class="text-button" disabled={busy} onclick={createBackup}>{busy ? '처리 중…' : '백업 파일 저장'}</button></div>
    <div class="setting-row"><div><h3>백업에서 복원</h3><p>백업을 먼저 검사해 포함된 데이터와 버전을 확인합니다. 확인 후 복원이 시작되며 앱이 다시 시작됩니다.</p></div><button class="text-button" disabled={busy} onclick={chooseBackup}>{busy ? '검사 중…' : '백업 파일 선택'}</button></div>
    {#if busy}
      <div class="inline-controls"><p class="muted" role="status">{cancelling ? '취소 요청을 처리하고 임시 파일을 정리하고 있습니다…' : '작업 중…'}</p><button class="text-button" disabled={cancelling} onclick={cancelOperation}>{cancelling ? '취소 요청됨' : '작업 취소'}</button></div>
    {/if}
    {#if preview}
      <div class="privacy-note" aria-live="polite"><div><h3>복원할 백업 확인</h3><p>형식 v{preview.format_version} · 데이터베이스 스키마 {preview.database_schema} · 공지 {preview.documents.toLocaleString('ko-KR')}건 · 첨부 자료 {preview.resources.toLocaleString('ko-KR')}개 ({archiveSize(preview.resource_bytes)})</p><p>생성 시각 {createdAt(preview.created_at)} · 모델 가중치는 포함되지 않습니다.</p></div></div>
      <div class="inline-controls"><button class="text-button" disabled={busy} onclick={() => { selected = null; preview = null; }}>취소</button><button class="text-button" disabled={busy} onclick={restoreBackup}>이 백업으로 복원</button></div>
    {/if}
  {/if}
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  {#if notice}<p class="archive-feedback" role="status">{notice}</p>{/if}
  <p class="footnote">복원 시 현재 데이터베이스와 첨부 자료는 복구용으로 보관합니다. AI 모델 가중치와 외부 모델 파일은 백업에서 제외되며 이동하거나 삭제하지 않습니다. 복원된 목록에서 파일이 없는 설치 모델은 다시 설치가 필요합니다.</p>
</section>
