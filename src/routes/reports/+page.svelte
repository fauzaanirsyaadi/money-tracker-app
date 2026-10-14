<script lang="ts">
  import { onMount } from "svelte";

  import {
    ApiError,
    reports as api,
    type NetWorth,
    type SpendingReport,
    type Trend,
  } from "$lib/api";
  import { formatShare, maxOf, percentOf } from "$lib/charts";
  import { formatMonth, isoMonth, shiftMonth } from "$lib/dates";
  import { formatMinor } from "$lib/money";

  const TREND_MONTHS = 6;

  let month = $state(isoMonth());
  let spending = $state<SpendingReport | null>(null);
  let trend = $state<Trend | null>(null);
  let worth = $state<NetWorth | null>(null);
  let expanded = $state<Set<number>>(new Set());
  let loading = $state(true);
  let error = $state("");

  const trendMax = $derived(
    trend ? maxOf(trend.months.flatMap((m) => [m.incomeMinor, m.expenseMinor])) : "0",
  );

  function show(err: unknown) {
    error =
      err instanceof ApiError && err.code === "unavailable"
        ? "Penyimpanan tidak tersedia di perangkat ini."
        : "Terjadi kesalahan.";
  }

  async function loadSpending() {
    try {
      // `YYYY-MM-31` is not always a date; the backend takes the real bounds.
      const [year, mon] = month.split("-").map(Number);
      const last = new Date(year ?? 1970, mon ?? 1, 0).getDate();
      spending = await api.spending(`${month}-01`, `${month}-${String(last).padStart(2, "0")}`);
      expanded = new Set();
    } catch (err) {
      show(err);
    }
  }

  async function go(delta: number) {
    month = shiftMonth(month, delta);
    await loadSpending();
  }

  function toggle(id: number) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

  onMount(async () => {
    try {
      const current = isoMonth();
      [trend, worth] = await Promise.all([
        api.trend(shiftMonth(current, -(TREND_MONTHS - 1)), current),
        api.netWorth(),
      ]);
      error = "";
    } catch (err) {
      show(err);
    }
    await loadSpending();
    loading = false;
  });
</script>

<main>
  <h1>Laporan</h1>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if loading}
    <p>Memuat…</p>
  {:else}
    {#if worth}
      <section aria-label="Kekayaan bersih">
        <h2>Kekayaan bersih</h2>
        <p class="big" class:negative={worth.totalMinor.startsWith("-")}>
          {worth.currency} {formatMinor(worth.totalMinor, worth.exponent)}
        </p>
        <ul class="plain">
          {#each worth.accounts as account (account.id)}
            <li class:archived={account.archived}>
              <span>{account.name}{account.archived ? " (diarsipkan)" : ""}</span>
              <span class="num" class:negative={account.balanceMinor.startsWith("-")}>
                {formatMinor(account.balanceMinor, account.exponent)}
              </span>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <section aria-label="Pengeluaran per kategori">
      <h2>Pengeluaran per kategori</h2>
      <div class="month" role="group" aria-label="Bulan">
        <button type="button" onclick={() => go(-1)} aria-label="Bulan sebelumnya">‹</button>
        <strong>{formatMonth(month)}</strong>
        <button type="button" onclick={() => go(1)} aria-label="Bulan berikutnya">›</button>
      </div>

      {#if spending && spending.categories.length > 0}
        <p class="total">
          Total: <strong>{formatMinor(spending.totalMinor, spending.exponent)}</strong>
        </p>
        <ul class="plain">
          {#each spending.categories as category (category.categoryId)}
            <li>
              <button
                type="button"
                class="row"
                aria-expanded={category.children.length > 0 ? expanded.has(category.categoryId) : undefined}
                onclick={() => category.children.length > 0 && toggle(category.categoryId)}
              >
                <span class="head">
                  <strong>{category.name}</strong>
                  <span class="num">{formatMinor(category.spentMinor, spending.exponent)}</span>
                </span>
                <span
                  class="bar"
                  role="img"
                  aria-label={`${formatShare(category.shareBp)} dari total`}
                >
                  <span style:width={`${percentOf(category.spentMinor, spending.totalMinor)}%`}></span>
                </span>
                <span class="sub">
                  {formatShare(category.shareBp)} · {category.count} transaksi
                  {#if category.children.length > 0}· {expanded.has(category.categoryId) ? "sembunyikan" : "rincian"}{/if}
                </span>
              </button>
              {#if expanded.has(category.categoryId)}
                <ul class="children">
                  {#each category.children as child (child.categoryId)}
                    <li>
                      <span>{child.name} · {child.count}×</span>
                      <span class="num">{formatMinor(child.spentMinor, spending.exponent)}</span>
                    </li>
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p>Tidak ada pengeluaran pada bulan ini.</p>
      {/if}
    </section>

    {#if trend}
      <section aria-label="Tren bulanan">
        <h2>Tren {TREND_MONTHS} bulan</h2>
        <p class="legend">
          <span class="swatch income"></span> Pemasukan
          <span class="swatch expense"></span> Pengeluaran
        </p>
        <ul class="plain">
          {#each trend.months as m (m.month)}
            <li class="trend">
              <div class="head">
                <strong>{formatMonth(m.month)}</strong>
                <span class="num" class:negative={m.netMinor.startsWith("-")}>
                  {m.netMinor.startsWith("-") ? "" : "+"}{formatMinor(m.netMinor, trend.exponent)}
                </span>
              </div>
              <span class="bar income" role="img" aria-label={`Pemasukan ${formatMinor(m.incomeMinor, trend.exponent)}`}>
                <span style:width={`${percentOf(m.incomeMinor, trendMax)}%`}></span>
              </span>
              <span class="bar expense" role="img" aria-label={`Pengeluaran ${formatMinor(m.expenseMinor, trend.exponent)}`}>
                <span style:width={`${percentOf(m.expenseMinor, trendMax)}%`}></span>
              </span>
              <span class="sub">
                Masuk {formatMinor(m.incomeMinor, trend.exponent)} · Keluar
                {formatMinor(m.expenseMinor, trend.exponent)} · Kekayaan
                {formatMinor(m.netWorthMinor, trend.exponent)}
              </span>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
  {/if}
</main>

<style>
  main {
    max-width: 36rem;
    margin: 0 auto;
    padding: 1rem;
  }
  section {
    margin-bottom: 2rem;
  }
  .plain,
  .children {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .plain > li {
    padding: 0.5rem 0;
    border-bottom: 1px solid color-mix(in srgb, currentColor 15%, transparent);
    display: flex;
    justify-content: space-between;
    gap: 1rem;
  }
  .plain > li:has(.row),
  .plain > li.trend {
    display: block;
  }
  .children {
    padding-left: 1rem;
  }
  .children li {
    display: flex;
    justify-content: space-between;
    padding: 0.25rem 0;
    opacity: 0.85;
  }
  .big {
    font-size: 1.75rem;
    font-weight: 600;
    margin: 0.25rem 0 0.75rem;
    font-variant-numeric: tabular-nums;
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  .negative {
    color: #b3261e;
  }
  .archived {
    opacity: 0.6;
  }
  .month {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.5rem;
  }
  .total {
    text-align: right;
    margin: 0.25rem 0;
  }
  .row {
    all: unset;
    display: grid;
    gap: 0.3rem;
    width: 100%;
    cursor: pointer;
    min-height: 2.75rem;
  }
  .head {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
  }
  .sub {
    font-size: 0.85em;
    opacity: 0.75;
  }
  .bar {
    display: block;
    height: 0.6rem;
    border-radius: 0.3rem;
    background: color-mix(in srgb, currentColor 12%, transparent);
    overflow: hidden;
  }
  .bar > span {
    display: block;
    height: 100%;
    background: #3949ab;
  }
  .bar.income > span {
    background: #2e7d32;
  }
  .bar.expense > span {
    background: #b3261e;
  }
  .trend {
    display: grid !important;
    gap: 0.3rem;
  }
  .legend {
    display: flex;
    gap: 0.75rem;
    align-items: center;
    font-size: 0.9em;
  }
  .swatch {
    display: inline-block;
    width: 0.75rem;
    height: 0.75rem;
    border-radius: 0.2rem;
  }
  .swatch.income {
    background: #2e7d32;
  }
  .swatch.expense {
    background: #b3261e;
  }
  .error {
    padding: 0.5rem 0.75rem;
    border-radius: 0.5rem;
    background: color-mix(in srgb, #b3261e 15%, transparent);
  }
  button {
    font: inherit;
    padding: 0.5rem 0.75rem;
    min-height: 2.75rem;
  }
</style>
