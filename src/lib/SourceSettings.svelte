<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { SourceInfo, SubscribeResult, SyncStatus } from './types';
  import { archiveDate, archiveError, sourceStatus } from './archive-format';
  import Icon from './Icon.svelte';
  import Hint from './Hint.svelte';
  import Popover from './Popover.svelte';
  import CybercampusSettings from './CybercampusSettings.svelte';

  // Each source extension's pane manages only the sources that extension owns.
  let { enabled, extension }: { enabled: boolean; extension: string } = $props();
  let catalog = $state<SourceInfo[]>([]);
  let sources = $derived(catalog.filter((source) => source.extension === extension));
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
  // Subscriptions stay open while a job runs; only starting another job waits.
  let switchesLocked = $derived(!enabled || busy || loading);
  let locked = $derived(switchesLocked || Boolean(status?.running));

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
      catalog = nextSources;
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
      const next = await invoke<SubscribeResult>('archive_subscribe', { id: source.id, subscribed });
      if (!active) return;
      catalog = next.sources;
      const running = Boolean(status?.running);
      feedback = !subscribed ? `${source.name} 구독을 해제했습니다.${running ? ' 이 게시판의 가져오기만 멈춥니다.' : ''} 보관한 공지는 남습니다.`
        : next.joined ? `${source.name} 구독을 시작했습니다. 진행 중인 새로고침에서 함께 가져옵니다.`
        : running ? `${source.name} 구독을 시작했습니다. 지금 진행 중인 작업이 끝난 뒤 새로고침을 눌러야 수집합니다.`
        : `${source.name} 구독을 시작했습니다. 가져오기를 눌러야 수집합니다.`;
    } catch (reason) { if (active) error = archiveError(reason); }
    finally { if (active) busy = false; }
  }

  async function start(mode: 'refresh' | 'skipped' | 'resources', ids: string[] = subscribed.map((source) => source.id)) {
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
    const labels: Record<string, string> = { refresh: '새로고침 준비 중', incremental: '새 소식 확인 중', historical: '이전 공지 가져오는 중', resource: '첨부 자료 내려받는 중', skipped: '가져오지 못한 공지 다시 확인 중', resources: '첨부 자료 모두 내려받는 중' };
    const boards = value.active.filter((board) => sources.some((item) => item.id === board.source_id));
    if (boards.length > 1) return `게시판 ${boards.length}개를 동시에 가져오는 중`;
    const board = boards[0];
    const mode = labels[board?.mode ?? value.mode] ?? '새 소식 확인 중';
    const source = sources.find((item) => item.id === board?.source_id)?.name;
    return [mode, source, board && board.page > 0 ? `${board.page}쪽` : ''].filter(Boolean).join(' · ');
  }

  function historyLabel(source: SourceInfo) {
    return source.history_complete ? '공개된 이전 공지 모두 가져옴' : !source.last_attempt ? '아직 가져오지 않음' : `이전 공지 ${source.next_page}쪽부터 이어받기`;
  }
</script>

<div class="source-manager" aria-label="정보원 관리" aria-busy={loading || busy}>
  <div class="collect-bar">
    <button class="button small" disabled={locked || !subscribed.length} onclick={() => start('refresh')}>새로고침</button>
    <button class="button secondary small" disabled={locked || !subscribed.length} onclick={() => start('resources')}>자료 모두 내려받기</button>
    <Hint label="가져오기">구독한 게시판 전체에 실행합니다. 구독만으로는 아무것도 가져오지 않습니다. 새로고침은 먼저 새 소식과 변경을 확인하고, 아직 다 가져오지 않은 이전 공지를 이어서 받습니다. 중단해도 지금까지 저장한 자료는 남고, 다시 누르면 이어갑니다. 처음 가져오는 기록은 새 소식으로 세지 않습니다. 자료 모두 내려받기는 보관한 공지에서 아직 받지 못한 첨부 파일과 이미지를 한 번에 최대 256 MB까지 받고, 다시 누르면 이어서 받습니다. 공지에 넣은 이미지는 다른 공개 HTTPS 사이트에서도 받습니다. 게시판은 학교가 공개한 자료만 가져옵니다. 사이버캠퍼스는 저장한 로그인으로 본인 강좌의 자료만 읽으며, 어떤 접근 제한도 우회하지 않습니다.</Hint>
    <span class="collect-status" role="status" aria-live="polite">
      {#if status?.running}<Icon name="refresh" size={14} />{runningLabel(status)}{:else if !loading}구독 {subscribed.length}개{/if}
    </span>
    {#if status?.running}<button class="text-button" disabled={busy} aria-label="가져오기 중단" onclick={cancel}>중단</button>{/if}
  </div>
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  <p class="archive-feedback" role="status">{feedback}</p>
  {#if loading && !sources.length}<p class="loading" role="status">게시판 목록을 불러오는 중입니다…</p>{/if}

  {#snippet tile(source: SourceInfo, organization: string)}
    {@const running = status?.running && status.active.some((board) => board.source_id === source.id)}
    <li class="board-tile" class:on={source.subscribed} class:unsupported={!source.supported}>
      <button class="board-switch" role="switch" aria-checked={source.subscribed} aria-label={`${organization} ${source.name} 구독`} aria-describedby={source.last_error ? `board-alert-${source.id}` : undefined} disabled={switchesLocked || !source.supported} onclick={() => subscribe(source)}>
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
          {#if source.subscribed}<div class="popover-actions"><button class="button secondary small" disabled={locked} onclick={() => start('refresh', [source.id])}>새로고침</button>{#if source.skipped_count}<button class="button secondary small" disabled={locked} onclick={() => start('skipped', [source.id])}>가져오지 못한 공지 다시 시도</button>{/if}</div>{/if}
          <code class="popover-code">{source.url}</code>
        </Popover>
      {/if}
    </li>
  {/snippet}

  {#each supportedGroups as [organization, members] (organization)}
    <h3 class="settings-group-title">{organizationLabel(organization)}</h3>
    <ul class="board-grid" aria-label={organization}>{#each members as source (source.id)}{@render tile(source, organization)}{/each}</ul>
    {#if members.some((source) => source.family === 'cybercampus')}<CybercampusSettings {locked} />{/if}
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
