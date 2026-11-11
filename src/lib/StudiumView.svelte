<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { archiveError, timeOptions, wallTime } from './archive-format';
  import { clock as clockSetting } from './clock.svelte';
  import Hint from './Hint.svelte';
  import StudiumSeatMap from './StudiumSeatMap.svelte';
  import { fade } from './motion';
  import type { NotificationAuthorization, ProactiveActivation, ProactiveSettings, ProactiveSnapshot, StudiumAlertRule, StudiumMiniStatus, StudiumRoomState, StudiumSnapshot } from './types';

  let { desktop, activation, proactiveEnabled, onactivation, onsettings }: { desktop: boolean; activation: number; proactiveEnabled: boolean; onactivation: (activation: ProactiveActivation) => void; onsettings: () => void } = $props();
  let snapshot = $state<StudiumSnapshot | null>(null);
  let draft = $state<string[]>([]);
  let intervalDraft = $state<number | undefined>(5);
  let intervalEditing = $state(false);
  let favoritesRoom = $state('');
  let favoritesDraft = $state('');
  let favoritesEditing = $state(false);
  let searchDraft = $state('');
  let alertEditing = $state(false);
  let alertEnabled = $state(false);
  let alertKind = $state<StudiumAlertRule['kind']>('remaining');
  let alertMinimum = $state<number | undefined>(1);
  let notificationSettings = $state<ProactiveSettings | null>(null);
  let notificationAuthorization = $state<NotificationAuthorization>('unknown');
  let notificationLoading = $state(false);
  let notificationError = $state('');
  let viewRoom = $state('');
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let pollError = $state('');
  let eventError = $state('');
  let miniBusy = $state(false);
  let miniError = $state('');
  let miniNotice = $state('');
  let notice = $state('');
  let now = $state(Date.now());
  let mounted = $state(false);
  let handledActivation = 0;
  let generation = 0;
  let polling = false;
  let stateRefreshPending = false;
  let startButton = $state<HTMLButtonElement>();
  let cancelButton = $state<HTMLButtonElement>();
  let automaticButton = $state<HTMLButtonElement>();
  let favoritesInput = $state<HTMLInputElement>();
  let searchInput = $state<HTMLInputElement>();
  let alertToggle = $state<HTMLInputElement>();
  let roomHeading = $state<HTMLHeadingElement>();
  let keepAutomaticFocus = false;
  // Set when the user operates 지금 확인/확인 취소: disabling or removing the
  // pressed button would otherwise drop keyboard focus to the page body.
  let keepActionFocus = false;
  let selected = $derived(snapshot?.rooms.filter((room) => snapshot?.preferences.room_ids.includes(room.id)) ?? []);
  let room = $derived(snapshot?.rooms.find((item) => item.id === viewRoom));
  let result = $derived(snapshot?.results.find((item) => item.room_id === viewRoom));
  let dirty = $derived(Boolean(snapshot && (draft.length !== snapshot.preferences.room_ids.length || draft.some((id) => !snapshot?.preferences.room_ids.includes(id)))));
  let intervalDirty = $derived(Boolean(snapshot && intervalDraft !== snapshot.automatic.interval_minutes));
  let intervalInvalid = $derived(intervalDraft === undefined || !Number.isInteger(intervalDraft) || intervalDraft < 2 || intervalDraft > 60);
  let savedFavorites = $derived(snapshot?.favorites[viewRoom] ?? []);
  let parsedFavorites = $derived(parseFavorites(favoritesDraft, room?.capacity ?? 0));
  let favoritesDirty = $derived(Boolean(parsedFavorites.error || parsedFavorites.numbers.length !== savedFavorites.length || parsedFavorites.numbers.some((number, index) => savedFavorites[index] !== number)));
  let searchActive = $derived(searchDraft.trim() !== '');
  let searchInvalid = $derived(searchActive && (!/^\d+$/.test(searchDraft.trim()) || !Number.isSafeInteger(Number(searchDraft)) || Number(searchDraft) < 1 || Number(searchDraft) > (room?.capacity ?? 0)));
  let filteredNumbers = $derived(result?.result?.seat_numbers?.filter((number) => !searchActive || (!searchInvalid && number === Number(searchDraft))) ?? []);
  let savedAlert = $derived(snapshot?.alert_rules[viewRoom] ?? { enabled: false, kind: 'remaining' as const, minimum: 1, version: 0 });
  let effectiveAlertMinimum = $derived(alertKind === 'preferred' ? 1 : alertMinimum);
  let alertInvalid = $derived(effectiveAlertMinimum === undefined || !Number.isInteger(effectiveAlertMinimum) || effectiveAlertMinimum < 1 || effectiveAlertMinimum > (room?.capacity ?? 0) || (alertEnabled && alertKind === 'preferred' && !savedFavorites.length));
  let alertDirty = $derived(alertEnabled !== savedAlert.enabled || alertKind !== savedAlert.kind || effectiveAlertMinimum !== savedAlert.minimum);

  function apply(value: StudiumSnapshot, resetDraft = false, resetInterval = false, resetFavorites = false, resetAlert = false) {
    // Capture before Svelte removes a focused map for loading/failed results.
    const losingMapFocus = document.hasFocus() && !value.results.find((room) => room.room_id === value.preferences.view_room)?.result
      && roomHeading?.closest('.room-detail')?.querySelector('.seat-map')?.contains(document.activeElement);
    if (favoritesRoom !== value.preferences.view_room) {
      favoritesEditing = false;
      alertEditing = false;
      searchDraft = '';
    }
    if (resetFavorites) favoritesEditing = false;
    favoritesRoom = value.preferences.view_room;
    snapshot = value;
    viewRoom = value.preferences.view_room;
    if (resetDraft) draft = [...value.preferences.room_ids];
    if (resetInterval) intervalEditing = false;
    if (!intervalEditing) intervalDraft = value.automatic.interval_minutes;
    if (!favoritesEditing) favoritesDraft = (value.favorites[viewRoom] ?? []).join(', ');
    if (resetAlert) alertEditing = false;
    if (!alertEditing) applyAlertDraft(value.alert_rules[viewRoom]);
    if (losingMapFocus) void restoreMapFocus();
  }

  async function load(resetEditors = false) {
    if (!desktop) return;
    const token = ++generation;
    loading = true;
    error = '';
    pollError = '';
    try {
      const value = await invoke<StudiumSnapshot>('studium_snapshot');
      if (mounted && token === generation) {
        if (resetEditors) searchDraft = '';
        apply(value, true, true, resetEditors, resetEditors);
      }
    } catch (reason) {
      if (mounted && token === generation) error = archiveError(reason);
    } finally {
      if (mounted && token === generation) { loading = false; void refreshChangedState(); }
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
    } finally { polling = false; void refreshChangedState(); }
  }

  function requestChangedState() {
    stateRefreshPending = true;
    void refreshChangedState();
  }

  async function refreshChangedState() {
    if (!stateRefreshPending || !mounted || loading || busy || polling) return;
    stateRefreshPending = false;
    polling = true;
    const token = generation;
    try {
      const value = await invoke<StudiumSnapshot>('studium_snapshot');
      if (mounted && token === generation) { apply(value); pollError = ''; }
    } catch (reason) {
      if (mounted && token === generation) pollError = archiveError(reason);
    } finally { polling = false; void refreshChangedState(); }
  }

  async function openMini() {
    if (!desktop || miniBusy) return;
    miniBusy = true;
    miniError = '';
    miniNotice = '';
    try {
      const status = await invoke<StudiumMiniStatus>('studium_open_mini');
      if (mounted && status.open) miniNotice = '미니 창을 열었습니다.';
    } catch (reason) { if (mounted) miniError = archiveError(reason); }
    finally { if (mounted) miniBusy = false; }
  }

  async function act(command: string, args: Record<string, unknown> = {}, resetDraft = false, success = '', resetInterval = false, resetFavorites = false, resetAlert = false) {
    if (!snapshot || busy) return;
    const token = ++generation;
    busy = true;
    error = '';
    pollError = '';
    notice = '';
    try {
      const value = await invoke<StudiumSnapshot>(command, args);
      if (mounted && token === generation) { apply(value, resetDraft, resetInterval, resetFavorites, resetAlert); notice = success; }
    } catch (reason) {
      if (mounted && token === generation) {
        viewRoom = snapshot.preferences.view_room;
        error = archiveError(reason);
      }
    } finally {
      if (mounted && token === generation) { busy = false; void refreshChangedState(); }
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

  function saveInterval() {
    if (intervalInvalid || !intervalDirty) return;
    void act('studium_save_interval', { intervalMinutes: intervalDraft }, false, '자동 확인 간격을 저장했습니다.', true);
  }

  function cancelIntervalEdit() {
    intervalEditing = false;
    intervalDraft = snapshot?.automatic.interval_minutes ?? 5;
  }

  function parseFavorites(value: string, capacity: number): { numbers: number[]; error: string } {
    const parts = value.trim() ? value.trim().split(/[,\s]+/) : [];
    const numbers = parts.map(Number);
    if (parts.some((part) => !/^\d+$/.test(part)) || numbers.some((number) => !Number.isSafeInteger(number) || number < 1 || number > capacity)) {
      return { numbers: [], error: `1부터 ${capacity} 사이의 좌석 번호를 쉼표나 공백으로 구분해 입력해 주세요.` };
    }
    if (new Set(numbers).size !== numbers.length) return { numbers: [], error: '같은 좌석 번호를 중복해서 입력할 수 없습니다.' };
    return { numbers: numbers.sort((a, b) => a - b), error: '' };
  }

  async function saveFavorites() {
    if (parsedFavorites.error || !favoritesDirty) return;
    await act('studium_save_favorites', { roomId: viewRoom, seatNumbers: parsedFavorites.numbers }, false, '선호 좌석을 저장했습니다.', false, true);
    await tick();
    const active = document.activeElement;
    // WebKit pointer activation can leave focus on the shell's main fallback;
    // restore the editor unless the user has moved to another control.
    if (mounted && (active === document.body || active?.id === 'main' || (active instanceof HTMLButtonElement && active.disabled && favoritesInput?.closest('form')?.contains(active)))) favoritesInput?.focus();
  }

  function cancelFavoritesEdit() {
    favoritesEditing = false;
    favoritesDraft = savedFavorites.join(', ');
    favoritesInput?.focus();
  }

  function favoriteAvailability(): string {
    if (!savedFavorites.length) return '선호 좌석을 저장하면 확인된 빈 번호를 ★로 강조합니다.';
    if (!result?.result) return '현재 조회로 선호 좌석의 빈자리 여부를 확인하지 못했습니다.';
    const data = result.result;
    if (data.seat_numbers === null) return '번호를 확인하지 못해 선호 좌석의 빈자리 여부를 알 수 없습니다.';
    const available = data.seat_numbers.filter((number) => savedFavorites.includes(number));
    if (stale(data.source_time)) return `이전 조회 기준 빈 선호 좌석: ${available.length ? available.join(', ') + '번' : '없음'}. 현재 빈자리 여부는 확인되지 않았습니다.`;
    return available.length ? `이번 조회에서 빈 선호 좌석: ${available.join(', ')}번` : '이번 조회에서 빈 선호 좌석이 없습니다.';
  }

  function applyAlertDraft(rule: StudiumAlertRule | undefined) {
    alertEnabled = rule?.enabled ?? false;
    alertKind = rule?.kind ?? 'remaining';
    alertMinimum = rule?.minimum ?? 1;
  }

  function cancelAlertEdit() {
    alertEditing = false;
    applyAlertDraft(savedAlert);
    alertToggle?.focus();
  }

  async function saveAlert() {
    if (!alertDirty || alertInvalid) return;
    await act('studium_save_alert', { roomId: viewRoom, enabled: alertEnabled, kind: alertKind, minimum: effectiveAlertMinimum }, false, '좌석 알림 조건을 저장했습니다.', false, false, true);
    await tick();
    const active = document.activeElement;
    if (mounted && (active === document.body || active?.id === 'main' || (active instanceof HTMLButtonElement && active.disabled && alertToggle?.closest('form')?.contains(active)))) alertToggle?.focus();
  }

  async function loadNotificationContext() {
    if (!desktop || notificationLoading) return;
    notificationLoading = true;
    notificationError = '';
    const [settings, authorization] = await Promise.allSettled([
      invoke<ProactiveSnapshot>('proactive_snapshot'), invoke<string>('notification_authorization'),
    ]);
    if (!mounted) return;
    if (settings.status === 'fulfilled') {
      notificationSettings = settings.value.settings;
      if (settings.value.pending_activation) onactivation(settings.value.pending_activation);
    } else { notificationSettings = null; notificationError = archiveError(settings.reason); }
    if (authorization.status === 'fulfilled') {
      const value = authorization.value;
      notificationAuthorization = ['authorized', 'provisional', 'ephemeral', 'granted'].includes(value) ? 'granted'
        : value === 'denied' || value === 'not_determined' || value === 'unavailable' ? value : 'unknown';
    } else notificationAuthorization = 'unavailable';
    notificationLoading = false;
  }

  function notificationPolicyStatus(): string {
    if (!proactiveEnabled) return '알림·백그라운드 확장 기능이 꺼져 있어 좌석 알림을 보내지 않습니다.';
    if (!notificationSettings) return '전체 알림 설정을 확인하지 못했습니다.';
    if (!notificationSettings.notifications_enabled) return '전체 운영체제 알림이 꺼져 있어 좌석 알림을 보내지 않습니다.';
    const labels = { granted: '운영체제 알림 권한이 허용되어 있습니다.', denied: '운영체제 알림 권한이 거부되어 있습니다. 알림 설정에서 허용해 주세요.', not_determined: '운영체제 알림 권한을 아직 허용하지 않았습니다. 알림 설정에서 직접 권한을 확인해 주세요.', unavailable: '이 환경에서는 운영체제 알림을 사용할 수 없습니다.', unknown: '운영체제 알림 권한을 확인할 수 없습니다.' };
    return labels[notificationAuthorization];
  }

  function toggleAutomatic() {
    keepAutomaticFocus = true;
    void act(snapshot?.automatic.active ? 'studium_automatic_pause' : 'studium_automatic_start');
  }

  function automaticStatus(value: StudiumSnapshot): string {
    if (!value.automatic.active) return value.automatic.error ? '자동 확인이 중단되었습니다.' : '자동 확인이 꺼져 있습니다.';
    if (value.running) return `자동 확인이 켜져 있습니다. 진행 중인 확인을 마친 뒤 ${value.automatic.interval_minutes}분 후 다음 확인을 시작합니다.`;
    return value.automatic.next_check_at ? `다음 자동 확인: ${clock(value.automatic.next_check_at)}` : '자동 확인을 준비하고 있습니다.';
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
    return new Intl.DateTimeFormat('ko-KR', { timeZone: 'Asia/Seoul', year: 'numeric', month: 'numeric', day: 'numeric', ...timeOptions(clockSetting.format, true) }).format(new Date(value));
  }

  function stale(value: string): boolean {
    const age = now - Date.parse(value);
    return !Number.isFinite(age) || age > 300_000 || age < -60_000;
  }

  $effect(() => {
    const token = activation;
    if (!mounted || token === handledActivation) return;
    handledActivation = token;
    void load(true);
  });

  $effect(() => {
    const target = snapshot?.running ? cancelButton : busy ? undefined : startButton;
    if (!keepActionFocus || !target || target.disabled) return;
    const active = document.activeElement;
    if (active && active !== document.body) return;
    target.focus();
    if (target === startButton) keepActionFocus = false;
  });

  $effect(() => {
    if (!keepAutomaticFocus || busy || !automaticButton || automaticButton.disabled) return;
    if (document.activeElement === document.body) automaticButton.focus();
    keepAutomaticFocus = false;
  });

  function runAction(command: string, args: Record<string, unknown> = {}) {
    keepActionFocus = true;
    void act(command, args);
  }

  async function restoreMapFocus() {
    await tick();
    const active = document.activeElement;
    if (mounted && document.hasFocus() && roomHeading?.isConnected && (active === document.body || active?.id === 'main')) roomHeading.focus();
  }

  onMount(() => {
    handledActivation = activation;
    mounted = true;
    let unlistenState: (() => void) | undefined;
    if (desktop) {
      void listen<null>('studium://state-changed', requestChangedState).then((unlisten) => {
        if (!mounted) { unlisten(); return; }
        unlistenState = unlisten;
        void load();
      }).catch((reason) => {
        if (!mounted) return;
        eventError = `다른 창의 좌석 상태를 구독하지 못했습니다: ${archiveError(reason)}`;
        void load();
      });
    } else loading = false;
    if (desktop) void loadNotificationContext();
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
      unlistenState?.();
    };
  });
</script>

<div class="studium" in:fade aria-busy={loading}>
  {#if !desktop}
    <p class="muted">데스크톱 앱에서 열람실 좌석을 확인할 수 있습니다.</p>
  {:else}
    {#if error || pollError || eventError}<p class="archive-error" role="alert">{error || pollError || eventError}</p>{/if}
    <div class="mini-launch"><button class="button secondary small" disabled={miniBusy} onclick={() => void openMini()}>미니 창 열기</button>{#if miniNotice}<span role="status">{miniNotice}</span>{/if}</div>
    {#if miniError}<p class="archive-error" role="alert">{miniError}</p>{/if}
    {#if loading}<p class="loading" role="status">열람실 설정을 불러오는 중입니다…</p>
    {:else if !snapshot}<button class="button secondary" onclick={() => void load()}>다시 불러오기</button>
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

      <section aria-labelledby="studium-automatic-title">
        <div class="section-heading"><h2 id="studium-automatic-title">자동 확인</h2><Hint label="자동 확인">시작하면 저장한 열람실을 바로 확인합니다. 이미 확인 중이면 그 작업을 마친 뒤 저장한 간격만큼 기다립니다. 일시정지는 진행 중인 자동 확인을 취소하고, 직접 시작한 확인은 계속합니다.</Hint></div>
        <form class="automatic-interval" onsubmit={(event) => { event.preventDefault(); saveInterval(); }}>
          <label for="studium-interval">자동 확인 간격 (분)</label>
          <input id="studium-interval" type="number" min="2" max="60" step="1" inputmode="numeric" bind:value={intervalDraft} oninput={() => intervalEditing = true} disabled={busy} aria-invalid={intervalInvalid ? 'true' : undefined} aria-describedby={intervalInvalid ? 'studium-interval-help studium-interval-error' : 'studium-interval-help'} />
          <button class="button secondary small" type="submit" disabled={busy || !intervalDirty || intervalInvalid}>간격 저장</button>
          <button class="button secondary small" type="button" disabled={busy || !intervalEditing} onclick={cancelIntervalEdit}>간격 변경 취소</button>
        </form>
        <p class="automatic-help" id="studium-interval-help">2–60분 · 저장한 간격 {snapshot.automatic.interval_minutes}분. 간격 저장만으로 확인을 시작하지 않습니다. 변경한 간격은 다음 확인이 끝난 뒤 적용됩니다.</p>
        {#if intervalInvalid}<p class="validation automatic-help" id="studium-interval-error" role="status">2부터 60 사이의 정수를 입력해 주세요.</p>{/if}
        <button class="button secondary" bind:this={automaticButton} disabled={busy} onclick={toggleAutomatic}>{snapshot.automatic.active ? '자동 확인 일시정지' : '자동 확인 시작'}</button>
        <p class="automatic-status" role="status" aria-live="polite" aria-atomic="true">{automaticStatus(snapshot)}</p>
        {#if snapshot.automatic.error}<p class="archive-error" role="alert">{snapshot.automatic.error}</p>{/if}
        <p class="automatic-help">앱 실행 중에만 동작하며, 앱을 다시 실행하면 자동 확인은 꺼집니다. 간격만 저장됩니다.</p>
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
          <h3 bind:this={roomHeading} tabindex="-1">{room?.label ?? '열람실'}</h3>
          <section class="favorites-editor" aria-labelledby="studium-favorites-title">
            <h4 id="studium-favorites-title">선호 좌석</h4>
            <form onsubmit={(event) => { event.preventDefault(); void saveFavorites(); }}>
              <label for="studium-favorites">선호 좌석 번호</label>
              <input id="studium-favorites" type="text" bind:this={favoritesInput} bind:value={favoritesDraft} oninput={() => favoritesEditing = true} disabled={busy} maxlength={(room?.capacity ?? 311) * 5} placeholder="예: 1, 42 84" aria-invalid={parsedFavorites.error ? 'true' : undefined} aria-describedby={parsedFavorites.error ? 'studium-favorites-help studium-favorites-error' : 'studium-favorites-help'} />
              <div class="editor-actions"><button class="button secondary small" type="submit" disabled={busy || !favoritesDirty || Boolean(parsedFavorites.error)}>선호 좌석 저장</button><button class="button secondary small" type="button" disabled={busy || !favoritesEditing} onclick={cancelFavoritesEdit}>선호 좌석 변경 취소</button></div>
            </form>
            <p class="editor-help" id="studium-favorites-help">{room?.label}의 1–{room?.capacity}번을 쉼표나 공백으로 구분합니다. 빈칸으로 저장하면 선호 설정을 지웁니다.</p>
            {#if parsedFavorites.error}<p class="validation editor-help" id="studium-favorites-error" role="status">{parsedFavorites.error}</p>{/if}
            <p class="saved-favorites">저장한 선호 좌석: {savedFavorites.length ? savedFavorites.join(', ') + '번' : '없음'}</p>
            <p class="favorite-availability" role="status">{favoriteAvailability()}</p>
            <p class="editor-help">표시할 열람실을 바꾸면 저장하지 않은 선호 번호와 검색을 초기화합니다. 조회 대상에서 뺀 방의 선호 설정은 유지합니다.</p>
          </section>
          <section class="alert-editor" aria-labelledby="studium-alert-title">
            <h4 id="studium-alert-title">좌석 알림 조건</h4>
            <form onsubmit={(event) => { event.preventDefault(); void saveAlert(); }}>
              <label class="alert-toggle"><input type="checkbox" bind:this={alertToggle} bind:checked={alertEnabled} onchange={() => alertEditing = true} disabled={busy} />이 열람실 좌석 알림</label>
              <label for="studium-alert-kind">알림 조건</label>
              <select id="studium-alert-kind" bind:value={alertKind} onchange={() => alertEditing = true} disabled={busy}><option value="remaining">빈 좌석 수</option><option value="preferred">선호 좌석 중 빈자리</option></select>
              {#if alertKind === 'remaining'}<label for="studium-alert-minimum">알림을 보낼 최소 빈 좌석 수</label><input id="studium-alert-minimum" type="number" min="1" max={room?.capacity} step="1" bind:value={alertMinimum} oninput={() => alertEditing = true} disabled={busy} aria-invalid={alertInvalid ? 'true' : undefined} aria-describedby="studium-alert-help" />{/if}
              <div class="editor-actions"><button class="button secondary small" type="submit" disabled={busy || !alertDirty || alertInvalid}>알림 조건 저장</button><button class="button secondary small" type="button" disabled={busy || !alertEditing} onclick={cancelAlertEdit}>알림 조건 변경 취소</button></div>
            </form>
            {#if alertKind === 'preferred' && !savedFavorites.length}<p class="validation editor-help" role="status">선호 좌석 알림을 켜려면 이 방의 선호 번호를 먼저 저장해 주세요.</p>
            {:else if alertInvalid}<p class="validation editor-help" role="status">1부터 {room?.capacity} 사이의 정수를 입력해 주세요.</p>{/if}
            <p class="saved-alert">저장한 조건: {savedAlert.enabled ? savedAlert.kind === 'remaining' ? `켜짐 · 빈 좌석 ${savedAlert.minimum}석 이상` : '켜짐 · 선호 좌석 중 빈자리' : '꺼짐'}</p>
            <p class="editor-help" id="studium-alert-help">최신 조회가 조건을 처음 만족하면 한 번 알립니다. 이후 확인에 성공한 결과가 조건을 벗어났다가 다시 만족할 때 다시 알립니다. 조회 실패로 반복 알리지 않습니다. 조건 저장만으로 조회하거나 알리지 않습니다.</p>
            <p class="notification-policy" role="status">{notificationLoading ? '전체 알림 설정과 권한을 확인하고 있습니다.' : notificationPolicyStatus()}</p>
            {#if notificationSettings?.quiet_hours.enabled}<p class="editor-help">조용한 시간 {wallTime(notificationSettings.quiet_hours.start, clockSetting.format)}–{wallTime(notificationSettings.quiet_hours.end, clockSetting.format)}(PC 현지 시각)에는 알리지 않습니다.</p>{/if}
            <p class="editor-help">전체 알림의 10분당 최대 3회 제한을 함께 적용합니다. 조용한 시간이나 빈도 제한으로 보내지 못하면 다음 최신 조회에서 다시 판단합니다. 알림을 눌러도 빈자리가 유지된다고 보장하지 않습니다.</p>
            <div class="editor-actions"><button class="button secondary small" onclick={onsettings}>{proactiveEnabled ? '알림 설정 열기' : '확장 기능 설정 열기'}</button><button class="button secondary small" disabled={notificationLoading} onclick={() => void loadNotificationContext()}>알림 상태 새로 확인</button></div>
            {#if notificationError}<p class="archive-error" role="alert">{notificationError}</p>{/if}
            {#if snapshot.alert_error}<p class="archive-error" role="alert">좌석 알림 전달 상태: {snapshot.alert_error}</p>{/if}
          </section>
          <div class="seat-search"><label for="studium-seat-search">빈 좌석 번호 검색</label><input id="studium-seat-search" type="search" inputmode="numeric" bind:this={searchInput} bind:value={searchDraft} disabled={busy} aria-invalid={searchInvalid ? 'true' : undefined} aria-describedby={searchInvalid ? 'studium-search-error' : undefined} /><button class="button secondary small" disabled={busy || !searchActive} onclick={() => { searchDraft = ''; searchInput?.focus(); }}>검색 해제</button></div>
          {#if searchInvalid}<p class="validation editor-help" id="studium-search-error" role="status">1부터 {room?.capacity} 사이의 번호 하나를 입력해 주세요.</p>{/if}
          {#if result?.result}
            {@const data = result.result}
            <p class="seat-count">빈 좌석 <strong>{data.remaining.toLocaleString('ko-KR')}석</strong><span> / 전체 {data.total.toLocaleString('ko-KR')}석</span></p>
            {#if stale(data.source_time)}<p class="stale" role="status">오래되었거나 PC 시계와 맞지 않는 정보입니다. 지금 확인으로 다시 확인해 주세요.</p>{/if}
            {#if data.seat_numbers !== null}
              {#if searchActive && !searchInvalid}<p class="search-count" role="status">전체 빈 좌석 {data.remaining}석 · 검색 결과 {filteredNumbers.length}개{stale(data.source_time) ? ' · 이전 조회 기준' : ''}</p>{/if}
              <p class="seat-numbers"><span>빈 좌석 번호</span><strong>{#if searchInvalid}검색 번호를 확인해 주세요.{:else if filteredNumbers.length}{#each filteredNumbers as number, index}{#if savedFavorites.includes(number)}<mark class="preferred"><span aria-hidden="true">★ </span>{number}<span class="sr-only">번 선호 좌석</span></mark>{:else}{number}{/if}{index < filteredNumbers.length - 1 ? ', ' : '번'}{/each}{:else}{data.remaining === 0 ? '빈 좌석 없음' : '검색한 번호가 빈 좌석 목록에 없습니다.'}{/if}</strong></p>
            {:else}<p class="seat-numbers"><span>빈 좌석 번호</span><strong>확인 불가 · 빈 좌석 수만 확인됨</strong></p>{#if searchActive}<p class="editor-help">좌석 번호를 검증하지 못해 검색할 수 없습니다.</p>{/if}{/if}
            <StudiumSeatMap {data} identity={`${viewRoom}:${snapshot.job_id ?? 0}:${data.source_time}:${result.checked_at ?? ''}`} roomLabel={room?.label ?? '열람실'} favorites={savedFavorites} searchNumber={searchActive && !searchInvalid ? Number(searchDraft) : null} stale={stale(data.source_time)} onfocuslost={() => void restoreMapFocus()} />
            {#if result.error}<p class="detail-error">{result.error}</p>{/if}
            <dl class="times"><div><dt>원본 기준 시각 · 한국시간</dt><dd><time datetime={data.source_time}>{clock(data.source_time)}</time></dd></div><div><dt>조회 시각 · 한국시간</dt><dd>{#if result.checked_at}<time datetime={result.checked_at}>{clock(result.checked_at)}</time>{:else}확인 불가{/if}</dd></div></dl>
          {:else}<p class="empty-result">{stateLabel(result)}</p>{#if searchActive}<p class="editor-help">좌석 번호를 확인한 뒤 검색할 수 있습니다.</p>{/if}{#if result?.error}<p class="detail-error">{result.error}</p>{/if}{/if}
        </section>
      </section>
    {/if}
  {/if}
</div>

<style>
  .studium { max-width: 860px; display: grid; gap: 28px; }
  .mini-launch { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; color: var(--secondary); font-size: 12px; }
  section { min-width: 0; }
  .section-heading { display: flex; align-items: center; gap: 5px; margin-bottom: 10px; }
  h2 { margin: 0; font-size: 15px; font-weight: 650; }
  h3 { margin: 0 0 12px; font-size: 14px; font-weight: 650; }
  h3:focus-visible { outline: 2px solid var(--accent); outline-offset: 4px; }
  h4 { margin: 0 0 10px; font-size: 13px; font-weight: 650; }
  .targets { border: 1px solid var(--line); border-radius: 10px; background: var(--pane); padding: 0; margin: 0; min-width: 0; }
  .target { display: flex; align-items: center; gap: 10px; min-height: 42px; padding: 8px 14px; font-size: 13px; }
  .target + .target { border-top: 1px solid var(--line); }
  input { accent-color: var(--accent-fill); }
  .capacity { margin-left: auto; color: var(--secondary); font-variant-numeric: tabular-nums; }
  .target-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; margin-top: 10px; font-size: 12px; }
  .validation, .stale, .detail-error { color: var(--danger); }
  .results-heading { justify-content: space-between; flex-wrap: wrap; }
  .actions { display: flex; gap: 8px; }
  .automatic-interval { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; font-size: 13px; }
  .automatic-interval input { width: 64px; padding: 5px 8px; color: var(--text); background: var(--field); border: 1px solid var(--line-strong); border-radius: 6px; }
  .automatic-help { margin: 8px 0 12px; color: var(--secondary); font-size: 12px; line-height: 1.6; }
  .automatic-help.validation { color: var(--danger); }
  .automatic-status { margin: 10px 0 0; font-size: 13px; line-height: 1.6; color: var(--secondary); }
  .live-status { margin: 0 0 10px; min-height: 20px; color: var(--secondary); font-size: 12px; }
  .room-summary { list-style: none; padding: 0; margin: 0 0 18px; border-top: 1px solid var(--line); }
  .room-summary li { display: flex; justify-content: space-between; flex-wrap: wrap; gap: 5px 12px; border-bottom: 1px solid var(--line); padding: 10px 0; font-size: 13px; }
  .room-summary li span:last-child { color: var(--secondary); font-variant-numeric: tabular-nums; }
  .view-picker { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; margin-bottom: 14px; font-size: 13px; }
  select { max-width: 100%; min-width: 0; padding: 5px 8px; color: var(--text); background: var(--field); border: 1px solid var(--line-strong); border-radius: 6px; }
  .room-detail { padding: 16px; border: 1px solid var(--line); border-radius: 10px; background: var(--pane); }
  .favorites-editor, .alert-editor { margin: 0 0 16px; padding-bottom: 14px; border-bottom: 1px solid var(--line); }
  .favorites-editor form, .alert-editor form { display: grid; gap: 8px; font-size: 13px; }
  .favorites-editor input, .seat-search input, .alert-editor input[type=number] { width: 100%; min-width: 0; padding: 6px 8px; color: var(--text); background: var(--field); border: 1px solid var(--line-strong); border-radius: 6px; }
  .alert-editor input[type=number] { max-width: 120px; }
  .alert-toggle { display: flex; align-items: center; gap: 8px; }
  .editor-actions, .seat-search { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .seat-search { margin-bottom: 14px; font-size: 13px; }
  .seat-search input { width: 110px; }
  .editor-help { margin: 8px 0; color: var(--secondary); font-size: 12px; line-height: 1.6; }
  .editor-help.validation { color: var(--danger); }
  .saved-favorites, .favorite-availability, .search-count, .saved-alert, .notification-policy { margin: 10px 0; font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
  .preferred { padding: 1px 3px; color: var(--text); background: var(--field); border: 1px solid var(--line-strong); border-radius: 4px; white-space: nowrap; }
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
