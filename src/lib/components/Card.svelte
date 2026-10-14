<script lang="ts">
  import type { Snippet } from "svelte";

  let { children, tone = "plain" }: { children: Snippet; tone?: "plain" | "warning" | "danger" } =
    $props();
</script>

<section class="card {tone}">
  {@render children()}
</section>

<style>
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: var(--space-4);
  }
  .warning {
    background: var(--warning-bg);
    border-color: transparent;
  }
  .danger {
    background: var(--danger-bg);
    border-color: transparent;
  }
</style>
