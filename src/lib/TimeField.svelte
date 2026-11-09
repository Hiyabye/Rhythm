<script lang="ts">
  import { clock } from './clock.svelte';

  // Edits a local wall-clock `HH:MM`: 오전/오후 plus a 12-hour list, or a 24-hour list,
  // following the clock setting. With `optional`, an empty hour means no time (null).
  let { value, label, optional = false, disabled = false, onchange }: {
    value: string | null;
    label: string;
    optional?: boolean;
    disabled?: boolean;
    onchange: (value: string | null) => void;
  } = $props();

  const minutes = Array.from({ length: 60 }, (_, index) => String(index).padStart(2, '0'));
  let twelve = $derived(clock.format === '12h');
  let hours = $derived(twelve ? Array.from({ length: 12 }, (_, index) => index + 1) : Array.from({ length: 24 }, (_, index) => index));
  let hour = $derived(value ? Number(value.slice(0, 2)) : null);
  let minute = $derived(value ? value.slice(3, 5) : '00');
  // Remembered while no hour is picked, so 오후 can be chosen first.
  let pmChoice = $state(false);
  let pm = $derived(hour === null ? pmChoice : hour >= 12);

  function emit(hours: number, minutes: string) {
    onchange(`${String(hours).padStart(2, '0')}:${minutes}`);
  }
  function setMeridiem(next: boolean) {
    pmChoice = next;
    if (hour !== null) emit(hour % 12 + (next ? 12 : 0), minute);
  }
  function setHour(next: string) {
    if (next === '') onchange(null);
    else emit(twelve ? Number(next) % 12 + (pm ? 12 : 0) : Number(next), minute);
  }
</script>

<div class="time-field" role="group" aria-label={label}>
  {#if twelve}<div class="segmented" role="group" aria-label="오전 또는 오후">{#each [[false, '오전'], [true, '오후']] as const as [value, text]}<button type="button" {disabled} class:selected={pm === value} aria-pressed={pm === value} onclick={() => setMeridiem(value)}>{text}</button>{/each}</div>{/if}
  <select aria-label="시" {disabled} value={hour === null ? '' : String(twelve ? hour % 12 || 12 : hour)} onchange={(event) => setHour(event.currentTarget.value)}>
    {#if optional}<option value="">시간 없음</option>{/if}
    {#each hours as option}<option value={String(option)}>{option}시</option>{/each}
  </select>
  <select aria-label="분" disabled={disabled || hour === null} value={minute} onchange={(event) => emit(hour!, event.currentTarget.value)}>
    {#each minutes as option}<option value={option}>{option}분</option>{/each}
  </select>
</div>

<style>
  .time-field { display: inline-flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  select { font: inherit; font-size: 13px; color: var(--text); background: var(--field); border: 1px solid var(--line-strong); border-radius: 6px; padding: 3px 6px; }
</style>
