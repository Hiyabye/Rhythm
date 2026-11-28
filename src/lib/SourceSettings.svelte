<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { FeedBoardHealth, SourceGroup, SourceInfo, SubscribeResult, SyncMode, SyncStatus } from './types';
  import { archiveDate, archiveError, organizationLabel, sourceStatus } from './archive-format';
  import { clock } from './clock.svelte';
  import Icon from './Icon.svelte';
  import Spinner from './Spinner.svelte';
  import Hint from './Hint.svelte';
  import Popover from './Popover.svelte';
  import CybercampusSettings from './CybercampusSettings.svelte';
  import { flip, pop } from './motion';

  // Each source extension's pane manages only the sources that extension owns.
  let { enabled, extension }: { enabled: boolean; extension: string } = $props();
  let catalog = $state<SourceInfo[]>([]);
  let sources = $derived(catalog.filter((source) => source.extension === extension));
  let groupList = $state<SourceGroup[]>([]);
  // A category chip is open exactly while its group is on: opening turns the group on and
  // closing turns it off. An enabled group starts open when a board below it is picked,
  // so the profile's picks stay in view; `open` keeps the person's own opening of an
  // enabled group without picks, only while the pane is open.
  let open = $state<Record<string, boolean>>({});
  const isOpen = (node: GroupNode) => node.group.enabled && (open[node.group.id] ?? node.total.some((source) => source.selected));
  let status = $state<SyncStatus | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let feedback = $state('');
  let active = true;
  let polling = false;
  let refreshGeneration = 0;
  let subscribed = $derived(sources.filter((source) => source.subscribed && source.supported));
  // Public boards arrive through the signed feed (#177); the app collects only
  // Cybercampus, so only its pane has collect controls and job status.
  let collected = $derived(sources.some((source) => source.family === 'cybercampus'));
  let tree = $derived(nodes(null));
  let ungrouped = $derived(sources.filter((source) => !source.group));
  let supportedGroups = $derived(groups(ungrouped.filter((source) => source.supported)));
  let unsupportedGroups = $derived(groups(ungrouped.filter((source) => !source.supported)));
  let unsupportedCount = $derived(ungrouped.filter((source) => !source.supported).length);
  // Subscriptions stay open while a job runs; only starting another job waits.
  let switchesLocked = $derived(!enabled || busy || loading);
  let locked = $derived(switchesLocked || Boolean(status?.running));

  onMount(() => {
    void refresh();
    const timer = window.setInterval(() => {
      if (!active || polling || busy || loading || !enabled || !collected) return;
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

  type GroupNode = { group: SourceGroup; boards: SourceInfo[]; children: GroupNode[]; total: SourceInfo[] };

  // Groups nest by `parent`; a group with no board of this extension anywhere
  // below it is left out.
  function nodes(parent: string | null): GroupNode[] {
    return groupList.filter((group) => group.parent === parent).map((group) => {
      const boards = sources.filter((source) => source.group === group.id);
      const children = nodes(group.id);
      return { group, boards, children, total: [...boards, ...children.flatMap((child) => child.total)] };
    }).filter((node) => node.total.length);
  }

  // One flat sequence per top-level group: a category chip is followed, while open, by its
  // sub-categories, then its own boards, so opening pushes the later chips along.
  // `depth` colours an open category: 1 for the categories under a card, deeper below.
  type Chip = { key: string; kind: 'group' | 'board'; node?: GroupNode; source?: SourceInfo; parent: string; depth: number };
  function chips(node: GroupNode, depth = 1): Chip[] {
    const items: Chip[] = [];
    for (const child of node.children) {
      items.push({ key: `g-${child.group.id}`, kind: 'group', node: child, parent: node.group.name, depth });
      if (isOpen(child)) items.push(...chips(child, depth + 1));
    }
    for (const source of node.boards) items.push({ key: `b-${source.id}`, kind: 'board', source, parent: node.group.name, depth });
    return items;
  }

  // Opening an enabled group needs no save; every other press switches the group.
  async function toggleCategory(node: GroupNode) {
    const opening = !isOpen(node);
    if (opening && node.group.enabled) { open[node.group.id] = true; return; }
    if (await setGroup(node)) open[node.group.id] = opening;
  }

  async function refresh(showLoading = true) {
    const request = ++refreshGeneration;
    if (showLoading) { loading = true; error = ''; }
    try {
      const [nextSources, nextGroups, nextStatus] = await Promise.all([invoke<SourceInfo[]>('archive_sources'), invoke<SourceGroup[]>('archive_source_groups'), invoke<SyncStatus>('archive_status')]);
      if (!active || request !== refreshGeneration) return;
      catalog = nextSources;
      groupList = nextGroups;
      status = nextStatus;
    } catch (reason) { if (active && request === refreshGeneration) error = archiveError(reason); }
    finally { if (active && request === refreshGeneration) loading = false; }
  }

  async function subscribe(source: SourceInfo) {
    if (busy || loading || !enabled) return;
    busy = true;
    refreshGeneration++;
    error = '';
    const subscribed = !source.selected;
    try {
      const next = await invoke<SubscribeResult>('archive_subscribe', { id: source.id, subscribed });
      if (!active) return;
      catalog = next.sources;
      groupList = next.groups;
      const running = Boolean(status?.running);
      feedback = source.family !== 'cybercampus' ? (subscribed ? `${source.name} 구독을 시작했습니다.` : `${source.name} 구독을 해제했습니다. 받은 공지는 남지만 추천과 알림에서 빠집니다.`)
        : !subscribed ? `${source.name} 구독을 해제했습니다. 보관한 공지는 남습니다.`
        : running ? `${source.name} 구독을 시작했습니다. 지금 작업이 끝난 뒤 새로고침하면 가져옵니다.`
        : `${source.name} 구독을 시작했습니다. 새로고침하면 가져옵니다.`;
    } catch (reason) { if (active) error = archiveError(reason); }
    finally { if (active) busy = false; }
  }

  async function setGroup(node: GroupNode): Promise<boolean> {
    if (busy || loading || !enabled) return false;
    busy = true;
    refreshGeneration++;
    error = '';
    const on = !node.group.enabled;
    try {
      const next = await invoke<SubscribeResult>('archive_set_group', { id: node.group.id, enabled: on });
      if (!active) return false;
      catalog = next.sources;
      groupList = next.groups;
      // Only public boards have groups.
      feedback = !on ? `${node.group.name} 그룹을 껐습니다. 받은 공지는 남지만 추천과 알림에서 빠집니다.`
        : `${node.group.name} 그룹을 켰습니다. 전에 고른 게시판을 다시 구독합니다.`;
      return true;
    } catch (reason) { if (active) error = archiveError(reason); return false; }
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
    try { status = await invoke<SyncStatus>('archive_cancel'); feedback = '중단을 요청했습니다. 저장한 공지는 남습니다.'; }
    catch (reason) { error = archiveError(reason); }
    finally { busy = false; }
  }

  function runningLabel(value: SyncStatus) {
    const labels: Record<SyncMode, string> = { refresh: '새로고침 준비 중', incremental: '새 소식 확인 중', resource: '첨부 자료 내려받는 중', skipped: '가져오지 못한 공지 다시 확인 중', resources: '첨부 자료 모두 내려받는 중' };
    const boards = value.active.filter((board) => sources.some((item) => item.id === board.source_id));
    if (boards.length > 1) return `게시판 ${boards.length}개를 동시에 가져오는 중`;
    const board = boards[0];
    const mode = labels[board?.mode ?? value.mode ?? 'incremental'];
    const source = sources.find((item) => item.id === board?.source_id)?.name;
    return [mode, source, board && board.page > 0 ? `${board.page}쪽` : ''].filter(Boolean).join(' · ');
  }

  function feedLabel(feed: FeedBoardHealth) {
    const health = feed.failing ? `서버가 연속 ${feed.consecutive_failures}회 가져오지 못함`
      : feed.consecutive_failures ? `정상 · 최근 ${feed.consecutive_failures}회 실패` : '정상';
    return `${health} · ${feed.history_complete ? '공개된 이전 공지 모두 받음' : '서버가 이전 공지를 모으는 중'}`;
  }

  function historyLabel(source: SourceInfo) {
    return source.history_complete ? '공개된 이전 공지 모두 가져옴' : !source.last_attempt ? '아직 가져오지 않음' : `이전 공지 ${source.next_page}쪽부터 이어받기`;
  }
</script>

<div class="source-manager" aria-label="정보원 관리" aria-busy={loading || busy}>
  {#if collected}
  <div class="collect-bar">
    <button class="button small" disabled={locked || !subscribed.length} onclick={() => start('refresh')}>새로고침</button>
    <button class="button secondary small" disabled={locked || !subscribed.length} onclick={() => start('resources')}>자료 모두 내려받기</button>
    <Hint label="가져오기">구독한 강좌의 공지와 과제·마감을 가져옵니다. 중단해도 받은 자료는 남고, 다시 누르면 이어서 가져옵니다. 자료 모두 내려받기는 남은 첨부 파일과 이미지를 한 번에 256 MB까지 받습니다.</Hint>
    <span class="collect-status" role="status" aria-live="polite">
      {#if status?.running}<Spinner />{runningLabel(status)}{:else if !loading}구독 {subscribed.length}개{/if}
    </span>
    {#if status?.running}<button class="text-button" disabled={busy} aria-label="가져오기 중단" onclick={cancel}>중단</button>{/if}
  </div>
  {/if}
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  <p class="archive-feedback" role="status">{feedback}</p>
  {#if loading && !sources.length}<p class="loading" role="status">게시판 목록을 불러오는 중입니다…</p>{/if}

  {#snippet info(source: SourceInfo, organization: string, own: boolean)}
      {#if source.supported && !own}
        <button class="board-info" popovertarget={`board-${source.id}`} aria-label={`${organization} ${source.name} 상태`}><Icon name="info" size={15} /></button>
        <Popover id={`board-${source.id}`} label={`${source.name} 상태`} wide>
          <p class="popover-title">{source.name}</p>
          {#if source.feed}
            <p>{feedLabel(source.feed)}</p>
            <dl class="popover-facts"><div><dt>서버 마지막 수집</dt><dd>{archiveDate(source.feed.last_success_at, clock.format)}</dd></div></dl>
          {:else}<p>아직 이 게시판 소식을 받지 않았습니다.</p>{/if}
          <code class="popover-code">{source.url}</code>
        </Popover>
      {:else if source.supported}
        <button class="board-info" popovertarget={`board-${source.id}`} aria-label={`${organization} ${source.name} 상태`}><Icon name="info" size={15} /></button>
        <Popover id={`board-${source.id}`} label={`${source.name} 상태`} wide>
          <p class="popover-title">{source.name}</p>
          <p>{sourceStatus(source.status)} · {historyLabel(source)}</p>
          <dl class="popover-facts"><div><dt>마지막 시도</dt><dd>{archiveDate(source.last_attempt, clock.format)}</dd></div><div><dt>마지막 성공</dt><dd>{archiveDate(source.last_success, clock.format)}</dd></div>
            {#if source.discovered || source.archived || source.failed || source.status === 'running'}<div><dt>최근 시도</dt><dd>발견 {source.discovered} · 보관 {source.archived} · 실패 {source.failed}</dd></div>{/if}</dl>
          {#if source.last_error}<p class="archive-error">{archiveError(source.last_error)}</p>{/if}
          {#if source.skipped_count}
            <p class="popover-subtitle">가져오지 못한 공지 {source.skipped_count}건</p>
            <p>원문을 읽지 못해 건너뛴 공지입니다.</p>
            <ul class="skipped-list">
              {#each source.skipped as item (item.external_id)}
                <li><button class="text-button" onclick={() => openOriginal(item.url)}>공지 {item.external_id} 원문 열기</button><span>{item.archived ? '전에 보관한 내용은 읽을 수 있습니다.' : archiveError(item.reason)}</span></li>
              {/each}
            </ul>
            {#if source.skipped_count > source.skipped.length}<p class="footnote">최근 {source.skipped.length}건만 표시합니다.</p>{/if}
          {/if}
          {#if source.subscribed}<div class="popover-actions"><button class="button secondary small" disabled={locked} onclick={() => start('refresh', [source.id])}>새로고침</button>{#if source.skipped_count}<button class="button secondary small" disabled={locked} onclick={() => start('skipped', [source.id])}>가져오지 못한 공지 다시 시도</button>{/if}</div>{/if}
          <code class="popover-code">{source.url}</code>
        </Popover>
      {/if}
  {/snippet}

  {#snippet tile(source: SourceInfo, organization: string)}
    {@const own = source.family === 'cybercampus'}
    {@const running = own && status?.running && status.active.some((board) => board.source_id === source.id)}
    {@const alert = own ? source.last_error && '최근 가져오기 오류' : source.feed?.failing && '서버가 계속 가져오지 못하는 게시판'}
    <li class="board-tile" class:on={source.selected} class:unsupported={!source.supported}>
      <button class="board-switch" role="switch" aria-checked={source.selected} aria-label={`${organization} ${source.name} 구독`} aria-describedby={alert ? `board-alert-${source.id}` : undefined} disabled={switchesLocked || !source.supported} onclick={() => subscribe(source)}>
        <span class="board-mark" aria-hidden="true">{#if source.subscribed}<Icon name="check" size={13} />{/if}</span>
        <span class="board-name">{source.name}</span>
        {#if running}<span class="board-state spinning" aria-hidden="true"><Spinner /></span>
        {:else if alert}<span class="board-state alert" aria-hidden="true"><Icon name="warning" size={14} /></span>{/if}
      </button>
      {#if alert}<span class="sr-only" id={`board-alert-${source.id}`}>{alert}</span>{/if}
      {@render info(source, organization, own)}
    </li>
  {/snippet}

  {#snippet chipButton(chip: Chip)}
    {#if chip.kind === 'group' && chip.node}
      {@const node = chip.node}
      {@const on = node.total.filter((source) => source.subscribed).length}
      {@const expanded = isOpen(node)}
      <button class="chip group" class:open={expanded} data-depth={Math.min(chip.depth, 3)} aria-expanded={expanded} aria-label={`${node.group.name}, 구독 ${on}/${node.total.length}`} aria-disabled={switchesLocked} onclick={() => toggleCategory(node)}>
        <span>{node.group.name}</span><span class="chip-count" aria-hidden="true">{on}/{node.total.length}</span>
        <Icon name="plus" size={13} />
      </button>
    {:else if chip.kind === 'board' && chip.source}
      {@const source = chip.source}
      {@const alert = source.family === 'cybercampus' ? source.last_error && '최근 가져오기 오류' : source.feed?.failing && '서버가 계속 가져오지 못하는 게시판'}
      <!-- One pill: the toggle and, at its right edge, the status button (a button cannot hold another). -->
      <span class="chip board" class:on={source.selected}>
        <button class="chip-toggle" aria-pressed={source.selected} aria-label={`${chip.parent} ${source.name} 구독`} aria-describedby={alert ? `board-alert-${source.id}` : undefined} disabled={!source.supported} aria-disabled={switchesLocked} onclick={() => subscribe(source)}>
          <!-- Both states carry a same-size icon, so toggling never changes the chip's width. -->
          <Icon name={source.subscribed ? 'check' : 'plus'} size={13} /><span>{source.name}</span>{#if alert}<Icon name="warning" size={13} />{/if}
        </button>
        {@render info(source, chip.parent, source.family === 'cybercampus')}
      </span>
      {#if alert}<span class="sr-only" id={`board-alert-${source.id}`}>{alert}</span>{/if}
    {/if}
  {/snippet}

  {#snippet group(node: GroupNode, level: number)}
    {@const on = node.total.filter((source) => source.subscribed).length}
    <section class="board-group" class:off={!node.group.enabled} data-level={level} role="group" aria-labelledby={`board-group-${node.group.id}`}>
      <div class="board-group-head">
        <span class="board-group-name" id={`board-group-${node.group.id}`} role="heading" aria-level={level + 2}>{node.group.name}</span>
        <span class="board-group-count">{node.group.enabled ? `구독 ${on}/${node.total.length}` : `꺼짐 · 게시판 ${node.total.length}개`}</span>
        <button class="switch" class:on={node.group.enabled} role="switch" aria-checked={node.group.enabled} aria-label={`${node.group.name} 전체`} disabled={switchesLocked} onclick={() => setGroup(node)}><span></span></button>
      </div>
      {#if node.group.enabled}
        <ul class="chips" aria-label={`${node.group.name} 게시판 고르기`}>
          {#each chips(node) as chip (chip.key)}
            <li class="chip-item" animate:flip in:pop>{@render chipButton(chip)}</li>
          {/each}
        </ul>
      {/if}
    </section>
  {/snippet}

  {#each tree as node (node.group.id)}{@render group(node, 1)}{/each}

  {#each supportedGroups as [organization, members] (organization)}
    <h3 class="settings-group-title">{organizationLabel(organization)}</h3>
    <ul class="board-grid" aria-label={organization}>{#each members as source (source.id)}{@render tile(source, organization)}{/each}</ul>
    {#if members.some((source) => source.family === 'cybercampus')}<CybercampusSettings {locked} />{/if}
  {/each}

  {#if unsupportedCount}
    <details class="board-unsupported">
      <summary>미지원 게시판 {unsupportedCount}개</summary>
      <p class="footnote">아직 지원하지 않아 구독할 수 없는 게시판입니다.</p>
      {#each unsupportedGroups as [organization, members] (organization)}
        <h3 class="settings-group-title">{organizationLabel(organization)}</h3>
        <ul class="board-grid" aria-label={organization}>{#each members as source (source.id)}{@render tile(source, organization)}{/each}</ul>
      {/each}
    </details>
  {/if}
</div>

<style>
  .chips { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: 8px; }
  .chip-item { position: relative; display: inline-flex; align-items: center; }
  .chip { display: inline-flex; align-items: center; gap: 6px; min-height: 34px; padding: 6px 14px; border: 1px solid var(--line-strong); border-radius: 999px; background: var(--surface); color: var(--text-2); font-size: 13px; font-weight: 550; transition: background .15s, border-color .15s, color .15s; }
  .chip:hover:not(:disabled, [aria-disabled="true"]), .chip.board:hover:has(.chip-toggle:not(:disabled, [aria-disabled="true"])) { border-color: var(--secondary); }
  /* aria-disabled marks a save in flight, so the pressed chip keeps keyboard focus; it is too
     brief to show, and dimming every chip at once reads as a flash. */
  .chip[aria-disabled="true"], .chip-toggle[aria-disabled="true"] { cursor: default; }
  .chip.board:has(.chip-toggle:disabled) { opacity: .6; }
  .chip-count { font-size: 11.5px; color: var(--secondary); font-variant-numeric: tabular-nums; }
  .chip.group :global(svg) { transition: transform .15s; }
  .chip.group.open :global(svg) { transform: rotate(45deg); }
  /* Open categories darken with breadth: deep navy, then mid blue, then the light tint.
     A subscribed board keeps the plain accent fill, so it never reads as a category. */
  .chip.group.open[data-depth="1"] { background: color-mix(in srgb, var(--accent-fill) 55%, #001433); border-color: transparent; color: #fff; }
  .chip.group.open[data-depth="1"] .chip-count { color: rgb(255 255 255 / .78); }
  .chip.group.open[data-depth="2"] { background: color-mix(in srgb, var(--accent-fill) 40%, var(--surface)); border-color: var(--accent); color: var(--text); }
  .chip.group.open[data-depth="2"] .chip-count { color: var(--text-2); }
  .chip.group.open[data-depth="3"] { background: var(--soft); border-color: var(--accent); color: var(--accent); }
  .chip.board { padding: 0; gap: 0; }
  .chip-toggle { display: inline-flex; align-items: center; gap: 6px; min-height: 32px; padding: 6px 14px 6px 12px; border: 0; border-radius: 999px; background: transparent; color: inherit; font: inherit; }
  .chip.board:has(:global(.board-info)) .chip-toggle { padding-right: 2px; }
  .chip-toggle:focus-visible { outline: none; }
  .chip.board:has(.chip-toggle:focus-visible) { outline: 2px solid var(--accent); outline-offset: 2px; }
  .chip.board.on { background: var(--accent-fill); border-color: var(--accent-fill); color: var(--accent-text); }
  .chip.board.on:hover { filter: brightness(1.08); }
  .chip.board :global(.board-info) { position: static; transform: none; width: 24px; height: 24px; margin-right: 5px; border-radius: 999px; color: var(--muted); }
  .chip.board :global(.board-info:hover) { background: var(--hover); color: var(--text); }
  .chip.board.on :global(.board-info) { color: rgb(255 255 255 / .85); }
  .chip.board.on :global(.board-info:hover) { background: rgb(255 255 255 / .16); color: var(--accent-text); }
  @media (prefers-reduced-motion: reduce) { .chip, .chip.group :global(svg) { transition: none; } }
</style>
