<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { FeedStatus } from './types';
  import { archiveDate, archiveError } from './archive-format';
  import { clock } from './clock.svelte';
  import Hint from './Hint.svelte';
  import Icon from './Icon.svelte';
  import Spinner from './Spinner.svelte';

  // The signed notice feed (#177): its age, how the last sync ended, boards the
  // server keeps failing on and an explicit 지금 받기.
  let { enabled }: { enabled: boolean } = $props();
  let feed = $state<FeedStatus | null>(null);
  let busy = $state(false);
  let error = $state('');
  let active = true;

  onMount(() => {
    void load();
    // A scheduled sync can start at any time; poll faster only while one runs.
    let wait = 0;
    const timer = window.setInterval(() => {
      if (!active || busy || !enabled || (!feed?.running && ++wait < 10)) return;
      wait = 0;
      void load();
    }, 1000);
    return () => { active = false; window.clearInterval(timer); };
  });

  async function load() {
    try {
      const next = await invoke<FeedStatus>('feed_status');
      if (active) feed = next;
    } catch (reason) { if (active) error = archiveError(reason); }
  }

  async function sync() {
    busy = true;
    error = '';
    if (feed) feed = { ...feed, running: true };
    try {
      const next = await invoke<FeedStatus>('feed_sync');
      if (active) feed = next;
    } catch (reason) {
      if (active) { error = archiveError(reason); void load(); }
    } finally { busy = false; }
  }
</script>

<section class="feed-settings" aria-labelledby="feed-settings-title" aria-busy={feed?.running ?? false}>
  <div class="feed-bar">
    <h3 class="settings-group-title" id="feed-settings-title">공지 피드</h3>
    <button class="button small" disabled={!enabled || busy || feed?.running} onclick={sync}>지금 받기</button>
    <Hint label="공지 피드">서버가 30분마다 공개 게시판을 모아 서명해 둔 공지 목록입니다. Rhythm은 시작할 때와 실행 중 30분마다 이 목록을 받고, 서명을 확인한 뒤에만 반영합니다. 요청에는 개인 정보가 들어가지 않습니다. 본문 전체와 첨부 자료는 공지를 열 때 학교 사이트에서 받습니다.</Hint>
    <span class="feed-state" role="status" aria-live="polite">
      {#if feed?.running}<Spinner />받는 중{:else if feed?.generated_at}{archiveDate(feed.generated_at, clock.format)} 기준{:else if feed}아직 받지 않음{/if}
    </span>
  </div>
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  {#if feed && !feed.running}
    {#if feed.result === 'update_required' || feed.result === 'failed'}
      <p class="archive-error" role="alert">{feed.error}</p>
    {:else if feed.result === 'offline'}
      <p class="footnote">인터넷에 연결되지 않아 받지 못했습니다. 연결되면 자동으로 다시 받습니다.</p>
    {/if}
    {#if feed.stale && feed.result !== 'offline' && feed.result !== 'update_required'}
      <p class="archive-error">서버의 공지 피드가 {archiveDate(feed.generated_at, clock.format)} 이후 갱신되지 않았습니다. 받은 공지는 그대로 볼 수 있습니다.</p>
    {/if}
    {#if feed.checked_at}<p class="footnote">마지막으로 확인한 시각: {archiveDate(feed.checked_at, clock.format)}</p>{/if}
    {#if feed.failing_boards.length}
      <p class="popover-subtitle">서버가 계속 가져오지 못하는 게시판</p>
      <ul class="feed-failing" aria-label="서버가 가져오지 못하는 게시판">
        {#each feed.failing_boards as board (board.source_id)}
          <li><Icon name="warning" size={14} /><span>{board.name}</span><span class="muted">연속 {board.consecutive_failures}회 실패 · 마지막 성공 {archiveDate(board.last_success_at, clock.format)}</span></li>
        {/each}
      </ul>
    {/if}
  {/if}
</section>

<style>
  .feed-settings { margin-bottom: 20px; }
  .feed-bar { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin-top: 22px; }
  /* As the boards' collect status, under its own name so each pane's status stays distinct. */
  .feed-state { flex: 1; display: inline-flex; align-items: center; gap: 6px; min-width: 0; margin-left: 6px; font-size: 12px; color: var(--secondary); font-variant-numeric: tabular-nums; }
  .feed-settings .settings-group-title { margin: 0; }
  .feed-failing { display: grid; gap: 6px; margin: 6px 0 0; padding: 0; list-style: none; font-size: 13px; }
  .feed-failing li { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .feed-failing :global(svg) { color: var(--danger); flex: none; }
</style>
