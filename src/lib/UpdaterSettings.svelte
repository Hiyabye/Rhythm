<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { confirmAction } from './confirm';
  import { isWindows } from './platform';
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
        if (await confirmAction('서명 검증이 끝난 업데이트를 지금 설치하고 앱을 다시 시작할까요?', '설치 및 다시 시작')) {
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
      const restart = await confirmAction('서명 검증을 마쳤습니다. 지금 업데이트를 설치하고 앱을 다시 시작할까요? 취소하면 업데이트는 이번 실행 중에만 준비된 상태로 유지됩니다.', '설치 및 다시 시작');
      if (restart) await invoke('updater_restart', { confirmed: true });
      else progress = { phase: 'staged', version: staged.version };
    }
    // After a successful download, only the restart failed; keep the staged package.
    catch (reason) { error = String(reason); progress = staged ? { phase: 'staged', version: staged.version } : null; }
    finally { installing = false; }
  }
</script>

<section id="about" class="settings-section about-section">
  <div class="section-heading"><h2 tabindex="-1">정보</h2><p>Rhythm 정보와 앱을 최신 버전으로 유지하는 방법을 확인합니다.</p></div>
  <div class="about-brand"><span class="beat" aria-hidden="true"><i></i><i></i><i></i><i></i></span><h2>Rhythm</h2><span>1.1.0</span></div>
  <p>나의 하루를 정돈하는 로컬 퍼스트 개인 커맨드 센터.</p>
  <p class="muted">조용하게 연결하고, 나에게 중요한 것에 집중합니다.</p>
  <div class="about-meta"><span>MIT 라이선스</span><span>github.com/Hiyabye/Rhythm</span></div>
  {#if desktop && configured}
    <div class="setting-row"><div><h3>앱 업데이트</h3><p>고정된 GitHub 안정 채널에서 서명과 현재 기기용 아키텍처를 확인합니다. 앱을 다시 시작할지는 설치 후 선택합니다.</p></div><button class="text-button" disabled={checking || installing} onclick={check}>{checking ? '확인 중…' : '업데이트 확인'}</button></div>
    <div class="setting-row"><div><h3>자동으로 업데이트 확인</h3><p>앱이 실행 중일 때 하루에 한 번 GitHub에서 새 버전이 있는지 확인합니다. 새 버전이 있으면 알려 드리며, 내려받기와 설치는 직접 선택할 때만 진행합니다.</p></div><button class="switch" class:on={autoCheck === true} role="switch" aria-label="자동으로 업데이트 확인" aria-checked={autoCheck === true} disabled={autoCheck === null || savingAutoCheck} onclick={toggleAutoCheck}><span></span></button></div>
    {#if update}<div class="setting-row"><div><h3>새 버전 {update.version}</h3><p>{update.body ?? '서명된 안정 버전을 설치할 수 있습니다.'}</p></div><button class="text-button" disabled={installing} onclick={install}>{installing ? '업데이트 중…' : progress?.phase === 'staged' ? '설치 및 다시 시작' : '다운로드 및 설치'}</button></div>{/if}
    {#if checked && !update}<p class="footnote" role="status">현재 최신 버전을 사용하고 있습니다.</p>{/if}
    {#if progress}<p class="footnote" role="status">{progress.phase === 'downloading' ? `다운로드 중${progress.total ? ` · ${Math.round((progress.downloaded ?? 0) / progress.total * 100)}%` : '…'}` : progress.phase === 'verifying' ? '다운로드한 서명 검증 중…' : progress.phase === 'installing' ? '검증된 업데이트를 설치 중…' : progress.phase === 'staged' ? '서명된 업데이트를 준비했습니다.' : '앱을 다시 시작합니다…'}</p>{/if}
    {#if error}<div class="banner error" role="alert"><div><strong>업데이트를 완료하지 못했습니다</strong><p>{error}</p></div><button class="text-button" disabled={checking || installing} onclick={progress?.phase === 'staged' ? install : check}>다시 시도</button></div>{/if}
  {:else if desktop && configured === false}
    <div class="setting-row"><div><h3>앱 업데이트</h3><p>자동 업데이트는 제공하지 않습니다. 새 버전은 아래 순서로 직접 설치합니다.</p></div></div>
    {#if isWindows}
    <ol class="manual-update">
      <li>Rhythm을 트레이 메뉴에서 완전히 종료합니다.</li>
      <li>필요하면 먼저 위의 백업 및 복원에서 백업 파일을 만듭니다.</li>
      <li>공식 GitHub 저장소(github.com/Hiyabye/Rhythm)의 Releases에서 최신 Windows 설치 파일을 내려받습니다.</li>
      <li>PowerShell에서 <code>Get-FileHash</code>로 설치 파일의 SHA-256을 확인하고, 릴리스에 게시된 값과 같은지 비교합니다.</li>
      <li>설치 파일을 실행해 같은 사용자 계정에 설치합니다.</li>
      <li>Rhythm을 다시 열고 보관한 공지와 설정이 그대로인지 확인합니다. 공지, 설정, AI 모델은 프로그램과 별도 폴더에 유지됩니다.</li>
    </ol>
    <p class="footnote">Windows 설치 파일에는 정식 코드 서명이 없어 처음 실행할 때 Windows 보안 경고가 표시될 수 있습니다. 확인 방법은 저장소의 README에 있습니다.</p>
    {:else}
    <ol class="manual-update">
      <li>메뉴 막대 아이콘이나 ⌘Q로 Rhythm을 완전히 종료합니다.</li>
      <li>필요하면 먼저 위의 백업 및 복원에서 백업 파일을 만듭니다.</li>
      <li>공식 GitHub 저장소(github.com/Hiyabye/Rhythm)의 Releases에서 최신 DMG를 내려받습니다.</li>
      <li>터미널에서 <code>shasum -a 256</code>으로 DMG의 SHA-256을 확인하고, 릴리스에 게시된 값과 같은지 비교합니다.</li>
      <li>DMG를 열고 Rhythm을 응용 프로그램 폴더로 끌어 기존 앱을 바꿉니다.</li>
      <li>Rhythm을 다시 열고 보관한 공지와 설정이 그대로인지 확인합니다. 공지, 설정, AI 모델은 앱과 별도로 저장되어 앱을 바꿔도 지워지지 않습니다.</li>
    </ol>
    <p class="footnote">Rhythm은 Ad-hoc 서명만 되어 있고 Apple 공증을 받지 않았습니다. 새 버전을 처음 열 때 macOS 보안 경고가 표시될 수 있습니다. 확인 방법은 저장소의 README에 있습니다.</p>
    {/if}
  {:else if !desktop}<p class="footnote">업데이트 확인과 설치는 데스크톱 앱에서 사용할 수 있습니다.</p>{/if}
</section>
