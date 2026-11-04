<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { SourceInfo, SyncStatus } from './types';
  import { archiveDate, archiveError, sourceStatus } from './archive-format';
  import Icon from './Icon.svelte';
  import Hint from './Hint.svelte';
  import Popover from './Popover.svelte';

  let { enabled }: { enabled: boolean } = $props();
  let sources = $state<SourceInfo[]>([]);
  let status = $state<SyncStatus | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let feedback = $state('');
  let active = true;
  let polling = false;
  let refreshGeneration = 0;
  let subscribed = $derived(sources.filter((source) => source.subscribed && source.supported));
  let supportedGroups = $derived(groups(sources.filter((source) => source.supported)));
  let unsupportedGroups = $derived(groups(sources.filter((source) => !source.supported)));
  let unsupportedCount = $derived(sources.filter((source) => !source.supported).length);
  let locked = $derived(!enabled || busy || loading || Boolean(status?.running));

  onMount(() => {
    void refresh();
    const timer = window.setInterval(() => {
      if (!active || polling || busy || loading || !enabled) return;
      polling = true;
      void invoke<SyncStatus>('archive_status').then(async (next) => {
        if (!active || busy || loading) return;
        const changed = status?.running !== next.running;
        status = next;
        if (next.running || changed) await refresh(false);
      }).catch((reason) => { if (active) error = archiveError(reason); }).finally(() => polling = false);
    }, 1000);
    return () => { active = false; refreshGeneration++; window.clearInterval(timer); };
  });

  function groups(list: SourceInfo[]): [string, SourceInfo[]][] {
    const map = new Map<string, SourceInfo[]>();
    for (const source of list) map.set(source.organization, [...(map.get(source.organization) ?? []), source]);
    return [...map];
  }

  function organizationLabel(organization: string): string {
    return organization.replace(/^서강대학교\s*·\s*/, '') || organization;
  }

  async function refresh(showLoading = true) {
    const request = ++refreshGeneration;
    if (showLoading) { loading = true; error = ''; }
    try {
      const [nextSources, nextStatus] = await Promise.all([invoke<SourceInfo[]>('archive_sources'), invoke<SyncStatus>('archive_status')]);
      if (!active || request !== refreshGeneration) return;
      sources = nextSources;
      status = nextStatus;
    } catch (reason) { if (active && request === refreshGeneration) error = archiveError(reason); }
    finally { if (active && request === refreshGeneration) loading = false; }
  }

  async function subscribe(source: SourceInfo) {
    if (busy || loading || !enabled) return;
    busy = true;
    refreshGeneration++;
    error = '';
    const subscribed = !source.subscribed;
    try {
      const next = await invoke<SourceInfo[]>('archive_subscribe', { id: source.id, subscribed });
      if (!active) return;
      sources = next;
      feedback = subscribed ? `${source.name} 구독을 시작했습니다. 가져오기를 눌러야 수집합니다.` : `${source.name} 구독을 해제했습니다. 보관한 공지는 남습니다.`;
    } catch (reason) { if (active) error = archiveError(reason); }
    finally { if (active) busy = false; }
  }

  async function start(mode: 'historical' | 'incremental' | 'skipped', ids: string[] | null = null) {
    busy = true;
    error = '';
    feedback = '';
    try {
      status = await invoke<SyncStatus>('archive_start', { sourceIds: ids, mode });
      feedback = status.message;
      await refresh(false);
    } catch (reason) { error = archiveError(reason); }
    finally { busy = false; }
  }

  async function openOriginal(url: string) {
    error = '';
    try { await invoke('archive_open_original', { url }); }
    catch (reason) { error = archiveError(reason); }
  }

  async function cancel() {
    busy = true;
    error = '';
    try { status = await invoke<SyncStatus>('archive_cancel'); feedback = '중단을 요청했습니다. 이미 저장한 공지는 유지됩니다.'; }
    catch (reason) { error = archiveError(reason); }
    finally { busy = false; }
  }

  function runningLabel(value: SyncStatus) {
    const mode = value.mode === 'historical' ? '과거 자료 가져오는 중' : value.mode === 'resource' ? '첨부 자료 내려받는 중' : value.mode === 'skipped' ? '가져오지 못한 공지 다시 확인 중' : '새 소식 확인 중';
    const source = sources.find((item) => item.id === value.source_id)?.name;
    return [mode, source, value.page > 0 ? `${value.page}쪽` : ''].filter(Boolean).join(' · ');
  }

  function historyLabel(source: SourceInfo) {
    return source.history_complete ? '공개 과거 자료 모두 가져옴' : !source.last_attempt ? '아직 가져오지 않음' : `과거 자료 ${source.next_page}쪽부터 이어받기`;
  }
</script>

<div class="source-manager" aria-label="서강대학교 공지 정보원 관리" aria-busy={loading || busy}>
  <div class="collect-bar">
    <button class="button small" disabled={locked || !subscribed.length} onclick={() => start('historical')}>과거 자료 가져오기</button>
    <button class="button secondary small" disabled={locked || !subscribed.length} onclick={() => start('incremental')}>새 소식 확인</button>
    <Hint label="가져오기">구독한 게시판 전체에 실행합니다. 구독만으로는 아무것도 가져오지 않습니다. 과거 자료는 중단해도 이어받을 수 있고, 이미 가져온 기록은 새 소식으로 세지 않습니다. 학교가 공개한 자료만 가져오며 로그인이나 접근 제한은 우회하지 않습니다.</Hint>
    <span class="collect-status" role="status" aria-live="polite">
      {#if status?.running}<Icon name="refresh" size={14} />{runningLabel(status)}{:else if !loading}구독 {subscribed.length}개{/if}
    </span>
    {#if status?.running}<button class="text-button" disabled={busy} aria-label="가져오기 중단" onclick={cancel}>중단</button>{/if}
  </div>
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  <p class="archive-feedback" role="status">{feedback}</p>
  {#if loading && !sources.length}<p class="loading" role="status">게시판 목록을 불러오는 중입니다…</p>{/if}

  {#snippet tile(source: SourceInfo, organization: string)}
    {@const running = status?.running && status.source_id === source.id}
    <li class="board-tile" class:on={source.subscribed} class:unsupported={!source.supported}>
      <button class="board-switch" role="switch" aria-checked={source.subscribed} aria-label={`${organization} ${source.name} 구독`} aria-describedby={source.last_error ? `board-alert-${source.id}` : undefined} disabled={locked || !source.supported} onclick={() => subscribe(source)}>
        <span class="board-mark" aria-hidden="true">{#if source.subscribed}<Icon name="check" size={13} />{/if}</span>
        <span class="board-name">{source.name}</span>
        {#if running}<span class="board-state spinning" aria-hidden="true"><Icon name="refresh" size={14} /></span>
        {:else if source.last_error}<span class="board-state alert" aria-hidden="true"><Icon name="warning" size={14} /></span>{/if}
      </button>
      {#if source.last_error}<span class="sr-only" id={`board-alert-${source.id}`}>최근 가져오기 오류</span>{/if}
      {#if source.supported}
        <button class="board-info" popovertarget={`board-${source.id}`} aria-label={`${organization} ${source.name} 상태`}><Icon name="info" size={15} /></button>
        <Popover id={`board-${source.id}`} label={`${source.name} 상태`} wide>
          <p class="popover-title">{source.name}</p>
          <p>{sourceStatus(source.status)} · {historyLabel(source)}</p>
          <dl class="popover-facts"><div><dt>마지막 시도</dt><dd>{archiveDate(source.last_attempt, true)}</dd></div><div><dt>마지막 성공</dt><dd>{archiveDate(source.last_success, true)}</dd></div>
            {#if source.discovered || source.archived || source.failed || source.status === 'running'}<div><dt>최근 시도</dt><dd>발견 {source.discovered} · 보관 {source.archived} · 실패 {source.failed}</dd></div>{/if}</dl>
          {#if source.last_error}<p class="archive-error">{archiveError(source.last_error)}</p>{/if}
          {#if source.skipped_count}
            <p class="popover-subtitle">가져오지 못한 공지 {source.skipped_count}건</p>
            <p>원문을 읽지 못해 건너뛰었고, 나머지 공지는 계속 가져왔습니다.</p>
            <ul class="skipped-list">
              {#each source.skipped as item (item.external_id)}
                <li><button class="text-button" onclick={() => openOriginal(item.url)}>공지 {item.external_id} 원문 열기</button><span>{item.archived ? '이전에 보관한 내용은 그대로 읽을 수 있습니다.' : archiveError(item.reason)}</span></li>
              {/each}
            </ul>
            {#if source.skipped_count > source.skipped.length}<p class="footnote">최근 {source.skipped.length}건만 표시합니다.</p>{/if}
          {/if}
          {#if source.subscribed}<div class="popover-actions"><button class="button secondary small" disabled={locked} onclick={() => start('historical', [source.id])}>{source.history_complete ? '과거 자료 다시 확인' : source.last_attempt ? '과거 자료 이어받기' : '과거 자료 가져오기'}</button><button class="button secondary small" disabled={locked} onclick={() => start('incremental', [source.id])}>새 소식 확인</button>{#if source.skipped_count}<button class="button secondary small" disabled={locked} onclick={() => start('skipped', [source.id])}>가져오지 못한 공지 다시 시도</button>{/if}</div>{/if}
          <code class="popover-code">{source.url}</code>
        </Popover>
      {/if}
    </li>
  {/snippet}

  {#each supportedGroups as [organization, members] (organization)}
    <h3 class="settings-group-title">{organizationLabel(organization)}</h3>
    <ul class="board-grid" aria-label={organization}>{#each members as source (source.id)}{@render tile(source, organization)}{/each}</ul>
  {/each}

  {#if unsupportedCount}
    <details class="board-unsupported">
      <summary>미지원 게시판 {unsupportedCount}개</summary>
      <p class="footnote">원문 수집을 확인하지 못한 게시판은 구독할 수 없습니다.</p>
      {#each unsupportedGroups as [organization, members] (organization)}
        <h3 class="settings-group-title">{organizationLabel(organization)}</h3>
        <ul class="board-grid" aria-label={organization}>{#each members as source (source.id)}{@render tile(source, organization)}{/each}</ul>
      {/each}
    </details>
  {/if}
</div>
