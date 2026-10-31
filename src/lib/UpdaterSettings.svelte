<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  type UpdateInfo = { version: string; currentVersion: string; date: string | null; body: string | null };
  type Progress = { phase: 'downloading' | 'verifying' | 'installing' | 'staged' | 'restarting'; version?: string; downloaded?: number; total?: number | null };
  let { desktop }: { desktop: boolean } = $props();
  let checked = $state(false);
  let checking = $state(false);
  let installing = $state(false);
  let error = $state('');
  let progress = $state<Progress | null>(null);

  $effect(() => {
    if (!desktop) return;
    let mounted = true;
    let stop: (() => void) | undefined;
    void listen<Progress>('updater://progress', (event) => { progress = event.payload; }).then((unlisten) => {
      if (mounted) stop = unlisten;
      else unlisten();
    }).catch((reason) => { error = String(reason); });
    return () => { mounted = false; stop?.(); };
  });

  let update = $state<UpdateInfo | null>(null);
  async function check() {
    if (!desktop || checking || installing) return;
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
    if (!desktop || installing || !update) return;
    if (progress?.phase === 'staged') {
      if (!window.confirm('서명 검증이 끝난 업데이트를 지금 설치하고 앱을 다시 시작할까요?')) return;
      installing = true;
      error = '';
      try { await invoke('updater_restart', { confirmed: true }); }
      catch (reason) { error = String(reason); }
      finally { installing = false; }
      return;
    }
    installing = true;
    error = '';
    progress = null;
    try {
      update = await invoke<UpdateInfo>('updater_install');
      const restart = window.confirm('서명 검증을 마쳤습니다. 지금 업데이트를 설치하고 앱을 다시 시작할까요? 취소하면 업데이트는 이번 실행 중에만 준비된 상태로 유지됩니다.');
      if (restart) await invoke('updater_restart', { confirmed: true });
      else progress = { phase: 'staged', version: update.version };
    } catch (reason) { error = String(reason); }
    finally { installing = false; }
  }
</script>

<section id="about" class="settings-section about-section">
  <div class="section-heading"><h2>정보</h2><p>공식 안정 채널의 서명된 업데이트와 Rhythm 정보를 확인합니다.</p></div>
  <div class="about-brand"><span class="beat" aria-hidden="true"><i></i><i></i><i></i><i></i></span><h2>Rhythm</h2><span>0.9.0</span></div>
  <p>나의 하루를 정돈하는 로컬 퍼스트 개인 커맨드 센터.</p>
  <p class="muted">조용하게 연결하고, 나에게 중요한 것에 집중합니다.</p>
  <div class="about-meta"><span>MIT 라이선스</span><span>github.com/Hiyabye/Rhythm</span></div>
  {#if desktop}
    <div class="setting-row"><div><h3>앱 업데이트</h3><p>고정된 GitHub 안정 채널에서 서명과 현재 기기용 아키텍처를 확인합니다. 앱을 다시 시작할지는 설치 후 선택합니다.</p></div><button class="text-button" disabled={checking || installing} onclick={check}>{checking ? '확인 중…' : '업데이트 확인'}</button></div>
    {#if update}<div class="setting-row"><div><h3>새 버전 {update.version}</h3><p>{update.body ?? '서명된 안정 버전을 설치할 수 있습니다.'}</p></div><button class="text-button" disabled={installing} onclick={install}>{installing ? '업데이트 중…' : progress?.phase === 'staged' ? '설치 및 다시 시작' : '다운로드 및 설치'}</button></div>{/if}
    {#if checked && !update}<p class="footnote" role="status">현재 최신 버전을 사용하고 있습니다.</p>{/if}
    {#if progress}<p class="footnote" role="status">{progress.phase === 'downloading' ? `다운로드 중${progress.total ? ` · ${Math.round((progress.downloaded ?? 0) / progress.total * 100)}%` : '…'}` : progress.phase === 'verifying' ? '다운로드한 서명 검증 중…' : progress.phase === 'installing' ? '검증된 업데이트를 설치 중…' : progress.phase === 'staged' ? '서명된 업데이트를 준비했습니다.' : '앱을 다시 시작합니다…'}</p>{/if}
    {#if error}<div class="banner error" role="alert"><div><strong>업데이트를 완료하지 못했습니다</strong><p>{error}</p></div><button class="text-button" disabled={checking || installing} onclick={check}>다시 시도</button></div>{/if}
  {:else}<p class="footnote">업데이트 확인과 설치는 데스크톱 앱에서 사용할 수 있습니다.</p>{/if}
</section>
