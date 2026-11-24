<script lang="ts">
  // Activity indicator for work with no measurable progress. Decorative: the text beside it
  // carries the state.
  let { size = 14 }: { size?: number } = $props();
</script>

<svg class="spinner" width={size} height={size} viewBox="0 0 16 16" fill="none" aria-hidden="true">
  <circle cx="8" cy="8" r="6.25" stroke="currentColor" stroke-width="1.75" opacity=".22" />
  <path d="M8 1.75a6.25 6.25 0 0 1 6.25 6.25" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" />
</svg>

<style>
  .spinner {
    flex: none;
    animation: spin .8s linear infinite;
  }
  /* The global rule stops the rotation; a full faint ring reads as waiting, not frozen. */
  @media (prefers-reduced-motion: reduce) {
    .spinner path { opacity: .5; }
  }
</style>
