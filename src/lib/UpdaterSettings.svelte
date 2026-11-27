<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { confirmAction } from './confirm';
  import { isWindows } from './platform';
  import Hint from './Hint.svelte';
  import ProgressBar from './Progress.svelte';
  import Spinner from './Spinner.svelte';
  import type { AutoUpdateCheck, UpdateInfo } from './types';

  type Progress = { phase: 'downloading' | 'verifying' | 'installing' | 'staged' | 'restarting'; version?: string; downloaded?: number; total?: number | null };
  let { desktop }: { desktop: boolean } = $props();
  // null until the build reports whether a trusted update key is embedded.
  let configured = $state<boolean | null>(null);
  let checked = $state(false);
  let checking = $state(false);
  let installing = $state(false);
  let error = $state('');
  let progress = $state<Progress | null>(null);
  let autoCheck = $state<boolean | null>(null);
  let savingAutoCheck = $state(false);

  $effect(() => {
    if (!desktop) return;
    void invoke<boolean>('updater_configured').then((value) => { configured = value; }).catch(() => { configured = false; });
  });

  $effect(() => {
    if (!desktop || !configured) return;
    let mounted = true;
    let stop: (() => void) | undefined;
    void listen<Progress>('updater://progress', (event) => { progress = event.payload; }).then((unlisten) => {
      if (mounted) stop = unlisten;
      else unlisten();
    }).catch((reason) => { error = String(reason); });
    return () => { mounted = false; stop?.(); };
  });

  let update = $state<UpdateInfo | null>(null);

  // Show what an automatic check already found instead of checking again.
  $effect(() => {
    if (!desktop || !configured) return;
    void invoke<AutoUpdateCheck>('updater_auto_check').then((value) => {
      autoCheck = value.enabled;
      if (value.available && !update && !checking) { update = value.available; checked = true; }
    }).catch((reason) => { error = String(reason); });
  });

  async function toggleAutoCheck() {
    if (autoCheck === null || savingAutoCheck) return;
    savingAutoCheck = true;
    error = '';
    try { autoCheck = await invoke<boolean>('updater_set_auto_check', { enabled: !autoCheck }); }
    catch (reason) { error = String(reason); }
    finally { savingAutoCheck = false; }
  }

  async function check() {
    if (!desktop || !configured || checking || installing) return;
    checking = true;
    checked = false;
    error = '';
    update = null;
    progress = null;
    try {
      update = await invoke<UpdateInfo | null>('updater_check');
      checked = true;
    } catch (reason) { error = String(reason); }
    finally { checking = false; }
  }
  async function install() {
    if (!desktop || !configured || installing || !update) return;
    if (progress?.phase === 'staged') {
      installing = true;
      error = '';
      try {
        if (await confirmAction('업데이트를 설치하고 앱을 다시 시작할까요?', '설치 및 다시 시작')) {
          await invoke('updater_restart', { confirmed: true });
        }
      }
      // A failed install leaves the verified package staged in the backend.
      catch (reason) { error = String(reason); progress = { phase: 'staged', version: update.version }; }
      finally { installing = false; }
      return;
    }
    installing = true;
    error = '';
    progress = null;
    let staged: UpdateInfo | null = null;
    try {
      staged = update = await invoke<UpdateInfo>('updater_install');
      const restart = await confirmAction('업데이트를 내려받았습니다. 지금 설치하고 앱을 다시 시작할까요? 취소해도 앱을 종료하기 전까지는 설치할 수 있습니다.', '설치 및 다시 시작');
      if (restart) await invoke('updater_restart', { confirmed: true });
      else progress = { phase: 'staged', version: staged.version };
    }
    // After a successful download, only the restart failed; keep the staged package.
    catch (reason) { error = String(reason); progress = staged ? { phase: 'staged', version: staged.version } : null; }
    finally { installing = false; }
  }
</script>

<div id="about">
  <h3 class="settings-group-title" id="about-title" tabindex="-1">정보</h3>
  <div class="settings-group">
    <div class="settings-row about-row"><span class="beat" aria-hidden="true"><i></i><i></i><i></i><i></i></span><span class="row-label">Rhythm<span class="row-detail">버전 <span>2.4.0</span> · GPL-3.0-or-later · github.com/Hiyabye/Rhythm</span></span></div>
    {#if desktop && configured}
      <div class="settings-row"><span class="row-label">앱 업데이트<Hint label="앱 업데이트">GitHub에서 새 버전을 받아 서명을 확인한 뒤 설치할지 묻습니다.</Hint>{#if checked && !update}<span class="row-detail" role="status">최신 버전입니다.</span>{/if}</span><button class="button secondary small" disabled={checking || installing} onclick={check}>{#if checking}<Spinner size={12} />{/if}{checking ? '확인 중…' : '업데이트 확인'}</button></div>
      <div class="settings-row"><span class="row-label">자동으로 업데이트 확인<Hint label="자동으로 업데이트 확인">앱이 켜져 있으면 하루에 한 번 새 버전을 확인합니다. 내려받기와 설치는 직접 선택할 때만 진행합니다.</Hint></span><button class="switch" class:on={autoCheck === true} role="switch" aria-label="자동으로 업데이트 확인" aria-checked={autoCheck === true} disabled={autoCheck === null || savingAutoCheck} onclick={toggleAutoCheck}><span></span></button></div>
      {#if update}<div class="settings-row"><span class="row-label">새 버전 {update.version}<span class="row-detail">{update.body ?? '설치할 수 있는 새 버전입니다.'}</span>{#if progress}<span class="row-detail" role="status">{progress.phase === 'downloading' ? `다운로드 중${progress.total ? ` · ${Math.round((progress.downloaded ?? 0) / progress.total * 100)}%` : '…'}` : progress.phase === 'verifying' ? '서명 확인 중…' : progress.phase === 'installing' ? '설치 중…' : progress.phase === 'staged' ? '업데이트를 설치할 준비가 됐습니다.' : '앱을 다시 시작합니다…'}</span>{#if progress.phase === 'downloading'}<label class="update-progress"><span class="sr-only">업데이트 다운로드</span><ProgressBar max={progress.total ?? 1} value={progress.total ? progress.downloaded ?? 0 : undefined} /></label>{/if}{/if}</span><button class="button small" disabled={installing} onclick={install}>{#if installing}<Spinner size={12} />{/if}{installing ? '업데이트 중…' : progress?.phase === 'staged' ? '설치 및 다시 시작' : '다운로드 및 설치'}</button></div>
      {:else if progress}<p class="footnote" role="status">{progress.phase === 'staged' ? '업데이트를 설치할 준비가 됐습니다.' : '업데이트를 처리하는 중…'}</p>{/if}
    {:else if desktop && configured === false}
      <div class="settings-row stacked">
        <span class="row-label">앱 업데이트<span class="row-detail">이 빌드는 앱 안에서 업데이트할 수 없습니다. 새 버전은 직접 설치하세요.</span></span>
        <details class="manual-update-steps">
          <summary>설치 방법</summary>
          {#if isWindows}
          <ol class="manual-update">
            <li>트레이 메뉴에서 Rhythm을 종료합니다.</li>
            <li>필요하면 위의 백업 및 복원에서 백업을 만듭니다.</li>
            <li>github.com/Hiyabye/Rhythm의 Releases에서 최신 Windows 설치 파일을 받습니다.</li>
            <li>PowerShell의 <code>Get-FileHash</code>로 SHA-256을 구해 릴리스에 적힌 값과 비교합니다.</li>
            <li>같은 사용자 계정에서 설치 파일을 실행합니다.</li>
            <li>Rhythm을 다시 열어 공지와 설정이 남아 있는지 확인합니다. 데이터는 앱과 따로 저장되어 지워지지 않습니다.</li>
          </ol>
          <p class="footnote">설치 파일에 코드 서명이 없어 Windows 보안 경고가 나올 수 있습니다. 여는 방법은 저장소 README에 있습니다.</p>
          {:else}
          <ol class="manual-update">
            <li>메뉴 막대 아이콘이나 ⌘Q로 Rhythm을 종료합니다.</li>
            <li>필요하면 위의 백업 및 복원에서 백업을 만듭니다.</li>
            <li>github.com/Hiyabye/Rhythm의 Releases에서 최신 DMG를 받습니다.</li>
            <li>터미널의 <code>shasum -a 256</code>으로 SHA-256을 구해 릴리스에 적힌 값과 비교합니다.</li>
            <li>DMG를 열고 Rhythm을 응용 프로그램 폴더로 끌어 기존 앱을 바꿉니다.</li>
            <li>Rhythm을 다시 열어 공지와 설정이 남아 있는지 확인합니다. 데이터는 앱과 따로 저장되어 지워지지 않습니다.</li>
          </ol>
          <p class="footnote">Apple 공증을 받지 않아 새 버전을 처음 열 때 macOS 보안 경고가 나올 수 있습니다. 여는 방법은 저장소 README에 있습니다.</p>
          {/if}
        </details>
      </div>
    {/if}
  </div>
  {#if desktop && configured && error}<div class="banner error" role="alert"><div><strong>업데이트를 완료하지 못했습니다</strong><p>{error}</p></div><button class="text-button" disabled={checking || installing} onclick={progress?.phase === 'staged' ? install : check}>다시 시도</button></div>{/if}
  {#if !desktop}<p class="footnote">업데이트 확인과 설치는 데스크톱 앱에서 사용할 수 있습니다.</p>{/if}
</div>

<style>
  .update-progress { display: block; flex-basis: 100%; max-width: 280px; margin-top: 6px; }
</style>
