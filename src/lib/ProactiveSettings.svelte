<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { LifecycleStatus, NotificationAuthorization, ProactiveHistoryItem, ProactiveSettings as Settings, ProactiveSnapshot } from './types';
  import { archiveError } from './archive-format';

  let { desktop, onopen }: { desktop: boolean; onopen: (documentId: number, revisionId: number, eventId: number, attemptCount?: number, eventIds?: number[]) => void } = $props();
  let data = $state<ProactiveSnapshot | null>(null);
  let lifecycle = $state<LifecycleStatus | null>(null);
  let notificationAuthorization = $state<NotificationAuthorization>('unknown');
  let permissionBusy = $state(false);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let historyBusy = $state(false);

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
        invoke<ProactiveHistoryItem[]>('proactive_history', { limit: 200 }),
      ]);
      data = { ...snapshot, history };
      lifecycle = status;
      try { notificationAuthorization = authorizationStatus(await invoke<string>('notification_authorization')); }
      catch { notificationAuthorization = 'unavailable'; }
      if (snapshot.pending_activation) {
        const activation = snapshot.pending_activation;
        onopen(activation.document_id, activation.revision_id, activation.event_id, activation.attempt_count, activation.event_ids);
      }
    } catch (reason) { error = archiveError(reason); }
    finally { loading = false; }
  }

  async function requestNotificationPermission() {
    if (permissionBusy) return;
    permissionBusy = true;
    error = '';
    notice = '';
    try {
      notificationAuthorization = authorizationStatus(await invoke<string>('notification_request_permission'));
      notice = notificationAuthorization === 'granted' ? '알림 권한을 허용했습니다.' : '';
    } catch (reason) {
      notificationAuthorization = 'unavailable';
      error = archiveError(reason);
    } finally { permissionBusy = false; }
  }

  function authorizationLabel(status: NotificationAuthorization) {
    return ({ granted: '허용됨', denied: '거부됨', not_determined: '아직 결정되지 않음', unknown: '확인할 수 없음', unavailable: '이 환경에서 사용할 수 없음' })[status];
  }

  async function save(patch: Partial<Settings>) {
    if (!data || busy) return;
    busy = true;
    error = '';
    notice = '';
    try {
      const settings = { ...data.settings, ...patch,
        // Login registration is reconciled with the OS only by its lifecycle command.
        launch_at_login: lifecycle?.launch_at_login ?? data.settings.launch_at_login };
      data = await invoke<ProactiveSnapshot>('proactive_save', { settings });
      notice = '백그라운드 설정을 저장했습니다.';
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
    historyBusy = true;
    error = '';
    notice = '';
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
    return Number.isNaN(date.getTime()) ? value : new Intl.DateTimeFormat('ko-KR', { dateStyle: 'short', timeStyle: 'short' }).format(date);
  }
</script>

<section id="background" class="settings-section" aria-busy={loading || busy}>
  <div class="section-heading"><h2>백그라운드</h2><p>원할 때만 앱을 켜고, 로컬에서 정한 간격으로 새 소식을 확인합니다.</p></div>
  {#if error}<p role="alert" class="archive-error">{error}</p>{/if}
  <p class="archive-feedback" role="status" aria-live="polite">{notice}</p>
  {#if !desktop}
    <p class="muted">브라우저 미리보기에서는 설정을 저장하거나 운영체제 기능을 확인할 수 없습니다.</p>
  {:else if loading}
    <p class="loading" role="status">백그라운드 설정을 불러오는 중입니다…</p>
  {:else if data && lifecycle}
    <div class="setting-row"><div><h3>창을 닫을 때 백그라운드에서 실행</h3><p>{lifecycle.tray_available ? '창만 숨깁니다. 메뉴 막대 아이콘에서 다시 열 수 있습니다.' : '메뉴 막대 아이콘을 사용할 수 없어 설정할 수 없습니다.'}</p></div><button class="switch" class:on={lifecycle.close_to_background} role="switch" aria-label="창을 닫아도 백그라운드에서 실행" aria-checked={lifecycle.close_to_background} disabled={busy || !lifecycle.tray_available} onclick={() => setLifecycle('lifecycle_set_close_to_background', !lifecycle?.close_to_background)}><span></span></button></div>
    <div class="setting-row"><div><h3>로그인 시 자동 시작</h3><p>{lifecycle.autostart_enabled === null ? '운영체제의 등록 상태를 확인할 수 없습니다.' : lifecycle.autostart_enabled ? '운영체제에서 로그인 항목으로 등록되어 있습니다.' : '운영체제 로그인 항목에 등록되어 있지 않습니다.'}</p></div><button class="switch" class:on={lifecycle.autostart_enabled === true} role="switch" aria-label="로그인 시 자동 시작" aria-checked={lifecycle.autostart_enabled === true} disabled={busy || lifecycle.autostart_enabled === null} onclick={() => setLifecycle('lifecycle_set_launch_at_login', !lifecycle?.autostart_enabled)}><span></span></button></div>
    <div class="setting-row"><div><h3>로그인 후 창 숨기기</h3><p>로그인으로 실행될 때만 적용됩니다. 직접 앱을 열면 창이 표시됩니다.</p></div><button class="switch" class:on={lifecycle.start_hidden} role="switch" aria-label="로그인 후 창 숨기기" aria-checked={lifecycle.start_hidden} disabled={busy || !lifecycle.autostart_enabled} onclick={() => setLifecycle('lifecycle_set_start_hidden', !lifecycle?.start_hidden)}><span></span></button></div>
    {#if lifecycle.launch_at_login !== (lifecycle.autostart_enabled ?? false)}<p class="footnote">저장된 로그인 설정과 운영체제 등록 상태가 다릅니다. 운영체제 상태를 자동으로 바꾸지 않았습니다.</p>{/if}

    <h3>자동 동기화</h3>
    <div class="setting-row"><div><h3>자동으로 새 공지 확인</h3><p>앱이 실행 중일 때 동기화합니다. 앱 종료 후에는 실행되지 않습니다.</p></div><button class="switch" class:on={data.settings.scheduler_enabled} role="switch" aria-label="자동 동기화" aria-checked={data.settings.scheduler_enabled} disabled={busy} onclick={() => save({ scheduler_enabled: !data?.settings.scheduler_enabled, scheduler_paused: false })}><span></span></button></div>
    <div class="setting-row"><div><h3>자동 동기화 일시 중지</h3><p>다시 켜면 저장된 간격으로 확인을 이어갑니다.</p></div><button class="switch" class:on={data.settings.scheduler_paused} role="switch" aria-label="자동 동기화 일시 중지" aria-checked={data.settings.scheduler_paused} disabled={busy || !data.settings.scheduler_enabled} onclick={() => save({ scheduler_paused: !data?.settings.scheduler_paused })}><span></span></button></div>
    <label class="field-label" for="scheduler-interval">확인 간격</label><select id="scheduler-interval" value={String(data.settings.scheduler_interval_minutes)} disabled={busy || !data.settings.scheduler_enabled} onchange={(event) => save({ scheduler_interval_minutes: Number(event.currentTarget.value) })}><option value="0">전원 모드 기본값</option><option value="15">15분</option><option value="30">30분</option><option value="60">1시간</option><option value="120">2시간</option><option value="240">4시간</option><option value="720">12시간</option></select>
    <p class="footnote">전원 모드 기본값은 절전 60분, 균형 30분, 성능 15분입니다. 직접 간격을 선택하면 전원 모드와 관계없이 해당 간격으로 확인합니다.</p>
    <div class="privacy-note"><p>상태: {data.scheduler.paused ? '일시 중지' : data.scheduler.enabled ? '사용 중' : '꺼짐'} · 대기 중인 알림 {data.scheduler.pending_events}개</p><p>최근 실행: {localTimestamp(data.scheduler.last_run_at, '아직 실행하지 않음')}{#if data.scheduler.last_outcome} · {outcomeLabel(data.scheduler.last_outcome)}{/if}</p><p>다음 확인 예정: {localTimestamp(data.scheduler.next_task_at, '예약된 작업 없음')}</p>{#if data.scheduler.last_error}<p role="alert">최근 오류: {data.scheduler.last_error}</p>{/if}</div>
    <button class="text-button" type="button" disabled={busy} onclick={load}>백그라운드 상태 새로고침</button>

    <h3 id="notifications">알림</h3>
    <div class="setting-row"><div><h3>알림 활성화</h3><p>공지 동기화와 알림 전달은 별도 설정입니다. 알림 전달은 시스템 권한, 이 설정, 방해 금지 시간 등에도 영향을 받습니다.</p></div><button class="switch" class:on={data.settings.notifications_enabled} role="switch" aria-label="알림 활성화" aria-checked={data.settings.notifications_enabled} disabled={busy} onclick={() => save({ notifications_enabled: !data?.settings.notifications_enabled })}><span></span></button></div>
    <div class="privacy-note" aria-live="polite">
      <p>운영체제 알림 권한: {authorizationLabel(notificationAuthorization)}</p>
      {#if notificationAuthorization === 'denied'}
        <p>권한이 거부되었습니다. macOS 시스템 설정 &gt; 알림 &gt; Rhythm에서 알림을 허용한 뒤 상태를 새로고침하세요.</p>
      {:else if notificationAuthorization === 'not_determined'}
        <p>아직 권한을 선택하지 않았습니다. 아래 버튼을 누를 때만 운영체제 권한 요청 창이 표시됩니다.</p>
      {:else if notificationAuthorization === 'unavailable' || notificationAuthorization === 'unknown'}
        <p>이 환경에서는 권한 상태를 확인할 수 없습니다. macOS 시스템 설정 &gt; 알림에서 Rhythm을 확인하세요.</p>
      {/if}
      <p>권한 허용은 알림 전달을 보장하지 않습니다. 운영체제의 집중 모드와 앱의 알림 설정에 따라 전달되지 않을 수 있습니다.</p>
    </div>
    <button class="text-button" type="button" disabled={permissionBusy || notificationAuthorization === 'unavailable'} onclick={requestNotificationPermission}>{permissionBusy ? '권한 확인 중…' : '운영체제 알림 권한 요청'}</button>
    <button class="text-button" type="button" disabled={busy || permissionBusy} onclick={load}>알림 권한 상태 새로고침</button>
    <label class="field-label" for="notification-sensitivity">알림 민감도</label><select id="notification-sensitivity" value={data.settings.notification_sensitivity} disabled={busy || !data.settings.notifications_enabled} onchange={(event) => save({ notification_sensitivity: event.currentTarget.value as Settings['notification_sensitivity'] })}><option value="important">중요한 공지만</option><option value="interested">관심 분야와 관련된 공지</option><option value="all">모든 새 공지와 변경</option></select>
    <div class="setting-row"><div><h3>방해 금지 시간</h3><p>이 시간에는 알림 전달을 미룹니다. 시작과 끝이 자정을 지나도 설정할 수 있습니다.</p></div><button class="switch" class:on={data.settings.quiet_hours.enabled} role="switch" aria-label="방해 금지 시간" aria-checked={data.settings.quiet_hours.enabled} disabled={busy} onclick={() => save({ quiet_hours: { ...data!.settings.quiet_hours, enabled: !data!.settings.quiet_hours.enabled } })}><span></span></button></div>
    <div class="form-grid"><label class="field-label" for="quiet-start">시작 시간<input id="quiet-start" type="time" value={data.settings.quiet_hours.start} disabled={busy} onchange={(event) => save({ quiet_hours: { ...data!.settings.quiet_hours, start: event.currentTarget.value } })} /></label><label class="field-label" for="quiet-end">종료 시간<input id="quiet-end" type="time" value={data.settings.quiet_hours.end} disabled={busy} onchange={(event) => save({ quiet_hours: { ...data!.settings.quiet_hours, end: event.currentTarget.value } })} /></label></div>
  {:else}
    <button class="text-button" type="button" onclick={load}>다시 시도</button>
  {/if}
</section>

<section id="notification-history" class="settings-section" aria-busy={historyBusy}>
  <div class="section-heading"><h2>알림 기록</h2><p>기록은 이 기기에만 저장됩니다. 공지 본문은 기록에 복사하지 않습니다.</p></div>
  {#if !desktop}<p class="muted">알림 기록은 데스크톱 앱에서 확인할 수 있습니다.</p>
  {:else if data}
    {#if data.history.length}
      <ul aria-label="알림 기록">{#each data.history as item (item.id)}<li class="setting-row"><div><h3>{item.title}</h3><p>{historyLabel(item)} · {item.created_at}</p>{#if item.error}<p role="alert">{item.error}</p>{/if}{#if item.reason}<p>{item.reason}</p>{/if}</div><button class="text-button" type="button" disabled={historyBusy} onclick={() => onopen(item.document_id, item.revision_id, item.id, item.attempt_count, [item.id])}>공지 열기</button></li>{/each}</ul>
      <button class="text-button" type="button" disabled={historyBusy} onclick={clearHistory}>{historyBusy ? '기록을 비우는 중…' : '알림 기록 비우기'}</button>
    {:else}<p class="muted">저장된 알림 기록이 없습니다.</p>{/if}
  {:else if loading}<p class="loading" role="status">알림 기록을 불러오는 중입니다…</p>
  {/if}
</section>
