<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { CybercampusStatus } from './types';
  import { archiveError } from './archive-format';
  import Hint from './Hint.svelte';

  let { locked }: { locked: boolean } = $props();
  let status = $state<CybercampusStatus | null>(null);
  let id = $state('');
  let password = $state('');
  let busy = $state(false);
  let error = $state('');
  let feedback = $state('');
  let active = true;

  onMount(() => {
    void invoke<CybercampusStatus>('cybercampus_status').then((value) => { if (active) status = value; }).catch((reason) => { if (active) error = archiveError(reason); });
    return () => { active = false; };
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy || !id.trim() || !password) return;
    busy = true;
    error = '';
    feedback = '';
    try {
      status = await invoke<CybercampusStatus>('cybercampus_save_login', { id: id.trim(), password });
      feedback = '로그인을 확인하고 운영체제 보안 저장소에 저장했습니다. 강의 공지를 구독하고 새로고침하면 가져옵니다.';
      id = '';
    } catch (reason) { error = archiveError(reason); }
    finally { password = ''; busy = false; }
  }

  async function forget() {
    busy = true;
    error = '';
    feedback = '';
    try {
      status = await invoke<CybercampusStatus>('cybercampus_forget_login');
      feedback = '저장된 로그인 정보를 삭제했습니다. 보관한 공지와 과제는 남습니다.';
    } catch (reason) { error = archiveError(reason); }
    finally { busy = false; }
  }
</script>

<div class="settings-group" role="group" aria-label="사이버캠퍼스 로그인">
  <div class="settings-row stacked">
    <span class="row-label">사이버캠퍼스 로그인<Hint label="사이버캠퍼스 로그인">학번과 SAINT 비밀번호로 eClass에 로그인해 이번 학기 수강 강좌의 공지, 첨부 파일, 과제와 마감을 이 기기에 가져옵니다. 비밀번호는 운영체제 보안 저장소(Keychain 또는 Credential Manager)에만 저장하며 Rhythm 데이터베이스, 백업, 진단에 포함되지 않습니다. 읽기만 하며 제출, 게시, 확인 표시는 하지 않습니다. 다만 공지 본문을 가져오면 eClass에는 그 공지를 연 것으로 기록됩니다. 사이버캠퍼스 자료는 원격 AI로 보내지 않습니다.</Hint>
      <span class="row-detail" class:error-text={status?.rejected}>{#if status?.rejected}eClass가 저장된 로그인을 거부했습니다. 다시 저장하기 전까지 자동으로 시도하지 않습니다.{:else if status?.id}{status.id} 계정으로 로그인 정보 저장됨{#if status.courses.length}{' · '}강좌 {status.courses.length}개{/if}{:else if status}저장된 로그인 정보 없음{/if}</span>
    </span>
    <form class="field-with-button" onsubmit={save}>
      <input aria-label="사이버캠퍼스 아이디(학번)" type="text" bind:value={id} placeholder="학번" autocomplete="username" spellcheck="false" disabled={busy || locked} />
      <input aria-label="사이버캠퍼스 비밀번호" type="password" bind:value={password} placeholder="비밀번호" autocomplete="current-password" disabled={busy || locked} />
      <button class="button secondary small" type="submit" disabled={busy || locked || !id.trim() || !password}>{busy ? '확인하는 중…' : status?.id ? '다시 저장' : '확인 후 저장'}</button>
    </form>
    {#if status?.id}<div class="popover-actions"><button class="text-button" disabled={busy || locked} onclick={forget}>저장된 로그인 정보 삭제</button></div>{/if}
    {#if status?.courses.length}<p class="row-detail">{status.courses.map((course) => course.name).join(' · ')}</p>{/if}
  </div>
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  <p class="archive-feedback" role="status">{feedback}</p>
</div>
