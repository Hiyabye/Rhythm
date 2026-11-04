<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import Popover from './Popover.svelte';

  // A row's explanation, kept out of the layout until asked for.
  let { label, children }: { label: string; children: Snippet } = $props();
  const id = $props.id();
</script>

<button type="button" class="hint" popovertarget={id} aria-label={`${label} 설명`}><Icon name="info" size={15} /></button>
<Popover {id} label={`${label} 설명`}>{@render children()}</Popover>
