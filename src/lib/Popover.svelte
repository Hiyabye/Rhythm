<script lang="ts">
  import type { Snippet } from 'svelte';

  // A light-dismiss popover anchored to the button whose `popovertarget` is `id`.
  // The browser handles Escape, outside clicks and returning focus to that button.
  let { id, label, wide = false, onclose, children }: { id: string; label: string; wide?: boolean; onclose?: () => void; children: Snippet } = $props();
  let panel: HTMLDivElement;

  function anchor() {
    return document.querySelector<HTMLElement>(`[popovertarget="${id}"]`);
  }

  function place() {
    const target = anchor();
    if (!target) return;
    const gutter = 16;
    const box = target.getBoundingClientRect();
    const width = Math.min(wide ? 360 : 300, window.innerWidth - gutter * 2);
    const left = Math.max(gutter, Math.min(box.left + box.width / 2 - width / 2, window.innerWidth - width - gutter));
    panel.style.width = `${width}px`;
    panel.style.left = `${left}px`;
    panel.style.top = `${box.bottom + 6}px`;
    const height = panel.offsetHeight;
    if (height && box.bottom + 6 + height > window.innerHeight - gutter && box.top - 6 - height >= gutter) {
      panel.style.top = `${box.top - 6 - height}px`;
    }
  }

  function close() {
    if (panel?.matches(':popover-open')) panel.hidePopover();
  }

  function toggled(event: ToggleEvent) {
    if (event.newState === 'open') {
      place();
      // A fixed panel would drift from its anchor when the page scrolls.
      window.addEventListener('scroll', close, { passive: true, once: true });
      window.addEventListener('resize', close, { once: true });
      panel.querySelector<HTMLElement>('[data-autofocus]')?.focus();
    } else {
      window.removeEventListener('scroll', close);
      window.removeEventListener('resize', close);
      onclose?.();
    }
  }

  $effect(() => close);
</script>

<div {id} bind:this={panel} popover="auto" class="popover" class:wide role="dialog" aria-label={label} ontoggle={toggled}>
  {@render children()}
</div>
