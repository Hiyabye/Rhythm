<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { ArchiveSummary, SourceInfo, SyncStatus } from './types';
  import { archiveDate, archiveError, archiveSize, sourceStatus } from './archive-format';

  let { enabled }: { enabled: boolean } = $props();
  let sources = $state<SourceInfo[]>([]);
  let summary = $state<ArchiveSummary | null>(null);
  let status = $state<SyncStatus | null>(null);
  let filter = $state('');
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let feedback = $state('');
  let selected = $state<string[]>([]);
  let active = true;
  let polling = false;
  let visibleSources = $derived(sources.filter((source) => `${source.organization} ${source.name} ${source.description}`.toLocaleLowerCase('ko').includes(filter.trim().toLocaleLowerCase('ko'))));
  let organizations = $derived([...new Set(visibleSources.map((source) => source.organization))]);
  let subscribed = $derived(sources.filter((source) => source.subscribed && source.supported));

  onMount(() => {
    void refresh();
    const timer = window.setInterval(() => {
      if (!active || polling || !enabled) return;
      polling = true;
      void invoke<SyncStatus>('archive_status').then(async (next) => {
        if (!active) return;
        const changed = status?.running !== next.running;
        status = next;
        if (next.running || changed) await refresh(false);
      }).catch((reason) => { if (active) error = archiveError(reason); }).finally(() => polling = false);
    }, 1000);
    return () => { active = false; window.clearInterval(timer); };
  });

  async function refresh(showLoading = true) {
    if (showLoading) loading = true;
    try {
      const [nextSources, nextSummary, nextStatus] = await Promise.all([
        invoke<SourceInfo[]>('archive_sources'), invoke<ArchiveSummary>('archive_summary'), invoke<SyncStatus>('archive_status'),
      ]);
      if (!active) return;
      sources = nextSources;
      summary = nextSummary;
      status = nextStatus;
      selected = selected.filter((id) => nextSources.some((source) => source.id === id && source.subscribed && source.supported));
    } catch (reason) { if (active) error = archiveError(reason); }
    finally { if (active) loading = false; }
  }

  async function subscribe(source: SourceInfo) {
    busy = true;
    error = '';
    try {
      sources = await invoke<SourceInfo[]>('archive_subscribe', { id: source.id, subscribed: !source.subscribed });
      selected = selected.filter((id) => sources.some((item) => item.id === id && item.subscribed));
      feedback = source.subscribed ? '구독을 해제했습니다. 이미 보관한 공지는 유지됩니다.' : '정보원을 구독했습니다. 가져오기를 누르면 수집을 시작합니다.';
    } catch (reason) { error = archiveError(reason); }
    finally { busy = false; }
  }

  async function start(mode: 'historical' | 'incremental', ids?: string[]) {
    busy = true;
    error = '';
    feedback = '';
    try {
      status = await invoke<SyncStatus>('archive_start', { sourceIds: ids ?? (selected.length ? selected : null), mode });
      feedback = status.message;
      await refresh(false);
    } catch (reason) { error = archiveError(reason); }
    finally { busy = false; }
  }

  async function cancel() {
    busy = true;
    error = '';
    try { status = await invoke<SyncStatus>('archive_cancel'); feedback = '중단을 요청했습니다. 이미 저장한 공지는 유지됩니다.'; }
    catch (reason) { error = archiveError(reason); }
    finally { busy = false; }
  }

  function select(id: string, checked: boolean) {
    selected = checked ? [...selected, id] : selected.filter((value) => value !== id);
  }
</script>

<div class="source-manager" aria-label="서강대학교 공지 정보원 관리" aria-busy={loading || busy}>
  <div class="section-heading"><h3>정보원 구독</h3><p>서강대학교의 공개 게시판을 선택하세요. 구독만으로 자동 수집을 시작하지 않습니다.</p></div>
  <p class="footnote">우선 지원: 학부 학사공지와 소프트웨어융합대학 5개 게시판. 다른 게시판은 원문 수집이 검증되기 전까지 구독할 수 없습니다.</p>
  <div class="source-operations">
    <button class="button small" disabled={!enabled || busy || status?.running || !subscribed.length} onclick={() => start('historical')}>과거 자료 가져오기{selected.length ? ` (${selected.length})` : ''}</button>
    <button class="button secondary small" disabled={!enabled || busy || status?.running || !subscribed.length} onclick={() => start('incremental')}>새 소식 및 변경 확인</button>
    {#if status?.running}<button class="text-button" disabled={busy} onclick={cancel}>가져오기 중단</button>{/if}
    <button class="text-button" disabled={busy || loading} onclick={() => refresh()}>상태 새로고침</button>
  </div>
  <p class="footnote">{selected.length ? '선택한 구독 정보원을 대상으로 실행합니다.' : '체크한 정보원이 없으면 구독 중인 지원 정보원 전체를 대상으로 실행합니다.'} 과거 자료는 이어받을 수 있으며, 이전에 가져온 기록을 새 소식으로 세지 않습니다.</p>
  {#if status}<div class="source-progress" role="status" aria-live="polite"><strong>{status.running ? '가져오는 중' : '수집 대기'}</strong><span>{status.message}</span>{#if status.source_id}<span>{sources.find((source) => source.id === status?.source_id)?.name ?? '정보원 확인 중'}{#if status.page > 0} · {status.page}쪽{/if}</span>{/if}</div>{/if}
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  <p class="archive-feedback" role="status">{feedback}</p>
  <label class="field-label" for="source-search">학과 또는 게시판 찾기</label>
  <input id="source-search" class="source-search" type="search" bind:value={filter} placeholder="예: 소프트웨어융합대학, 학사, 전자공학" />
  <div class="source-catalog-heading"><span>정보원 {visibleSources.length}개 · 구독 {subscribed.length}개</span>{#if summary}<span>공지 {summary.documents}개 · 자료 {archiveSize(summary.resource_bytes)}</span>{/if}</div>
  {#if loading}<p class="loading" role="status">정보원 목록을 불러오는 중입니다…</p>{/if}
  {#each organizations as organization}
    <section class="source-group" aria-label={organization}>
      <h4>{organization}</h4>
      {#each visibleSources.filter((source) => source.organization === organization) as source (source.id)}
        <article class="source-entry" class:source-unsupported={!source.supported}>
          <div class="source-entry-heading">
            <label class="source-job-select"><input type="checkbox" checked={selected.includes(source.id)} disabled={!source.subscribed || !source.supported || busy || status?.running} onchange={(event) => select(source.id, event.currentTarget.checked)} aria-label={`${organization} ${source.name} 동기화 대상으로 선택`} /><strong>{source.name}</strong></label>
            <button class="switch" class:on={source.subscribed} role="switch" aria-checked={source.subscribed} aria-label={`${organization} ${source.name} 구독`} disabled={!enabled || !source.supported || busy || status?.running} onclick={() => subscribe(source)}><span></span></button>
          </div>
          <p>{source.description}</p>
          <div class="source-state-line"><span>{source.supported ? sourceStatus(source.status) : '원문 수집 미지원'}</span>{#if source.supported}<span>{source.history_complete ? '공개 과거 자료 가져오기 완료' : `과거 자료 ${source.next_page}쪽부터 이어받기`}</span>{/if}</div>
          {#if source.supported}
            <dl class="source-metadata"><div><dt>마지막 시도</dt><dd>{archiveDate(source.last_attempt, true)}</dd></div><div><dt>마지막 성공</dt><dd>{archiveDate(source.last_success, true)}</dd></div></dl>
            {#if source.discovered || source.archived || source.failed || source.status === 'running'}<p class="source-counts">발견 {source.discovered} · 보관 {source.archived} · 실패 {source.failed}</p>{/if}
            {#if source.last_error}<p class="archive-error">{source.last_error}</p>{/if}
            {#if source.subscribed}<div class="source-entry-actions"><button class="text-button" disabled={!enabled || busy || status?.running} onclick={() => start('historical', [source.id])}>{source.history_complete ? '과거 자료 상태 확인' : '과거 자료 이어받기'}</button><button class="text-button" disabled={!enabled || busy || status?.running} onclick={() => start('incremental', [source.id])}>변경 확인</button></div>{/if}
          {/if}
          <details class="source-reference"><summary>공식 게시판 정보</summary><p>{source.url}</p><p>수집 방식: {source.family}</p></details>
        </article>
      {/each}
    </section>
  {/each}
  {#if !loading && !visibleSources.length}<p class="muted">검색 조건에 맞는 정보원이 없습니다.</p>{/if}
  <p class="footnote">학교가 공개한 자료만 수집합니다. 로그인, 차단 또는 접근 제한이 필요한 자료는 우회하지 않습니다. 파일의 용량·접근 제한은 각 공지의 첨부 자료 상태에 표시됩니다.</p>
</div>
