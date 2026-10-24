<script lang="ts">
  let { text, query = '' }: { text: string; query?: string } = $props();
  let terms = $derived(query.trim().split(/\s+/).filter(Boolean).map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
  let parts = $derived(terms.length ? text.split(new RegExp(`(${terms.join('|')})`, 'giu')) : [text]);
</script>

{#each parts as part, index}{#if index % 2 === 1}<mark>{part}</mark>{:else}{part}{/if}{/each}
