<script lang="ts">
  import { formatMinor } from "$lib/money";

  /**
   * A money value from a minor-unit string. `signed` colours negatives red and,
   * with `plus`, prefixes positives; `neutral` keeps the text colour.
   */
  let {
    minor,
    exponent,
    currency,
    plus = false,
    neutral = false,
    strong = false,
  }: {
    minor: string;
    exponent: number;
    currency?: string;
    plus?: boolean;
    neutral?: boolean;
    strong?: boolean;
  } = $props();

  const negative = $derived(minor.startsWith("-"));
  const text = $derived(formatMinor(minor, exponent));
</script>

<span class="amount num" class:negative={!neutral && negative} class:positive={!neutral && plus && !negative && minor !== "0"} class:strong>
  {#if currency}<span class="cur">{currency}</span>{" "}{/if}{plus && !negative && minor !== "0" ? "+" : ""}{text}
</span>

<style>
  .amount {
    white-space: nowrap;
  }
  .strong {
    font-weight: 700;
  }
  .negative {
    color: var(--expense);
  }
  .positive {
    color: var(--income);
  }
  .cur {
    font-size: 0.85em;
    color: var(--text-muted);
    font-weight: 500;
  }
</style>
