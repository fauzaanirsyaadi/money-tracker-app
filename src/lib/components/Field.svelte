<script lang="ts">
  import type { Snippet } from "svelte";

  let { label, hint, children }: { label: string; hint?: string; children: Snippet } = $props();
</script>

<label class="field">
  <span class="label">{label}</span>
  {@render children()}
  {#if hint}<span class="hint">{hint}</span>{/if}
</label>

<style>
  .field {
    display: grid;
    gap: var(--space-1);
  }
  .label {
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--text-muted);
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .field :global(input),
  .field :global(select) {
    min-height: var(--tap);
    padding: 0 var(--space-3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--surface);
    width: 100%;
  }
</style>
