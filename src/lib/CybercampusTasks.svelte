<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { CybercampusTask } from './types';
  import { archiveError, kstInstant as instant } from './archive-format';
  import Icon from './Icon.svelte';

  let { desktop, onsettings }: { desktop: boolean; onsettings: () => void } = $props();
  let tasks = $state<CybercampusTask[]>([]);
  let loading = $state(true);
  let error = $state('');
  let now = $state(Date.now());
  const submitted = (task: CybercampusTask) => Boolean(task.submitted_at);
  let upcoming = $derived(tasks.filter((task) => !submitted(task) && (task.pending || (task.due_at !== null && instant(task.late_due_at ?? task.due_at) >= now)))
    .sort((a, b) => (a.due_at ?? '9999').localeCompare(b.due_at ?? '9999')));
  let rest = $derived(tasks.filter((task) => !upcoming.includes(task)).sort((a, b) => (b.due_at ?? '').localeCompare(a.due_at ?? '')));

  onMount(() => {
    let active = true;
    const timer = window.setInterval(() => now = Date.now(), 60_000);
    if (desktop) {
      void invoke<CybercampusTask[]>('cybercampus_tasks')
        .then((value) => { if (active) tasks = value; })
        .catch((reason) => { if (active) error = archiveError(reason); })
        .finally(() => { if (active) loading = false; });
    } else loading = false;
    return () => { active = false; window.clearInterval(timer); };
  });

  const weekdays = ['일', '월', '화', '수', '목', '금', '토'];
  function when(value: string) {
    const [year, month, day] = value.slice(0, 10).split('-').map(Number);
    const weekday = weekdays[new Date(Date.UTC(year, month - 1, day)).getUTCDay()];
    return `${month}월 ${day}일 (${weekday})${value.length > 10 ? ` ${value.slice(11, 16)}` : ''}`;
  }
  function remaining(value: string) {
    const left = instant(value) - now;
    if (left < 0) return '마감 지남';
    const hours = Math.floor(left / 3_600_000);
    return hours < 24 ? `${Math.max(hours, 0)}시간 남음` : `D-${Math.ceil(left / 86_400_000)}`;
  }
  function progress(task: CybercampusTask) {
    if (task.submitted_at) return `${task.status ?? '제출'} · ${when(task.submitted_at)}`;
    return task.status ?? (task.pending ? '미완료' : '제출 기록 없음');
  }
</script>

{#snippet row(task: CybercampusTask, open: boolean)}
  {@const urgent = open && task.due_at !== null && instant(task.due_at) - now < 86_400_000}
  <li class="task" class:urgent>
    <span class="task-title">{#if task.label}<span class="task-label">{task.label}</span>{/if}{task.title}</span>
    <span class="notice-meta">
      <span>{task.course_name}</span>
      {#if task.week}<span>{task.week}주차</span>{/if}
      {#if task.due_at}<time datetime={task.due_at}>마감 {when(task.due_at)}</time>{#if open}<strong class="task-left">{remaining(task.due_at)}</strong>{/if}{:else}<span>마감 정보 없음</span>{/if}
      {#if task.late_due_at}<span>지각 제출 {when(task.late_due_at)}까지</span>{/if}
      <span>{progress(task)}</span>
    </span>
  </li>
{/snippet}

<div class="cybercampus-tasks" aria-busy={loading}>
  <p class="footnote">사이버캠퍼스에서 마지막으로 가져온 과제와 할 일입니다. 제출은 사이버캠퍼스에서 해 주세요. 새로고침하면 최신 상태로 바뀝니다.</p>
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  {#if loading}<p class="loading" role="status">과제와 마감을 불러오는 중입니다…</p>
  {:else if !tasks.length}
    <div class="empty-state"><span class="empty-icon"><Icon name="check" size={30} /></span><h2>가져온 과제가 없습니다</h2><p>{desktop ? '설정의 게시판에서 사이버캠퍼스 로그인 정보를 저장하고 강의 공지를 구독한 뒤 새로고침하면 이번 학기 과제와 마감을 가져옵니다.' : '데스크톱 앱에서 사이버캠퍼스 과제를 가져올 수 있습니다.'}</p>{#if desktop}<button class="button secondary" onclick={onsettings}>게시판 설정하기 <Icon name="arrow" size={16} /></button>{/if}</div>
  {:else}
    <h2 class="settings-group-title">해야 할 일 {upcoming.length}개</h2>
    {#if upcoming.length}<ul class="notice-list" aria-label="해야 할 일">{#each upcoming as task (`${task.course_key}-${task.category}-${task.item_id}`)}{@render row(task, true)}{/each}</ul>
    {:else}<p class="muted">마감 전인 할 일이 없습니다.</p>{/if}
    {#if rest.length}
      <details class="task-history">
        <summary>제출했거나 마감이 지난 항목 {rest.length}개</summary>
        <ul class="notice-list" aria-label="제출했거나 마감이 지난 항목">{#each rest as task (`${task.course_key}-${task.category}-${task.item_id}`)}{@render row(task, false)}{/each}</ul>
      </details>
    {/if}
  {/if}
</div>

<style>
  .cybercampus-tasks { display: grid; gap: 12px; max-width: 820px; }
  .task { display: grid; gap: 4px; padding: 11px 22px; }
  .task-title { font-size: 14px; font-weight: 600; line-height: 1.5; color: var(--text); overflow-wrap: anywhere; }
  .task-label { margin-right: 8px; font-size: 11px; font-weight: 600; color: var(--secondary); }
  .task-left { font-weight: 650; color: var(--text); }
  .urgent .task-left { color: var(--danger); }
  .task-history summary { cursor: pointer; padding: 8px 0; font-size: 13px; color: var(--secondary); }
</style>
