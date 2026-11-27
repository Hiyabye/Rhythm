<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { AiSnapshot, ArchivePage, ArchiveQuery, ArchiveSummary, AiClassification, CalendarEvent, CalendarNoticeDates, CalendarTodo, CybercampusTask, NoticeDate, TodoPrefill, DocumentItem, DuplicateNotices, SimilarNotice, PersonalizedRecommendation, PersonalizationRecommendations, SourceGroup, SourceInfo, SyncStatus, FeedStatus } from './types';
  import { archiveDate, archiveDay, archiveError, calendarEventMeta, calendarEventOn, deadlineLabel, groupScope, kstInstant, localDayKey, noticeDateMark, noticeDateMeta, noticeDateOn, scopeGroup, wallTime } from './archive-format';
  import { clock } from './clock.svelte';
  import { confirmAction } from './confirm';
  import CalendarView from './CalendarView.svelte';
  import DocumentReader from './DocumentReader.svelte';
  import Highlighted from './Highlighted.svelte';
  import Icon from './Icon.svelte';
  import { fade, flip, slide } from './motion';

  let { page, desktop, enabled, sourceRequest, activation, onactivationhandled, onsettings, onsetup, onextensions, onsourcechange, oncontext, onweekpane, ontasks, calendarRequest, oncalendar }: {
    page: string; desktop: boolean; enabled: Set<string>; sourceRequest: { id: string; category: string; token: number };
    activation: { document_id: number; revision_id: number; event_id: number } | null;
    onactivationhandled: () => void; onsettings: (pane?: 'boards' | 'personalization') => void; onsetup: () => void; onextensions: () => void; onsourcechange: (id: string, category: string) => void;
    oncontext: (context: { summary: ArchiveSummary | null; sources: SourceInfo[] | null; groups: SourceGroup[] | null; status: SyncStatus | null }) => void;
    onweekpane: (visible: boolean) => void;
    ontasks: () => void;
    calendarRequest: { day: string; token: number; prefill: TodoPrefill | null };
    oncalendar: (day: string, prefill?: TodoPrefill) => void;
  } = $props();
  // Today's widgets and the AI annotations follow their extensions.
  let showRecommendations = $derived(enabled.has('recommendations'));
  let showWeek = $derived(enabled.has('week'));
  let calendarOn = $derived(enabled.has('calendar'));
  let ai = $derived(enabled.has('ai'));
  let sourcesOn = $derived(enabled.has('sogang_notices') || enabled.has('cybercampus'));
  // The feed mirrors every board; these pages follow subscriptions, while 검색, 보관함 and 휴지통 cover the whole archive.
  let feedPage = $derived(page === 'today' || page === 'inbox' || page === 'opportunities');
  let result = $state<ArchivePage>({ items: [], total: 0, categories: [] });
  let summary = $state<ArchiveSummary | null>(null);
  let sources = $state<SourceInfo[]>([]);
  let groups = $state<SourceGroup[]>([]);
  let classifications = $state<Record<number, AiClassification>>({});
  let duplicates = $state<Record<number, SimilarNotice[]>>({});
  let expandedDuplicates = $state<Record<number, boolean>>({});
  let aiSnapshot = $state<AiSnapshot | null>(null);
  let recommendations = $state<PersonalizedRecommendation[]>([]);
  let tasks = $state<CybercampusTask[]>([]);
  let weekTodos = $state<CalendarTodo[]>([]);
  // Dates found in notice text from today through two months ahead, for the week and what follows it.
  let weekDates = $state<NoticeDate[]>([]);
  let weekAllDeadlines = $state(false);
  // The academic calendar's events, empty while its layer is hidden.
  let weekEvents = $state<CalendarEvent[]>([]);
  let calendarDay = $state(localDayKey(new Date()));
  let calendarPrefill = $state<TodoPrefill | null>(null);
  // The row the calendar focuses after returning from the reader.
  let calendarFocus = $state('');
  let hasPreferences = $state(false);
  let recommendationError = $state('');
  let status = $state<SyncStatus | null>(null);
  // The notice feed's age; a stale or unreadable feed shows a banner above the list.
  let feed = $state<FeedStatus | null>(null);
  let feedNotice = $derived(enabled.has('sogang_notices') && feed && !feed.running && (feed.result === 'update_required' || feed.stale) ? feed : null);
  let draft = $state('');
  let query = $state('');
  let source = $state('');
  let category = $state('');
  // Local AI category; the chips show only with the AI extension and an installed model.
  let aiCategory = $state('');
  let unread = $state(false);
  // 기회: also list notices whose deadlines have all passed.
  let includeClosed = $state(false);
  // Relevance is offered only on the search page; it needs a query.
  let sort = $state<'newest' | 'oldest' | 'relevance'>('newest');
  let offset = $state(0);
  let loading = $state(false);
  let error = $state('');
  let feedback = $state('');
  let pendingSaves = $state<Record<number, boolean>>({});
  // Bulk selection covers the rows on the current page only.
  let selecting = $state(false);
  let chosen = $state<Set<number>>(new Set());
  let trashBusy = $state(false);
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
  let contextRequest = 0;
  let labelRequest = 0;

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
    category = request.category;
    aiCategory = '';
    unread = false;
    includeClosed = false;
    selecting = false;
    chosen = new Set();
    sort = currentPage === 'search' ? 'relevance' : 'newest';
    offset = 0;
    calendarDay = localDayKey(new Date());
    calendarFocus = '';
  });
  // The week pane opens the calendar on one day; this runs after the page change above.
  $effect(() => {
    if (calendarRequest.token && calendarRequest.day) {
      calendarDay = calendarRequest.day;
      calendarPrefill = calendarRequest.prefill;
    }
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
    const scopedGroup = scopeGroup(source);
    const filters = { query, source_id: scopedGroup ? null : source || null, group_id: scopedGroup, category: category || null,
      ai_category: ai ? aiCategory || null : null, saved_only: page === 'bookmark', unread_only: unread, subscribed_only: feedPage, trashed: page === 'trash', ascending: sort === 'oldest',
      opportunities: page === 'opportunities', include_closed: page === 'opportunities' && includeClosed, offset, limit: pageSize };
    const ranked = page === 'search' && sort === 'relevance' && query !== '';
    void revision;
    // The calendar shows no notice list; it uses only deadlines and AI dates.
    if (desktop && page !== 'calendar') void fetchPage(filters, ranked);
  });

  // The week agenda is a trailing window pane, so the shell reserves its width while it is shown.
  $effect(() => {
    onweekpane(page === 'today' && readerId === null && showWeek);
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
    if ((currentPage === 'today' || currentPage === 'calendar') && isDesktop) {
      void archiveRevision;
      if (currentPage === 'today' && showRecommendations) void loadRecommendations();
      if (currentPage === 'today' && showWeek && calendarOn) void loadWeekTodos();
      if (currentPage === 'today' && showWeek) void loadWeekDates();
      if (enabled.has('cybercampus')) void loadTasks();
    }
  });

  onMount(() => {
    let wasRunning = false;
    const timer = window.setInterval(() => {
      if (!desktop) return;
      void invoke<SyncStatus>('archive_status').then((value) => {
        if (!active) return;
        status = value;
        oncontext({ summary: null, sources: null, groups: null, status: value });
        if (wasRunning && !value.running) { revision++; void refreshContext(); }
        wasRunning = value.running;
      }).catch((reason) => { if (active) error = archiveError(reason); });
      if (!enabled.has('sogang_notices')) return;
      // A finished feed sync may have changed the list.
      void invoke<FeedStatus>('feed_status').then((value) => {
        if (!active) return;
        const finished = feed?.running && !value.running;
        feed = value;
        if (finished) { revision++; void refreshContext(); }
      }).catch(() => {});
    }, 2000);
    return () => { active = false; requestId++; window.clearInterval(timer); };
  });

  async function refreshContext() {
    const request = ++contextRequest;
    try {
      const [counts, catalog, tree, sync] = await Promise.all([
        invoke<ArchiveSummary>('archive_summary'), invoke<SourceInfo[]>('archive_sources'), invoke<SourceGroup[]>('archive_source_groups'), invoke<SyncStatus>('archive_status'),
      ]);
      // Saving, closing the reader and sync completion each refresh; an older answer must not overwrite a newer one.
      if (active && request === contextRequest) { summary = counts; sources = catalog; groups = tree; status = sync; oncontext({ summary: counts, sources: catalog, groups: tree, status: sync }); }
    } catch (reason) { if (active) error = archiveError(reason); }
    try {
      const snapshot = await invoke<AiSnapshot>('ai_snapshot');
      if (active && request === contextRequest) aiSnapshot = snapshot;
    } catch { if (active && request === contextRequest) aiSnapshot = null; }
  }
  // Stored AI output stays in SQLite while the AI extension is off; it is just not shown.
  let labels = $derived(ai ? classifications : {});
  let groupedCopies = $derived(ai ? duplicates : {});
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

  async function loadTasks() {
    try {
      const rows = await invoke<CybercampusTask[]>('cybercampus_tasks');
      if (active) tasks = rows;
    } catch {
      // Deadlines are a supplement to the week; the 과제·마감 page reports load failures.
      if (active) tasks = [];
    }
  }

  async function loadWeekTodos() {
    const from = localDayKey(new Date());
    const now = new Date();
    const to = localDayKey(new Date(now.getFullYear(), now.getMonth(), now.getDate() + 6));
    try {
      const rows = await invoke<CalendarTodo[]>('calendar_todos', { from, to });
      if (active) weekTodos = rows;
    } catch {
      // Todos are a supplement to the week; the calendar page reports load failures.
      if (active) weekTodos = [];
    }
  }

  async function loadWeekDates() {
    const now = new Date();
    const from = localDayKey(now);
    const to = localDayKey(new Date(now.getFullYear(), now.getMonth(), now.getDate() + 62));
    try {
      const value = await invoke<CalendarNoticeDates>('calendar_notice_dates', { from, to });
      if (active) { weekDates = value.dates; weekAllDeadlines = value.all_deadlines; weekEvents = value.events; }
    } catch {
      // Notice dates are a supplement to the week; the calendar page reports load failures.
      if (active) { weekDates = []; weekEvents = []; }
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
    const request = ++labelRequest;
    if (!ids.length) { classifications = {}; duplicates = {}; return; }
    try {
      const rows = await invoke<AiClassification[]>('ai_classifications', { documentIds: ids });
      if (request === labelRequest) classifications = Object.fromEntries(rows.map((row) => [row.document_id, row]));
    } catch { if (request === labelRequest) classifications = {}; }
    try {
      const rows = await invoke<DuplicateNotices[]>('ai_duplicate_notices', { documentIds: ids });
      if (request === labelRequest) duplicates = Object.fromEntries(rows.map((row) => [row.document_id, row.duplicates]));
    } catch { if (request === labelRequest) duplicates = {}; }
  }

  // Cross-posts on the same page fold under their first row. They are never
  // dropped: the leader names the other boards and expands to show the copies.
  // A collapsed leader also carries the unread state of its folded copies.
  const listRows = $derived.by(() => {
    const byId = new Map(result.items.map((item) => [item.id, item]));
    const placed = new Set<number>();
    const rows: { item: DocumentItem; folded: DocumentItem[]; member: boolean }[] = [];
    for (const item of result.items) {
      if (placed.has(item.id)) continue;
      placed.add(item.id);
      const folded = (groupedCopies[item.id] ?? []).flatMap((notice) => {
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
    return [...new Set((groupedCopies[item.id] ?? []).map((notice) => notice.source_name))].join(', ');
  }

  async function fetchPage(filters: ArchiveQuery, ranked: boolean) {
    const current = ++requestId;
    loading = true;
    error = '';
    let clamped = false;
    try {
      // The ranked query adds notices similar in meaning when the local
      // embedding model is available, and is text-only otherwise.
      const next = await invoke<ArchivePage>(ranked ? 'archive_ranked_query' : 'archive_query', { query: filters });
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
      // A row that left this page (moved, restored, deleted) is no longer selected.
      const visible = new Set(next.items.map((item) => item.id));
      if ([...chosen].some((id) => !visible.has(id))) chosen = new Set([...chosen].filter((id) => visible.has(id)));
      // A course scope keeps its category while the course has no notices yet.
      if (category && !next.categories.includes(category) && category !== sourceRequest.category) { category = ''; onsourcechange(source, ''); }
      // Removing the model takes the chips away; a chosen category would leave no way back.
      if (aiCategory && !next.ai_categories) aiCategory = '';
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

  function choose(id: number) {
    const next = new Set(chosen);
    if (!next.delete(id)) next.add(id);
    chosen = next;
  }
  let allChosen = $derived(result.items.length > 0 && result.items.every((item) => chosen.has(item.id)));
  function chooseAll() {
    chosen = allChosen ? new Set() : new Set(result.items.map((item) => item.id));
  }
  function stopSelecting() {
    selecting = false;
    chosen = new Set();
  }
  // 휴지통 keeps the content, so moving there or back needs no confirmation.
  async function moveToTrash(ids: number[], trash: boolean) {
    if (trashBusy || !ids.length) return;
    trashBusy = true;
    error = '';
    feedback = '';
    try {
      await invoke(trash ? 'archive_trash' : 'archive_restore', { ids });
      if (!active) return;
      feedback = trash ? `${ids.length}개 공지를 휴지통으로 옮겼습니다. 휴지통에서 언제든 복원할 수 있습니다.` : `${ids.length}개 공지를 복원했습니다.`;
      stopSelecting();
      revision++;
      await refreshContext();
    } catch (reason) { if (active) error = archiveError(reason); }
    finally { if (active) trashBusy = false; }
  }
  // `null` empties 휴지통.
  async function deletePermanently(ids: number[] | null) {
    if (trashBusy || (ids !== null && !ids.length)) return;
    // Emptying removes everything in 휴지통, whatever the list is filtered to.
    const count = ids?.length ?? summary?.trashed ?? result.total;
    let confirmed = false;
    try {
      confirmed = await confirmAction(`${ids === null ? '휴지통의 공지' : '선택한 공지'} ${count}개를 영구 삭제할까요? 본문, 첨부 자료, AI 결과가 이 기기에서 지워집니다. 학교 사이트에 남아 있어도 다시 가져오지 않습니다.`, '영구 삭제');
    } catch (reason) { if (active) error = archiveError(reason); return; }
    if (!confirmed || !active) return;
    trashBusy = true;
    error = '';
    feedback = '';
    try {
      await invoke('archive_purge', { ids });
      if (!active) return;
      feedback = ids === null ? '휴지통을 비웠습니다.' : `${count}개 공지를 영구 삭제했습니다.`;
      stopSelecting();
      revision++;
      await refreshContext();
    } catch (reason) { if (active) error = archiveError(reason); }
    finally { if (active) trashBusy = false; }
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
    // The calendar loads no notice list; it focuses the row once its todos have loaded again.
    if (page === 'calendar') calendarFocus = lastOpenedTarget;
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

  const weekdayLabels = ['일', '월', '화', '수', '목', '금', '토'];
  const weekDayLimit = 5;
  function familyOf(sourceId: string): string {
    return sources.find((item) => item.id === sourceId)?.family ?? 'other';
  }
  // Stored tasks outlive an unsubscribed board; the week follows the 과제·마감 navigation and shows them only while subscribed.
  // The 정보원 select lists each group's own boards under it; ungrouped boards follow.
  // The feed pages list only subscribed boards (and the one chosen), like their notices.
  let listedSources = $derived(feedPage ? sources.filter((item) => item.subscribed || item.id === source) : sources);
  let scopeOptions = $derived(groups.map((group) => ({ group, boards: listedSources.filter((item) => item.group === group.id) })).filter((entry) => entry.boards.length));
  let ungroupedSources = $derived(listedSources.filter((item) => !item.group || !groups.some((group) => group.id === item.group)));
  // A group feed, or one of its boards, offers chips for the group's subscribed boards.
  let chipGroup = $derived(groups.find((group) => group.id === (scopeGroup(source) ?? sources.find((item) => item.id === source)?.group)) ?? null);
  let chipBoards = $derived(chipGroup ? sources.filter((item) => item.group === chipGroup.id && ((item.supported && item.subscribed) || item.id === source)) : []);
  let aiCounts = $derived(ai ? result.ai_categories ?? null : null);
  let deadlines = $derived(new Map((result.deadlines ?? []).map((deadline) => [deadline.document_id, deadline])));
  // Read when the list renders, so a page left open past midnight relabels on its next refresh.
  let listToday = $derived.by(() => { void result; return localDayKey(new Date()); });
  // Deadlines up to three days ahead stand out.
  let soonDay = $derived.by(() => { void result; const now = new Date(); return localDayKey(new Date(now.getFullYear(), now.getMonth(), now.getDate() + 3)); });
  // Empty categories are left out, except the one chosen.
  let aiChips = $derived(aiCounts ? aiCounts.counts.filter((row) => row.count > 0 || row.category === aiCategory) : []);
  function chooseAiCategory(value: string) {
    aiCategory = value;
    offset = 0;
  }
  function chooseScope(scope: string) {
    source = scope;
    category = '';
    offset = 0;
    onsourcechange(scope, '');
  }
  let showTasks = $derived(enabled.has('cybercampus') && sources.some((item) => item.family === 'cybercampus' && item.subscribed));
  // Open deadlines only: a submitted task or one whose deadline (and late window) has passed is no longer something to plan for.
  let openTasks = $derived.by(() => {
    if (!showTasks) return [];
    const now = Date.now();
    return tasks.filter((task) => task.due_at !== null && !task.submitted_at && kstInstant(task.late_due_at ?? task.due_at) >= now)
      .sort((a, b) => a.due_at!.localeCompare(b.due_at!));
  });
  // A rolling window from today, so a weekend still shows the days ahead.
  let week = $derived.by(() => {
    const now = new Date();
    return Array.from({ length: 7 }, (_, index) => {
      const day = new Date(now.getFullYear(), now.getMonth(), now.getDate() + index);
      const key = localDayKey(day);
      return { key, label: weekdayLabels[day.getDay()], month: day.getMonth() + 1, date: day.getDate(),
        notices: result.items.filter((item) => archiveDay(item.published_at) === key),
        tasks: openTasks.filter((task) => task.due_at!.slice(0, 10) === key),
        // Done todos need no planning, like submitted deadlines.
        todos: calendarOn ? weekTodos.filter((todo) => todo.day === key && !todo.done) : [],
        // The 학사 일정 layer belongs to the calendar extension, like todos.
        events: calendarOn ? weekEvents.filter((event) => calendarEventOn(event, key)) : [],
        dates: weekDates.filter((entry) => noticeDateOn(entry, key)) };
    });
  });
  // The window's first day is today, so the highlight and the range cannot disagree after midnight.
  let todayKey = $derived(week[0].key);
  let laterDates = $derived(weekDates.filter((entry) => entry.day > week[6].key).slice(0, 5));
  // The Today query holds the newest page only; say so when today's notices may extend past it.
  let weekTruncated = $derived(result.items.length === pageSize && (archiveDay(result.items[pageSize - 1].published_at) ?? '') >= week[0].key);
  function shortDay(day: string): string {
    return `${Number(day.slice(5, 7))}월 ${Number(day.slice(8, 10))}일`;
  }
  function pickDate(value: string | null): string {
    const day = archiveDay(value);
    if (!day) return archiveDate(value);
    return day.slice(0, 4) === todayKey.slice(0, 4) ? shortDay(day) : `${day.slice(0, 4)}년 ${shortDay(day)}`;
  }

  async function paginate(delta: number) {
    offset = Math.max(0, offset + delta * pageSize);
    await tick();
    listHeading?.focus({ preventScroll: true });
  }
</script>

{#if readerId !== null}
  {#key `${readerId}:${readerRevisionId ?? ''}`}<DocumentReader id={readerId} initialRevisionId={readerRevisionId} enabledSources={sources.filter((item) => item.extension && enabled.has(item.extension)).map((item) => item.id)} {ai} onclose={closeReader} onchange={changed} onopen={(id) => void openDocument(id, `notice-${id}`)}
    oncalendar={calendarOn && desktop ? (prefill) => oncalendar(prefill.todo.day, prefill) : undefined} />{/key}
{:else}
  <section class="archive-view" aria-busy={loading} in:fade>
    {#if page === 'today' && !showRecommendations && !showWeek}
      <section class="empty-state today-blank" aria-labelledby="today-blank-heading">
        <span class="empty-icon"><Icon name="extensions" size={30} /></span>
        <h2 id="today-blank-heading" bind:this={listHeading} tabindex="-1">오늘을 채울 확장 기능을 켜 보세요</h2>
        <p>Rhythm은 필요한 기능만 켜서 씁니다. 학교 공지나 사이버캠퍼스를 연결하고, 나를 위한 소식이나 앞으로 7일을 켜면 이곳에 나타납니다.</p>
        <button class="button" onclick={() => onextensions()}>확장 기능 열기 <Icon name="arrow" size={16} /></button>
      </section>
    {:else if page === 'today'}
      <div class="today-layout">
        <div class="today-main">
          {#if desktop && summary?.documents === 0}
            <section class="first-run" aria-labelledby="first-run-heading">
              <h2 id="first-run-heading">Rhythm 시작하기</h2>
              <p>Rhythm은 공지를 이 기기에 보관하므로 인터넷 없이도 읽고 검색할 수 있습니다. 계정과 클라우드 없이 자료와 설정을 모두 이 기기에만 저장합니다.</p>
              <ol>
                <li>설정 → 확장 기능에서 서강대학교 공지나 사이버캠퍼스를 켭니다.</li>
                <li>서강대학교 공지를 켜면 Rhythm이 열려 있는 동안 30분마다 서버의 공지 피드를 받습니다. 사이버캠퍼스는 게시판 설정에서 새로고침을 누르거나 자동 동기화를 켰을 때만 가져옵니다.</li>
                <li>AI 분류, 알림·백그라운드 같은 기능도 확장 기능에서 켤 수 있습니다. 켜도 Rhythm이 실행 중이고 기기가 절전 상태가 아닐 때만 동작합니다.</li>
              </ol>
              <p>데이터 저장 위치는 설정 → 데이터 및 정보에서 확인할 수 있습니다.</p>
              <button class="button" onclick={() => onsetup()}>초기 설정 열기</button>
            </section>
          {/if}
          {#if error}<div class="archive-error" role="alert">{error}<button class="text-button" onclick={() => { revision++; void refreshContext(); }}>다시 시도</button></div>{/if}
          {#if showRecommendations && recommendationError}<p role="alert" class="archive-error">{recommendationError}</p>{/if}
          {#if showRecommendations && recommendations.length}
            <section class="today-picks" aria-label="오늘의 추천">
              <div class="pane-heading">
                <h2 bind:this={listHeading} tabindex="-1">나를 위한 소식</h2>
                <span class="pane-note">{hasPreferences ? '관심 설정과 저장·피드백을 반영한 추천' : '최근 공지 · 개인화 시작 전'}</span>
                {#if !hasPreferences}<button class="text-button" onclick={() => onsettings('personalization')}>관심 분야 설정</button>{/if}
              </div>
              <ul class="pick-list">
                {#each recommendations as recommendation (recommendation.item.id)}
                  {@const item = recommendation.item}
                  <li class:notice-unread={!item.is_read} animate:flip>
                    <button class="pick-open" id={`notice-${item.id}`} title={item.title} onclick={() => void open(item)}>
                      <span class="pick-title"><span class="notice-read-dot" aria-hidden="true"></span><strong>{item.title}</strong><span class="sr-only">{item.is_read ? '읽음' : '읽지 않음'}</span></span>
                      <span class="pick-meta">{#if recommendation.feedback}<span class="pick-feedback">{recommendation.feedback === 'interested' ? '관심' : '관심 없음'}</span>{/if}<span class={`board-swatch family-${familyOf(item.source_id)}`} aria-hidden="true"></span><span class="pick-source">{item.source_name}</span><time datetime={item.published_at}>{pickDate(item.published_at)}</time></span>
                    </button>
                    <div class="recommendation-actions" role="group" aria-label={`${item.title} 추천 피드백`}>
                      <button class="chip" aria-pressed={recommendation.feedback === 'interested'} disabled={pendingRecommendationFeedback[item.id]} onclick={() => void setRecommendationFeedback(recommendation, recommendation.feedback === 'interested' ? null : 'interested')}>관심 있어요</button>
                      <button class="chip" aria-pressed={recommendation.feedback === 'not_interested'} disabled={pendingRecommendationFeedback[item.id]} onclick={() => void setRecommendationFeedback(recommendation, recommendation.feedback === 'not_interested' ? null : 'not_interested')}>관심 없어요</button>
                    </div>
                  </li>
                {/each}
              </ul>
            </section>
          {:else if showRecommendations && !recommendationError}
            <div class="recommendation-empty">
              <h2 bind:this={listHeading} tabindex="-1">나를 위한 소식</h2>
              <p>{sourcesOn ? hasPreferences ? '관심 분야와 맞는 소식을 아직 찾지 못했습니다.' : '개인화 전에는 최근 공지를 표시합니다. 관심사를 설정하면 추천에 반영됩니다.' : '정보원 확장 기능이 꺼져 있어 새 추천은 표시하지 않습니다. 보관한 공지는 그대로 남아 있습니다.'}</p>
              {#if !hasPreferences}<button class="text-button" onclick={() => onsettings('personalization')}>관심 분야 설정</button>{/if}
            </div>
          {/if}
        </div>

        {#if showWeek}
        <aside class="week-pane" aria-labelledby="week-heading">
          <div class="pane-heading"><h2 id="week-heading">앞으로 7일</h2><span class="pane-note">{shortDay(week[0].key)} – {week[6].month === week[0].month ? `${week[6].date}일` : shortDay(week[6].key)}</span></div>
          {#if loading && !result.items.length}<p class="loading" role="status">오늘 공지를 불러오는 중입니다…</p>{/if}
          <ol class="week-days">
            {#each week as day (day.key)}
              <li class="week-day" class:is-today={day.key === todayKey}>
                <div class="day-label"><span class="day-name">{day.key === todayKey ? '오늘' : day.label}</span><span class="day-number">{day.date}</span>{#if day.key === todayKey}<span class="sr-only">{day.label}요일</span>{/if}</div>
                <ul class="day-events">
                  {#each day.tasks as task (`${task.course_key}-${task.category}-${task.item_id}`)}
                    <li><button class="event event-task" id={`week-task-${task.course_key}-${task.category}-${task.item_id}`} onclick={() => ontasks()}>
                      <span class="event-bar" aria-hidden="true"></span>
                      <span class="event-title"><span class="ai-mark task-mark">마감{task.due_at!.length > 10 ? ` ${wallTime(task.due_at!.slice(11, 16), clock.format)}` : ''}</span> {task.title}</span>
                      <span class="event-meta">{task.course_name}{#if kstInstant(task.due_at!) < Date.now()}{' · '}지각 제출 가능{/if}</span>
                    </button></li>
                  {/each}
                  {#each day.todos as todo (todo.id)}
                    <li><button class="event event-todo" id={`week-todo-${todo.id}`} onclick={() => oncalendar(day.key)}>
                      <span class="event-bar" aria-hidden="true"></span>
                      <span class="event-title"><span class="ai-mark todo-mark">할 일{todo.time ? ` ${wallTime(todo.time, clock.format)}` : ''}</span> {todo.title}</span>
                      <span class="event-meta">캘린더에서 보기</span>
                    </button></li>
                  {/each}
                  {#each day.events as event (`${event.calendar_id}-${event.uid}`)}
                    <li><button class="event event-academic" id={`week-academic-${day.key}-${event.uid}`} onclick={() => oncalendar(day.key)}>
                      <span class="event-bar" aria-hidden="true"></span>
                      <span class="event-title"><span class="ai-mark academic-mark">학사</span> {event.title}</span>
                      <span class="event-meta">{calendarEventMeta(event)}</span>
                    </button></li>
                  {/each}
                  {#each day.notices.slice(0, weekDayLimit) as item (item.id)}
                    <li><button class="event" class:notice-unread={!item.is_read} id={`week-${item.id}`} onclick={() => void open(item, `week-${item.id}`)}>
                      <span class={`event-bar family-${familyOf(item.source_id)}`} aria-hidden="true"></span>
                      <span class="event-title">{item.title}</span>
                      <span class="event-meta">{item.source_name}</span>
                    </button></li>
                  {/each}
                  {#if day.notices.length > weekDayLimit}<li class="day-more">외 {day.notices.length - weekDayLimit}건 · 받은 소식에서 모두 보기</li>{/if}
                  {#each day.dates as entry (`${entry.document_id}-${entry.ordinal}`)}
                    {@const target = `week-notice-${entry.document_id}-${entry.ordinal}`}
                    <li><button class="event event-notice" id={target} onclick={() => void openDocument(entry.document_id, target)}>
                      <span class="event-bar" aria-hidden="true"></span>
                      <span class="event-title"><span class="ai-mark notice-mark">{noticeDateMark(entry, clock.format)}</span> {entry.title}</span>
                      <span class="event-meta">{noticeDateMeta(entry, clock.format)}</span>
                    </button></li>
                  {/each}
                  {#if !day.tasks.length && !day.todos.length && !day.events.length && !day.notices.length && !day.dates.length}<li class="day-empty">일정 없음</li>{/if}
                </ul>
              </li>
            {/each}
          </ol>
          {#if weekTruncated}<p class="pane-footnote">오늘 공지가 많아 최근 {pageSize}건만 표시합니다. 전체는 받은 소식에서 볼 수 있습니다.</p>{/if}
          {#if laterDates.length}
            <h3 class="later-heading">이후 공지 날짜</h3>
            <ul class="day-events">
              {#each laterDates as entry (`${entry.document_id}-${entry.ordinal}`)}
                {@const target = `later-notice-${entry.document_id}-${entry.ordinal}`}
                <li><button class="event event-notice" id={target} onclick={() => void openDocument(entry.document_id, target)}>
                  <span class="event-bar" aria-hidden="true"></span>
                  <span class="event-title"><span class="ai-mark notice-mark">{shortDay(entry.day)} · {noticeDateMark(entry, clock.format)}</span> {entry.title}</span>
                  <span class="event-meta">{noticeDateMeta(entry, clock.format)}</span>
                </button></li>
              {/each}
            </ul>
          {/if}
          <p class="pane-footnote">오늘 게시된 공지{#if showTasks}, 사이버캠퍼스에서 마지막으로 가져온 과제 마감{/if}{#if calendarOn}, 캘린더에 직접 만든 할 일{#if weekEvents.length}, 학교 학사 일정{/if}{/if}{showTasks || calendarOn ? '과' : '와'} {weekAllDeadlines ? '저장한 공지의 날짜, 모든 공지의 마감' : '저장한 공지의 본문에서 찾은 날짜'}입니다. 찾은 날짜는 틀릴 수 있으니 원문에서 확인하세요.</p>
        </aside>
        {/if}
      </div>
    {:else if page === 'calendar'}
      <CalendarView {desktop} notices={enabled.has('sogang_notices')} bind:selected={calendarDay} bind:prefill={calendarPrefill} bind:focus={calendarFocus} tasks={openTasks} {ontasks} onopen={(id, target) => void openDocument(id, target)} />
    {:else}
    {#if page === 'search'}
      <form class="search-form archive-search" onsubmit={search} role="search"><Icon name="search" size={21} /><label class="sr-only" for="local-query">보관된 공지 검색</label><input id="local-query" type="search" bind:value={draft} maxlength="512" placeholder="제목과 본문에서 검색" autocomplete="off" disabled={!desktop} /><button class="text-button" type="submit" disabled={!desktop || loading}>검색</button></form>
      <p class="footnote">한국어 일부 단어, 영문, 숫자를 검색할 수 있습니다. 띄어 쓴 검색어는 모두 포함된 공지를 찾습니다. 검색어는 외부로 보내지 않습니다.{#if result.semantic && sort === 'relevance'} 관련도순에서는 이 기기의 분류 모델로 뜻이 비슷한 공지도 함께 찾습니다.{/if}</p>
    {/if}
    <div class="archive-toolbar">
      <div class="archive-filters">
        <label>정보원<select value={source} onchange={(event) => chooseScope(event.currentTarget.value)} disabled={!desktop}><option value="">모든 정보원</option>{#each scopeOptions as { group, boards } (group.id)}<optgroup label={group.name}><option value={groupScope(group.id)}>{group.name} 전체</option>{#each boards as item (item.id)}<option value={item.id}>{item.name}</option>{/each}</optgroup>{/each}{#each ungroupedSources as item (item.id)}<option value={item.id}>{item.organization} · {item.name}</option>{/each}</select></label>
        <label>게시판 분류<select bind:value={category} onchange={() => { offset = 0; onsourcechange(source, category); }} disabled={!desktop}><option value="">모든 분류</option>{#if category && !result.categories.includes(category)}<option value={category}>{category}</option>{/if}{#each result.categories as value}<option value={value}>{value}</option>{/each}</select></label>
        {#if page !== 'opportunities'}<label>정렬<select bind:value={sort} onchange={() => offset = 0} disabled={!desktop}>{#if page === 'search'}<option value="relevance">관련도순</option>{/if}<option value="newest">게시일 최신순</option><option value="oldest">게시일 오래된순</option></select></label>{/if}
      </div>
      {#if page === 'inbox' && chipGroup && chipBoards.length > 1}
        <div class="board-chips" role="group" aria-label={`${chipGroup.name} 게시판`}>
          <button class="chip" aria-pressed={source === groupScope(chipGroup.id)} disabled={!desktop} onclick={() => chooseScope(groupScope(chipGroup.id))}>전체</button>
          {#each chipBoards as item (item.id)}<button class="chip" aria-pressed={source === item.id} disabled={!desktop} onclick={() => chooseScope(item.id)}>{item.name}</button>{/each}
        </div>
      {/if}
      {#if aiCounts && (aiChips.length || aiCategory)}
        <div class="ai-category-chips" role="group" aria-label="AI 분류">
          <span class="ai-category-label" aria-hidden="true">AI 분류</span>
          <button class="chip" aria-pressed={!aiCategory} disabled={!desktop} onclick={() => chooseAiCategory('')}>전체</button>
          {#each aiChips as row (row.category)}<button class="chip" aria-pressed={aiCategory === row.category} disabled={!desktop} onclick={() => chooseAiCategory(row.category)}>{row.category} <span class="chip-count">{row.count.toLocaleString('ko-KR')}</span></button>{/each}
        </div>
      {/if}
      {#if page === 'inbox'}<label class="archive-checkbox"><input type="checkbox" bind:checked={unread} onchange={() => offset = 0} disabled={!desktop} /> 읽지 않은 공지만</label>{/if}
      {#if page === 'opportunities'}<label class="archive-checkbox"><input type="checkbox" bind:checked={includeClosed} onchange={() => offset = 0} disabled={!desktop} /> 마감 지난 공지도 보기</label>{/if}
    </div>
    {#if page === 'inbox' && feedNotice}
      <div class="banner" class:error={feedNotice.result === 'update_required'} role="status" transition:slide><Icon name="info" /><div>
        {#if feedNotice.result === 'update_required'}<strong>공지 피드를 받으려면 앱을 업데이트하세요</strong><p>공지 피드 형식이 바뀌어 이 버전에서는 새 공지를 받을 수 없습니다. 받은 공지는 그대로 볼 수 있습니다.</p>
        {:else if feedNotice.result === 'offline'}<strong>{archiveDate(feedNotice.generated_at, clock.format)} 기준 공지입니다</strong><p>인터넷에 연결되지 않아 새 공지를 받지 못했습니다. 연결되면 자동으로 받습니다.</p>
        {:else}<strong>공지 피드가 {archiveDate(feedNotice.generated_at, clock.format)} 이후 갱신되지 않았습니다</strong><p>서버에서 3시간 넘게 새 피드를 만들지 못했습니다. 받은 공지는 그대로 볼 수 있고, 갱신되면 자동으로 받습니다.</p>{/if}
      </div><button class="text-button" onclick={() => onsettings('boards')}>상태 보기</button></div>
    {/if}
    <div class="collection-heading archive-list-heading"><h2 bind:this={listHeading} tabindex="-1">{page === 'trash' ? '휴지통' : page === 'bookmark' ? '저장한 공지' : page === 'opportunities' ? '기회 공지' : query ? '검색 결과' : '보관된 공지'} <span class="muted">{result.total.toLocaleString('ko-KR')}</span></h2><div class="heading-actions">{#if result.items.length}<button class="text-button" aria-pressed={selecting} disabled={!desktop} onclick={() => selecting ? stopSelecting() : selecting = true}>{selecting ? '선택 끝내기' : '선택'}</button>{/if}{#if page === 'trash' && (summary?.trashed ?? result.total)}<button class="text-button danger" disabled={!desktop || trashBusy} onclick={() => void deletePermanently(null)}>휴지통 비우기</button>{/if}<button class="text-button" disabled={!desktop || loading} onclick={() => { revision++; void refreshContext(); }}>목록 새로고침</button></div></div>
    {#if aiCounts && aiCategory && aiCounts.unclassified}<p class="footnote">아직 분류되지 않은 공지 {aiCounts.unclassified.toLocaleString('ko-KR')}건은 이 목록에 없습니다.</p>{/if}
    {#if page === 'opportunities'}<p class="footnote">AI가 공모전·대회, 인턴·채용, 교육·프로그램, 행사·활동으로 분류한 공지를 마감이 가까운 순서로 보여 줍니다. 마감을 찾지 못한 공지는 그 뒤에 최신순으로 표시합니다. 마감은 본문에서 찾은 날짜라 틀릴 수 있으니 원문에서 확인하세요.</p>{/if}
    {#if page === 'trash'}<p class="footnote">휴지통의 공지는 직접 비우기 전까지 이 기기에 보관됩니다. 목록, 검색, 추천, 알림에는 나타나지 않습니다. 영구 삭제한 공지는 학교 사이트에 남아 있어도 다시 가져오지 않습니다.</p>{/if}
    {#if selecting && result.items.length}
      <div class="bulk-bar" role="group" aria-label="선택한 공지 작업">
        <label class="archive-checkbox"><input type="checkbox" checked={allChosen} indeterminate={chosen.size > 0 && !allChosen} onchange={chooseAll} /> 이 페이지 전체 선택</label>
        <span class="bulk-count" aria-live="polite">{chosen.size}개 선택</span>
        {#if page === 'trash'}
          <button class="button secondary small" disabled={!chosen.size || trashBusy} onclick={() => void moveToTrash([...chosen], false)}>복원</button>
          <button class="button secondary small danger" disabled={!chosen.size || trashBusy} onclick={() => void deletePermanently([...chosen])}>영구 삭제</button>
        {:else}
          <button class="button secondary small" disabled={!chosen.size || trashBusy} onclick={() => void moveToTrash([...chosen], true)}><Icon name="trash" size={16} />휴지통으로 이동</button>
        {/if}
      </div>
    {/if}
    {#if error}<div class="archive-error" role="alert">{error}<button class="text-button" onclick={() => { revision++; void refreshContext(); }}>다시 시도</button></div>{/if}
    <p class="sr-only archive-result-status" role="status">{loading ? '로컬 공지를 불러오는 중입니다.' : error ? '목록을 새로 불러오지 못했습니다.' : `공지 ${result.total.toLocaleString('ko-KR')}개${result.total ? ` · ${offset + 1}–${Math.min(offset + pageSize, result.total)}` : ''}`}</p>
    <p class="archive-feedback" role="status">{feedback}</p>
    {#if loading}<p class="loading" role="status">로컬 공지를 불러오는 중입니다…</p>{/if}
    {#if result.items.length}
      <ul class="notice-list" aria-label="공지 목록">
        {#each listRows as { item, folded, member } (item.id)}
          {@const hiddenUnread = expandedDuplicates[item.id] ? 0 : folded.filter((copy) => !copy.is_read).length}
          <li class:notice-unread={!item.is_read || hiddenUnread > 0} class:notice-duplicate-member={member} class:has-duplicates={folded.length > 0} class:is-selecting={selecting} class:in-trash={page === 'trash'} animate:flip in:slide={{ duration: member ? 180 : 0 }}>
            {#if selecting}<input class="notice-select" type="checkbox" aria-label={`${item.title} 선택`} checked={chosen.has(item.id)} onchange={() => choose(item.id)} />{/if}
            <button class="notice-open" id={`notice-${item.id}`} onclick={() => open(item)}>
              <span class="notice-title"><span class="notice-read-dot" aria-hidden="true"></span><Highlighted text={item.title} {query} /><span class="sr-only">{item.is_read ? '읽음' : '읽지 않음'}{hiddenUnread ? `, 접힌 같은 공지 ${hiddenUnread}건 읽지 않음` : ''}</span></span>
              <span class="notice-meta"><span>{item.source_name}</span><time datetime={item.published_at}>{archiveDate(item.published_at)}</time>{#if item.category}<span>{item.category}</span>{/if}{#if deadlines.get(item.id)}{@const deadline = deadlines.get(item.id)!}<span class="notice-deadline" class:closed={deadline.closed} class:soon={!deadline.closed && deadline.day <= soonDay}>{deadlineLabel(deadline, listToday, clock.format)}</span>{/if}</span>
              {#if labels[item.id] && page !== 'search'}<span class="notice-ai-preview"><span class="ai-tag">분류 · {labels[item.id].category}{labels[item.id].low_confidence ? ' · 확신 낮음' : ''}</span></span>{/if}
              {#if result.meaning_only?.includes(item.id)}<span class="notice-meaning-note">검색어는 없지만 뜻이 비슷한 공지</span>{/if}
              {#if groupedCopies[item.id]?.length}<span class="notice-duplicate-note">같은 공지 {groupedCopies[item.id].length}건 · {duplicateBoards(item)}</span>{/if}
              {#if page === 'search' && item.snippet}<span class="notice-snippet"><Highlighted text={item.snippet} {query} /></span>{/if}
              {#if item.content_status === 'missing_resources'}<span class="notice-resource-warning">일부 첨부 자료 미저장</span>{/if}
            </button>
            {#if page === 'trash'}
              <div class="notice-trash-actions"><button class="text-button" aria-label={`${item.title} 복원`} disabled={trashBusy} onclick={() => void moveToTrash([item.id], false)}>복원</button><button class="text-button danger" aria-label={`${item.title} 영구 삭제`} disabled={trashBusy} onclick={() => void deletePermanently([item.id])}>영구 삭제</button></div>
            {:else}
              <button class="notice-trash icon-button" aria-label={`${item.title} 휴지통으로 이동`} disabled={trashBusy} onclick={() => void moveToTrash([item.id], true)}><Icon name="trash" size={18} /></button>
              <button class="notice-save icon-button" aria-label={`${item.title} ${item.saved ? '저장 해제' : '보관함에 저장'}`} class:is-saved={item.saved} disabled={pendingSaves[item.id]} onclick={() => save(item)}><Icon name="bookmark" size={19} /></button>
            {/if}
            {#if folded.length}<button class="text-button notice-duplicate-toggle" aria-expanded={Boolean(expandedDuplicates[item.id])} onclick={() => expandedDuplicates = { ...expandedDuplicates, [item.id]: !expandedDuplicates[item.id] }}>{expandedDuplicates[item.id] ? `같은 공지 ${folded.length}건 접기` : `같은 공지 ${folded.length}건 펼치기${hiddenUnread ? ` · 읽지 않음 ${hiddenUnread}건` : ''}`}</button>{/if}
          </li>
        {/each}
      </ul>
      <nav class="archive-pagination" aria-label="공지 목록 페이지"><button class="button secondary small" disabled={offset === 0 || loading} onclick={() => paginate(-1)}>이전</button><span aria-live="polite">{offset + 1}–{Math.min(offset + pageSize, result.total)} / {result.total}</span><button class="button secondary small" disabled={offset + pageSize >= result.total || loading} onclick={() => paginate(1)}>다음</button></nav>
    {:else if !loading && !error}
      <div class="empty-state archive-empty"><span class="empty-icon"><Icon name={page === 'trash' ? 'trash' : page === 'bookmark' ? 'bookmark' : page === 'search' ? 'search' : page === 'opportunities' ? 'opportunities' : 'inbox'} size={30} /></span><h2>{query || source || category || (ai && aiCategory) || unread ? '조건에 맞는 공지가 없습니다' : page === 'trash' ? '휴지통이 비어 있습니다' : page === 'bookmark' ? '아직 저장한 공지가 없습니다' : page === 'opportunities' ? includeClosed ? '기회로 분류된 공지가 없습니다' : '마감이 남은 기회가 없습니다' : '아직 보관된 공지가 없습니다'}</h2><p>{query || source || category || (ai && aiCategory) || unread ? '검색어나 필터를 바꿔 보세요. 보관한 공지만 검색하므로 인터넷 연결은 필요하지 않습니다.' : page === 'trash' ? '필요 없는 공지를 휴지통으로 옮기면 여기에 모입니다. 비우기 전까지 언제든 복원할 수 있습니다.' : page === 'bookmark' ? '공지 옆의 저장 버튼으로 중요한 공지를 모아 보세요. 확장 기능을 꺼도 보관함은 그대로 남습니다.' : page === 'opportunities' ? '공모전·대회, 인턴·채용, 교육·프로그램, 행사·활동으로 분류된 공지가 여기에 모입니다. 마감이 지난 공지는 마감 지난 공지도 보기를 켜면 볼 수 있습니다.' : '설정에서 정보원을 구독하고 공개된 공지를 가져오세요. 보관한 본문은 오프라인에서도 읽을 수 있습니다.'}</p>{#if page !== 'bookmark' && page !== 'trash' && !query}<button class="button secondary" onclick={() => onsettings()}>정보원 설정하기 <Icon name="arrow" size={16} /></button>{/if}</div>
    {/if}
    {/if}
  </section>
{/if}
