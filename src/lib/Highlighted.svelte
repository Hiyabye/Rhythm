<script lang="ts">
  let { text, query = '' }: { text: string; query?: string } = $props();
  // Longest first: the leftmost alternative wins, so `컴` before `컴퓨터` would mark only the first syllable.
  let terms = $derived(query.trim().split(/\s+/).filter(Boolean).sort((a, b) => b.length - a.length).map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
  let parts = $derived(terms.length ? text.split(new RegExp(`(${terms.join('|')})`, 'giu')) : [text]);
</script>

{#each parts as part, index}{#if index % 2 === 1}<mark>{part}</mark>{:else}{part}{/if}{/each}
