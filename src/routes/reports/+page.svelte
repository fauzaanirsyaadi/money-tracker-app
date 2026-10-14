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
  import Amount from "$lib/components/Amount.svelte";
  import Card from "$lib/components/Card.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import { formatMonth, isoMonth, shiftMonth } from "$lib/dates";
  import { formatMinor } from "$lib/money";
  import { ui } from "$lib/session.svelte";

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
      const [year, mon] = month.split("-").map(Number);
      const last = new Date(year ?? 1970, mon ?? 1, 0).getDate();
      spending = await api.spending(`${month}-01`, `${month}-${String(last).padStart(2, "0")}`);
      expanded = new Set();
    } catch (err) {
      show(err);
    }
  }

  async function loadAll() {
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

  onMount(loadAll);

  $effect(() => {
    if (ui.version > 0) void loadAll();
  });
</script>

<PageHeader title="Laporan" />

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

{#if loading}
  <p class="muted">Memuat…</p>
{:else}
  <div class="grid">
    {#if worth}
      <Card>
        <h2>Kekayaan bersih</h2>
        <p class="hero">
          <Amount minor={worth.totalMinor} exponent={worth.exponent} currency={worth.currency} strong />
        </p>
        <ul class="plain" aria-label="Saldo per rekening">
          {#each worth.accounts as account (account.id)}
            <li class:archived={account.archived}>
              <span>{account.name}{account.archived ? " (diarsipkan)" : ""}</span>
              <Amount minor={account.balanceMinor} exponent={account.exponent} />
            </li>
          {/each}
        </ul>
      </Card>
    {/if}

    <Card>
      <h2>Pengeluaran per kategori</h2>
      <div class="month" role="group" aria-label="Bulan">
        <button type="button" onclick={() => go(-1)} aria-label="Bulan sebelumnya">
          <Icon name="chevron-left" />
        </button>
        <strong>{formatMonth(month)}</strong>
        <button type="button" onclick={() => go(1)} aria-label="Bulan berikutnya">
          <Icon name="chevron-right" />
        </button>
      </div>

      {#if spending && spending.categories.length > 0}
        <p class="total muted">
          Total <Amount minor={spending.totalMinor} exponent={spending.exponent} neutral strong />
        </p>
        <ul class="plain cats" aria-label="Pengeluaran per kategori">
          {#each spending.categories as category (category.categoryId)}
            <li>
              <button
                type="button"
                class="cat"
                aria-expanded={category.children.length > 0
                  ? expanded.has(category.categoryId)
                  : undefined}
                onclick={() => category.children.length > 0 && toggle(category.categoryId)}
              >
                <span class="top">
                  <strong>{category.name}</strong>
                  <span class="num">{formatMinor(category.spentMinor, spending.exponent)}</span>
                </span>
                <ProgressBar
                  percent={percentOf(category.spentMinor, spending.totalMinor)}
                  label={`${category.name} ${formatShare(category.shareBp)}`}
                />
                <span class="sub">
                  {formatShare(category.shareBp)} · {category.count} transaksi{category.children
                    .length > 0
                    ? expanded.has(category.categoryId)
                      ? " · sembunyikan"
                      : " · rincian"
                    : ""}
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
        <EmptyState title="Tidak ada pengeluaran" text="Belum ada pengeluaran pada bulan ini." />
      {/if}
    </Card>

    {#if trend}
      <Card>
        <h2>Tren {TREND_MONTHS} bulan</h2>
        <p class="legend">
          <span class="dot income"></span> Pemasukan
          <span class="dot expense"></span> Pengeluaran
        </p>
        <ul class="plain trend" aria-label="Tren bulanan">
          {#each trend.months as m (m.month)}
            <li>
              <span class="top">
                <strong>{formatMonth(m.month)}</strong>
                <Amount minor={m.netMinor} exponent={trend.exponent} plus />
              </span>
              <ProgressBar
                percent={percentOf(m.incomeMinor, trendMax)}
                tone="income"
                label={`Pemasukan ${formatMonth(m.month)}`}
              />
              <ProgressBar
                percent={percentOf(m.expenseMinor, trendMax)}
                tone="expense"
                label={`Pengeluaran ${formatMonth(m.month)}`}
              />
              <span class="sub">
                Masuk {formatMinor(m.incomeMinor, trend.exponent)} · Keluar
                {formatMinor(m.expenseMinor, trend.exponent)} · Kekayaan
                {formatMinor(m.netWorthMinor, trend.exponent)}
              </span>
            </li>
          {/each}
        </ul>
      </Card>
    {/if}
  </div>
{/if}

<style>
  .grid {
    display: grid;
    gap: var(--space-4);
  }
  @media (min-width: 960px) {
    .grid {
      grid-template-columns: 1fr 1fr;
      align-items: start;
    }
    .grid > :global(:last-child) {
      grid-column: 1 / -1;
    }
  }
  h2 {
    margin-bottom: var(--space-3);
  }
  .muted,
  .sub {
    color: var(--text-muted);
  }
  .sub {
    font-size: var(--fs-sm);
  }
  .hero {
    font-size: var(--fs-2xl);
    margin: 0 0 var(--space-3);
  }
  .plain,
  .children {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .plain > li {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-2) 0;
    border-top: 1px solid var(--border);
  }
  .plain.cats > li,
  .plain.trend > li {
    display: block;
  }
  .trend > li {
    display: grid;
    gap: var(--space-2);
  }
  .archived {
    opacity: 0.6;
  }
  .month {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-2);
  }
  .month button {
    display: grid;
    place-items: center;
    width: var(--tap);
    height: var(--tap);
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
  }
  .total {
    text-align: right;
    margin-bottom: var(--space-2);
  }
  .cat {
    all: unset;
    box-sizing: border-box;
    display: grid;
    gap: var(--space-2);
    width: 100%;
    min-height: var(--tap);
    cursor: pointer;
  }
  .top {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .children {
    padding-left: var(--space-4);
  }
  .children li {
    display: flex;
    justify-content: space-between;
    padding: var(--space-1) 0;
    color: var(--text-muted);
  }
  .legend {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    font-size: var(--fs-sm);
    color: var(--text-muted);
    margin-bottom: var(--space-2);
  }
  .dot {
    display: inline-block;
    width: 0.75rem;
    height: 0.75rem;
    border-radius: 0.2rem;
  }
  .dot.income {
    background: var(--income);
  }
  .dot.expense {
    background: var(--expense);
  }
  .error {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--danger-bg);
    color: var(--expense);
    margin-bottom: var(--space-3);
  }
</style>
