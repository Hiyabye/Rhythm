<script lang="ts">
  import { cubicOut } from 'svelte/easing';
  import { Tween } from 'svelte/motion';
  import { quick } from './motion';

  // A download bar that glides between status polls instead of jumping.
  let { value, max }: { value: number; max: number } = $props();
  const shown = Tween.of(() => value, { duration: () => quick(400), easing: cubicOut });
</script>

<progress {max} value={shown.current}></progress>
