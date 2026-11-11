<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { archiveError, timeOptions } from './archive-format';
  import { clock as clockSetting } from './clock.svelte';
  import { fade } from './motion';
  import type { ClockFormat, Snapshot, StudiumMiniStatus, StudiumRoomState, StudiumSnapshot, Theme } from './types';

  let desktop = $state(false);
  let snapshot = $state<StudiumSnapshot | null>(null);
  let enabled = $state(false);
  let miniStatus = $state<StudiumMiniStatus>({ open: true, always_on_top: false });
  let onTop = $state(false);
  let theme = $state<Theme>('system');
  let systemDark = $state(false);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let eventError = $state('');
  let closed = $state(false);
  let now = $state(Date.now());
  let viewRoom = $state('');
  let startButton = $state<HTMLButtonElement>();
  let cancelButton = $state<HTMLButtonElement>();
  let topInput = $state<HTMLInputElement>();
  let mounted = false;
  let reading = false;
  let refreshPending = false;
  let fullRefreshPending = false;
  let initialized = false;
  let generation = 0;
  let themeGeneration = 0;
  let clockGeneration = 0;
  let keepActionFocus = false;
  let unlistenState: (() => void) | undefined;
  let unlistenTheme: (() => void) | undefined;
  let unlistenClock: (() => void) | undefined;
  let selected = $derived(snapshot?.rooms.filter((room) => snapshot?.preferences.room_ids.includes(room.id)) ?? []);
  let result = $derived(snapshot?.results.find((room) => room.room_id === viewRoom));
  let room = $derived(snapshot?.rooms.find((room) => room.id === viewRoom));
  let dark = $derived(theme === 'dark' || (theme === 'system' && systemDark));

  $effect(() => {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
  });

  $effect(() => {
    const target = snapshot?.running ? cancelButton : busy ? undefined : startButton;
    if (!keepActionFocus || !target || target.disabled) return;
    const active = document.activeElement;
    if (active && active !== document.body && active.id !== 'mini-main') return;
    target.focus();
    if (target === startButton) keepActionFocus = false;
  });

  function apply(value: StudiumSnapshot) {
    snapshot = value;
    viewRoom = value.preferences.view_room;
  }

  function requestRefresh() {
    refreshPending = true;
    void refresh();
  }

  async function refresh(full = false, status = false) {
    if (!desktop || !mounted || closed) return;
    if (busy || reading) { if (full) fullRefreshPending = true; if (full || !status) refreshPending = true; return; }
    full ||= fullRefreshPending;
    if (!full && !initialized) { refreshPending = true; return; }
    fullRefreshPending = false;
    reading = true;
    refreshPending = false;
    const token = generation;
    const themeToken = themeGeneration;
    const clockToken = clockGeneration;
    try {
      if (full) {
        const [foundation, windowStatus, value] = await Promise.all([
          invoke<Snapshot>('get_snapshot'), invoke<StudiumMiniStatus>('studium_mini_status'), invoke<StudiumSnapshot>('studium_snapshot'),
        ]);
        if (mounted && token === generation && !closed) {
          enabled = foundation.extensions.some((extension) => extension.id === 'studium' && extension.enabled);
          initialized = true;
          if (themeToken === themeGeneration) theme = foundation.settings.theme;
          if (clockToken === clockGeneration) clockSetting.format = foundation.settings.clock_format;
          miniStatus = windowStatus;
          onTop = windowStatus.always_on_top;
          apply(value);
          error = '';
        }
      } else {
        const value = await invoke<StudiumSnapshot>(status ? 'studium_status' : 'studium_snapshot');
        if (mounted && token === generation && !closed) { apply(value); error = ''; }
      }
    } catch (reason) { if (mounted && token === generation && !closed) error = archiveError(reason); }
    finally {
      reading = false;
      if (mounted && token === generation) loading = false;
      if (refreshPending) void refresh();
    }
  }

  async function act(command: string, args: Record<string, unknown> = {}) {
    if (!snapshot || busy || !enabled || closed) return;
    const token = ++generation;
    busy = true;
    error = '';
    try {
      const value = await invoke<StudiumSnapshot>(command, args);
      if (mounted && token === generation && !closed) apply(value);
    } catch (reason) {
      if (mounted && token === generation && !closed) {
        viewRoom = snapshot.preferences.view_room;
        error = archiveError(reason);
      }
    } finally {
      if (mounted && token === generation) { busy = false; if (refreshPending) void refresh(); }
    }
  }

  function runAction(command: string, args: Record<string, unknown> = {}) {
    keepActionFocus = true;
    void act(command, args);
  }

  async function setOnTop() {
    if (busy || closed) return;
    const ownedFocus = document.activeElement === topInput;
    generation++;
    busy = true;
    error = '';
    try { miniStatus = await invoke<StudiumMiniStatus>('studium_mini_set_on_top', { enabled: onTop }); onTop = miniStatus.always_on_top; }
    catch (reason) { if (mounted) { onTop = miniStatus.always_on_top; error = archiveError(reason); } }
    finally {
      if (mounted) {
        busy = false;
        if (refreshPending) void refresh();
        await tick();
        const active = document.activeElement;
        if (ownedFocus && !closed && (active === document.body || active?.id === 'mini-main' || active === topInput)) topInput?.focus();
      }
    }
  }

  function removeListeners() {
    unlistenState?.(); unlistenState = undefined;
    unlistenTheme?.(); unlistenTheme = undefined;
    unlistenClock?.(); unlistenClock = undefined;
  }

  async function windowAction(command: 'studium_close_mini' | 'studium_mini_show_main') {
    if (busy || closed) return;
    busy = true;
    error = '';
    try {
      await invoke<void>(command);
      if (mounted && command === 'studium_close_mini') { closed = true; generation++; removeListeners(); }
    } catch (reason) { if (mounted) error = archiveError(reason); }
    finally { if (mounted) { busy = false; if (refreshPending && !closed) void refresh(); } }
  }

  function stateLabel(value: StudiumRoomState | undefined) {
    if (value?.status === 'ready') return value.result?.remaining === 0 ? '빈 좌석 없음' : `빈 좌석 ${value.result?.remaining ?? '—'}석`;
    if (value?.status === 'count_only') return `빈 좌석 ${value.result?.remaining ?? '—'}석 · 번호 확인 불가`;
    return ({ loading: '확인 중', failed: '확인 실패', cancelled: '확인 취소됨', not_checked: '아직 확인하지 않았습니다' })[value?.status ?? 'not_checked'];
  }

  function clock(value: string | null | undefined) {
    if (!value || !Number.isFinite(Date.parse(value))) return '확인 불가';
    return new Intl.DateTimeFormat('ko-KR', { timeZone: 'Asia/Seoul', month: 'numeric', day: 'numeric', ...timeOptions(clockSetting.format, true) }).format(new Date(value));
  }

  function stale(value: string) {
    const age = now - Date.parse(value);
    return !Number.isFinite(age) || age > 300_000 || age < -60_000;
  }

  onMount(() => {
    mounted = true;
    desktop = isTauri();
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    systemDark = media.matches;
    const updateMedia = () => systemDark = media.matches;
    media.addEventListener('change', updateMedia);
    if (desktop) {
      void Promise.allSettled([
        listen<null>('studium://state-changed', requestRefresh),
        listen<Theme>('rhythm://theme-changed', (event) => {
          if (!['light', 'dark', 'system'].includes(event.payload)) return;
          themeGeneration++;
          theme = event.payload;
        }),
        listen<ClockFormat>('rhythm://clock-format-changed', (event) => {
          if (!['12h', '24h'].includes(event.payload)) return;
          clockGeneration++;
          clockSetting.format = event.payload;
        }),
      ]).then(([state, colors, format]) => {
        if (!mounted) { if (state.status === 'fulfilled') state.value(); if (colors.status === 'fulfilled') colors.value(); if (format.status === 'fulfilled') format.value(); return; }
        if (state.status === 'fulfilled') unlistenState = state.value;
        if (colors.status === 'fulfilled') unlistenTheme = colors.value;
        if (format.status === 'fulfilled') unlistenClock = format.value;
        if (state.status === 'rejected' || colors.status === 'rejected' || format.status === 'rejected') eventError = '다른 창의 상태 변경을 구독하지 못했습니다. 다시 열어 주세요.';
        void refresh(true);
      });
    } else loading = false;
    const pollTimer = window.setInterval(() => {
      if (!document.hidden && enabled && (snapshot?.running || snapshot?.automatic.active)) void refresh(false, true);
    }, 500);
    const clockTimer = window.setInterval(() => now = Date.now(), 1000);
    const focus = () => { now = Date.now(); if (!document.hidden) void refresh(true); };
    window.addEventListener('focus', focus);
    document.addEventListener('visibilitychange', focus);
    return () => {
      mounted = false;
      generation++;
      removeListeners();
      media.removeEventListener('change', updateMedia);
      window.removeEventListener('focus', focus);
      document.removeEventListener('visibilitychange', focus);
      window.clearInterval(pollTimer);
      window.clearInterval(clockTimer);
    };
  });
</script>

<svelte:head><title>Studium 미니 창</title></svelte:head>

<main id="mini-main" class="mini" aria-labelledby="mini-title" in:fade aria-busy={loading}>
  <header><h1 id="mini-title">Studium 미니 창</h1><p>조회한 좌석 현황</p></header>
  {#if !desktop}<p class="muted">데스크톱 앱에서 미니 창을 사용할 수 있습니다.</p>
  {:else if closed}<p role="status">미니 창을 닫았습니다.</p>
  {:else}
    {#if error || eventError}<p class="archive-error" role="alert">{error || eventError}</p>{/if}
    <div class="window-actions"><button class="button secondary small" disabled={busy} onclick={() => void windowAction('studium_mini_show_main')}>기본 창 열기</button><button class="button secondary small" disabled={busy} onclick={() => void windowAction('studium_close_mini')}>미니 창 닫기</button></div>
    <label class="on-top"><input type="checkbox" bind:this={topInput} bind:checked={onTop} disabled={busy || loading} onchange={() => void setOnTop()} />항상 위에 표시</label>
    {#if loading}<p role="status">좌석 현황을 불러오는 중입니다…</p>
    {:else if !snapshot}<button class="button secondary" onclick={() => void refresh(true)}>다시 불러오기</button>
    {:else if !enabled}<p role="status">Studium 확장 기능이 꺼져 있습니다. 기본 창에서 켜 주세요.</p>
    {:else}
      <div class="view-picker"><label for="mini-room">표시할 열람실</label><select id="mini-room" value={viewRoom} disabled={busy} onchange={(event) => { viewRoom = event.currentTarget.value; void act('studium_set_view_room', { roomId: viewRoom }); }}>{#each selected as item (item.id)}<option value={item.id}>{item.label}</option>{/each}</select></div>
      <section class="detail" aria-label={`${room?.label ?? '열람실'} 좌석 정보`}>
        <h2>{room?.label}</h2>
        <p class="current-state" role="status">{stateLabel(result)}</p>
        {#if result?.result}
          {@const data = result.result}
          <p class="seat-count">빈 좌석 <strong>{data.remaining}석</strong><span> / 전체 {data.total}석</span></p>
          {#if stale(data.source_time)}<p class="stale" role="status">이전 조회 기준입니다. 현재 빈자리 여부는 다시 확인해 주세요.</p>{/if}
          <dl><div><dt>원본 기준 시각 · 한국시간</dt><dd><time datetime={data.source_time}>{clock(data.source_time)}</time></dd></div><div><dt>조회 시각 · 한국시간</dt><dd>{clock(result.checked_at)}</dd></div></dl>
        {/if}
        {#if result?.error}<p class="detail-error">{result.error}</p>{/if}
      </section>
      <ul class="room-summary" aria-label="열람실별 확인 상태">{#each selected as item (item.id)}{@const value = snapshot.results.find((value) => value.room_id === item.id)}<li><span>{item.label}</span><div class="summary-values"><span>{stateLabel(value)}</span>{#if value?.result}<time datetime={value.result.source_time}>기준 {clock(value.result.source_time)} · 한국시간</time>{#if stale(value.result.source_time)}<span class="stale">오래됨 · 이전 조회 기준</span>{/if}{/if}</div></li>{/each}</ul>
      <div class="lookup-actions"><button class="button primary" bind:this={startButton} disabled={busy || snapshot.running} onclick={() => runAction('studium_start')}>지금 확인</button>{#if snapshot.running}<button class="button secondary" bind:this={cancelButton} disabled={busy || snapshot.job_id === null} onclick={() => runAction('studium_cancel', { jobId: snapshot!.job_id })}>확인 취소</button>{/if}</div>
      <p class="automatic" role="status">{snapshot.automatic.active ? snapshot.running ? '자동 확인 켜짐 · 현재 확인 중' : snapshot.automatic.next_check_at ? `다음 자동 확인: ${clock(snapshot.automatic.next_check_at)}` : '자동 확인 켜짐 · 대기 중' : '자동 확인 꺼짐'}</p>
      {#if snapshot.automatic.error}<p class="detail-error">{snapshot.automatic.error}</p>{/if}
      <p class="mini-hint">창 열기·표시 변경만으로 새로 조회하지 않습니다. 자동 확인 설정은 기본 창에서 바꿀 수 있습니다.</p>
    {/if}
  {/if}
</main>

<style>
  main.mini { margin: 0; width: 100%; min-width: 0; max-width: none; padding: 16px; display: grid; align-content: start; gap: 12px; min-height: 100vh; color: var(--text); background: var(--bg); }
  header h1 { margin: 0; font-size: 18px; font-weight: 650; }
  header p, .mini-hint { margin: 4px 0 0; color: var(--secondary); font-size: 12px; line-height: 1.6; }
  .window-actions, .lookup-actions { display: flex; flex-wrap: wrap; gap: 8px; }
  .on-top { display: flex; align-items: center; gap: 8px; font-size: 12px; }
  input { accent-color: var(--accent-fill); }
  .view-picker { display: grid; gap: 6px; font-size: 12px; min-width: 0; }
  select { width: 100%; min-width: 0; max-width: 100%; padding: 6px 8px; color: var(--text); background: var(--field); border: 1px solid var(--line-strong); border-radius: 6px; font: inherit; }
  .detail { min-width: 0; padding: 12px; border: 1px solid var(--line); border-radius: 8px; background: var(--pane); }
  h2 { margin: 0 0 8px; font-size: 13px; font-weight: 650; }
  .current-state, .automatic { margin: 0; color: var(--secondary); font-size: 12px; line-height: 1.6; }
  .seat-count { margin: 10px 0; font-size: 13px; font-variant-numeric: tabular-nums; }
  .seat-count strong { font-size: 23px; }
  .seat-count > span { font-size: 11px; color: var(--secondary); }
  dl { display: grid; gap: 6px; margin: 12px 0 0; font-size: 11px; color: var(--secondary); font-variant-numeric: tabular-nums; }
  dl > div { display: flex; flex-wrap: wrap; gap: 2px 10px; } dt { min-width: 80px; } dd { margin: 0; }
  .stale, .detail-error { margin: 8px 0 0; color: var(--danger); font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
  .room-summary { list-style: none; padding: 0; margin: 0; font-size: 12px; }
  .room-summary li { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 4px 10px; padding: 8px 0; border-bottom: 1px solid var(--line); }
  .summary-values { display: grid; gap: 3px; max-width: 100%; margin-left: auto; text-align: right; color: var(--secondary); font-variant-numeric: tabular-nums; }
  .summary-values time { font-size: 11px; }
  .summary-values .stale { margin: 0; }
</style>
