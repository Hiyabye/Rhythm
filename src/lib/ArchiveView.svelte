<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { AiSnapshot, ArchivePage, ArchiveSummary, AiResult, DocumentItem, PersonalizedRecommendation, PersonalizationRecommendations, SourceInfo, SyncStatus } from './types';
  import analysisContract from './analysis-contract.json' with { type: 'json' };
  import { archiveDate, archiveError, archiveSize } from './archive-format';
  import DocumentReader from './DocumentReader.svelte';
  import Highlighted from './Highlighted.svelte';
  import Icon from './Icon.svelte';

  let { page, desktop, enabled, activation, onactivationhandled, onsettings, onaisettings }: { page: string; desktop: boolean; enabled: boolean; activation: { document_id: number; revision_id: number; event_id: number } | null; onactivationhandled: () => void; onsettings: () => void; onaisettings: () => void } = $props();
  let result = $state<ArchivePage>({ items: [], total: 0, categories: [] });
  let summary = $state<ArchiveSummary | null>(null);
  let sources = $state<SourceInfo[]>([]);
  let recentAnalyses = $state<AiResult[]>([]);
  let aiError = $state('');
  let aiSnapshot = $state<AiSnapshot | null>(null);
  let recommendations = $state<PersonalizedRecommendation[]>([]);
  let hasPreferences = $state(false);
  let recommendationError = $state('');
  let status = $state<SyncStatus | null>(null);
  let draft = $state('');
  let query = $state('');
  let source = $state('');
  let category = $state('');
  let unread = $state(false);
  let ascending = $state(false);
  let offset = $state(0);
  let loading = $state(false);
  let error = $state('');
  let feedback = $state('');
  let pendingSaves = $state<Record<number, boolean>>({});
  let restoreListFocus = false;
  let readerId = $state<number | null>(null);
  let readerRevisionId = $state<number | null>(null);
  let revision = $state(0);
  let listHeading = $state<HTMLHeadingElement>();
  let active = true;
  let requestId = 0;
  let pendingRecommendationFeedback = $state<Record<number, boolean>>({});
  const pageSize = 30;
  let recommendationRequest = 0;

  $effect(() => {
    const currentPage = page;
    void currentPage;
    readerId = null;
    readerRevisionId = null;
    lastOpened = 0;
    restoreListFocus = false;
    draft = '';
    query = '';
    source = '';
    category = '';
    unread = false;
    ascending = false;
    offset = 0;
  });

  $effect(() => {
    const target = activation;
    if (!target || !desktop) return;
    // Notification payloads use the stable document/revision/event identifiers,
    // never titles or transient list positions.
    lastOpened = target.document_id;
    readerRevisionId = target.revision_id;
    readerId = target.document_id;
    onactivationhandled();
  });
  $effect(() => {
    const filters = { query, source_id: source || null, category: category || null,
      saved_only: page === 'bookmark', unread_only: unread, ascending, offset, limit: pageSize };
    void revision;
    if (desktop) void fetchPage(filters);
  });

  $effect(() => {
    // App detects Tauri after mount; initial child mounting may see false.
    if (desktop) void refreshContext();
  });

  $effect(() => {
    const currentPage = page;
    const isDesktop = desktop;
    const archiveRevision = revision;
    if (currentPage === 'today' && isDesktop) {
      void archiveRevision;
      void loadRecommendations();
    }
  });

  onMount(() => {
    let wasRunning = false;
    const timer = window.setInterval(() => {
      if (!desktop) return;
      void invoke<SyncStatus>('archive_status').then((value) => {
        if (!active) return;
        status = value;
        if (wasRunning && !value.running) { revision++; void refreshContext(); }
        wasRunning = value.running;
      }).catch((reason) => { if (active) error = archiveError(reason); });
    }, 2000);
    return () => { active = false; requestId++; window.clearInterval(timer); };
  });

  async function refreshContext() {
    try {
      const [counts, catalog, sync] = await Promise.all([
        invoke<ArchiveSummary>('archive_summary'), invoke<SourceInfo[]>('archive_sources'), invoke<SyncStatus>('archive_status'),
      ]);
      if (active) { summary = counts; sources = catalog; status = sync; }
    } catch (reason) { if (active) error = archiveError(reason); }
    try {
      const analyses = await invoke<AiResult[]>('ai_recent');
      if (active) { recentAnalyses = analyses; aiError = ''; }
    } catch (reason) { if (active) aiError = archiveError(reason); }
    try {
      const snapshot = await invoke<AiSnapshot>('ai_snapshot');
      if (active) aiSnapshot = snapshot;
    } catch { if (active) aiSnapshot = null; }
  }
  function currentSummary(documentId: number): AiResult | null {
    const analysis = recentAnalyses.find((value) => value.document_id === documentId);
    const contract = aiSnapshot?.analysis_contract;
    if (!analysis || !aiSnapshot || !contract || !contract.configuration_hash) return null;
    if (contract.prompt_version !== analysisContract.prompt_version || contract.schema_version !== analysisContract.schema_version) return null;
    if (analysis.prompt_version !== contract.prompt_version
      || analysis.schema_version !== contract.schema_version
      || analysis.configuration_hash !== contract.configuration_hash) return null;
    const activeModel = aiSnapshot.models.find((model) => model.id === aiSnapshot?.preferences.active_model_id);
    if (activeModel?.sha256 && analysis.model_sha256 !== activeModel.sha256) return null;
    if (analysis.model_identity.max_tokens !== undefined
      && analysis.model_identity.max_tokens !== aiSnapshot.preferences.max_tokens) return null;
    return analysis;
  }
  async function loadRecommendations() {
    const request = ++recommendationRequest;
    recommendationError = '';
    try {
      const result = await invoke<PersonalizationRecommendations>('personalization_recommendations', { limit: 8 });
      if (!active || request !== recommendationRequest) return;
      recommendations = result.items;
      hasPreferences = result.has_preferences;
    } catch (reason) {
      if (active && request === recommendationRequest) {
        recommendations = [];
        hasPreferences = false;
        recommendationError = archiveError(reason);
      }
    }
  }

  async function setRecommendationFeedback(recommendation: PersonalizedRecommendation, feedbackValue: string | null) {
    const id = recommendation.item.id;
    if (pendingRecommendationFeedback[id]) return;
    pendingRecommendationFeedback = { ...pendingRecommendationFeedback, [id]: true };
    recommendationError = '';
    try {
      const feedback = await invoke<string | null>('personalization_set_feedback', { documentId: id, feedback: feedbackValue });
      recommendations = recommendations.map((item) => item.item.id === id ? { ...item, feedback } : item);
      await loadRecommendations();
    } catch (reason) {
      recommendationError = archiveError(reason);
    } finally {
      pendingRecommendationFeedback = { ...pendingRecommendationFeedback, [id]: false };
    }
  }

  async function fetchPage(filters: { query: string; source_id: string | null; category: string | null; saved_only: boolean; unread_only: boolean; ascending: boolean; offset: number; limit: number }) {
    const current = ++requestId;
    loading = true;
    error = '';
    let clamped = false;
    try {
      const next = await invoke<ArchivePage>('archive_query', { query: filters });
      if (!active || current !== requestId) return;
      // Saving or marking read can remove the final item of a filtered page.
      // Re-query the last valid page rather than presenting a false empty archive.
      const lastOffset = Math.max(0, Math.floor((next.total - 1) / pageSize) * pageSize);
      if (filters.offset > lastOffset) {
        clamped = true;
        offset = lastOffset;
        return;
      }
      result = next;
      if (category && !next.categories.includes(category)) category = '';
    } catch (reason) { if (active && current === requestId) error = archiveError(reason); }
    finally {
      if (active && current === requestId && !clamped) {
        loading = false;
        if (restoreListFocus) void focusList();
      }
    }
  }

  function search(event: SubmitEvent) {
    event.preventDefault();
    query = draft.trim();
    offset = 0;
  }

  async function save(item: DocumentItem) {
    if (pendingSaves[item.id]) return;
    pendingSaves = { ...pendingSaves, [item.id]: true };
    error = '';
    feedback = '';
    const saved = !item.saved;
    try {
      await invoke('archive_state', { id: item.id, isRead: null, saved });
      if (!active) return;
      item.saved = saved;
      feedback = saved ? '보관함에 저장했습니다.' : '보관함에서 해제했습니다.';
      revision++;
      await refreshContext();
    } catch (reason) { if (active) error = archiveError(reason); }
    finally { if (active) pendingSaves = { ...pendingSaves, [item.id]: false }; }
  }

  function changed() {
    if (active) {
      // A reader's automatic state write can finish after returning to the list.
      // Reconcile focus again if that write removes the previously focused row.
      if (readerId === null && lastOpened > 0) restoreListFocus = true;
      revision++;
      void refreshContext();
    }
  }

  function closeReader() {
    readerId = null;
    readerRevisionId = null;
    changed();
  }
  async function focusList() {
    await tick();
    if (!active || readerId !== null || loading) return;
    restoreListFocus = false;
    const target = document.getElementById(`notice-${lastOpened}`) ?? listHeading;
    target?.focus({ preventScroll: true });
    target?.scrollIntoView({ block: 'nearest', behavior: 'instant' });
  }
  let lastOpened = 0;
  async function open(item: DocumentItem) {
    lastOpened = item.id;
    readerRevisionId = null;
    readerId = item.id;
    await tick();
    window.scrollTo({ top: 0, behavior: 'instant' });
  }

  async function paginate(delta: number) {
    offset = Math.max(0, offset + delta * pageSize);
    await tick();
    listHeading?.focus({ preventScroll: true });
  }
</script>

{#if readerId !== null}
  {#key `${readerId}:${readerRevisionId ?? ''}`}<DocumentReader id={readerId} initialRevisionId={readerRevisionId} {enabled} onclose={closeReader} onchange={changed} {onaisettings} />{/key}
{:else}
  <section class="archive-view" aria-busy={loading}>
    {#if page === 'today'}
      <div class="archive-intro"><span class="section-kicker">내 기기에 쌓이는 학교 소식</span><h2>오늘도, 필요한 소식부터.</h2><p>직접 보관한 공지와 이미 분석한 핵심을 차분히 살펴보세요. {hasPreferences ? '관심사와 보관 행동을 바탕으로 정렬한 추천입니다.' : '아직 개인화 선호가 없어 최근 공지부터 보여드립니다.'}</p></div>
      <dl class="archive-statistics">
        <div><dt>보관된 공지</dt><dd>{summary?.documents ?? '—'}</dd></div>
        <div><dt>읽지 않은 공지</dt><dd>{summary?.unread ?? '—'}</dd></div>
        <div><dt>보관함</dt><dd>{summary?.saved ?? '—'}</dd></div>
        <div><dt>최근 동기화의 새 공지</dt><dd>{summary?.new_since_sync ?? '—'}</dd></div>
      </dl>
      <p class="footnote">과거 자료 가져오기는 새 공지 수에서 제외됩니다. 저장된 자료 {archiveSize(summary?.resource_bytes ?? 0)}.</p>
      {#if recommendationError}<p role="alert" class="archive-error">{recommendationError}</p>{/if}
      {#if recommendations.length}
        <section class="ai-recent" aria-label="오늘의 추천">
          <div class="collection-heading">
            <h2>나를 위한 소식</h2>
            <span class="muted">{hasPreferences ? '관심 설정과 저장·피드백을 반영한 추천' : '최근 공지 · 개인화 시작 전'}</span>
            {#if !hasPreferences}<button class="text-button" onclick={onsettings}>관심 분야 설정</button>{/if}
          </div>
          <ul>
            {#each recommendations as recommendation (recommendation.item.id)}
              {@const analysis = currentSummary(recommendation.item.id)}
              <li>
                <button class="notice-open" onclick={() => void open(recommendation.item)}>
                  <strong>{recommendation.item.title}</strong>
                  <span class="notice-meta">
                    {recommendation.item.source_name} · {archiveDate(recommendation.item.published_at)}
                    {#if recommendation.item.category} · {recommendation.item.category}{/if}
                    · {recommendation.item.is_read ? '읽음' : '읽지 않음'} · {recommendation.item.saved ? '보관함에 저장' : '미저장'}
                  </span>
                  {#each recommendation.reasons as reason}<span class="notice-meta">{reason}</span>{/each}
                  {#if analysis}
                    <span class="notice-ai-preview">저장된 AI 요약{analysis.partial ? ' · 일부 분석' : ''} · {analysis.result.short_summary}</span>
                  {/if}
                  {#if recommendation.feedback}
                    <span class="notice-meta">내 피드백: {recommendation.feedback === 'interested' ? '관심 있음' : '관심 없음'}</span>
                  {/if}
                </button>
                <div class="recommendation-actions" role="group" aria-label={`${recommendation.item.title} 추천 피드백`}>
                  <button class="button secondary small" aria-pressed={recommendation.feedback === 'interested'} disabled={pendingRecommendationFeedback[recommendation.item.id]} onclick={() => void setRecommendationFeedback(recommendation, recommendation.feedback === 'interested' ? null : 'interested')}>관심 있어요</button>
                  <button class="button secondary small" aria-pressed={recommendation.feedback === 'not_interested'} disabled={pendingRecommendationFeedback[recommendation.item.id]} onclick={() => void setRecommendationFeedback(recommendation, recommendation.feedback === 'not_interested' ? null : 'not_interested')}>관심 없어요</button>
                </div>
              </li>
            {/each}
          </ul>
        </section>
      {:else if !recommendationError}
        <div class="recommendation-empty">
          <p>{enabled ? hasPreferences ? '관심 분야와 맞는 소식을 아직 찾지 못했습니다.' : '개인화 전에는 최근 공지를 표시합니다. 관심사를 설정하면 추천에 반영됩니다.' : '확장 기능을 끄면 새 추천은 표시하지 않습니다. 기존 공지는 받은 소식과 탐색에서 계속 볼 수 있습니다.'}</p>
          {#if !hasPreferences}<button class="text-button" onclick={onsettings}>관심 분야 설정</button>{/if}
        </div>
      {/if}
      {#if recentAnalyses.length}
        <section class="ai-recent" aria-label="최근 분석한 소식">
          <div class="collection-heading"><h2>최근 분석한 소식</h2><span class="muted">저장된 AI 결과 · 추천 순위에는 영향 없음</span></div>
          <ul>
            {#each recentAnalyses.slice(0, 5) as analysis}
              {@const currentContract = currentSummary(analysis.document_id)?.id === analysis.id}
              <li>
                <button class="notice-open" onclick={() => { lastOpened = analysis.document_id; readerId = analysis.document_id; }}>
                  <span class="ai-tag">AI 분류 · {analysis.result.category} · {currentContract ? '현재 계약' : '이전 계약'}</span>
                  <strong>{analysis.result.short_summary}</strong>
                  {#if !currentContract}<span class="notice-meta">현재 분석 설정과 다를 수 있습니다 · 원문과 근거 확인</span>{/if}
                  {#if analysis.partial}<span class="muted">일부 내용 분석</span>{/if}
                  {#if currentContract}
                    {#each analysis.result.facts.filter((fact) => fact.date).slice(0, 2) as fact}
                      <span class="notice-meta">{fact.label} · {fact.date} · 원문에서 추출한 참고 일정</span>
                    {/each}
                  {/if}
                  <span class="notice-meta">{analysis.model_identity.name ?? '로컬 모델'} · 원문과 근거 확인 →</span>
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {:else if aiError}
        <p class="footnote">저장된 AI 결과를 불러오지 못했습니다. 공지 목록은 계속 사용할 수 있습니다.</p>
      {/if}
    {/if}
    {#if status?.running}<div class="archive-sync-strip" role="status"><Icon name="refresh" size={16} /><span>{status.message}{#if status.page > 0} · {status.page}쪽{/if}</span><button class="text-button" onclick={onsettings}>설정에서 진행 확인</button></div>{/if}
    {#if page === 'search'}
      <form class="search-form archive-search" onsubmit={search} role="search"><Icon name="search" size={21} /><label class="sr-only" for="local-query">보관된 공지 검색</label><input id="local-query" type="search" bind:value={draft} maxlength="512" placeholder="제목과 본문에서 검색" autocomplete="off" disabled={!desktop} /><button class="text-button" type="submit" disabled={!desktop || loading}>검색</button></form>
      <p class="footnote">한국어 일부 단어, 영문, 숫자를 검색할 수 있습니다. 띄어 쓴 검색어는 모두 포함된 공지를 찾습니다. 검색어는 외부로 보내지 않습니다.</p>
    {/if}
    <div class="archive-toolbar">
      <div class="archive-filters">
        <label>정보원<select bind:value={source} onchange={() => { category = ''; offset = 0; }} disabled={!desktop}><option value="">모든 정보원</option>{#each sources as item}<option value={item.id}>{item.organization} · {item.name}</option>{/each}</select></label>
        <label>분류<select bind:value={category} onchange={() => offset = 0} disabled={!desktop}><option value="">모든 분류</option>{#each result.categories as value}<option value={value}>{value}</option>{/each}</select></label>
        <label>정렬<select bind:value={ascending} onchange={() => offset = 0} disabled={!desktop}><option value={false}>게시일 최신순</option><option value={true}>게시일 오래된순</option></select></label>
      </div>
      {#if page === 'inbox'}<label class="archive-checkbox"><input type="checkbox" bind:checked={unread} onchange={() => offset = 0} disabled={!desktop} /> 읽지 않은 공지만</label>{/if}
    </div>
    <div class="collection-heading archive-list-heading"><h2 bind:this={listHeading} tabindex="-1">{page === 'today' ? '최근 공지' : page === 'bookmark' ? '저장한 공지' : query ? '검색 결과' : '보관된 공지'} <span class="muted">{result.total.toLocaleString('ko-KR')}</span></h2><button class="text-button" disabled={!desktop || loading} onclick={() => { revision++; void refreshContext(); }}>목록 새로고침</button></div>
    {#if error}<div class="archive-error" role="alert">{error}<button class="text-button" onclick={() => { revision++; void refreshContext(); }}>다시 시도</button></div>{/if}
    <p class="sr-only archive-result-status" role="status">{loading ? '로컬 공지를 불러오는 중입니다.' : error ? '목록을 새로 불러오지 못했습니다.' : `공지 ${result.total.toLocaleString('ko-KR')}개${result.total ? ` · ${offset + 1}–${Math.min(offset + pageSize, result.total)}` : ''}`}</p>
    <p class="archive-feedback" role="status">{feedback}</p>
    {#if loading}<p class="loading" role="status">로컬 공지를 불러오는 중입니다…</p>{/if}
    {#if result.items.length}
      <ul class="notice-list" aria-label="공지 목록">
        {#each result.items as item (item.id)}
          {@const analysis = recentAnalyses.find((value) => value.document_id === item.id)}
          <li class:notice-unread={!item.is_read}>
            <button class="notice-open" id={`notice-${item.id}`} onclick={() => open(item)}>
              <span class="notice-title"><span class="notice-read-dot" aria-hidden="true"></span><Highlighted text={item.title} {query} /><span class="sr-only">{item.is_read ? '읽음' : '읽지 않음'}</span></span>
              <span class="notice-meta"><span>{item.source_name}</span><time datetime={item.published_at}>{archiveDate(item.published_at)}</time>{#if item.category}<span>{item.category}</span>{/if}{#if item.historical}<span class="notice-history">과거 자료</span>{/if}</span>
              {#if analysis && page !== 'search'}<span class="notice-ai-preview"><span class="ai-tag">AI · {analysis.result.category}</span> {analysis.result.short_summary}</span>{/if}
              {#if page === 'search' && item.snippet}<span class="notice-snippet"><Highlighted text={item.snippet} {query} /></span>{/if}
              {#if item.content_status === 'missing_resources'}<span class="notice-resource-warning">일부 첨부 자료 미저장</span>{/if}
            </button>
            <button class="notice-save icon-button" aria-label={`${item.title} ${item.saved ? '저장 해제' : '보관함에 저장'}`} aria-pressed={item.saved} class:is-saved={item.saved} disabled={pendingSaves[item.id]} onclick={() => save(item)}><Icon name="bookmark" size={19} /></button>
          </li>
        {/each}
      </ul>
      <nav class="archive-pagination" aria-label="공지 목록 페이지"><button class="button secondary small" disabled={offset === 0 || loading} onclick={() => paginate(-1)}>이전</button><span aria-live="polite">{offset + 1}–{Math.min(offset + pageSize, result.total)} / {result.total}</span><button class="button secondary small" disabled={offset + pageSize >= result.total || loading} onclick={() => paginate(1)}>다음</button></nav>
    {:else if !loading && !error}
      <div class="empty-state archive-empty"><span class="empty-icon"><Icon name={page === 'bookmark' ? 'bookmark' : page === 'search' ? 'search' : 'inbox'} size={30} /></span><h2>{query || source || category || unread ? '조건에 맞는 공지가 없습니다' : page === 'bookmark' ? '아직 저장한 공지가 없습니다' : '아직 보관된 공지가 없습니다'}</h2><p>{query || source || category || unread ? '검색어 또는 필터를 바꿔 보세요. 이미 보관한 내용만 검색하며 인터넷 연결은 필요하지 않습니다.' : page === 'bookmark' ? '공지 옆의 저장 버튼으로 중요한 정보를 모아 보세요. 확장 기능을 꺼도 보관함은 유지됩니다.' : '설정에서 정보원을 구독하고 공개된 공지를 가져오세요. 한 번 보관한 본문은 오프라인에서도 읽을 수 있습니다.'}</p>{#if page !== 'bookmark' && !query}<button class="button secondary" onclick={onsettings}>정보원 설정하기 <Icon name="arrow" size={16} /></button>{/if}</div>
    {/if}
  </section>
{/if}
