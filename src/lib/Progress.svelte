<script lang="ts">
  import { cubicOut } from 'svelte/easing';
  import { Tween } from 'svelte/motion';
  import { quick } from './motion';

  // A bar that glides between status polls instead of jumping. Without a value it is
  // indeterminate. The native <progress> stays underneath for assistive tech and the
  // enclosing <label>; the drawn track and fill are decoration.
  let { value, max }: { value?: number; max: number } = $props();
  const ratio = $derived(value === undefined ? null : Math.min(1, Math.max(0, value / (max || 1))));
  const shown = Tween.of(() => ratio ?? 0, { duration: () => quick(400), easing: cubicOut });
</script>

<span class="progress" class:indeterminate={ratio === null} class:done={ratio === 1}>
  {#if ratio === null}<progress {max}></progress>{:else}<progress {max} {value}></progress>{/if}
  <span class="fill" aria-hidden="true" style:width={ratio === null ? undefined : `${shown.current * 100}%`}></span>
</span>

<style>
  .progress {
    position: relative;
    display: block;
    width: 100%;
    height: 6px;
    overflow: hidden;
    border-radius: 3px;
    background: var(--line);
  }
  progress {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    pointer-events: none;
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: inherit;
    background-color: var(--accent-fill);
    /* A slow sheen says the transfer is still alive between polls. */
    background-image: linear-gradient(90deg, transparent, rgb(255 255 255 / .32), transparent);
    background-size: 48px 100%;
    background-repeat: no-repeat;
    background-position: -48px 0;
    animation: progress-sheen 1.8s cubic-bezier(.4, 0, .2, 1) infinite;
  }
  .done .fill {
    background-image: none;
    animation: none;
  }
  .indeterminate .fill {
    width: 32%;
    background-image: none;
    animation: progress-sweep 1.3s cubic-bezier(.65, 0, .35, 1) infinite;
  }
  @keyframes progress-sheen {
    to { background-position: calc(100% + 48px) 0; }
  }
  @keyframes progress-sweep {
    from { transform: translateX(-100%); }
    to { transform: translateX(312%); }
  }
  /* The global rule stops the sweep; show a calm full track instead of a frozen segment. */
  @media (prefers-reduced-motion: reduce) {
    .indeterminate .fill { width: 100%; opacity: .45; }
  }
</style>
