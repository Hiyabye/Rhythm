import { flip as baseFlip } from 'svelte/animate';
import { cubicOut } from 'svelte/easing';
import { prefersReducedMotion } from 'svelte/motion';
import * as base from 'svelte/transition';

// Svelte motion runs through the Web Animations API, which the app.css reduced-motion rule
// cannot reach, so every Svelte transition and tween in the app goes through these wrappers.
export function quick(duration: number) {
  return prefersReducedMotion.current ? 0 : duration;
}

export function fade(node: Element, { duration = 140, ...params }: base.FadeParams = {}) {
  return base.fade(node, { easing: cubicOut, ...params, duration: quick(duration) });
}

export function fly(node: Element, { duration = 180, y = 6, ...params }: base.FlyParams = {}) {
  return base.fly(node, { easing: cubicOut, y, ...params, duration: quick(duration) });
}

export function slide(node: Element, { duration = 180, ...params }: base.SlideParams = {}) {
  return base.slide(node, { easing: cubicOut, ...params, duration: quick(duration) });
}

export function flip(node: Element, positions: { from: DOMRect; to: DOMRect }, { duration = 200, ...params }: Parameters<typeof baseFlip>[2] = {}) {
  return baseFlip(node, positions, { easing: cubicOut, ...params, duration: quick(Number(duration)) });
}
