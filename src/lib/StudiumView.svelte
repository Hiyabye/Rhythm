<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { archiveError, timeOptions } from './archive-format';
  import { clock as clockSetting } from './clock.svelte';
  import Hint from './Hint.svelte';
  import { fade } from './motion';
  import type { StudiumRoomState, StudiumSnapshot } from './types';

  let { desktop }: { desktop: boolean } = $props();
  let snapshot = $state<StudiumSnapshot | null>(null);
  let draft = $state<string[]>([]);
  let viewRoom = $state('');
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let pollError = $state('');
  let notice = $state('');
  let now = $state(Date.now());
  let mounted = false;
  let generation = 0;
  let polling = false;
  let startButton = $state<HTMLButtonElement>();
  let cancelButton = $state<HTMLButtonElement>();
  // Set when the user operates 지금 확인/확인 취소: disabling or removing the
  // pressed button would otherwise drop keyboard focus to the page body.
  let keepActionFocus = false;
  let selected = $derived(snapshot?.rooms.filter((room) => snapshot?.preferences.room_ids.includes(room.id)) ?? []);
  let room = $derived(snapshot?.rooms.find((item) => item.id === viewRoom));
  let result = $derived(snapshot?.results.find((item) => item.room_id === viewRoom));
  let dirty = $derived(Boolean(snapshot && (draft.length !== snapshot.preferences.room_ids.length || draft.some((id) => !snapshot?.preferences.room_ids.includes(id)))));

  function apply(value: StudiumSnapshot, resetDraft = false) {
    snapshot = value;
    viewRoom = value.preferences.view_room;
    if (resetDraft) draft = [...value.preferences.room_ids];
  }

  async function load() {
    if (!desktop) return;
    const token = ++generation;
    loading = true;
    error = '';
    pollError = '';
    try {
      const value = await invoke<StudiumSnapshot>('studium_snapshot');
      if (mounted && token === generation) apply(value, true);
    } catch (reason) {
      if (mounted && token === generation) error = archiveError(reason);
    } finally {
      if (mounted && token === generation) loading = false;
    }
  }

  async function poll() {
    if (!mounted || !snapshot?.running || busy || polling) return;
    polling = true;
    const token = generation;
    try {
      const value = await invoke<StudiumSnapshot>('studium_status');
      if (mounted && token === generation) { apply(value); pollError = ''; }
    } catch (reason) {
      if (mounted && token === generation) pollError = archiveError(reason);
    } finally { polling = false; }
  }

  async function act(command: string, args: Record<string, unknown> = {}, resetDraft = false, success = '') {
    if (!snapshot || busy) return;
    const token = ++generation;
    busy = true;
    error = '';
    pollError = '';
    notice = '';
    try {
      const value = await invoke<StudiumSnapshot>(command, args);
      if (mounted && token === generation) { apply(value, resetDraft); notice = success; }
    } catch (reason) {
      if (mounted && token === generation) {
        viewRoom = snapshot.preferences.view_room;
        error = archiveError(reason);
      }
    } finally {
      if (mounted && token === generation) busy = false;
    }
  }

  function selectTarget(id: string, checked: boolean) {
    draft = checked ? [...draft, id] : draft.filter((value) => value !== id);
  }

  function saveTargets() {
    if (!draft.length || !snapshot) return;
    const nextView = draft.includes(viewRoom) ? viewRoom : snapshot.rooms.find((item) => draft.includes(item.id))!.id;
    void act('studium_save_preferences', { roomIds: draft, viewRoom: nextView }, true, '확인할 열람실을 저장했습니다.');
  }

  function stateLabel(value: StudiumRoomState | undefined): string {
    if (value?.status === 'ready') return value.result?.remaining === 0 ? '빈 좌석 없음' : `빈 좌석 ${value.result?.remaining ?? '—'}석`;
    if (value?.status === 'count_only') return `빈 좌석 ${value.result?.remaining ?? '—'}석 · 번호 확인 불가`;
    if (value?.status === 'loading') return '확인 중';
    if (value?.status === 'failed') return '확인 실패';
    if (value?.status === 'cancelled') return '확인 취소됨';
    return '아직 확인하지 않았습니다';
  }

  function cycleStatus(value: StudiumSnapshot): string {
    if (value.running) return '선택한 열람실을 확인하고 있습니다.';
    if (notice) return notice;
    const labels = [
      ['ready', '수와 번호 확인'], ['count_only', '수만 확인'],
      ['failed', '확인 실패'], ['cancelled', '확인 취소'], ['not_checked', '미조회'],
    ] as const;
    if (!value.results.some((item) => item.status !== 'not_checked')) return '지금 확인을 누르면 저장한 열람실을 한 번 확인합니다.';
    const counts = labels.flatMap(([status, label]) => {
      const count = value.results.filter((item) => item.status === status).length;
      return count ? [`${label} ${count}곳`] : [];
    });
    return `열람실 확인이 끝났습니다. ${counts.join(', ')}.`;
  }

  function clock(value: string | null | undefined): string {
    if (!value || !Number.isFinite(Date.parse(value))) return '확인 불가';
    return new Intl.DateTimeFormat('ko-KR', { year: 'numeric', month: 'numeric', day: 'numeric', ...timeOptions(clockSetting.format, true) }).format(new Date(value));
  }

  function stale(value: string): boolean {
    const age = now - Date.parse(value);
    return !Number.isFinite(age) || age > 300_000 || age < -60_000;
  }

  $effect(() => {
    const target = snapshot?.running ? cancelButton : busy ? undefined : startButton;
    if (!keepActionFocus || !target || target.disabled) return;
    const active = document.activeElement;
    if (active && active !== document.body) return;
    target.focus();
    if (target === startButton) keepActionFocus = false;
  });

  function runAction(command: string, args: Record<string, unknown> = {}) {
    keepActionFocus = true;
    void act(command, args);
  }

  onMount(() => {
    mounted = true;
    if (desktop) void load();
    else loading = false;
    const pollTimer = window.setInterval(() => void poll(), 500);
    const tick = () => now = Date.now();
    const clockTimer = window.setInterval(tick, 1000);
    window.addEventListener('focus', tick);
    document.addEventListener('visibilitychange', tick);
    return () => {
      mounted = false;
      generation++;
      window.clearInterval(pollTimer);
      window.clearInterval(clockTimer);
      window.removeEventListener('focus', tick);
      document.removeEventListener('visibilitychange', tick);
    };
  });
</script>

<div class="studium" in:fade aria-busy={loading}>
  {#if !desktop}
    <p class="muted">데스크톱 앱에서 열람실 좌석을 확인할 수 있습니다.</p>
  {:else}
    {#if error || pollError}<p class="archive-error" role="alert">{error || pollError}</p>{/if}
    {#if loading}<p class="loading" role="status">열람실 설정을 불러오는 중입니다…</p>
    {:else if !snapshot}<button class="button secondary" onclick={load}>다시 불러오기</button>
    {:else}
      <section aria-labelledby="studium-targets-title">
        <div class="section-heading"><h2 id="studium-targets-title">확인할 열람실</h2><Hint label="확인할 열람실">선택하고 저장한 열람실을 지금 확인 버튼으로 한 번에 확인합니다. 선택만 바꾸면 학교에 요청하지 않습니다. 조회 결과는 앱을 종료하면 지워집니다.</Hint></div>
        <fieldset class="targets" disabled={busy || snapshot.running}>
          <legend class="sr-only">확인할 열람실 선택</legend>
          {#each snapshot.rooms as item (item.id)}
            <label class="target"><input type="checkbox" checked={draft.includes(item.id)} onchange={(event) => selectTarget(item.id, event.currentTarget.checked)} /><span>{item.label}</span><span class="capacity">{item.capacity}석</span></label>
          {/each}
        </fieldset>
        <div class="target-actions"><button class="button secondary small" disabled={busy || snapshot.running || !dirty || !draft.length} onclick={saveTargets}>선택 저장</button>{#if !draft.length}<span class="validation" role="status">열람실을 하나 이상 선택해 주세요.</span>{:else if dirty}<span class="muted">아직 저장하지 않은 선택입니다.</span>{/if}</div>
      </section>

      <section aria-labelledby="studium-results-title">
        <div class="section-heading results-heading"><h2 id="studium-results-title">좌석 현황</h2><div class="actions"><button class="button primary" bind:this={startButton} disabled={busy || snapshot.running} onclick={() => runAction('studium_start')}>지금 확인</button>{#if snapshot.running}<button class="button secondary" bind:this={cancelButton} disabled={busy || snapshot.job_id === null} onclick={() => runAction('studium_cancel', { jobId: snapshot!.job_id })}>확인 취소</button>{/if}</div></div>
        <p class="live-status" role="status" aria-live="polite" aria-atomic="true">{cycleStatus(snapshot)}</p>
        <ul class="room-summary" aria-label="열람실별 확인 상태">
          {#each selected as item (item.id)}
            <li><span>{item.label}</span><span>{stateLabel(snapshot.results.find((value) => value.room_id === item.id))}</span></li>
          {/each}
        </ul>
        <div class="view-picker"><label for="studium-view-room">표시할 열람실</label><select id="studium-view-room" value={viewRoom} disabled={busy} onchange={(event) => { viewRoom = event.currentTarget.value; void act('studium_set_view_room', { roomId: viewRoom }); }}>{#each selected as item (item.id)}<option value={item.id}>{item.label}</option>{/each}</select></div>
        <section class="room-detail" aria-label={`${room?.label ?? '열람실'} 좌석 정보`}>
          <h3>{room?.label ?? '열람실'}</h3>
          {#if result?.result}
            {@const data = result.result}
            <p class="seat-count">빈 좌석 <strong>{data.remaining.toLocaleString('ko-KR')}석</strong><span> / 전체 {data.total.toLocaleString('ko-KR')}석</span></p>
            {#if stale(data.source_time)}<p class="stale" role="status">오래되었거나 PC 시계와 맞지 않는 정보입니다. 지금 확인으로 다시 확인해 주세요.</p>{/if}
            {#if data.seat_numbers !== null}<p class="seat-numbers"><span>빈 좌석 번호</span><strong>{data.seat_numbers.length ? data.seat_numbers.join(', ') + '번' : '빈 좌석 없음'}</strong></p>
            {:else}<p class="seat-numbers"><span>빈 좌석 번호</span><strong>확인 불가 · 빈 좌석 수만 확인됨</strong></p>{/if}
            {#if result.error}<p class="detail-error">{result.error}</p>{/if}
            <dl class="times"><div><dt>원본 기준 시각</dt><dd><time datetime={data.source_time}>{clock(data.source_time)}</time></dd></div><div><dt>조회 시각</dt><dd>{#if result.checked_at}<time datetime={result.checked_at}>{clock(result.checked_at)}</time>{:else}확인 불가{/if}</dd></div></dl>
          {:else}<p class="empty-result">{stateLabel(result)}</p>{#if result?.error}<p class="detail-error">{result.error}</p>{/if}{/if}
        </section>
      </section>
    {/if}
  {/if}
</div>

<style>
  .studium { max-width: 860px; display: grid; gap: 28px; }
  section { min-width: 0; }
  .section-heading { display: flex; align-items: center; gap: 5px; margin-bottom: 10px; }
  h2 { margin: 0; font-size: 15px; font-weight: 650; }
  h3 { margin: 0 0 12px; font-size: 14px; font-weight: 650; }
  .targets { border: 1px solid var(--line); border-radius: 10px; background: var(--pane); padding: 0; margin: 0; min-width: 0; }
  .target { display: flex; align-items: center; gap: 10px; min-height: 42px; padding: 8px 14px; font-size: 13px; }
  .target + .target { border-top: 1px solid var(--line); }
  input { accent-color: var(--accent-fill); }
  .capacity { margin-left: auto; color: var(--secondary); font-variant-numeric: tabular-nums; }
  .target-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; margin-top: 10px; font-size: 12px; }
  .validation, .stale, .detail-error { color: var(--danger); }
  .results-heading { justify-content: space-between; flex-wrap: wrap; }
  .actions { display: flex; gap: 8px; }
  .live-status { margin: 0 0 10px; min-height: 20px; color: var(--secondary); font-size: 12px; }
  .room-summary { list-style: none; padding: 0; margin: 0 0 18px; border-top: 1px solid var(--line); }
  .room-summary li { display: flex; justify-content: space-between; flex-wrap: wrap; gap: 5px 12px; border-bottom: 1px solid var(--line); padding: 10px 0; font-size: 13px; }
  .room-summary li span:last-child { color: var(--secondary); font-variant-numeric: tabular-nums; }
  .view-picker { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; margin-bottom: 14px; font-size: 13px; }
  select { max-width: 100%; min-width: 0; padding: 5px 8px; color: var(--text); background: var(--field); border: 1px solid var(--line-strong); border-radius: 6px; }
  .room-detail { padding: 16px; border: 1px solid var(--line); border-radius: 10px; background: var(--pane); }
  .seat-count { margin: 0 0 14px; font-size: 14px; font-variant-numeric: tabular-nums; }
  .seat-count strong { font-size: 20px; }
  .seat-count > span { color: var(--secondary); font-size: 12px; }
  .seat-numbers { display: grid; gap: 6px; margin: 0 0 14px; font-size: 13px; }
  .seat-numbers > span { color: var(--secondary); font-size: 12px; }
  .seat-numbers strong { font-weight: 500; line-height: 1.8; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
  .stale, .detail-error, .empty-result { font-size: 13px; line-height: 1.6; }
  .times { display: grid; gap: 7px; margin: 16px 0 0; font-size: 12px; color: var(--secondary); font-variant-numeric: tabular-nums; }
  .times > div { display: flex; flex-wrap: wrap; gap: 6px 12px; }
  dt { min-width: 90px; } dd { margin: 0; }
</style>
