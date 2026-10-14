<script lang="ts">
  import type { Snippet } from "svelte";

  let { title, children }: { title: string; children?: Snippet } = $props();
</script>

<div class="head">
  <h1>{title}</h1>
  {#if children}<div class="extra">{@render children()}</div>{/if}
</div>

<style>
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }
  .extra {
    display: flex;
    gap: var(--space-2);
  }
</style>
