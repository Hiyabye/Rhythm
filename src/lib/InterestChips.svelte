<script lang="ts">
  import type { PersonalInterest } from './types';
  import { interestSuggestions, type InterestSuggestion } from './interest-suggestions';
  import { fade, flip } from './motion';

  // Spotify-style picker: a chosen chip saves as an explicit interest and its related chips
  // open right after it. Openness is derived from the selection, so nothing else is stored.
  let { interests, busy, onadd, onremove }: {
    interests: PersonalInterest[];
    busy: boolean;
    onadd: (name: string) => Promise<boolean>;
    onremove: (interest: PersonalInterest) => Promise<boolean>;
  } = $props();

  const fold = (value: string) => value.trim().toLowerCase();
  const terms = (interest: PersonalInterest) => [interest.name, ...interest.aliases].map(fold);
  let positive = $derived(interests.filter((interest) => interest.origin === 'explicit' && interest.polarity === 'positive'));
  let negative = $derived(interests.filter((interest) => interest.origin === 'explicit' && interest.polarity === 'negative'));

  const chosen = (label: string) => positive.find((interest) => terms(interest).includes(fold(label)));
  const blocked = (label: string) => negative.some((interest) => terms(interest).includes(fold(label)));
  const isOpen = (node: InterestSuggestion): boolean => !!node.children
    && (!!chosen(node.label) || node.children.some(isOpen));

  type Row = { node: InterestSuggestion; depth: number };
  function flatten(nodes: InterestSuggestion[], depth: number): Row[] {
    return nodes.flatMap((node) => [
      { node, depth },
      ...(node.children && isOpen(node) ? flatten(node.children, depth + 1) : []),
    ]);
  }
  let rows = $derived(flatten(interestSuggestions, 0));

  // aria-disabled instead of disabled while saving, so the pressed chip keeps keyboard focus.
  async function toggle(label: string) {
    if (busy || blocked(label)) return;
    const current = chosen(label);
    if (current) await onremove(current);
    else await onadd(label);
  }
</script>

<ul class="suggest-list" aria-label="추천 관심 주제">
  {#each rows as { node, depth } (node.label)}
    {@const picked = !!chosen(node.label)}
    {@const off = blocked(node.label)}
    <li animate:flip in:fade out:fade>
      <button
        type="button"
        class="suggest-chip"
        class:picked
        class:child={depth > 0}
        aria-pressed={picked}
        aria-expanded={node.children ? isOpen(node) : undefined}
        aria-disabled={busy || off}
        title={off ? '관심 없음으로 설정된 주제입니다. 아래 목록에서 방향을 바꿀 수 있습니다.' : undefined}
        onclick={() => void toggle(node.label)}
      >{node.label}</button>
    </li>
  {/each}
</ul>

<style>
  .suggest-list {
    flex: 1;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .suggest-chip {
    min-height: 26px;
    padding: 2px 11px;
    border: 1px solid var(--line-strong);
    border-radius: 13px;
    background: var(--surface);
    font-size: 12px;
    color: var(--text);
  }
  .suggest-chip.child {
    border-style: dashed;
  }
  .suggest-chip:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .suggest-chip.picked {
    border-style: solid;
    border-color: var(--accent);
    background: var(--soft);
    color: var(--accent);
    font-weight: 600;
  }
  .suggest-chip[aria-disabled='true'] {
    cursor: default;
    opacity: .6;
  }
  .suggest-chip:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
