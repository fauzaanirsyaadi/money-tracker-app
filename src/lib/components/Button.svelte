<script lang="ts">
  import type { Snippet } from "svelte";

  type Props = {
    variant?: "primary" | "secondary" | "ghost" | "danger";
    type?: "button" | "submit";
    disabled?: boolean;
    block?: boolean;
    label?: string;
    /** Id of the form a submit button belongs to (when rendered outside it). */
    form?: string;
    onclick?: (event: MouseEvent) => void;
    children: Snippet;
  };

  let {
    variant = "secondary",
    type = "button",
    disabled = false,
    block = false,
    label,
    form,
    onclick,
    children,
  }: Props = $props();
</script>

<button class="btn {variant}" class:block {type} {form} {disabled} aria-label={label} {onclick}>
  {@render children()}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    min-height: var(--tap);
    min-width: var(--tap);
    padding: 0 var(--space-4);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font-weight: 600;
    font-size: var(--fs-md);
    transition: background-color 0.12s;
  }
  .btn:active:not(:disabled) {
    background: var(--surface-2);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .block {
    width: 100%;
  }
  .primary {
    background: var(--primary);
    border-color: var(--primary);
    color: var(--on-primary);
  }
  .primary:active:not(:disabled) {
    filter: brightness(0.92);
    background: var(--primary);
  }
  .ghost {
    background: transparent;
    border-color: transparent;
  }
  .danger {
    color: var(--expense);
  }
</style>
