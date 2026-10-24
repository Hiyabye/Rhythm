<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import Icon from './lib/Icon.svelte';
  import type { EnergyProfile, Settings, Snapshot, Theme, WorkerStatus } from './lib/types';

  const navigation = [
    { id: 'today', label: '오늘', caption: '나에게 필요한 하루의 흐름' },
    { id: 'inbox', label: '받은 소식', caption: '여러 곳의 소식을 한곳에서' },
    { id: 'search', label: '탐색', caption: '쌓인 정보에서 필요한 것을 발견하세요' },
    { id: 'bookmark', label: '보관함', caption: '다시 보고 싶은 정보의 자리' },
    { id: 'settings', label: '설정', caption: '나의 방식으로 Rhythm을 조율하세요' },
  ];
  const categories = [
    ['general', '일반'], ['extensions', '확장 기능'], ['ai', 'AI 모델'],
    ['personalization', '개인화'], ['notifications', '알림'],
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
  let interestDraft = $state('');
  let query = $state('');
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

  function message(reason: unknown): string {
    return typeof reason === 'string' ? reason : '작업을 완료하지 못했습니다. 잠시 후 다시 시도해 주세요.';
  }

  async function load() {
    loading = true;
    error = '';
    try {
      snapshot = await invoke<Snapshot>('get_snapshot');
      theme = snapshot.settings.theme;
      interestDraft = snapshot.settings.interests;
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

  async function refreshWorker() {
    if (!snapshot || busy) return;
    busy = true;
    error = '';
    try { snapshot.worker = await invoke<WorkerStatus>('worker_status'); }
    catch (reason) { error = message(reason); }
    finally { busy = false; }
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
    <div class="sidebar-bottom"><span class="local-dot"></span><span>내 기기에 머무는 정보</span><small>v0.1.0 · Foundation</small></div>
  </aside>

  <main id="main" tabindex="-1">
    <header class="page-header"><div><div class="eyebrow">{page === 'today' ? date : '나의 커맨드 센터'}</div><h1>{heading.label}</h1><p>{heading.caption}</p></div><span class="header-note"><Icon name="lock" size={14} /> 로컬 퍼스트</span></header>
    {#if !desktop && !loading}<div class="banner browser-notice" role="status"><Icon name="info" /><div><strong>데스크톱 앱에서 시작해 주세요</strong><p>브라우저에서는 화면만 둘러볼 수 있습니다. 설정 저장과 로컬 데이터 접근은 Rhythm 앱에서 사용할 수 있습니다.</p></div></div>{/if}
    {#if error}<div class="banner error" role="alert"><Icon name="info" /><div><strong>작업을 완료하지 못했습니다</strong><p>{error}</p></div>{#if !snapshot && desktop}<button class="text-button" onclick={load}>다시 시도</button>{/if}</div>{/if}
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
            {#if snapshot}{#each snapshot.extensions as extension}<div class="extension-row"><div class="extension-symbol"><Icon name="inbox" size={22} /></div><div class="extension-description"><h3>{extension.display_name}</h3><p>{extension.description}</p><span class="availability">{extension.available ? '사용 가능' : '준비 중 · 소식 동기화는 아직 제공되지 않습니다'}</span></div><button class="switch" class:on={extension.enabled} role="switch" aria-checked={extension.enabled} aria-label={`${extension.display_name} 사용 설정`} disabled={busy} onclick={() => toggleExtension(extension.id, !extension.enabled)}><span></span></button></div>{/each}{:else}<p class="muted">{loading ? '확장 기능을 불러오는 중입니다.' : '데스크톱 앱에서 등록된 확장 기능을 확인할 수 있습니다.'}</p>{/if}
            <p class="footnote">준비 중인 확장 기능은 사용 설정만 저장하며, 동기화나 알림을 실행하지 않습니다.</p>
          </section>
          <section id="ai" class="settings-section"><div class="section-heading"><h2>AI 모델</h2><p>나의 기기에서 실행하는 AI. 정보는 외부 추론 서버로 보내지 않습니다.</p></div>
            <div class="setting-row"><div><h3>AI 엔진</h3><p>별도 프로세스에서 실행되는 mistral.rs</p></div><div class="inline-controls"><span class="engine-state" class:healthy={snapshot?.worker.state === 'ready'}><span></span>{!snapshot ? '상태 확인 전' : snapshot.worker.state === 'ready' ? (snapshot.worker.runtime_available ? '엔진 대기 중' : '추론 런타임 없음') : snapshot.worker.state === 'failed' ? '응답 없음' : '사용할 수 없음'}</span><button class="icon-button" disabled={!snapshot || busy} aria-label="AI 엔진 상태 새로고침" onclick={refreshWorker}><Icon name="refresh" size={17} /></button></div></div>
            {#if snapshot?.worker.detail}<p class="footnote">{snapshot.worker.detail}</p>{/if}
            <div class="model-empty"><Icon name="cpu" size={28} /><div><h3>{snapshot?.worker.model ? '설치된 모델' : '아직 설치된 모델이 없습니다'}</h3><p>{snapshot?.worker.model ?? '모델 없이도 정보 탐색과 설정을 사용할 수 있습니다.'}</p></div></div>
            <div class="planned"><strong>모델 선택 · 향후 제공</strong><p>Qwen3.5 2B · 4B · 9B를 지원할 예정입니다. 모델 다운로드와 관리 기능은 v0.3.0에서 준비합니다.</p></div>
          </section>
          <section id="personalization" class="settings-section"><div class="section-heading"><h2>개인화</h2><p>어떤 소식에 관심이 있는지 직접 기록해 두세요.</p></div>
            <label class="field-label" for="interests">나의 관심사</label><textarea id="interests" bind:value={interestDraft} disabled={!snapshot || busy} maxlength="4000" rows="4" placeholder="예: 컴퓨터 비전, 장학금, 학부 연구 참여"></textarea>
            <div class="field-footer"><span>관심사는 이 기기에만 저장됩니다.</span><button class="button small" disabled={!snapshot || busy || interestDraft === snapshot.settings.interests} onclick={() => save({ interests: interestDraft })}>{busy ? '저장 중…' : '관심사 저장'}</button></div>
            <div class="planned"><strong>취향과 기억 · 향후 제공</strong><p>자동 취향 학습과 개인 기억 관리는 아직 제공되지 않습니다. 현재 관심사는 추천이나 순위에 반영되지 않습니다.</p></div>
          </section>
          <section id="notifications" class="settings-section"><div class="section-heading"><h2>알림</h2><p>중요한 소식이 생기면, 필요한 순간에만.</p></div><div class="planned"><strong>알림 설정 · 향후 제공</strong><p>이 버전은 시스템 알림을 보내지 않습니다. 알림 종류와 수신 시간 설정은 v0.5.0에서 제공할 예정입니다.</p></div></section>
          <section id="energy" class="settings-section"><div class="section-heading"><h2>성능 및 전원</h2><p>기기에 맞는 작업 방식을 선택하세요.</p></div><div class="energy-options" role="group" aria-label="에너지 프로필">{#each energyProfiles as option}<button disabled={!snapshot || busy} class:selected={snapshot?.settings.energy_profile === option.id} aria-pressed={snapshot?.settings.energy_profile === option.id} onclick={() => save({ energy_profile: option.id })}><span class="option-title">{option.label}{#if snapshot?.settings.energy_profile === option.id}<Icon name="check" size={17} />{/if}</span><span>{option.description}</span></button>{/each}</div><p class="footnote">현재는 선택한 프로필만 저장합니다. AI 실행 빈도와 메모리 사용량 조절은 향후 스케줄러에 적용됩니다.</p></section>
          <section id="data" class="settings-section"><div class="section-heading"><h2>데이터 및 개인정보</h2><p>내 정보는 내 기기에. 클라우드 계정이나 동기화가 필요하지 않습니다.</p></div>
            <div class="setting-row"><div><h3>로컬 데이터베이스</h3><p>SQLite · {snapshot ? `스키마 버전 ${snapshot.storage.schema_version}` : '앱에서 확인 가능'}</p></div><span class="value">{snapshot ? storageSize(snapshot.storage.bytes) : '—'}</span></div>
            <div class="storage-path"><span>저장 위치</span><code>{snapshot?.storage.path ?? '데스크톱 앱에서 확인할 수 있습니다.'}</code></div>
            <div class="privacy-note"><Icon name="lock" size={18} /><p>설정과 관심사는 로컬에 저장됩니다. 클라우드 추론, 원격 저장소, 사용 정보 수집을 사용하지 않습니다.</p></div>
            <div class="planned"><strong>백업 및 복원 · 향후 제공</strong><p>이 버전에는 백업과 복원 기능이 없습니다. 앱 업데이트 시에는 마이그레이션으로 기존 데이터를 유지합니다.</p></div>
          </section>
          <section id="about" class="settings-section about-section"><div class="about-brand"><span class="beat" aria-hidden="true"><i></i><i></i><i></i><i></i></span><h2>Rhythm</h2><span>0.1.0</span></div><p>나의 하루를 정돈하는 로컬 퍼스트 개인 커맨드 센터.</p><p class="muted">조용하게 연결하고, 나에게 중요한 것에 집중합니다.</p><div class="about-meta"><span>MIT 라이선스</span><span>github.com/Hiyabye/Rhythm</span></div></section>
        </div>
      </div>
    {:else if page === 'today'}
      <div class="today-intro"><span class="section-kicker">하루의 시작을 조금 더 가볍게</span><h2>중요한 것에 집중할 수 있도록.</h2><p>여러 곳에 흩어진 소식과 다가오는 일정을<br class="desktop-break" /> 나만의 흐름으로 정리할 공간입니다.</p></div>
      <section class="briefing-empty"><div class="empty-glyph"><span class="beat large" aria-hidden="true"><i></i><i></i><i></i><i></i></span></div><span class="section-kicker">나만의 브리핑</span><h3>아직 도착한 소식이 없어요</h3><p>정보원이 연결되면 관심 있는 소식과 중요한 마감일을<br class="desktop-break" /> 이곳에서 한눈에 확인할 수 있습니다.</p><button class="button" onclick={() => navigate('settings')}>나의 공간 설정하기 <Icon name="arrow" size={16} /></button><span class="quiet-caption">현재는 앱의 기반을 준비하는 첫 번째 버전입니다.</span></section>
      <div class="principles"><div><span>01</span><h3>내 기기에 안전하게</h3><p>정보와 관심사는 내 기기에 남습니다.</p></div><div><span>02</span><h3>필요한 정보만 차분하게</h3><p>앞으로 연결될 소식을 위한 나만의 자리.</p></div><div><span>03</span><h3>나의 속도에 맞춰</h3><p>설정에서 테마와 에너지 프로필을 골라보세요.</p></div></div>
    {:else if page === 'search'}
      <form class="search-form" onsubmit={(event) => event.preventDefault()} role="search"><Icon name="search" size={22} /><label class="sr-only" for="archive-search">보관된 정보 검색</label><input id="archive-search" type="search" bind:value={query} placeholder="어떤 정보를 찾고 있나요?" autocomplete="off" /><span class="search-hint">로컬 검색</span></form>
      <div class="search-meta"><span>보관된 문서 0개</span><span>검색 기능 준비 중</span></div>
      <section class="empty-state"><span class="empty-icon"><Icon name="search" size={32} /></span><h2>{query.trim() ? '아직 검색할 문서가 없습니다' : '필요한 정보를 찾을 공간'}</h2><p>{query.trim() ? '현재는 문서가 수집되거나 색인되지 않았습니다. 검색어는 저장하거나 외부로 보내지 않습니다.' : '앞으로 수집한 소식을 제목과 내용으로 검색할 수 있습니다. 지금은 검색 화면의 기반만 준비되어 있습니다.'}</p><span class="quiet-caption">로컬 문서 검색은 v0.2.0에서 제공할 예정입니다.</span></section>
    {:else if page === 'inbox'}
      <div class="collection-heading"><span>모든 소식 <b>0</b></span><span>정보원 연결 준비 중</span></div><section class="empty-state"><span class="empty-icon"><Icon name="inbox" size={34} /></span><h2>새로운 소식이 모이는 곳</h2><p>확장 기능이 연결되면 여러 정보원의 소식이 이곳에 모입니다. 아직 수집된 소식은 없습니다.</p><button class="button secondary" onclick={() => navigate('settings')}>확장 기능 살펴보기 <Icon name="arrow" size={16} /></button><span class="quiet-caption">서강대학교 공지 수집은 v0.2.0에서 시작합니다.</span></section>
    {:else}
      <div class="collection-heading"><span>저장한 항목 <b>0</b></span><span>내가 고른 중요한 정보</span></div><section class="empty-state"><span class="empty-icon"><Icon name="bookmark" size={32} /></span><h2>다시 꺼내 보고 싶은 소식</h2><p>앞으로 중요한 공지와 기회를 저장하면 이곳에서 모아볼 수 있습니다. 아직 보관된 항목은 없습니다.</p><span class="quiet-caption">소식 저장 기능은 공지 아카이브와 함께 제공할 예정입니다.</span></section>
    {/if}
    <footer class="page-footer"><span>Rhythm</span><span>조용하게, 나의 리듬으로.</span></footer>
  </main>
</div>
