<script lang="ts" generics="T">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";

  import { nearEnd, visibleRange } from "$lib/virtual";

  /**
   * Window-scrolled list that renders only the rows in view. Rows have a fixed
   * height of `rowRem` (so it scales with the user's font size) and must not
   * grow; long text should truncate.
   */
  let {
    items,
    rowRem = 4.5,
    key,
    row,
    onEnd,
    label,
  }: {
    items: T[];
    rowRem?: number;
    key: (item: T) => string | number;
    row: Snippet<[T]>;
    onEnd?: () => void;
    label: string;
  } = $props();

  let host = $state<HTMLElement>();
  let rowPx = $state(72);
  let scrollTop = $state(0);
  let viewport = $state(800);
  let frame = 0;

  const range = $derived(visibleRange(scrollTop, viewport, rowPx, items.length));
  const slice = $derived(items.slice(range.start, range.end));

  function measure() {
    if (!host) return;
    const rem = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
    rowPx = rem * rowRem;
    viewport = window.innerHeight;
    // Distance the list's top edge has scrolled above the viewport.
    scrollTop = Math.max(0, -host.getBoundingClientRect().top);
    if (nearEnd(scrollTop, viewport, rowPx, items.length)) onEnd?.();
  }

  function schedule() {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      measure();
    });
  }

  onMount(() => {
    measure();
    window.addEventListener("scroll", schedule, { passive: true });
    window.addEventListener("resize", schedule);
    return () => {
      window.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
      if (frame) cancelAnimationFrame(frame);
    };
  });

  // The list can change length (filter, load more) without any scroll event.
  $effect(() => {
    void items.length;
    schedule();
  });
</script>

<ul
  bind:this={host}
  aria-label={label}
  style:padding-top={`${range.start * rowPx}px`}
  style:padding-bottom={`${Math.max(0, items.length - range.end) * rowPx}px`}
>
  {#each slice as item (key(item))}
    <li style:height={`${rowRem}rem`}>
      {@render row(item)}
    </li>
  {/each}
</ul>

<style>
  ul {
    list-style: none;
    margin: 0;
  }
  li {
    overflow: hidden;
    border-bottom: 1px solid var(--border);
  }
</style>
