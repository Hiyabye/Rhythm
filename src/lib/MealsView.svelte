<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { Meal, Meals, MealWeek } from './types';
  import { archiveError, wallTime } from './archive-format';
  import { clock } from './clock.svelte';
  import Icon from './Icon.svelte';
  import { fade } from './motion';

  // `notices`: whether 서강대학교 공지 is on; the feed that carries the menu syncs only then.
  let { desktop, notices, onsettings }: { desktop: boolean; notices: boolean; onsettings: () => void } = $props();
  let meals = $state<Meals | null>(null);
  let loading = $state(true);
  let error = $state('');
  let showNext = $state(false);
  let selected = $state(0);
  let tabs: HTMLButtonElement[] = $state([]);

  const weekdays = ['월', '화', '수', '목', '금', '토', '일'];
  // Days are Korean dates as plain YYYY-MM-DD; UTC arithmetic keeps them exact.
  function shift(day: string, days: number) {
    return new Date(Date.parse(`${day}T00:00:00Z`) + days * 86_400_000).toISOString().slice(0, 10);
  }
  function weekday(day: string) {
    return (new Date(`${day}T00:00:00Z`).getUTCDay() + 6) % 7;
  }
  function short(day: string) {
    return `${Number(day.slice(5, 7))}/${Number(day.slice(8, 10))}`;
  }
  let monday = $derived(meals ? shift(meals.today, -weekday(meals.today)) : '');
  let current = $derived(meals?.weeks.find((week) => week.week === monday) ?? null);
  // Only the week right after this one; no other week stands in for a missing one.
  let next = $derived(meals?.weeks.find((week) => week.week === shift(monday, 7)) ?? null);
  let week = $derived<MealWeek | null>(showNext ? next : current);
  let start = $derived(showNext ? shift(monday, 7) : monday);
  let day = $derived(week?.status === 'ok' ? week.days[selected] ?? null : null);

  onMount(() => {
    let active = true;
    if (desktop) {
      void invoke<Meals>('meal_weeks')
        .then((value) => {
          if (!active) return;
          meals = value;
          selected = weekday(value.today);
        })
        .catch((reason) => { if (active) error = archiveError(reason); })
        .finally(() => { if (active) loading = false; });
    } else loading = false;
    return () => { active = false; };
  });

  function showWeek(nextWeek: boolean) {
    if (showNext === nextWeek) return;
    showNext = nextWeek;
    selected = nextWeek || !meals ? 0 : weekday(meals.today);
  }
  async function keydown(event: KeyboardEvent, index: number) {
    const target = event.key === 'ArrowRight' ? (index + 1) % 7
      : event.key === 'ArrowLeft' ? (index + 6) % 7
      : event.key === 'Home' ? 0
      : event.key === 'End' ? 6
      : null;
    if (target === null) return;
    event.preventDefault();
    selected = target;
    await tick();
    tabs[target]?.focus();
  }
  async function openImage(url: string) {
    error = '';
    try { await invoke('archive_open_original', { url }); }
    catch (reason) { error = archiveError(reason); }
  }
</script>

{#snippet cell(label: string, meal: Meal, detail?: string)}
  <div class="meal-row">
    <dt>{label}{#if detail}<span class="meal-detail">{detail}</span>{/if}</dt>
    <dd>
      {#if meal.status === 'available'}
        <ul class="meal-items">{#each meal.items as item, index (index)}<li>{item}</li>{/each}</ul>
      {:else}<span class="muted">{meal.status === 'closed' ? '운영하지 않음' : '메뉴표에 없음'}</span>{/if}
      {#if meal.time}<span class="meal-time">{wallTime(meal.time, clock.format)}</span>{/if}
    </dd>
  </div>
{/snippet}

{#snippet imageButton(url: string)}
  <button class="button secondary" onclick={() => void openImage(url)}>메뉴표 원본 보기 <Icon name="arrow" size={16} /></button>
{/snippet}

<div class="meals" aria-busy={loading} in:fade>
  {#if !notices}<p class="meals-note" role="note">식단은 서강대학교 공지와 함께 받아 옵니다. 서강대학교 공지를 끄면 새 메뉴표를 받지 않습니다. <button class="text-button" onclick={onsettings}>확장 기능 설정</button></p>{/if}
  {#if error}<p class="archive-error" role="alert">{error}</p>{/if}
  {#if loading}<p class="loading" role="status">식단을 불러오는 중입니다…</p>
  {:else if !desktop}
    <div class="empty-state"><span class="empty-icon"><Icon name="meals" size={30} /></span><h2>데스크톱 앱에서 볼 수 있습니다</h2><p>식단은 데스크톱 앱이 받은 피드에서 읽어 옵니다.</p></div>
  {:else if meals}
    <div class="meals-toolbar">
      <h2 class="settings-group-title">{short(start)} – {short(shift(start, 6))}</h2>
      {#if next}<div class="segmented" role="group" aria-label="주 선택"><button class:selected={!showNext} aria-pressed={!showNext} onclick={() => showWeek(false)}>이번 주</button><button class:selected={showNext} aria-pressed={showNext} onclick={() => showWeek(true)}>다음 주</button></div>{/if}
    </div>
    {#if !week}
      <div class="empty-state"><span class="empty-icon"><Icon name="meals" size={30} /></span><h2>이번 주 식단이 아직 없습니다</h2><p>기숙사 게시판에 이번 주 메뉴표가 올라오면 다음 동기화 때 받아 옵니다.</p></div>
    {:else if week.status === 'unreadable'}
      <div class="empty-state"><span class="empty-icon"><Icon name="warning" size={30} /></span><h2>메뉴표를 글자로 읽지 못했습니다</h2><p>{showNext ? '다음 주' : '이번 주'} 메뉴는 원본 이미지에서 확인해 주세요.</p>{@render imageButton(week.image_url)}</div>
    {:else}
      <div class="meal-days" role="tablist" aria-label="요일">
        {#each week.days as entry, index (entry.day)}
          {@const today = !showNext && entry.day === meals.today}
          <button bind:this={tabs[index]} role="tab" id={`meal-tab-${index}`} aria-selected={selected === index} aria-controls="meal-panel" tabindex={selected === index ? 0 : -1} class:selected={selected === index} class:today onclick={() => selected = index} onkeydown={(event) => void keydown(event, index)}>
            <span class="meal-weekday">{weekdays[index]}{#if today}<span class="sr-only"> (오늘)</span>{/if}</span><span class="meal-date">{short(entry.day)}</span>
          </button>
        {/each}
      </div>
      {#if day}
        <div id="meal-panel" role="tabpanel" aria-labelledby={`meal-tab-${selected}`}><dl class="meal-panel">
          {@render cell('조식 · 한식', day.breakfast_korean)}
          {@render cell('조식 · 간편식', day.breakfast_simple)}
          {#if week.breakfast_common}{@render cell('조식 · 공통', week.breakfast_common, '매일')}{/if}
          {@render cell('컵밥', day.cup_rice)}
          {@render cell('석식', day.dinner)}
          {#if week.drinks}{@render cell('음료', week.drinks, '매일')}{/if}
        </dl></div>
      {/if}
      <div class="meals-footer">
        <p class="footnote">메뉴표 이미지에서 자동으로 읽은 글자입니다.{#if week.low_resolution} 작은 이미지에서 읽어 메뉴 이름이 원본과 다를 수 있습니다.{/if} 정확한 메뉴는 원본에서 확인해 주세요.</p>
        {@render imageButton(week.image_url)}
      </div>
    {/if}
  {/if}
</div>

<style>
  .meals { display: grid; gap: 14px; max-width: 820px; }
  .meals-note { font-size: 13px; line-height: 1.6; color: var(--secondary); }
  .meals-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .meal-days { display: grid; grid-template-columns: repeat(7, minmax(0, 1fr)); gap: 4px; border-bottom: 1px solid var(--line); padding-bottom: 8px; }
  .meal-days button { display: grid; gap: 2px; justify-items: center; padding: 6px 0; border: 0; border-radius: 6px; background: transparent; color: var(--secondary); font-size: 12px; }
  .meal-days button:hover { background: var(--hover); }
  .meal-days button.selected { background: var(--surface); color: var(--accent); box-shadow: 0 1px 3px #0000000a; }
  .meal-days button.today .meal-weekday { color: var(--text); font-weight: 700; }
  .meal-days button.selected .meal-weekday { color: var(--accent); }
  .meal-weekday { font-size: 14px; font-weight: 600; }
  .meal-date { font-variant-numeric: tabular-nums; }
  .meal-panel { display: grid; margin: 0; }
  .meal-row { display: grid; grid-template-columns: 120px 1fr; gap: 16px; padding: 12px 4px; border-bottom: 1px solid var(--line); }
  dt { font-size: 13px; font-weight: 600; color: var(--text); }
  .meal-detail { display: block; font-size: 11px; font-weight: 400; color: var(--secondary); }
  dd { margin: 0; display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 12px; font-size: 14px; line-height: 1.6; }
  .meal-items { display: flex; flex-wrap: wrap; gap: 2px 12px; margin: 0; padding: 0; list-style: none; overflow-wrap: anywhere; }
  .meal-time { font-size: 12px; color: var(--secondary); font-variant-numeric: tabular-nums; }
  .meals-footer { display: grid; gap: 10px; justify-items: start; }
  @media (max-width: 560px) { .meal-row { grid-template-columns: 1fr; gap: 4px; } }
</style>
