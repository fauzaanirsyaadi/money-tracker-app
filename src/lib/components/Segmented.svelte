<script lang="ts" generics="T extends string">
  type Option = { value: T; label: string; disabled?: boolean };

  let {
    options,
    value = $bindable(),
    label,
    onchange,
  }: {
    options: Option[];
    value: T;
    label: string;
    onchange?: (value: T) => void;
  } = $props();
</script>

<div class="seg" role="radiogroup" aria-label={label}>
  {#each options as option (option.value)}
    <button
      type="button"
      role="radio"
      aria-checked={value === option.value}
      disabled={option.disabled}
      onclick={() => {
        value = option.value;
        onchange?.(option.value);
      }}
    >
      {option.label}
    </button>
  {/each}
</div>

<style>
  .seg {
    display: flex;
    padding: 0.25rem;
    gap: 0.25rem;
    background: var(--surface-2);
    border-radius: var(--radius-md);
  }
  button {
    flex: 1;
    min-height: 2.5rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    font-weight: 600;
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  button[aria-checked="true"] {
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow);
  }
  button:disabled {
    opacity: 0.45;
  }
</style>
