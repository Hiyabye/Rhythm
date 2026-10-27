<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import Icon from './lib/Icon.svelte';
  import ArchiveView from './lib/ArchiveView.svelte';
  import SourceSettings from './lib/SourceSettings.svelte';
  import AiSettings from './lib/AiSettings.svelte';
  import PersonalizationSettings from './lib/PersonalizationSettings.svelte';
  import './lib/ai.css';
  import { archiveError as message } from './lib/archive-format';
  import type { EnergyProfile, Settings, Snapshot, Theme, WorkerStatus, AiSnapshot } from './lib/types';

  const navigation = [
    { id: 'today', label: '오늘', caption: '나에게 필요한 하루의 흐름' },
    { id: 'inbox', label: '받은 소식', caption: '여러 곳의 소식을 한곳에서' },
    { id: 'search', label: '탐색', caption: '쌓인 정보에서 필요한 것을 발견하세요' },
    { id: 'bookmark', label: '보관함', caption: '다시 보고 싶은 정보의 자리' },
    { id: 'settings', label: '설정', caption: '나의 방식으로 Rhythm을 조율하세요' },
  ];
  const categories = [
    ['general', '일반'], ['extensions', '확장 기능'], ['ai', 'AI 모델'],
    ['ai-processing', 'AI 처리'], ['personalization', '개인화'], ['notifications', '알림'],
    ['energy', '성능 및 전원'], ['data', '데이터 및 개인정보'], ['about', '정보'],
  ];
  const themes: { id: Theme; label: string; icon: string }[] = [
    { id: 'light', label: '라이트', icon: 'today' },
    { id: 'dark', label: '다크', icon: 'moon' },
    { id: 'system', label: '시스템', icon: 'monitor' },
  ];
  const energyProfiles: { id: EnergyProfile; label: string; description: string }[] = [
    { id: 'eco', label: '절전', description: '에너지 사용을 최소한으로' },
    { id: 'balanced', label: '균형', description: '응답성과 전력의 균형' },
    { id: 'performance', label: '성능', description: '빠른 작업 처리를 우선으로' },
  ];
  let page = $state('today');
  let snapshot = $state<Snapshot | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let desktop = $state(false);
  let setupNeeded = $state(false);
  let noticesEnabled = $derived(snapshot?.extensions.some((extension) => extension.id === 'sogang_notices' && extension.enabled) ?? false);
  let theme = $state<Theme>('system');
  let systemDark = $state(false);
  let heading = $derived(navigation.find((item) => item.id === page)!);
  let dark = $derived(theme === 'dark' || (theme === 'system' && systemDark));
  let date = new Intl.DateTimeFormat('ko-KR', { month: 'long', day: 'numeric', weekday: 'long' }).format(new Date());

  $effect(() => {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
  });

  onMount(() => {
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    systemDark = media.matches;
    const update = () => { systemDark = media.matches; };
    media.addEventListener('change', update);
    desktop = isTauri();
    if (desktop) void load();
    else loading = false;
    const healthTimer = window.setInterval(() => {
      if (desktop && snapshot && page === 'settings' && !busy) {
        void invoke<WorkerStatus>('worker_status').then((status) => {
          if (snapshot) snapshot.worker = status;
        }).catch(() => { /* Explicit refresh also exposes transport errors. */ });
      }
    }, 5000);
    return () => {
      media.removeEventListener('change', update);
      window.clearInterval(healthTimer);
    };
  });

  async function navigate(destination: string) {
    page = destination;
    notice = '';
    history.replaceState(null, '', location.pathname + location.search);
    await tick();
    window.scrollTo({ top: 0, behavior: 'instant' });
    document.getElementById('main')?.focus({ preventScroll: true });
  }

  async function load() {
    loading = true;
    error = '';
    try {
      snapshot = await invoke<Snapshot>('get_snapshot');
      theme = snapshot.settings.theme;
      try { setupNeeded = !(await invoke<AiSnapshot>('ai_snapshot')).preferences.setup_completed; }
      catch { /* AI availability must not prevent archive access. Settings exposes recovery. */ }
    } catch (reason) {
      error = message(reason);
    } finally {
      loading = false;
    }
  }

  async function save(patch: Partial<Settings>) {
    if (!snapshot || busy) return;
    busy = true;
    error = '';
    notice = '';
    if (patch.theme) theme = patch.theme;
    try {
      snapshot = await invoke<Snapshot>('save_settings', { settings: { ...snapshot.settings, ...patch } });
      theme = snapshot.settings.theme;
      notice = '설정을 저장했습니다.';
    } catch (reason) {
      theme = snapshot.settings.theme;
      error = message(reason);
    } finally {
      busy = false;
    }
  }

  async function toggleExtension(id: string, enabled: boolean) {
    if (busy) return;
    busy = true;
    error = '';
    notice = '';
    try {
      snapshot = await invoke<Snapshot>('set_extension', { id, enabled });
      notice = '확장 기능 사용 설정을 저장했습니다.';
    } catch (reason) {
      error = message(reason);
    } finally { busy = false; }
  }

  async function aiSettings() {
    await navigate('settings');
    await tick();
    document.getElementById('ai')?.scrollIntoView({ block: 'start', behavior: 'instant' });
    document.getElementById('ai')?.querySelector('h2')?.focus({ preventScroll: true });
  }

  function storageSize(bytes: number) {
    return bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
</script>

<svelte:head><title>Rhythm</title><meta name="description" content="나의 하루를 정돈하는 로컬 퍼스트 개인 커맨드 센터" /></svelte:head>

<a class="skip-link" href="#main">본문으로 건너뛰기</a>
<div class="app-shell">
  <aside class="sidebar" aria-label="주 메뉴">
    <div class="brand"><span class="beat" aria-hidden="true"><i></i><i></i><i></i><i></i></span><span>Rhythm</span></div>
    <div class="sidebar-kicker">나의 공간</div>
    <nav aria-label="기본 탐색">
      {#each navigation as item}
        <button class:active={page === item.id} aria-label={item.label} aria-current={page === item.id ? 'page' : undefined} onclick={() => navigate(item.id)}>
          <Icon name={item.id} /><span>{item.label}</span>{#if page === item.id}<span class="nav-mark" aria-hidden="true"></span>{/if}
        </button>
      {/each}
    </nav>
    <div class="sidebar-bottom"><span class="local-dot"></span><span>내 기기에 머무는 정보</span><small>v0.4.0 · 로컬 인텔리전스</small></div>
  </aside>

  <main id="main" tabindex="-1">
    <header class="page-header"><div><div class="eyebrow">{page === 'today' ? date : '나의 커맨드 센터'}</div><h1>{heading.label}</h1><p>{heading.caption}</p></div><span class="header-note"><Icon name="lock" size={14} /> 로컬 퍼스트</span></header>
    {#if !desktop && !loading}<div class="banner browser-notice" role="status"><Icon name="info" /><div><strong>데스크톱 앱에서 시작해 주세요</strong><p>브라우저에서는 화면만 둘러볼 수 있습니다. 설정 저장과 로컬 데이터 접근은 Rhythm 앱에서 사용할 수 있습니다.</p></div></div>{/if}
    {#if error}<div class="banner error" role="alert"><Icon name="info" /><div><strong>작업을 완료하지 못했습니다</strong><p>{error}</p></div>{#if !snapshot && desktop}<button class="text-button" onclick={load}>다시 시도</button>{/if}</div>{/if}
    {#if desktop && setupNeeded && page !== 'settings'}<div class="banner ai-setup-banner" role="status"><Icon name="cpu" /><div><strong>나의 기기에서 공지의 핵심을 정리하세요</strong><p>로컬 AI 모델을 선택하면 한국어 요약과 일정의 근거를 확인할 수 있습니다. 준비 전이나 오프라인에서도 공지는 계속 읽을 수 있습니다.</p></div><button class="text-button" onclick={aiSettings}>로컬 AI 설정하기</button><button class="text-button" onclick={() => setupNeeded = false}>나중에</button></div>{/if}
    <div class="save-notice" class:visible={Boolean(notice)} role="status" aria-live="polite">{#if notice}<Icon name="check" size={15} />{notice}{/if}</div>

    {#if page === 'settings'}
      <div class="settings-layout">
        <nav class="settings-nav" aria-label="설정 항목">{#each categories as [id, label]}<a href={`#${id}`}>{label}</a>{/each}</nav>
        <div class="settings-content" aria-busy={busy || loading}>
          {#if loading}<div class="loading" role="status">저장된 설정을 불러오는 중입니다…</div>{/if}
          <section id="general" class="settings-section"><div class="section-heading"><h2>일반</h2><p>편안하게 집중할 수 있는 환경을 만드세요.</p></div>
            <div class="setting-row"><div><h3>화면 테마</h3><p>시스템 설정에 맞추거나 직접 선택하세요.</p></div><div class="segmented" role="group" aria-label="화면 테마">{#each themes as option}<button disabled={!snapshot || busy} class:selected={theme === option.id} aria-pressed={theme === option.id} onclick={() => save({ theme: option.id })}><Icon name={option.icon} size={16} />{option.label}</button>{/each}</div></div>
            <div class="setting-row"><div><h3>앱 언어</h3><p>Rhythm은 한국어로 함께합니다.</p></div><span class="value">한국어</span></div>
            <div class="setting-row"><div><h3>시작과 종료</h3><p>마지막으로 저장한 설정으로 시작하며, 창을 닫으면 앱도 종료됩니다.</p></div></div>
          </section>
          <section id="extensions" class="settings-section"><div class="section-heading"><h2>확장 기능</h2><p>필요한 정보원만 선택하세요. 꺼도 저장된 정보는 지워지지 않습니다.</p></div>
            {#if snapshot}{#each snapshot.extensions as extension}<div class="extension-row"><div class="extension-symbol"><Icon name="inbox" size={22} /></div><div class="extension-description"><h3>{extension.display_name}</h3><p>{extension.description}</p><span class="availability">{extension.available ? '사용 가능 · 수동 동기화' : '현재 사용할 수 없습니다'}</span></div><button class="switch" class:on={extension.enabled} role="switch" aria-checked={extension.enabled} aria-label={`${extension.display_name} 사용 설정`} disabled={busy} onclick={() => toggleExtension(extension.id, !extension.enabled)}><span></span></button></div>{/each}{:else}<p class="muted">{loading ? '확장 기능을 불러오는 중입니다.' : '데스크톱 앱에서 등록된 확장 기능을 확인할 수 있습니다.'}</p>{/if}
            {#if noticesEnabled}<SourceSettings enabled={noticesEnabled} />{:else}<p class="footnote">서강대학교 공지를 켜면 정보원을 구독하고 자료를 가져올 수 있습니다. 꺼도 이미 보관한 공지와 구독 설정은 유지됩니다.</p>{/if}
          </section>
          <AiSettings {desktop} onsetup={() => setupNeeded = false} />
          <PersonalizationSettings {desktop} />
          <section id="notifications" class="settings-section"><div class="section-heading"><h2>알림</h2><p>중요한 소식이 생기면, 필요한 순간에만.</p></div><div class="planned"><strong>알림 설정 · 향후 제공</strong><p>이 버전은 시스템 알림을 보내지 않습니다. 알림 종류와 수신 시간 설정은 v0.5.0에서 제공할 예정입니다.</p></div></section>
          <section id="energy" class="settings-section"><div class="section-heading"><h2>성능 및 전원</h2><p>기기에 맞는 작업 방식을 선택하세요.</p></div><div class="energy-options" role="group" aria-label="에너지 프로필">{#each energyProfiles as option}<button disabled={!snapshot || busy} class:selected={snapshot?.settings.energy_profile === option.id} aria-pressed={snapshot?.settings.energy_profile === option.id} onclick={() => save({ energy_profile: option.id })}><span class="option-title">{option.label}{#if snapshot?.settings.energy_profile === option.id}<Icon name="check" size={17} />{/if}</span><span>{option.description}</span></button>{/each}</div><p class="footnote">AI는 한 번에 한 공지만 분석합니다. 절전에서는 직접 요청을 우선하며, 모델 유지와 최근 공지의 소량 처리는 선택한 프로필과 AI 처리 설정을 따릅니다. 앱 종료 후 백그라운드 처리는 하지 않습니다.</p></section>
          <section id="data" class="settings-section"><div class="section-heading"><h2>데이터 및 개인정보</h2><p>내 정보는 내 기기에. 클라우드 계정이나 동기화가 필요하지 않습니다.</p></div>
            <div class="setting-row"><div><h3>로컬 데이터베이스</h3><p>SQLite · {snapshot ? `스키마 버전 ${snapshot.storage.schema_version}` : '앱에서 확인 가능'}</p></div><span class="value">{snapshot ? storageSize(snapshot.storage.bytes) : '—'}</span></div>
            <div class="storage-path"><span>저장 위치</span><code>{snapshot?.storage.path ?? '데스크톱 앱에서 확인할 수 있습니다.'}</code></div>
            <div class="privacy-note"><Icon name="lock" size={18} /><p>프로필, 관심 분야, 피드백과 학습 근거는 로컬 SQLite에만 저장합니다. 클라우드 추론, 원격 저장소, 사용 정보 수집을 사용하지 않습니다.</p></div>
            <div class="planned"><strong>백업 및 복원 · 향후 제공</strong><p>이 버전에는 백업과 복원 기능이 없습니다. 앱 업데이트 시에는 마이그레이션으로 기존 데이터를 유지합니다.</p></div>
          </section>
          <section id="about" class="settings-section about-section"><div class="about-brand"><span class="beat" aria-hidden="true"><i></i><i></i><i></i><i></i></span><h2>Rhythm</h2><span>0.4.0</span></div><p>나의 하루를 정돈하는 로컬 퍼스트 개인 커맨드 센터.</p><p class="muted">조용하게 연결하고, 나에게 중요한 것에 집중합니다.</p><div class="about-meta"><span>MIT 라이선스</span><span>github.com/Hiyabye/Rhythm</span></div></section>
        </div>
      </div>
    {:else}
      <ArchiveView {page} {desktop} enabled={noticesEnabled} onsettings={() => navigate('settings')} onaisettings={aiSettings} />
    {/if}
    <footer class="page-footer"><span>Rhythm</span><span>조용하게, 나의 리듬으로.</span></footer>
  </main>
</div>
