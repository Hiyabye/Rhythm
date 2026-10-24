<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { ArchivePage, ArchiveSummary, DocumentItem, SourceInfo, SyncStatus } from './types';
  import { archiveDate, archiveError, archiveSize } from './archive-format';
  import DocumentReader from './DocumentReader.svelte';
  import Highlighted from './Highlighted.svelte';
  import Icon from './Icon.svelte';

  let { page, desktop, enabled, onsettings }: { page: string; desktop: boolean; enabled: boolean; onsettings: () => void } = $props();
  let result = $state<ArchivePage>({ items: [], total: 0, categories: [] });
  let summary = $state<ArchiveSummary | null>(null);
  let sources = $state<SourceInfo[]>([]);
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
  let readerId = $state<number | null>(null);
  let revision = $state(0);
  let listHeading = $state<HTMLHeadingElement>();
  let active = true;
  let requestId = 0;
  const pageSize = 30;

  $effect(() => {
    const currentPage = page;
    void currentPage;
    readerId = null;
    draft = '';
    query = '';
    source = '';
    category = '';
    unread = false;
    ascending = false;
    offset = 0;
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
  }

  async function fetchPage(filters: { query: string; source_id: string | null; category: string | null; saved_only: boolean; unread_only: boolean; ascending: boolean; offset: number; limit: number }) {
    const current = ++requestId;
    loading = true;
    error = '';
    try {
      const next = await invoke<ArchivePage>('archive_query', { query: filters });
      if (!active || current !== requestId) return;
      result = next;
      if (category && !next.categories.includes(category)) category = '';
    } catch (reason) { if (active && current === requestId) error = archiveError(reason); }
    finally { if (active && current === requestId) loading = false; }
  }

  function search(event: SubmitEvent) {
    event.preventDefault();
    query = draft.trim();
    offset = 0;
  }

  async function save(item: DocumentItem) {
    try {
      await invoke('archive_state', { id: item.id, isRead: null, saved: !item.saved });
      revision++;
      await refreshContext();
    } catch (reason) { error = archiveError(reason); }
  }

  function changed() { revision++; void refreshContext(); }

  async function closeReader() {
    readerId = null;
    changed();
    await tick();
    const target = document.getElementById(`notice-${lastOpened}`) ?? listHeading;
    target?.focus({ preventScroll: true });
  }
  let lastOpened = 0;
  async function open(item: DocumentItem) {
    lastOpened = item.id;
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
  {#key readerId}<DocumentReader id={readerId} {enabled} onclose={closeReader} onchange={changed} />{/key}
{:else}
  <section class="archive-view" aria-busy={loading}>
    {#if page === 'today'}
      <div class="archive-intro"><span class="section-kicker">내 기기에 쌓이는 학교 소식</span><h2>오늘도, 필요한 소식부터.</h2><p>직접 보관한 공지를 차분히 살펴보세요. AI 분석이나 개인화 없이 실제 보관 현황을 보여드립니다.</p></div>
      <dl class="archive-statistics">
        <div><dt>보관된 공지</dt><dd>{summary?.documents ?? '—'}</dd></div>
        <div><dt>읽지 않은 공지</dt><dd>{summary?.unread ?? '—'}</dd></div>
        <div><dt>보관함</dt><dd>{summary?.saved ?? '—'}</dd></div>
        <div><dt>최근 동기화의 새 공지</dt><dd>{summary?.new_since_sync ?? '—'}</dd></div>
      </dl>
      <p class="footnote">과거 자료 가져오기는 새 공지 수에서 제외됩니다. 저장된 자료 {archiveSize(summary?.resource_bytes ?? 0)}.</p>
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
    {#if loading}<p class="loading" role="status">로컬 공지를 불러오는 중입니다…</p>{/if}
    {#if result.items.length}
      <ul class="notice-list" aria-label="공지 목록">
        {#each result.items as item (item.id)}
          <li class:notice-unread={!item.is_read}>
            <button class="notice-open" id={`notice-${item.id}`} onclick={() => open(item)}>
              <span class="notice-title"><span class="notice-read-dot" aria-hidden="true"></span><Highlighted text={item.title} {query} /><span class="sr-only">{item.is_read ? '읽음' : '읽지 않음'}</span></span>
              <span class="notice-meta"><span>{item.source_name}</span><time datetime={item.published_at}>{archiveDate(item.published_at)}</time>{#if item.category}<span>{item.category}</span>{/if}{#if item.historical}<span class="notice-history">과거 자료</span>{/if}</span>
              {#if page === 'search' && item.snippet}<span class="notice-snippet"><Highlighted text={item.snippet} {query} /></span>{/if}
              {#if item.content_status === 'missing_resources'}<span class="notice-resource-warning">일부 첨부 자료 미저장</span>{/if}
            </button>
            <button class="notice-save icon-button" aria-label={`${item.title} ${item.saved ? '저장 해제' : '보관함에 저장'}`} aria-pressed={item.saved} class:is-saved={item.saved} onclick={() => save(item)}><Icon name="bookmark" size={19} /></button>
          </li>
        {/each}
      </ul>
      <nav class="archive-pagination" aria-label="공지 목록 페이지"><button class="button secondary small" disabled={offset === 0 || loading} onclick={() => paginate(-1)}>이전</button><span aria-live="polite">{offset + 1}–{Math.min(offset + pageSize, result.total)} / {result.total}</span><button class="button secondary small" disabled={offset + pageSize >= result.total || loading} onclick={() => paginate(1)}>다음</button></nav>
    {:else if !loading}
      <div class="empty-state archive-empty"><span class="empty-icon"><Icon name={page === 'bookmark' ? 'bookmark' : page === 'search' ? 'search' : 'inbox'} size={30} /></span><h2>{query || source || category || unread ? '조건에 맞는 공지가 없습니다' : page === 'bookmark' ? '아직 저장한 공지가 없습니다' : '아직 보관된 공지가 없습니다'}</h2><p>{query || source || category || unread ? '검색어 또는 필터를 바꿔 보세요. 이미 보관한 내용만 검색하며 인터넷 연결은 필요하지 않습니다.' : page === 'bookmark' ? '공지 옆의 저장 버튼으로 중요한 정보를 모아 보세요. 확장 기능을 꺼도 보관함은 유지됩니다.' : '설정에서 정보원을 구독하고 공개된 공지를 가져오세요. 한 번 보관한 본문은 오프라인에서도 읽을 수 있습니다.'}</p>{#if page !== 'bookmark' && !query}<button class="button secondary" onclick={onsettings}>정보원 설정하기 <Icon name="arrow" size={16} /></button>{/if}</div>
    {/if}
  </section>
{/if}
