<script lang="ts">
  import { tick } from 'svelte';
  import { timeOptions } from './archive-format';
  import { clock } from './clock.svelte';
  import type { StudiumSeatData, StudiumSeatLayout } from './types';

  let { data, identity, roomLabel, favorites, searchNumber, stale, onfocuslost }: {
    data: StudiumSeatData; identity: string; roomLabel: string; favorites: number[]; searchNumber: number | null; stale: boolean; onfocuslost: () => void;
  } = $props();
  const padding = 24;
  const mapId = $props.id();
  let open = $state(false);
  let zoom = $state(100);
  let viewport = $state<HTMLDivElement>();
  let toggle = $state<HTMLButtonElement>();
  let previousIdentity = '';
  let available = $derived(new Set(data.seat_numbers ?? []));
  let preferred = $derived(new Set(favorites));
  let issue = $derived(layoutIssue(data));
  let sourceWidth = $derived((data.layout?.width ?? 0) / 1000);
  let sourceHeight = $derived((data.layout?.height ?? 0) / 1000);
  let searchMatches = $derived(searchNumber !== null && available.has(searchNumber) ? 1 : 0);

  $effect(() => {
    const current = identity;
    if (current === previousIdentity) return;
    previousIdentity = current;
    const focused = document.hasFocus() && document.getElementById(`${mapId}-content`)?.contains(document.activeElement);
    open = false;
    zoom = 100;
    if (focused) void restoreToggleFocus();
  });

  $effect(() => {
    if (!issue) return;
    const focused = document.hasFocus() && document.getElementById(`${mapId}-content`)?.contains(document.activeElement);
    open = false;
    if (focused) onfocuslost();
  });

  async function restoreToggleFocus() {
    await tick();
    const active = document.activeElement;
    if (!document.hasFocus() || (active && active !== document.body && active.id !== 'main')) return;
    if (toggle?.isConnected && !toggle.disabled) toggle.focus();
    else onfocuslost();
  }

  function validLayout(layout: StudiumSeatLayout, total: number) {
    if (!Number.isSafeInteger(total) || total < 1 || total > 311
      || !Number.isSafeInteger(layout.width) || layout.width <= 0 || layout.width > 8_192_000
      || !Number.isSafeInteger(layout.height) || layout.height <= 0 || layout.height > 8_192_000
      || !Array.isArray(layout.points) || layout.points.length !== total) return false;
    const numbers = new Set<number>();
    const positions = new Set<string>();
    for (const point of layout.points) {
      if (!point || !Number.isSafeInteger(point.number) || point.number < 1 || point.number > total || numbers.has(point.number)
        || !Number.isSafeInteger(point.x) || point.x < 0 || point.x > layout.width
        || !Number.isSafeInteger(point.y) || point.y < 0 || point.y > layout.height || positions.has(`${point.x}:${point.y}`)) return false;
      numbers.add(point.number);
      positions.add(`${point.x}:${point.y}`);
    }
    return Math.min(...layout.points.map((point) => point.x)) === 0 && Math.max(...layout.points.map((point) => point.x)) === layout.width
      && Math.min(...layout.points.map((point) => point.y)) === 0 && Math.max(...layout.points.map((point) => point.y)) === layout.height;
  }

  function layoutIssue(value: StudiumSeatData) {
    if (value.layout_error) return value.layout_error;
    if (value.seat_numbers === null) return '좌석 번호와 위치를 확인하지 못했습니다. 확인된 빈 좌석 수를 이용해 주세요.';
    if (!value.layout) return '이 조회에는 좌석 위치 정보가 없습니다. 번호 목록을 이용해 주세요.';
    return validLayout(value.layout, value.total) ? '' : '좌석 위치를 확인하지 못했습니다. 번호 목록을 이용해 주세요.';
  }

  async function resetView() {
    const currentViewport = viewport;
    zoom = 100;
    // Reset the scroll origin against the updated SVG size.
    await tick();
    if (currentViewport?.isConnected && currentViewport === viewport) {
      currentViewport.scrollTo({ left: 0, top: 0, behavior: 'instant' });
    }
  }

  function sourceClock() {
    if (!Number.isFinite(Date.parse(data.source_time))) return '확인 불가';
    return new Intl.DateTimeFormat('ko-KR', { timeZone: 'Asia/Seoul', month: 'numeric', day: 'numeric', ...timeOptions(clock.format, true) }).format(new Date(data.source_time));
  }
</script>

<div class="seat-map">
  <div class="map-heading"><h4 id={`${mapId}-title`}>좌석 배치도</h4><button class="button secondary small" bind:this={toggle} disabled={Boolean(issue)} aria-expanded={open && !issue} aria-controls={`${mapId}-content`} onclick={() => open = !open}>{open ? '좌석 배치도 닫기' : '좌석 배치도 열기'}</button></div>
  {#if issue}<p class="map-error" role="status">{issue}</p>
  {:else if open && data.layout}
    <div id={`${mapId}-content`}>
      <p class="map-summary" id={`${mapId}-summary`} role="status">{roomLabel} · 전체 {data.total}석 · 빈 좌석 {data.remaining}석 · 사용 중 {data.used}석{searchNumber !== null ? ` · 검색 결과 ${searchMatches}개` : ''}{stale ? ' · 이전 조회 기준' : ''}</p>
      <p class="map-help">원본 기준 시각 · 한국시간: <time datetime={data.source_time}>{sourceClock()}</time></p>
      {#if stale}<p class="map-warning" role="status">이전 조회의 위치와 상태입니다. 현재 빈자리 여부는 확인되지 않았습니다.</p>{/if}
      <p class="map-help" id={`${mapId}-help`}>번호의 상대 위치만 표시합니다. 실제 좌석의 크기·형태나 벽·문을 나타내지 않습니다. 번호 목록도 함께 이용할 수 있습니다.</p>
      <ul class="map-legend" id={`${mapId}-legend`} aria-label="배치도 기호"><li>○ 빈 좌석</li><li>× 사용 중</li><li>★ 선호 설정</li>{#if searchNumber !== null}<li><span class="search-example">밑줄</span> 검색한 빈 좌석</li>{/if}</ul>
      <div class="map-controls" role="group" aria-label="배치도 보기 조절"><button class="button secondary small" disabled={zoom <= 50} onclick={() => zoom = Math.max(50, zoom - 25)}>배치도 축소</button><output aria-label="배치도 확대 비율" aria-live="polite">{zoom}%</output><button class="button secondary small" disabled={zoom >= 200} onclick={() => zoom = Math.min(200, zoom + 25)}>배치도 확대</button><button class="button secondary small" onclick={resetView}>배치도 보기 초기화</button></div>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (Native overflow scrolling needs keyboard focus.) -->
      <div class="map-viewport" bind:this={viewport} tabindex="0" role="region" aria-label={`${roomLabel} 좌석 배치도 · 방향키로 이동`} aria-describedby={`${mapId}-summary ${mapId}-legend ${mapId}-help`}>
        <svg width={(sourceWidth + padding * 2) * zoom / 100} height={(sourceHeight + padding * 2) * zoom / 100} viewBox={`${-padding} ${-padding} ${sourceWidth + padding * 2} ${sourceHeight + padding * 2}`} aria-hidden="true" focusable="false">
          {#each data.layout.points as point (point.number)}
            {@const empty = available.has(point.number)}
            {@const favorite = preferred.has(point.number)}
            {@const match = searchNumber !== null && empty && searchNumber === point.number}
            <g class:available={empty} class:occupied={!empty} class:preferred={favorite} class:search-match={match} data-seat-number={point.number}>
              <text class="marker" x={point.x / 1000} y={point.y / 1000} text-anchor="middle" dominant-baseline="central">{empty ? '○' : '×'}</text>
              <text class="number" x={point.x / 1000} y={point.y / 1000 + 13} text-anchor="middle">{point.number}</text>
              {#if favorite}<text class="favorite" x={point.x / 1000 + 11} y={point.y / 1000 - 6} text-anchor="middle">★</text>{/if}
            </g>
          {/each}
        </svg>
      </div>
      <p class="map-help">배치도에 포커스를 두고 방향키나 스크롤로 이동합니다. 확대·이동·검색은 새로 조회하지 않습니다.</p>
    </div>
  {/if}
</div>

<style>
  .seat-map { min-width: 0; margin: 16px 0; padding: 14px 0; border-top: 1px solid var(--line); border-bottom: 1px solid var(--line); }
  .map-heading, .map-controls { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  h4 { margin: 0; font-size: 13px; font-weight: 650; }
  .map-heading { justify-content: space-between; }
  .map-summary, .map-help, .map-error, .map-warning { font-size: 12px; line-height: 1.6; margin: 10px 0; }
  .map-help { color: var(--secondary); }
  .map-error, .map-warning { color: var(--danger); }
  .map-legend { list-style: none; display: flex; flex-wrap: wrap; gap: 6px 14px; padding: 0; margin: 10px 0; font-size: 12px; }
  .map-controls { margin: 10px 0; font-size: 12px; }
  output { min-width: 40px; text-align: center; font-variant-numeric: tabular-nums; }
  .map-viewport { min-width: 0; width: 100%; max-height: 420px; overflow: auto; border: 1px solid var(--line-strong); border-radius: 6px; background: var(--field); overscroll-behavior: contain; }
  .map-viewport:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; }
  svg { display: block; max-width: none; flex: none; }
  .marker { font-size: 16px; font-weight: 650; }
  .number { font-size: 10px; font-weight: 500; font-variant-numeric: tabular-nums; }
  .favorite { font-size: 9px; fill: var(--text); }
  .available { fill: var(--accent); }
  .occupied { fill: var(--secondary); }
  .search-match .number, .search-example { font-weight: 750; text-decoration: underline; text-decoration-thickness: 2px; }
</style>
