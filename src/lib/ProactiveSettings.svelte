<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { LifecycleStatus, NotificationAuthorization, PlatformCapabilities, ProactiveActivation, ProactiveHistoryItem, ProactiveSettings as Settings, ProactiveSnapshot } from './types';
  import { archiveError, timeOptions } from './archive-format';
  import { clock } from './clock.svelte';
  import TimeField from './TimeField.svelte';
  import { confirmAction } from './confirm';
  import { isWindows } from './platform';
  import Icon from './Icon.svelte';
  import Hint from './Hint.svelte';

  // Background (lifecycle and scheduler) and notifications share one snapshot but live in separate panes.
  // `compact` (the setup flow) leaves out live status, pause and history.
  let { desktop, part, compact = false, onopen, onactivation }: { desktop: boolean; part: 'background' | 'notifications'; compact?: boolean; onopen: (documentId: number, revisionId: number, eventId: number, attemptCount?: number, eventIds?: number[]) => void; onactivation: (activation: ProactiveActivation) => void } = $props();
  const historyPreview = 10;
  let data = $state<ProactiveSnapshot | null>(null);
  let lifecycle = $state<LifecycleStatus | null>(null);
  let notificationAuthorization = $state<NotificationAuthorization>('unknown');
  let capabilities = $state<PlatformCapabilities | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let historyBusy = $state(false);
  let showAllHistory = $state(false);

  function authorizationStatus(value: string): NotificationAuthorization {
    if (value === 'authorized' || value === 'provisional' || value === 'ephemeral' || value === 'granted') return 'granted';
    if (value === 'denied' || value === 'not_determined' || value === 'unavailable') return value;
    return 'unknown';
  }

  onMount(() => { if (desktop) void load(); else loading = false; });

  async function load() {
    loading = true;
    error = '';
    try {
      const [snapshot, status, history] = await Promise.all([
        invoke<ProactiveSnapshot>('proactive_snapshot'),
        invoke<LifecycleStatus>('lifecycle_status'),
        part === 'notifications' && !compact ? invoke<ProactiveHistoryItem[]>('proactive_history', { limit: 200 }) : Promise.resolve([]),
      ]);
      data = { ...snapshot, history };
      lifecycle = status;
      if (part === 'notifications') {
        try { notificationAuthorization = authorizationStatus(await invoke<string>('notification_authorization')); }
        catch { notificationAuthorization = 'unavailable'; }
        try { capabilities = await invoke<PlatformCapabilities>('platform_capabilities'); }
        catch { capabilities = null; }
      }
      if (snapshot.pending_activation) {
        onactivation(snapshot.pending_activation);
      }
    } catch (reason) { error = archiveError(reason); }
    finally { loading = false; }
  }

  function authorizationLabel(status: NotificationAuthorization) {
    return ({ granted: '허용됨', denied: '거부됨', not_determined: isWindows ? '연결 확인 필요' : '아직 결정되지 않음', unknown: '확인할 수 없음', unavailable: '이 환경에서 사용할 수 없음' })[status];
  }

  // macOS shows its permission prompt only on this explicit request.
  async function requestNotificationPermission() {
    if (busy) return;
    busy = true;
    error = '';
    notice = '';
    try {
      const result = await invoke<{ status: string }>('notification_request_permission');
      notificationAuthorization = authorizationStatus(result.status);
      notice = notificationAuthorization === 'granted' ? '알림 권한을 허용했습니다.' : '';
    } catch (reason) {
      notificationAuthorization = 'unavailable';
      error = archiveError(reason);
    } finally { busy = false; }
  }

  async function testNotificationConnection() {
    if (busy || notificationAuthorization !== 'not_determined') return;
    busy = true;
    error = '';
    notice = '';
    try {
      const result = await invoke<{ status: string; test_notification_requested: boolean }>('notification_request_permission');
      await load();
      notice = result.test_notification_requested
        ? 'Windows에 시험 알림 표시를 요청했습니다. 실제 화면 표시 여부는 확인하지 못했습니다.'
        : '시험 알림을 요청하지 못했습니다. Windows 알림 상태를 확인해 주세요.';
    } catch (reason) { error = archiveError(reason); }
    finally { busy = false; }
  }

  async function save(patch: Partial<Settings>) {
    if (!data || busy) return;
    busy = true;
    error = '';
    notice = '';
    try {
      // The background and notification parts can share a pane; merging into
      // freshly read settings keeps one part from undoing the other's change.
      const current = await invoke<ProactiveSnapshot>('proactive_snapshot');
      // Reading the snapshot hands over a pending notification click.
      if (current.pending_activation) onactivation(current.pending_activation);
      const settings = { ...current.settings, ...patch,
        // Login registration is reconciled with the OS only by its lifecycle command.
        launch_at_login: lifecycle?.launch_at_login ?? current.settings.launch_at_login };
      data = await invoke<ProactiveSnapshot>('proactive_save', { settings });
      notice = '설정을 저장했습니다.';
    } catch (reason) { error = archiveError(reason); }
    finally { busy = false; }
  }

  async function setLifecycle(command: string, enabled: boolean) {
    if (busy) return;
    busy = true;
    error = '';
    notice = '';
    try {
      lifecycle = await invoke<LifecycleStatus>(command, { enabled });
      // Lifecycle setters persist these fields. Refresh the scheduler snapshot to
      // keep its full-settings payload synchronized with the authoritative value.
      data = await invoke<ProactiveSnapshot>('proactive_snapshot');
      notice = '시작 및 종료 설정을 저장했습니다.';
    } catch (reason) { error = archiveError(reason); }
    finally { busy = false; }
  }

  async function clearHistory() {
    if (!data || busy || historyBusy) return;
    error = '';
    notice = '';
    let confirmed: boolean;
    try {
      confirmed = await confirmAction('표시된 알림 기록을 모두 지울까요? 지운 기록은 되돌릴 수 없습니다.\n\n보관한 공지와 알림 설정은 그대로이며, 같은 공지를 다시 알리지 않도록 하는 중복 방지 정보와 최근 알림 횟수 제한도 유지됩니다.', '기록 비우기');
    } catch (reason) {
      error = `${archiveError(reason)} 확인 창을 열지 못해 기록을 비우지 않았습니다.`;
      return;
    }
    if (!confirmed || historyBusy) return;
    historyBusy = true;
    try {
      data = await invoke<ProactiveSnapshot>('proactive_clear_history');
      notice = '알림 기록을 비웠습니다.';
    } catch (reason) { error = archiveError(reason); }
    finally { historyBusy = false; }
  }

  function historyLabel(item: ProactiveHistoryItem) {
    const state: Record<string, string> = { pending: '대기 중', claimed: '처리 중', attempted: '운영체제에 전달 요청 · 수신 확인 안 됨', deferred: '나중에 전달 예정', delivered: '알림을 열었음', suppressed: '기준 미달', failed: '실패' };
    return `${item.event_type === 'new' ? '새 공지' : '수정된 공지'} · ${state[item.state] ?? item.state}`;
  }

  function outcomeLabel(value: string | null) {
    return ({ success: '완료', partial: '일부 완료', skipped: '건너뜀', failure: '실패', cancelled: '중단' } as Record<string, string>)[value ?? ''] ?? '';
  }
  function localTimestamp(value: string | null, fallback = '기록 없음') {
    if (!value) return fallback;
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : new Intl.DateTimeFormat('ko-KR', { year: '2-digit', month: '2-digit', day: '2-digit', ...timeOptions(clock.format) }).format(date);
  }
</script>

<div id={part} class="proactive-settings" aria-busy={loading || busy || historyBusy}>
  {#if error}<p role="alert" class="archive-error">{error}</p>{/if}
  <p class="archive-feedback" role="status" aria-live="polite">{notice}</p>
  {#if !desktop}
    <p class="muted">브라우저 미리보기에서는 설정을 저장하거나 운영체제 기능을 확인할 수 없습니다.</p>
  {:else if loading}
    <p class="loading" role="status">설정을 불러오는 중입니다…</p>
  {:else if data && lifecycle && part === 'background'}
    {@const trayName = isWindows ? '알림 영역 아이콘' : '메뉴 막대 아이콘'}
    <h3 class="settings-group-title">시작과 종료</h3>
    <div class="settings-group">
      <div class="settings-row"><span class="row-label">창을 닫아도 계속 실행<Hint label="창을 닫아도 계속 실행">창만 숨깁니다. {trayName}에서 다시 열 수 있습니다.</Hint>{#if !lifecycle.tray_available}<span class="row-detail">{trayName}을 사용할 수 없어 켤 수 없습니다.</span>{/if}</span><button class="switch" class:on={lifecycle.close_to_background} role="switch" aria-label="창을 닫아도 백그라운드에서 실행" aria-checked={lifecycle.close_to_background} disabled={busy || !lifecycle.tray_available} onclick={() => setLifecycle('lifecycle_set_close_to_background', !lifecycle?.close_to_background)}><span></span></button></div>
      <div class="settings-row"><span class="row-label">로그인 시 자동 시작{#if lifecycle.autostart_enabled === null}<span class="row-detail">운영체제의 등록 상태를 확인할 수 없습니다.</span>{:else if lifecycle.launch_at_login !== lifecycle.autostart_enabled}<span class="row-detail">저장된 설정과 운영체제 등록 상태가 다릅니다. 운영체제 상태는 바꾸지 않았습니다.</span>{/if}</span><button class="switch" class:on={lifecycle.autostart_enabled === true} role="switch" aria-label="로그인 시 자동 시작" aria-checked={lifecycle.autostart_enabled === true} disabled={busy || lifecycle.autostart_enabled === null} onclick={() => setLifecycle('lifecycle_set_launch_at_login', !lifecycle?.autostart_enabled)}><span></span></button></div>
      <div class="settings-row"><span class="row-label">로그인 후 창 숨기기<Hint label="로그인 후 창 숨기기">로그인으로 실행될 때만 적용됩니다. 직접 앱을 열면 창이 보입니다.</Hint></span><button class="switch" class:on={lifecycle.start_hidden} role="switch" aria-label="로그인 후 창 숨기기" aria-checked={lifecycle.start_hidden} disabled={busy || !lifecycle.autostart_enabled} onclick={() => setLifecycle('lifecycle_set_start_hidden', !lifecycle?.start_hidden)}><span></span></button></div>
    </div>

    <div class="settings-group-title"><h3>자동 동기화</h3><Hint label="자동 동기화">앱이 실행 중일 때만 새 공지를 확인합니다. 앱을 종료하면 멈추고, 잠자기 중에는 확인하지 않습니다. 전원 모드 기본값은 절전 60분, 균형 30분, 성능 15분이며, 간격을 직접 고르면 전원 모드와 관계없이 그 간격을 지킵니다.</Hint></div>
    <div class="settings-group">
      <div class="settings-row"><span class="row-label">새 공지 자동 확인</span><button class="switch" class:on={data.settings.scheduler_enabled} role="switch" aria-label="자동 동기화" aria-checked={data.settings.scheduler_enabled} disabled={busy} onclick={() => save({ scheduler_enabled: !data?.settings.scheduler_enabled, scheduler_paused: false })}><span></span></button></div>
      {#if !compact}<div class="settings-row"><span class="row-label">일시 중지</span><button class="switch" class:on={data.settings.scheduler_paused} role="switch" aria-label="자동 동기화 일시 중지" aria-checked={data.settings.scheduler_paused} disabled={busy || !data.settings.scheduler_enabled} onclick={() => save({ scheduler_paused: !data?.settings.scheduler_paused })}><span></span></button></div>{/if}
      <div class="settings-row"><label class="row-label" for="scheduler-interval">확인 간격</label><select id="scheduler-interval" value={String(data.settings.scheduler_interval_minutes)} disabled={busy || !data.settings.scheduler_enabled} onchange={(event) => save({ scheduler_interval_minutes: Number(event.currentTarget.value) })}><option value="0">전원 모드 기본값</option><option value="15">15분</option><option value="30">30분</option><option value="60">1시간</option><option value="120">2시간</option><option value="240">4시간</option><option value="720">12시간</option></select></div>
      {#if !compact}<div class="settings-row scheduler-status"><span class="row-label">상태<span class="row-detail">{data.scheduler.paused ? '일시 중지' : data.scheduler.enabled ? '사용 중' : '꺼짐'} · 최근 실행 {localTimestamp(data.scheduler.last_run_at, '없음')}{#if data.scheduler.last_outcome} ({outcomeLabel(data.scheduler.last_outcome)}){/if} · 다음 확인 {localTimestamp(data.scheduler.next_task_at, '예약 없음')} · 대기 중인 알림 {data.scheduler.pending_events}개</span>{#if data.scheduler.last_error}<span class="row-detail error-text">최근 오류: {archiveError(data.scheduler.last_error)} · 다음 확인 때 다시 시도합니다.</span>{/if}</span><button class="row-icon" type="button" aria-label="백그라운드 상태 새로고침" disabled={busy} onclick={load}><Icon name="refresh" size={15} /></button></div>{/if}
    </div>
  {:else if data && part === 'notifications'}
    <div class="settings-group">
      <div class="settings-row"><span class="row-label">알림<Hint label="알림">공지 동기화와 알림은 별개입니다. 알림은 {isWindows ? 'Windows 설정' : '시스템 권한'}, 집중 모드, 방해 금지 시간의 영향을 받으며, 권한을 허용해도 전달을 보장하지는 않습니다.</Hint>{#if !capabilities?.notifications.available}<span class="row-detail">{capabilities?.notifications.reason ?? '알림 사용 가능 여부를 확인하지 못했습니다.'}</span>{/if}</span><button class="switch" class:on={data.settings.notifications_enabled} role="switch" aria-label="알림 활성화" aria-checked={data.settings.notifications_enabled} disabled={busy || (isWindows && (!capabilities?.notifications.available || notificationAuthorization !== 'granted') && !data.settings.notifications_enabled)} onclick={() => save({ notifications_enabled: !data?.settings.notifications_enabled })}><span></span></button></div>
      <div class="settings-row" aria-live="polite">
        <span class="row-label">운영체제 알림 권한<span class="row-detail">{authorizationLabel(notificationAuthorization)}</span>
          {#if notificationAuthorization === 'denied'}<span class="row-detail">{isWindows ? 'Windows 설정 → 시스템 → 알림에서 Rhythm을 허용한 뒤 새로고침하세요.' : '시스템 설정의 알림에서 Rhythm을 허용한 뒤 새로고침하세요.'}</span>
          {:else if notificationAuthorization === 'unavailable'}<span class="row-detail">{isWindows ? 'Windows 알림 상태를 확인할 수 없습니다. 앱 설치 상태와 Windows 알림 설정을 확인하세요.' : '이 환경에서는 권한을 확인하거나 요청할 수 없습니다.'}</span>
          {:else if notificationAuthorization === 'unknown'}<span class="row-detail">운영체제가 상태를 알려주지 않았습니다. 전달은 확인되지 않았습니다.</span>{/if}
        </span>
        {#if isWindows && notificationAuthorization === 'not_determined'}<Hint label="시험 알림">Windows에 시험 알림을 한 번 요청해 연결을 확인합니다. 공지 알림을 켜지는 않습니다.</Hint><button class="button secondary small" type="button" disabled={busy} aria-label="시험 알림으로 연결 확인" onclick={testNotificationConnection}>시험 알림</button>
        {:else if !isWindows && notificationAuthorization === 'not_determined'}<Hint label="권한 요청">이 버튼을 누를 때만 운영체제 권한 요청 창이 뜹니다.</Hint>{/if}
        {#if !isWindows}<button class="button secondary small" type="button" disabled={busy || notificationAuthorization === 'unavailable'} aria-label="운영체제 알림 권한 요청" onclick={requestNotificationPermission}>권한 요청</button>{/if}
        <button class="row-icon" type="button" disabled={busy} aria-label="알림 권한 상태 새로고침" onclick={load}><Icon name="refresh" size={15} /></button>
      </div>
      <div class="settings-row"><label class="row-label" for="notification-sensitivity">알림 민감도</label><select id="notification-sensitivity" value={data.settings.notification_sensitivity} disabled={busy || !data.settings.notifications_enabled} onchange={(event) => save({ notification_sensitivity: event.currentTarget.value as Settings['notification_sensitivity'] })}><option value="important">중요한 공지만</option><option value="interested">관심 분야 공지</option><option value="all">모든 새 공지와 변경</option></select></div>
      <div class="settings-row quiet-hours"><span class="row-label">방해 금지 시간<Hint label="방해 금지 시간">이 시간에는 알림을 미뤘다가 끝난 뒤 보냅니다. 자정을 넘겨도 됩니다.</Hint></span><span class="time-range"><TimeField value={data.settings.quiet_hours.start} label="시작 시간" disabled={busy} onchange={(start) => save({ quiet_hours: { ...data!.settings.quiet_hours, start: start! } })} /><span aria-hidden="true">–</span><TimeField value={data.settings.quiet_hours.end} label="종료 시간" disabled={busy} onchange={(end) => save({ quiet_hours: { ...data!.settings.quiet_hours, end: end! } })} /></span><button class="switch" class:on={data.settings.quiet_hours.enabled} role="switch" aria-label="방해 금지 시간" aria-checked={data.settings.quiet_hours.enabled} disabled={busy} onclick={() => save({ quiet_hours: { ...data!.settings.quiet_hours, enabled: !data!.settings.quiet_hours.enabled } })}><span></span></button></div>
    </div>

    {#if !compact}
    <div class="settings-group-title"><h3 id="notification-history-title" tabindex="-1">알림 기록</h3><Hint label="알림 기록">기록은 이 기기에만 남고 공지 본문은 복사하지 않습니다. 운영체제에 전달을 요청한 것만으로는 표시나 수신이 확인되지 않습니다.</Hint></div>
    {#if data.history.length}
      <ul class="settings-group" aria-label="알림 기록">{#each showAllHistory ? data.history : data.history.slice(0, historyPreview) as item (item.id)}<li class="settings-row"><span class="row-label history-title">{item.title}<span class="row-detail">{historyLabel(item)} · {localTimestamp(item.created_at)}</span>{#if item.error}<span class="row-detail">{item.error}</span>{/if}{#if item.reason}<span class="row-detail">{item.reason}</span>{/if}</span><button class="text-button" type="button" disabled={historyBusy} aria-label={`${item.title} 공지 열기`} onclick={() => onopen(item.document_id, item.revision_id, item.id, item.attempt_count, [item.id])}>열기</button></li>{/each}</ul>
      <div class="group-footer">
        {#if data.history.length > historyPreview}<button class="text-button" type="button" aria-expanded={showAllHistory} onclick={() => showAllHistory = !showAllHistory}>{showAllHistory ? '최근 기록만 보기' : `모두 보기 (${data.history.length}개)`}</button>{/if}
        <button class="text-button" type="button" disabled={historyBusy} onclick={clearHistory}>{historyBusy ? '기록을 비우는 중…' : '알림 기록 비우기'}</button>
      </div>
    {:else}<p class="muted group-empty">저장된 알림 기록이 없습니다.</p>{/if}
    {/if}
  {:else}
    <button class="text-button" type="button" onclick={load}>다시 시도</button>
  {/if}
</div>
