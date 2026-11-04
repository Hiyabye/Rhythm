<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { save, open } from '@tauri-apps/plugin-dialog';
  import { archiveError, archiveSize } from './archive-format';
  import { confirmAction } from './confirm';
  import Hint from './Hint.svelte';

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
  let restoreAvailable = $state<boolean | null>(null);
  let restoreReason = $state('');
  onMount(() => { if (desktop) void checkRestoreSupport(); });
  async function checkRestoreSupport() {
    restoreAvailable = null;
    try { await invoke('backup_restore_support'); restoreAvailable = true; restoreReason = ''; }
    catch (reason) { restoreAvailable = false; restoreReason = archiveError(reason); }
  }
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
    if (!desktop || busy || restoreAvailable !== true) return;
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
    if (!desktop || !selected || !preview || busy || restoreAvailable !== true) return;
    error = ''; notice = '';
    let confirmed: boolean;
    try {
      confirmed = await confirmAction('현재 Rhythm 데이터 전체를 이 백업의 데이터로 바꿉니다. 현재 데이터는 지우지 않고 복구용으로 보관합니다.\n\n복원하면 Rhythm이 종료됩니다. Rhythm을 직접 다시 열면 복원이 마무리됩니다.\n\n계속하시겠습니까?', '복원하고 종료');
    } catch (reason) {
      error = `${archiveError(reason)} 확인 창을 열지 못해 복원을 시작하지 않았습니다.`;
      return;
    }
    if (!confirmed || busy) return;
    const id = startOperation();
    try {
      await invoke('backup_restore', { source: selected, confirmed: true, operationId: id });
      notice = '복원을 준비했습니다. Rhythm을 종료합니다. 다시 열면 복원이 마무리됩니다.';
    } catch (reason) { error = archiveError(reason); finishOperation(id); }
  }
</script>

<div id="data-backup" aria-busy={busy}>
  <div class="settings-group-title"><h3>백업 및 복원</h3><Hint label="백업 및 복원">설정, 보관한 공지와 첨부 자료를 암호화하지 않은 파일 하나에 담습니다. 안전한 곳에 직접 보관하세요. AI 모델 파일은 포함하지 않습니다. 복원하면 현재 데이터는 지우지 않고 복구용으로 남겨 두며, Rhythm이 종료됩니다. 다시 열면 복원이 마무리되고, 파일이 없는 설치 모델은 다시 설치해야 합니다.</Hint></div>
  {#if !desktop}<p class="muted">백업과 복원은 데스크톱 앱에서 사용할 수 있습니다.</p>
  {:else}
    <div class="settings-group">
      <div class="settings-row"><span class="row-label">백업 만들기</span><button class="button secondary small" disabled={busy} onclick={createBackup}>{busy ? '처리 중…' : '백업 파일 저장'}</button></div>
      <div class="settings-row"><span class="row-label">백업에서 복원{#if restoreAvailable === null}<span class="row-detail">복원 가능 여부를 확인하고 있습니다.</span>{:else if !restoreAvailable}<span class="row-detail" role="status">복원할 수 없습니다. {restoreReason}</span>{/if}</span>{#if restoreAvailable === false}<button class="text-button" onclick={checkRestoreSupport}>다시 확인</button>{/if}<button class="button secondary small" disabled={busy || restoreAvailable !== true} onclick={chooseBackup}>{busy ? '검사 중…' : '백업 파일 선택'}</button></div>
      {#if busy}
        <div class="settings-row"><span class="row-label"><span class="row-detail" role="status">{cancelling ? '취소 요청을 처리하고 임시 파일을 정리하고 있습니다…' : '작업 중…'}</span></span><button class="text-button" disabled={cancelling} onclick={cancelOperation}>{cancelling ? '취소 요청됨' : '작업 취소'}</button></div>
      {/if}
      {#if preview}
        <div class="settings-row stacked" aria-live="polite">
          <div class="row-label"><h4>복원할 백업 확인</h4><span class="row-detail">공지 {preview.documents.toLocaleString('ko-KR')}건 · 첨부 자료 {preview.resources.toLocaleString('ko-KR')}개 ({archiveSize(preview.resource_bytes)}) · {createdAt(preview.created_at)} · 형식 v{preview.format_version} · 스키마 {preview.database_schema}</span><span class="row-detail">모델 가중치는 포함되지 않습니다. 복원하면 Rhythm이 종료되고 자동으로 다시 열리지 않습니다. 직접 다시 열면 복원이 마무리됩니다.</span></div>
          <div class="popover-actions"><button class="button small" disabled={busy || restoreAvailable !== true} onclick={restoreBackup}>이 백업으로 복원</button><button class="text-button" disabled={busy} onclick={() => { selected = null; preview = null; }}>취소</button></div>
        </div>
      {/if}
    </div>
  {/if}
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  {#if notice}<p class="archive-feedback" role="status">{notice}</p>{/if}
</div>
