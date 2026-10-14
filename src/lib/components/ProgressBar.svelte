<script lang="ts">
  /** `percent` is 0..100; `tone` picks the colour. */
  let {
    percent,
    tone = "primary",
    label,
  }: { percent: number; tone?: "primary" | "income" | "expense" | "warning"; label: string } =
    $props();
</script>

<div
  class="bar {tone}"
  role="progressbar"
  aria-label={label}
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={Math.round(percent)}
>
  <span style:width={`${Math.max(0, Math.min(100, percent))}%`}></span>
</div>

<style>
  .bar {
    height: 0.5rem;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  span {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--primary);
    transition: width 0.2s;
  }
  .income span {
    background: var(--income);
  }
  .expense span {
    background: var(--expense);
  }
  .warning span {
    background: var(--warning);
  }
</style>
