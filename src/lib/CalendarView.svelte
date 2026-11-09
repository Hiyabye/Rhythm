<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { CalendarTodo, CybercampusTask, ReferenceDate, TodoInput } from './types';
  import { archiveError, localDayKey } from './archive-format';
  import { confirmAction } from './confirm';
  import Icon from './Icon.svelte';
  import { fade, slide } from './motion';

  let { desktop, selected = $bindable(), tasks, dates, ontasks, onopen }: {
    desktop: boolean;
    // Owned by the archive view, so returning from the reader keeps the day.
    selected: string;
    // Open eClass deadlines and current AI reference dates; the archive view owns both rules.
    tasks: CybercampusTask[];
    dates: ReferenceDate[];
    ontasks: () => void;
    onopen: (documentId: number, target: string) => void;
  } = $props();

  const weekdays = ['일', '월', '화', '수', '목', '금', '토'];
  const cellLimit = 3;
  let today = $state(localDayKey(new Date()));
  let month = $derived(selected.slice(0, 7));
  let todos = $state<CalendarTodo[]>([]);
  let loading = $state(false);
  let error = $state('');
  let feedback = $state('');
  let busy = $state(false);
  let editing = $state<number | null>(null);
  let draft = $state<TodoInput>({ title: '', day: '', time: null, memo: '' });
  let grid = $state<HTMLElement>();
  let loadRequest = 0;
  let active = true;

  function parse(day: string): Date {
    const [year, monthNumber, date] = day.split('-').map(Number);
    return new Date(year, monthNumber - 1, date);
  }
  function shift(day: string, days: number): string {
    const date = parse(day);
    date.setDate(date.getDate() + days);
    return localDayKey(date);
  }
  function shiftMonth(delta: number) {
    const date = parse(`${month}-01`);
    const target = new Date(date.getFullYear(), date.getMonth() + delta, 1);
    // Keep the day of month where the target month has it.
    const last = new Date(target.getFullYear(), target.getMonth() + 1, 0).getDate();
    target.setDate(Math.min(Number(selected.slice(8, 10)), last));
    selected = localDayKey(target);
  }
  // Whole weeks from the Sunday on or before the 1st through the Saturday on or after the last day.
  let weeks = $derived.by(() => {
    const first = parse(`${month}-01`);
    const start = new Date(first.getFullYear(), first.getMonth(), 1 - first.getDay());
    const last = new Date(first.getFullYear(), first.getMonth() + 1, 0);
    const end = new Date(last.getFullYear(), last.getMonth(), last.getDate() + 6 - last.getDay());
    const rows: string[][] = [];
    for (let day = start; day <= end; day = new Date(day.getFullYear(), day.getMonth(), day.getDate() + 1)) {
      if (day.getDay() === 0) rows.push([]);
      rows[rows.length - 1].push(localDayKey(day));
    }
    return rows;
  });
  let range = $derived({ from: weeks[0][0], to: weeks[weeks.length - 1][6] });

  $effect(() => {
    const { from, to } = range;
    if (desktop) void load(from, to);
  });
  onMount(() => {
    const refresh = () => { today = localDayKey(new Date()); };
    const timer = window.setInterval(refresh, 60_000);
    window.addEventListener('focus', refresh);
    return () => { active = false; window.clearInterval(timer); window.removeEventListener('focus', refresh); };
  });

  async function load(from = range.from, to = range.to) {
    const current = ++loadRequest;
    loading = true;
    try {
      const rows = await invoke<CalendarTodo[]>('calendar_todos', { from, to });
      if (active && current === loadRequest) { todos = rows; error = ''; }
    } catch (reason) {
      if (active && current === loadRequest) error = archiveError(reason);
    } finally {
      if (active && current === loadRequest) loading = false;
    }
  }

  function itemsOf(day: string) {
    return {
      todos: todos.filter((todo) => todo.day === day),
      tasks: tasks.filter((task) => task.due_at!.slice(0, 10) === day),
      dates: dates.filter((entry) => entry.day === day),
    };
  }
  let selectedItems = $derived(itemsOf(selected));
  let selectedEmpty = $derived(!selectedItems.todos.length && !selectedItems.tasks.length && !selectedItems.dates.length);

  function label(day: string, withWeekday = true): string {
    const date = parse(day);
    return `${date.getMonth() + 1}월 ${date.getDate()}일${withWeekday ? ` ${weekdays[date.getDay()]}요일` : ''}`;
  }
  function cellLabel(day: string): string {
    const items = itemsOf(day);
    const open = items.todos.filter((todo) => !todo.done).length;
    const parts = [label(day)];
    if (day === today) parts.push('오늘');
    if (open) parts.push(`할 일 ${open}개`);
    if (items.tasks.length) parts.push(`마감 ${items.tasks.length}개`);
    if (items.dates.length) parts.push(`AI 참고 일정 ${items.dates.length}개`);
    return parts.join(', ');
  }

  async function select(day: string, focus = false) {
    selected = day;
    editing = null;
    if (!focus) return;
    await tick();
    grid?.querySelector<HTMLButtonElement>(`[data-day="${day}"]`)?.focus();
  }
  function navigateGrid(event: KeyboardEvent) {
    const step = ({ ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 } as Record<string, number>)[event.key];
    if (step) { event.preventDefault(); void select(shift(selected, step), true); return; }
    if (event.key === 'Home') { event.preventDefault(); void select(shift(selected, -parse(selected).getDay()), true); }
    if (event.key === 'End') { event.preventDefault(); void select(shift(selected, 6 - parse(selected).getDay()), true); }
    if (event.key === 'PageUp' || event.key === 'PageDown') {
      event.preventDefault();
      shiftMonth(event.key === 'PageUp' ? -1 : 1);
      void select(selected, true);
    }
  }

  function startAdd() {
    editing = 0;
    draft = { title: '', day: selected, time: null, memo: '' };
  }
  function startEdit(todo: CalendarTodo) {
    editing = todo.id;
    draft = { title: todo.title, day: todo.day, time: todo.time, memo: todo.memo };
  }
  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = '';
    feedback = '';
    const todo = { ...draft, time: draft.time || null };
    try {
      if (editing) await invoke<CalendarTodo>('calendar_update_todo', { id: editing, todo });
      else await invoke<CalendarTodo>('calendar_create_todo', { todo });
      feedback = editing ? '할 일을 고쳤습니다.' : '할 일을 추가했습니다.';
      editing = null;
      // A moved todo jumps with the selection, so it stays in view.
      selected = todo.day;
      await load();
    } catch (reason) {
      error = archiveError(reason);
    } finally {
      busy = false;
    }
  }
  async function toggle(todo: CalendarTodo) {
    if (busy) return;
    busy = true;
    error = '';
    try {
      const updated = await invoke<CalendarTodo>('calendar_set_todo_done', { id: todo.id, done: !todo.done });
      todos = todos.map((item) => item.id === updated.id ? updated : item);
    } catch (reason) {
      error = archiveError(reason);
    } finally {
      busy = false;
    }
  }
  async function remove(todo: CalendarTodo) {
    if (busy) return;
    let confirmed = false;
    try { confirmed = await confirmAction(`'${todo.title}' 할 일을 삭제할까요?`, '삭제'); }
    catch { error = '확인 창을 열지 못해 삭제하지 않았습니다.'; return; }
    if (!confirmed) return;
    busy = true;
    error = '';
    feedback = '';
    try {
      await invoke('calendar_delete_todo', { id: todo.id });
      feedback = '할 일을 삭제했습니다.';
      await load();
    } catch (reason) {
      error = archiveError(reason);
    } finally {
      busy = false;
    }
  }
</script>

{#snippet form()}
  <form class="todo-form" onsubmit={submit} transition:slide>
    <label class="todo-field todo-title-field"><span class="field-label">제목</span><input bind:value={draft.title} maxlength="200" required placeholder="예: 장학금 서류 내기" /></label>
    <div class="todo-row">
      {#if editing}<label class="todo-field"><span class="field-label">날짜</span><input type="date" bind:value={draft.day} required /></label>{/if}
      <label class="todo-field"><span class="field-label">시간 (선택)</span><input type="time" bind:value={draft.time} /></label>
    </div>
    <label class="todo-field"><span class="field-label">메모 (선택)</span><textarea bind:value={draft.memo} maxlength="2000" rows="2"></textarea></label>
    <div class="todo-actions">
      <button type="button" class="button secondary small" onclick={() => editing = null}>취소</button>
      <button class="button small" disabled={busy || !draft.title.trim()}>{editing ? '저장' : '추가'}</button>
    </div>
  </form>
{/snippet}

<div class="calendar-view" aria-busy={loading} in:fade>
  {#if !desktop}<p class="footnote">데스크톱 앱에서 할 일을 만들고 저장할 수 있습니다.</p>{/if}
  <section class="calendar-month" aria-labelledby="calendar-month-heading">
    <div class="calendar-toolbar">
      <h2 id="calendar-month-heading">{month.slice(0, 4)}년 {Number(month.slice(5, 7))}월</h2>
      <div class="calendar-nav">
        <button class="icon-button" aria-label="이전 달" onclick={() => shiftMonth(-1)}><span class="flip"><Icon name="arrow" size={15} /></span></button>
        <button class="button secondary small" aria-label="오늘로 이동" onclick={() => void select(today, true)}>오늘</button>
        <button class="icon-button" aria-label="다음 달" onclick={() => shiftMonth(1)}><Icon name="arrow" size={15} /></button>
      </div>
    </div>
    <div class="calendar-grid" role="grid" aria-labelledby="calendar-month-heading" bind:this={grid} tabindex="-1">
      <div class="calendar-row calendar-head" role="row">
        {#each weekdays as weekday}<span role="columnheader" class="calendar-weekday">{weekday}</span>{/each}
      </div>
      {#each weeks as week (week[0])}
        <div class="calendar-row" role="row">
          {#each week as day (day)}
            {@const items = itemsOf(day)}
            {@const entries = [
              ...items.tasks.map((task) => ({ kind: 'task', text: task.title, done: false })),
              ...items.todos.map((todo) => ({ kind: 'todo', text: todo.title, done: todo.done })),
              ...items.dates.map((entry) => ({ kind: 'ai', text: entry.label, done: false })),
            ]}
            <div role="gridcell" aria-selected={day === selected}>
              <button class="calendar-day" class:outside={day.slice(0, 7) !== month} class:is-today={day === today} class:selected={day === selected}
                data-day={day} tabindex={day === selected ? 0 : -1} aria-label={cellLabel(day)} onclick={() => void select(day)} onkeydown={navigateGrid}>
                <span class="day-number">{Number(day.slice(8, 10))}</span>
                <span class="calendar-entries" aria-hidden="true">
                  {#each entries.slice(0, cellLimit) as entry}<span class={`calendar-entry entry-${entry.kind}`} class:done={entry.done}>{entry.text}</span>{/each}
                  {#if entries.length > cellLimit}<span class="calendar-more">+{entries.length - cellLimit}</span>{/if}
                </span>
              </button>
            </div>
          {/each}
        </div>
      {/each}
    </div>
  </section>

  <section class="calendar-day-panel" aria-labelledby="calendar-day-heading">
    <div class="pane-heading">
      <h2 id="calendar-day-heading">{label(selected)}{#if selected === today}<span class="pane-note"> · 오늘</span>{/if}</h2>
      {#if desktop && editing === null}<button class="button small" onclick={startAdd}><Icon name="plus" size={15} />할 일 추가</button>{/if}
    </div>
    <p class="calendar-feedback" role="status" aria-live="polite">{feedback}</p>
    {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
    {#if editing === 0}{@render form()}{/if}
    <ul class="day-events calendar-list" aria-label={`${label(selected, false)} 일정`}>
      {#each selectedItems.tasks as task (`${task.course_key}-${task.category}-${task.item_id}`)}
        <li><button class="event event-task" onclick={ontasks}>
          <span class="event-bar" aria-hidden="true"></span>
          <span class="event-title"><span class="ai-mark task-mark">마감{task.due_at!.length > 10 ? ` ${task.due_at!.slice(11, 16)}` : ''}</span> {task.title}</span>
          <span class="event-meta">{task.course_name} · 과제·마감에서 보기</span>
        </button></li>
      {/each}
      {#each selectedItems.todos as todo (todo.id)}
        <li class="todo" class:done={todo.done}>
          {#if editing === todo.id}{@render form()}
          {:else}
            <label class="todo-check"><input type="checkbox" checked={todo.done} disabled={busy || !desktop} onchange={() => void toggle(todo)} /><span class="sr-only">{todo.done ? '완료함' : '완료 표시'}</span></label>
            <div class="todo-body">
              <span class="todo-title">{#if todo.time}<span class="ai-mark todo-mark">{todo.time}</span>{' '}{/if}{todo.title}</span>
              {#if todo.memo}<span class="todo-memo">{todo.memo}</span>{/if}
            </div>
            <div class="todo-buttons">
              <button class="text-button" disabled={busy || editing !== null} onclick={() => startEdit(todo)} aria-label={`${todo.title} 고치기`}>고치기</button>
              <button class="text-button danger" disabled={busy || editing !== null} onclick={() => void remove(todo)} aria-label={`${todo.title} 삭제`}>삭제</button>
            </div>
          {/if}
        </li>
      {/each}
      {#each selectedItems.dates as entry (entry.key)}
        <li><button class="event event-ai" id={`calendar-ai-${entry.key}`} onclick={() => onopen(entry.documentId, `calendar-ai-${entry.key}`)}>
          <span class="event-bar" aria-hidden="true"></span>
          <span class="event-title"><span class="ai-mark">AI 참고 일정</span> {entry.label}</span>
          <span class="event-meta">{entry.title} · 원문 확인</span>
        </button></li>
      {/each}
      {#if selectedEmpty && editing !== 0}<li class="day-empty">일정 없음</li>{/if}
    </ul>
  </section>
</div>

<style>
  .calendar-view { display: grid; grid-template-columns: minmax(0, 1fr) minmax(260px, 340px); gap: 24px; align-items: start; }
  .calendar-toolbar { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }
  .calendar-toolbar h2 { font-size: 17px; font-weight: 650; }
  .calendar-nav { display: flex; align-items: center; gap: 6px; }
  .flip { display: grid; transform: scaleX(-1); }
  .calendar-grid { border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
  .calendar-row { display: grid; grid-template-columns: repeat(7, minmax(0, 1fr)); }
  .calendar-row + .calendar-row { border-top: 1px solid var(--line); }
  .calendar-weekday { padding: 6px 8px; font-size: 11px; font-weight: 600; color: var(--secondary); }
  .calendar-row > [role=gridcell] + [role=gridcell] { border-left: 1px solid var(--line); }
  .calendar-day { width: 100%; height: 100%; min-height: 92px; display: flex; flex-direction: column; gap: 3px; padding: 4px 5px 6px; border: 0; background: transparent; text-align: left; color: var(--text); }
  .calendar-day:hover { background: var(--hover); }
  .calendar-day.selected { background: var(--soft); }
  .calendar-day.outside .day-number, .calendar-day.outside .calendar-entries { opacity: .45; }
  .calendar-day .day-number { width: 24px; height: 24px; font-size: 13px; }
  .calendar-day.is-today .day-number { background: var(--accent-fill); color: var(--accent-text); }
  .calendar-entries { display: grid; gap: 2px; min-width: 0; width: 100%; }
  .calendar-entry { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding: 0 4px; border-left: 2px solid var(--family-other); font-size: 11px; line-height: 16px; }
  .entry-task { border-left-color: var(--accent); }
  .entry-todo { border-left-color: var(--text-2); }
  .entry-ai { border-left-style: dashed; border-left-color: var(--muted); color: var(--secondary); }
  .calendar-entry.done { text-decoration: line-through; color: var(--muted); }
  .calendar-more { padding: 0 4px; font-size: 11px; color: var(--secondary); }
  .calendar-day-panel { position: sticky; top: 16px; display: grid; gap: 8px; }
  .calendar-day-panel .pane-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .calendar-day-panel h2 { font-size: 15px; font-weight: 650; }
  .calendar-feedback { font-size: 12px; color: var(--secondary); }
  .calendar-feedback:empty { display: none; }
  .todo { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 8px; align-items: start; padding: 6px; border-radius: 6px; }
  .todo:hover { background: var(--hover); }
  .todo:has(.todo-form) { display: block; }
  .todo-check { padding-top: 2px; }
  .todo-body { display: grid; gap: 2px; min-width: 0; }
  .todo-title { font-size: 12.5px; font-weight: 550; line-height: 1.4; overflow-wrap: anywhere; }
  .todo.done .todo-title { text-decoration: line-through; color: var(--muted); }
  .todo-memo { font-size: 11.5px; color: var(--secondary); white-space: pre-wrap; overflow-wrap: anywhere; }
  .todo-buttons { display: flex; gap: 2px; }
  .text-button.danger { color: var(--danger); }
  .todo-form { display: grid; gap: 8px; padding: 10px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); }
  .todo-row { display: flex; gap: 8px; flex-wrap: wrap; }
  .todo-field .field-label { margin-bottom: 4px; color: var(--secondary); }
  .todo-field input { width: 100%; min-width: 0; font: inherit; font-size: 13px; color: var(--text); background: var(--field); border: 1px solid var(--line-strong); border-radius: 6px; padding: 4px 8px; }
  .todo-field textarea { min-height: 52px; padding: 6px 8px; }
  .todo-actions { display: flex; justify-content: flex-end; gap: 6px; }
  @media (max-width: 1000px) {
    .calendar-view { grid-template-columns: minmax(0, 1fr); }
    .calendar-day-panel { position: static; }
    .calendar-day { min-height: 64px; }
  }
</style>
