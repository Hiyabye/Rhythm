<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { AiSnapshot, ArchivePage, ArchiveSummary, AiResult, AiClassification, DocumentItem, DuplicateNotices, SimilarNotice, PersonalizedRecommendation, PersonalizationRecommendations, SourceInfo, SyncStatus } from './types';
  import analysisContract from './analysis-contract.json' with { type: 'json' };
  import { archiveDate, archiveDay, archiveError, localDayKey } from './archive-format';
  import DocumentReader from './DocumentReader.svelte';
  import Highlighted from './Highlighted.svelte';
  import Icon from './Icon.svelte';

  let { page, desktop, enabled, sourceRequest, activation, onactivationhandled, onsettings, onaisettings, onsourcechange, oncontext, onweekpane }: {
    page: string; desktop: boolean; enabled: boolean; sourceRequest: { id: string; token: number };
    activation: { document_id: number; revision_id: number; event_id: number } | null;
    onactivationhandled: () => void; onsettings: (pane?: 'boards' | 'personalization') => void; onaisettings: () => void; onsourcechange: (id: string) => void;
    oncontext: (context: { summary: ArchiveSummary | null; sources: SourceInfo[] | null; status: SyncStatus | null }) => void;
    onweekpane: (visible: boolean) => void;
  } = $props();
  let result = $state<ArchivePage>({ items: [], total: 0, categories: [] });
  let summary = $state<ArchiveSummary | null>(null);
  let sources = $state<SourceInfo[]>([]);
  let recentAnalyses = $state<AiResult[]>([]);
  let classifications = $state<Record<number, AiClassification>>({});
  let duplicates = $state<Record<number, SimilarNotice[]>>({});
  let expandedDuplicates = $state<Record<number, boolean>>({});
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
    const request = sourceRequest;
    void currentPage;
    void request.token;
    readerId = null;
    expandedDuplicates = {};
    readerRevisionId = null;
    lastOpened = 0;
    restoreListFocus = false;
    draft = '';
    query = '';
    source = request.id;
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

  // The week agenda is a trailing window pane, so the shell reserves its width while it is shown.
  $effect(() => {
    onweekpane(page === 'today' && readerId === null);
    return () => onweekpane(false);
  });

  $effect(() => {
    // App detects Tauri after mount; initial child mounting may see false.
    if (desktop) void refreshContext();
  });

  let lastClassificationProcessed = 0;
  $effect(() => {
    // The background classifier changes categories without a page navigation.
    const processed = aiSnapshot?.classification?.processed ?? 0;
    if (processed !== lastClassificationProcessed) {
      lastClassificationProcessed = processed;
      if (result.items.length) void loadEmbeddingLabels(result.items.map((item) => item.id));
    }
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
        oncontext({ summary: null, sources: null, status: value });
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
      if (active) { summary = counts; sources = catalog; status = sync; oncontext({ summary: counts, sources: catalog, status: sync }); }
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
      const rows = await invoke<AiClassification[]>('ai_classifications', { documentIds: result.items.map((item) => item.item.id) });
      if (!active || request !== recommendationRequest) return;
      classifications = { ...classifications, ...Object.fromEntries(rows.map((row) => [row.document_id, row])) };
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

  async function loadEmbeddingLabels(ids: number[]) {
    if (!ids.length) { classifications = {}; duplicates = {}; return; }
    try {
      const rows = await invoke<AiClassification[]>('ai_classifications', { documentIds: ids });
      classifications = Object.fromEntries(rows.map((row) => [row.document_id, row]));
    } catch { classifications = {}; }
    try {
      const rows = await invoke<DuplicateNotices[]>('ai_duplicate_notices', { documentIds: ids });
      duplicates = Object.fromEntries(rows.map((row) => [row.document_id, row.duplicates]));
    } catch { duplicates = {}; }
  }

  // Cross-posts on the same page fold under their first row. They are never
  // dropped: the leader names the other boards and expands to show the copies.
  const listRows = $derived.by(() => {
    const byId = new Map(result.items.map((item) => [item.id, item]));
    const placed = new Set<number>();
    const rows: { item: DocumentItem; folded: DocumentItem[]; member: boolean }[] = [];
    for (const item of result.items) {
      if (placed.has(item.id)) continue;
      placed.add(item.id);
      const folded = (duplicates[item.id] ?? []).flatMap((notice) => {
        const copy = byId.get(notice.document_id);
        if (!copy || placed.has(copy.id)) return [];
        placed.add(copy.id);
        return [copy];
      });
      rows.push({ item, folded, member: false });
      if (expandedDuplicates[item.id]) for (const copy of folded) rows.push({ item: copy, folded: [], member: true });
    }
    return rows;
  });
  function duplicateBoards(item: DocumentItem) {
    return [...new Set((duplicates[item.id] ?? []).map((notice) => notice.source_name))].join(', ');
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
      await loadEmbeddingLabels(next.items.map((item) => item.id));
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
    const target = (lastOpenedTarget && document.getElementById(lastOpenedTarget)) || document.getElementById(`notice-${lastOpened}`) || listHeading;
    target?.focus({ preventScroll: true });
    target?.scrollIntoView({ block: 'nearest', behavior: 'instant' });
  }
  let lastOpened = 0;
  let lastOpenedTarget = '';
  async function open(item: DocumentItem, target = `notice-${item.id}`) {
    await openDocument(item.id, target);
  }
  async function openDocument(id: number, target: string) {
    lastOpened = id;
    lastOpenedTarget = target;
    readerRevisionId = null;
    readerId = id;
    await tick();
    window.scrollTo({ top: 0, behavior: 'instant' });
  }

  const weekdayLabels = ['월', '화', '수', '목', '금', '토', '일'];
  const weekDayLimit = 5;
  const todayKey = localDayKey(new Date());
  function familyOf(sourceId: string): string {
    return sources.find((item) => item.id === sourceId)?.family ?? 'other';
  }
  function titleOf(documentId: number, fallback: string): string {
    return result.items.find((item) => item.id === documentId)?.title
      ?? recommendations.find((item) => item.item.id === documentId)?.item.title ?? fallback;
  }
  // Only dates from analyses that match the current contract appear, exactly as in the recent-analysis list.
  let referenceDates = $derived(recentAnalyses.flatMap((analysis) => currentSummary(analysis.document_id)?.id !== analysis.id ? [] :
    analysis.result.facts.filter((fact) => fact.date && /^\d{4}-\d{2}-\d{2}/.test(fact.date)).map((fact, index) => ({
      key: `${analysis.id}-${index}`, day: fact.date!.slice(0, 10), label: fact.label, documentId: analysis.document_id,
      title: titleOf(analysis.document_id, analysis.result.short_summary),
    }))));
  let week = $derived.by(() => {
    const now = new Date();
    const monday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - (now.getDay() + 6) % 7);
    return weekdayLabels.map((label, index) => {
      const day = new Date(monday.getFullYear(), monday.getMonth(), monday.getDate() + index);
      const key = localDayKey(day);
      return { key, label, month: day.getMonth() + 1, date: day.getDate(),
        notices: result.items.filter((item) => archiveDay(item.published_at) === key),
        dates: referenceDates.filter((entry) => entry.day === key) };
    });
  });
  let laterDates = $derived(referenceDates.filter((entry) => entry.day > week[6].key).sort((a, b) => a.day.localeCompare(b.day)).slice(0, 5));
  // The Today query holds the newest page only; say so when this week may extend past it.
  let weekTruncated = $derived(result.items.length === pageSize && (archiveDay(result.items[pageSize - 1].published_at) ?? '') >= week[0].key);
  function shortDay(day: string): string {
    return `${Number(day.slice(5, 7))}월 ${Number(day.slice(8, 10))}일`;
  }

  async function paginate(delta: number) {
    offset = Math.max(0, offset + delta * pageSize);
    await tick();
    listHeading?.focus({ preventScroll: true });
  }
</script>

{#if readerId !== null}
  {#key `${readerId}:${readerRevisionId ?? ''}`}<DocumentReader id={readerId} initialRevisionId={readerRevisionId} {enabled} onclose={closeReader} onchange={changed} {onaisettings} onopen={(id) => void openDocument(id, `notice-${id}`)} />{/key}
{:else}
  <section class="archive-view" aria-busy={loading}>
    {#if page === 'today'}
      <div class="today-layout">
        <div class="today-main">
          {#if desktop && summary?.documents === 0}
            <section class="first-run" aria-labelledby="first-run-heading">
              <h2 id="first-run-heading">Rhythm 시작하기</h2>
              <p>Rhythm은 서강대학교의 공개 공지를 이 기기에 보관해 인터넷 없이도 읽고 검색할 수 있게 합니다. 계정이나 클라우드 없이 모든 자료와 설정은 이 기기에만 저장됩니다.</p>
              <ol>
                <li>설정 → 확장 기능에서 서강대학교 공지를 켜고 게시판을 구독합니다. 원문 수집을 확인한 게시판만 구독할 수 있습니다.</li>
                <li>새로고침을 누르면 새 소식과 아직 받지 않은 이전 공지를 차례로 가져옵니다. 직접 시작하거나 자동 동기화를 켠 경우에만 네트워크를 사용합니다.</li>
                <li>로컬 AI 모델은 선택 사항입니다. 모델 없이도 읽기, 검색과 추천을 사용할 수 있습니다.</li>
                <li>백그라운드 실행, 자동 동기화와 알림은 처음에 모두 꺼져 있습니다. 켜더라도 Rhythm이 실행 중이고 기기가 깨어 있을 때만 동작합니다.</li>
              </ol>
              <p>데이터 저장 위치는 설정 → 데이터 및 개인정보에서 확인할 수 있습니다.</p>
              <button class="button" onclick={() => onsettings()}>설정에서 시작하기</button>
            </section>
          {/if}
          {#if error}<div class="archive-error" role="alert">{error}<button class="text-button" onclick={() => { revision++; void refreshContext(); }}>다시 시도</button></div>{/if}
          {#if recommendationError}<p role="alert" class="archive-error">{recommendationError}</p>{/if}
          {#if recommendations.length}
            <section class="today-picks" aria-label="오늘의 추천">
              <div class="pane-heading">
                <h2 bind:this={listHeading} tabindex="-1">나를 위한 소식</h2>
                <span class="pane-note">{hasPreferences ? '관심 설정과 저장·피드백을 반영한 추천' : '최근 공지 · 개인화 시작 전'}</span>
                {#if !hasPreferences}<button class="text-button" onclick={() => onsettings('personalization')}>관심 분야 설정</button>{/if}
              </div>
              <ul class="pick-list">
                {#each recommendations as recommendation (recommendation.item.id)}
                  {@const item = recommendation.item}
                  {@const analysis = currentSummary(item.id)}
                  <li class:notice-unread={!item.is_read}>
                    <button class="pick-open" id={`notice-${item.id}`} onclick={() => void open(item)}>
                      <span class="pick-title"><span class="notice-read-dot" aria-hidden="true"></span><strong>{item.title}</strong><span class="sr-only">{item.is_read ? '읽음' : '읽지 않음'}</span></span>
                      <span class="pick-meta"><span class={`board-swatch family-${familyOf(item.source_id)}`} aria-hidden="true"></span>{item.source_name} · {archiveDate(item.published_at)}{#if item.category}{' · '}{item.category}{/if}{#if item.saved}{' · '}보관함에 저장됨{/if}</span>
                      {#if recommendation.reasons.length}<span class="pick-reasons">{#each recommendation.reasons as reason, index}{#if index}{' · '}{/if}<span>{reason}</span>{/each}</span>{/if}
                      {#if classifications[item.id] || analysis}<span class="notice-ai-preview">{#if classifications[item.id]}<span class="ai-tag">분류 · {classifications[item.id].category}{classifications[item.id].low_confidence ? ' · 확신 낮음' : ''}</span>{/if}{#if analysis}{#if classifications[item.id]}{' · '}{/if}<span class="ai-tag">저장된 AI 요약{analysis.partial ? ' · 일부 분석' : ''}</span> {analysis.result.short_summary}{/if}</span>{/if}
                    </button>
                    <div class="recommendation-actions" role="group" aria-label={`${item.title} 추천 피드백`}>
                      <button class="chip" aria-pressed={recommendation.feedback === 'interested'} disabled={pendingRecommendationFeedback[item.id]} onclick={() => void setRecommendationFeedback(recommendation, recommendation.feedback === 'interested' ? null : 'interested')}>관심 있어요</button>
                      <button class="chip" aria-pressed={recommendation.feedback === 'not_interested'} disabled={pendingRecommendationFeedback[item.id]} onclick={() => void setRecommendationFeedback(recommendation, recommendation.feedback === 'not_interested' ? null : 'not_interested')}>관심 없어요</button>
                    </div>
                  </li>
                {/each}
              </ul>
            </section>
          {:else if !recommendationError}
            <div class="recommendation-empty">
              <h2 bind:this={listHeading} tabindex="-1">나를 위한 소식</h2>
              <p>{enabled ? hasPreferences ? '관심 분야와 맞는 소식을 아직 찾지 못했습니다.' : '개인화 전에는 최근 공지를 표시합니다. 관심사를 설정하면 추천에 반영됩니다.' : '확장 기능을 끄면 새 추천은 표시하지 않습니다. 기존 공지는 받은 소식과 탐색에서 계속 볼 수 있습니다.'}</p>
              {#if !hasPreferences}<button class="text-button" onclick={() => onsettings('personalization')}>관심 분야 설정</button>{/if}
            </div>
          {/if}
          {#if recentAnalyses.length}
            <section class="today-analyses" aria-label="최근 분석한 소식">
              <div class="pane-heading"><h2>최근 분석한 소식</h2><span class="pane-note">저장된 AI 결과 · 추천 순위에는 영향 없음</span></div>
              <ul class="pick-list">
                {#each recentAnalyses.slice(0, 5) as analysis}
                  {@const currentContract = currentSummary(analysis.document_id)?.id === analysis.id}
                  <li>
                    <button class="pick-open" id={`analysis-${analysis.id}`} onclick={() => void openDocument(analysis.document_id, `analysis-${analysis.id}`)}>
                      <strong class="pick-summary">{analysis.result.short_summary}</strong>
                      <span class="pick-meta">저장된 AI 분석{#if !currentContract}{' · '}현재 분석 설정과 다를 수 있습니다{/if}</span>
                      {#if analysis.partial}<span class="pick-meta">일부 내용 분석</span>{/if}
                      {#if currentContract}
                        {#each analysis.result.facts.filter((fact) => fact.date).slice(0, 2) as fact}
                          <span class="pick-meta">{fact.label} · {fact.date} · 원문에서 추출한 참고 일정</span>
                        {/each}
                      {/if}
                      <span class="pick-meta">{analysis.model_identity.name ?? '로컬 모델'} · 원문과 근거 확인</span>
                    </button>
                  </li>
                {/each}
              </ul>
            </section>
          {:else if aiError}
            <p class="footnote">저장된 AI 결과를 불러오지 못했습니다. 공지 목록은 계속 사용할 수 있습니다.</p>
          {/if}
        </div>

        <aside class="week-pane" aria-labelledby="week-heading">
          <div class="pane-heading"><h2 id="week-heading">이번 주</h2><span class="pane-note">{shortDay(week[0].key)} – {week[6].month === week[0].month ? `${week[6].date}일` : shortDay(week[6].key)}</span></div>
          {#if loading && !result.items.length}<p class="loading" role="status">이번 주 공지를 불러오는 중입니다…</p>{/if}
          <ol class="week-days">
            {#each week as day (day.key)}
              <li class="week-day" class:is-today={day.key === todayKey} class:is-past={day.key < todayKey}>
                <div class="day-label"><span class="day-name">{day.label}</span><span class="day-number">{day.date}</span>{#if day.key === todayKey}<span class="sr-only">오늘</span>{/if}</div>
                <ul class="day-events">
                  {#each day.notices.slice(0, weekDayLimit) as item (item.id)}
                    <li><button class="event" class:notice-unread={!item.is_read} id={`week-${item.id}`} onclick={() => void open(item, `week-${item.id}`)}>
                      <span class={`event-bar family-${familyOf(item.source_id)}`} aria-hidden="true"></span>
                      <span class="event-title">{item.title}</span>
                      <span class="event-meta">{item.source_name}</span>
                    </button></li>
                  {/each}
                  {#if day.notices.length > weekDayLimit}<li class="day-more">외 {day.notices.length - weekDayLimit}건 · 받은 소식에서 모두 보기</li>{/if}
                  {#each day.dates as entry (entry.key)}
                    <li><button class="event event-ai" id={`week-ai-${entry.key}`} onclick={() => void openDocument(entry.documentId, `week-ai-${entry.key}`)}>
                      <span class="event-bar" aria-hidden="true"></span>
                      <span class="event-title"><span class="ai-mark">AI 참고 일정</span> {entry.label}</span>
                      <span class="event-meta">{entry.title} · 원문 확인</span>
                    </button></li>
                  {/each}
                  {#if !day.notices.length && !day.dates.length}<li class="day-empty">공지 없음</li>{/if}
                </ul>
              </li>
            {/each}
          </ol>
          {#if weekTruncated}<p class="pane-footnote">이번 주 공지가 많아 최근 {pageSize}건만 표시합니다. 전체 목록은 받은 소식에서 볼 수 있습니다.</p>{/if}
          {#if laterDates.length}
            <h3 class="later-heading">이후 AI 참고 일정</h3>
            <ul class="day-events">
              {#each laterDates as entry (entry.key)}
                <li><button class="event event-ai" id={`later-ai-${entry.key}`} onclick={() => void openDocument(entry.documentId, `later-ai-${entry.key}`)}>
                  <span class="event-bar" aria-hidden="true"></span>
                  <span class="event-title"><span class="ai-mark">AI · {shortDay(entry.day)}</span> {entry.label}</span>
                  <span class="event-meta">{entry.title} · 원문 확인</span>
                </button></li>
              {/each}
            </ul>
          {/if}
          <p class="pane-footnote">게시일 기준 공지와 로컬 AI가 원문에서 추출한 참고 일정입니다. 참고 일정은 추천 순위와 알림에 쓰지 않으니 원문에서 확인하세요.</p>
        </aside>
      </div>
    {:else}
    {#if page === 'search'}
      <form class="search-form archive-search" onsubmit={search} role="search"><Icon name="search" size={21} /><label class="sr-only" for="local-query">보관된 공지 검색</label><input id="local-query" type="search" bind:value={draft} maxlength="512" placeholder="제목과 본문에서 검색" autocomplete="off" disabled={!desktop} /><button class="text-button" type="submit" disabled={!desktop || loading}>검색</button></form>
      <p class="footnote">한국어 일부 단어, 영문, 숫자를 검색할 수 있습니다. 띄어 쓴 검색어는 모두 포함된 공지를 찾습니다. 검색어는 외부로 보내지 않습니다.</p>
    {/if}
    <div class="archive-toolbar">
      <div class="archive-filters">
        <label>정보원<select bind:value={source} onchange={() => { category = ''; offset = 0; onsourcechange(source); }} disabled={!desktop}><option value="">모든 정보원</option>{#each sources as item}<option value={item.id}>{item.organization} · {item.name}</option>{/each}</select></label>
        <label>분류<select bind:value={category} onchange={() => offset = 0} disabled={!desktop}><option value="">모든 분류</option>{#each result.categories as value}<option value={value}>{value}</option>{/each}</select></label>
        <label>정렬<select bind:value={ascending} onchange={() => offset = 0} disabled={!desktop}><option value={false}>게시일 최신순</option><option value={true}>게시일 오래된순</option></select></label>
      </div>
      {#if page === 'inbox'}<label class="archive-checkbox"><input type="checkbox" bind:checked={unread} onchange={() => offset = 0} disabled={!desktop} /> 읽지 않은 공지만</label>{/if}
    </div>
    <div class="collection-heading archive-list-heading"><h2 bind:this={listHeading} tabindex="-1">{page === 'bookmark' ? '저장한 공지' : query ? '검색 결과' : '보관된 공지'} <span class="muted">{result.total.toLocaleString('ko-KR')}</span></h2><button class="text-button" disabled={!desktop || loading} onclick={() => { revision++; void refreshContext(); }}>목록 새로고침</button></div>
    {#if error}<div class="archive-error" role="alert">{error}<button class="text-button" onclick={() => { revision++; void refreshContext(); }}>다시 시도</button></div>{/if}
    <p class="sr-only archive-result-status" role="status">{loading ? '로컬 공지를 불러오는 중입니다.' : error ? '목록을 새로 불러오지 못했습니다.' : `공지 ${result.total.toLocaleString('ko-KR')}개${result.total ? ` · ${offset + 1}–${Math.min(offset + pageSize, result.total)}` : ''}`}</p>
    <p class="archive-feedback" role="status">{feedback}</p>
    {#if loading}<p class="loading" role="status">로컬 공지를 불러오는 중입니다…</p>{/if}
    {#if result.items.length}
      <ul class="notice-list" aria-label="공지 목록">
        {#each listRows as { item, folded, member } (item.id)}
          {@const analysis = recentAnalyses.find((value) => value.document_id === item.id)}
          <li class:notice-unread={!item.is_read} class:notice-duplicate-member={member} class:has-duplicates={folded.length > 0}>
            <button class="notice-open" id={`notice-${item.id}`} onclick={() => open(item)}>
              <span class="notice-title"><span class="notice-read-dot" aria-hidden="true"></span><Highlighted text={item.title} {query} /><span class="sr-only">{item.is_read ? '읽음' : '읽지 않음'}</span></span>
              <span class="notice-meta"><span>{item.source_name}</span><time datetime={item.published_at}>{archiveDate(item.published_at)}</time>{#if item.category}<span>{item.category}</span>{/if}</span>
              {#if (analysis || classifications[item.id]) && page !== 'search'}<span class="notice-ai-preview">{#if classifications[item.id]}<span class="ai-tag">분류 · {classifications[item.id].category}{classifications[item.id].low_confidence ? ' · 확신 낮음' : ''}</span>{/if}{#if analysis}{#if classifications[item.id]}{' · '}{/if}<span class="ai-tag">AI 요약</span> {analysis.result.short_summary}{/if}</span>{/if}
              {#if duplicates[item.id]?.length}<span class="notice-duplicate-note">같은 공지 {duplicates[item.id].length}건 · {duplicateBoards(item)}</span>{/if}
              {#if page === 'search' && item.snippet}<span class="notice-snippet"><Highlighted text={item.snippet} {query} /></span>{/if}
              {#if item.content_status === 'missing_resources'}<span class="notice-resource-warning">일부 첨부 자료 미저장</span>{/if}
            </button>
            <button class="notice-save icon-button" aria-label={`${item.title} ${item.saved ? '저장 해제' : '보관함에 저장'}`} class:is-saved={item.saved} disabled={pendingSaves[item.id]} onclick={() => save(item)}><Icon name="bookmark" size={19} /></button>
            {#if folded.length}<button class="text-button notice-duplicate-toggle" aria-expanded={Boolean(expandedDuplicates[item.id])} onclick={() => expandedDuplicates = { ...expandedDuplicates, [item.id]: !expandedDuplicates[item.id] }}>{expandedDuplicates[item.id] ? `같은 공지 ${folded.length}건 접기` : `같은 공지 ${folded.length}건 펼치기`}</button>{/if}
          </li>
        {/each}
      </ul>
      <nav class="archive-pagination" aria-label="공지 목록 페이지"><button class="button secondary small" disabled={offset === 0 || loading} onclick={() => paginate(-1)}>이전</button><span aria-live="polite">{offset + 1}–{Math.min(offset + pageSize, result.total)} / {result.total}</span><button class="button secondary small" disabled={offset + pageSize >= result.total || loading} onclick={() => paginate(1)}>다음</button></nav>
    {:else if !loading && !error}
      <div class="empty-state archive-empty"><span class="empty-icon"><Icon name={page === 'bookmark' ? 'bookmark' : page === 'search' ? 'search' : 'inbox'} size={30} /></span><h2>{query || source || category || unread ? '조건에 맞는 공지가 없습니다' : page === 'bookmark' ? '아직 저장한 공지가 없습니다' : '아직 보관된 공지가 없습니다'}</h2><p>{query || source || category || unread ? '검색어 또는 필터를 바꿔 보세요. 이미 보관한 내용만 검색하며 인터넷 연결은 필요하지 않습니다.' : page === 'bookmark' ? '공지 옆의 저장 버튼으로 중요한 정보를 모아 보세요. 확장 기능을 꺼도 보관함은 유지됩니다.' : '설정에서 정보원을 구독하고 공개된 공지를 가져오세요. 한 번 보관한 본문은 오프라인에서도 읽을 수 있습니다.'}</p>{#if page !== 'bookmark' && !query}<button class="button secondary" onclick={() => onsettings()}>정보원 설정하기 <Icon name="arrow" size={16} /></button>{/if}</div>
    {/if}
    {/if}
  </section>
{/if}
