<script lang="ts">
  import type { Snippet } from "svelte";

  let { title, text, children }: { title: string; text?: string; children?: Snippet } = $props();
</script>

<div class="empty">
  <strong>{title}</strong>
  {#if text}<p>{text}</p>{/if}
  {#if children}{@render children()}{/if}
</div>

<style>
  .empty {
    display: grid;
    gap: var(--space-2);
    justify-items: center;
    text-align: center;
    padding: var(--space-6) var(--space-4);
    color: var(--text-muted);
    background: var(--surface);
    border: 1px dashed var(--border);
    border-radius: var(--radius-lg);
  }
  strong {
    color: var(--text);
  }
</style>
