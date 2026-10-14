<script lang="ts">
  import { onMount } from "svelte";

  import {
    accounts as accountsApi,
    ApiError,
    categories as categoriesApi,
    transactions as api,
    type Account,
    type Category,
    type Cursor,
    type Transaction,
    type TransactionFilter,
  } from "$lib/api";
  import Amount from "$lib/components/Amount.svelte";
  import Button from "$lib/components/Button.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import Field from "$lib/components/Field.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Sheet from "$lib/components/Sheet.svelte";
  import VirtualList from "$lib/components/VirtualList.svelte";
  import { formatDate } from "$lib/dates";
  import { dataChanged, notify, openQuickAdd, ui } from "$lib/session.svelte";

  const PAGE = 50;

  let accountList = $state<Account[]>([]);
  let categoryList = $state<Category[]>([]);
  let items = $state<Transaction[]>([]);
  let next = $state<Cursor | null>(null);
  let loading = $state(true);
  let loadingMore = false;
  let error = $state("");

  let search = $state("");
  let filterAccount = $state("");
  let filterCategory = $state("");
  let filterKind = $state("");
  let filterFrom = $state("");
  let filterTo = $state("");

  let filterOpen = $state(false);
  let detailOpen = $state(false);
  let detailSheet = $state<{ close: () => void }>();
  let transfer = $state<Transaction | null>(null);

  const activeFilters = $derived(
    [filterAccount, filterCategory, filterKind, filterFrom, filterTo].filter(Boolean).length,
  );

  function currentFilter(): TransactionFilter {
    const filter: TransactionFilter = {};
    if (search.trim()) filter.search = search.trim();
    if (filterAccount) filter.accountId = Number(filterAccount);
    if (filterCategory) filter.categoryId = Number(filterCategory);
    if (filterKind) filter.kind = filterKind as TransactionFilter["kind"];
    if (filterFrom) filter.from = filterFrom;
    if (filterTo) filter.to = filterTo;
    return filter;
  }

  function show(err: unknown) {
    error =
      err instanceof ApiError && err.code === "unavailable"
        ? "Penyimpanan tidak tersedia di perangkat ini."
        : "Terjadi kesalahan.";
  }

  async function reload() {
    try {
      const page = await api.list(currentFilter(), null, PAGE);
      items = page.items;
      next = page.next;
      error = "";
    } catch (err) {
      show(err);
    } finally {
      loading = false;
    }
  }

  async function more() {
    if (!next || loadingMore) return;
    loadingMore = true;
    try {
      const page = await api.list(currentFilter(), next, PAGE);
      items = [...items, ...page.items];
      next = page.next;
    } catch (err) {
      show(err);
    } finally {
      loadingMore = false;
    }
  }

  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  function searchChanged() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(reload, 300);
  }

  function clearFilters() {
    filterAccount = filterCategory = filterKind = filterFrom = filterTo = "";
    void reload();
  }

  function describe(t: Transaction): string {
    if (t.kind === "transfer_out") return "Transfer keluar";
    if (t.kind === "transfer_in") return "Transfer masuk";
    return t.categoryName ?? "";
  }

  function open(t: Transaction) {
    if (t.kind === "income" || t.kind === "expense") {
      openQuickAdd(t);
    } else {
      transfer = t;
      detailOpen = true;
    }
  }

  async function removeTransfer() {
    if (!transfer) return;
    try {
      await api.remove(transfer.id);
      detailSheet?.close();
      notify("Transfer dihapus (kedua sisi).");
      dataChanged();
    } catch (err) {
      show(err);
    }
  }

  onMount(async () => {
    try {
      [accountList, categoryList] = await Promise.all([
        accountsApi.list(true),
        categoriesApi.list(true),
      ]);
    } catch (err) {
      show(err);
    }
  });

  $effect(() => {
    void ui.version;
    void reload();
  });
</script>

<PageHeader title="Transaksi" />

<div class="tools">
  <label class="search">
    <span class="visually-hidden">Cari catatan atau kategori</span>
    <Icon name="search" size={20} />
    <input
      type="search"
      placeholder="Cari catatan atau kategori"
      bind:value={search}
      oninput={searchChanged}
    />
  </label>
  <button type="button" class="filter-btn" onclick={() => (filterOpen = true)} aria-label="Filter">
    <Icon name="filter" />
    {#if activeFilters > 0}<span class="badge">{activeFilters}</span>{/if}
  </button>
</div>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

{#if loading}
  <p class="muted">Memuat…</p>
{:else if items.length === 0}
  <EmptyState
    title={activeFilters > 0 || search ? "Tidak ada yang cocok" : "Belum ada transaksi"}
    text={activeFilters > 0 || search
      ? "Ubah pencarian atau filter."
      : "Ketuk tombol + untuk mencatat transaksi pertama."}
  />
{:else}
  <div class="list">
    <VirtualList
      {items}
      label="Daftar transaksi"
      key={(t: Transaction) => t.id}
      onEnd={more}
    >
      {#snippet row(t: Transaction)}
        <button type="button" class="tx" onclick={() => open(t)}>
          <span class="meta">
            <strong>{describe(t)}</strong>
            <small>
              {formatDate(t.occurredOn)} · {t.accountName}{t.note ? ` · ${t.note}` : ""}{t.recurring ? " · berulang" : ""}
            </small>
          </span>
          <Amount minor={t.amountMinor} exponent={t.exponent} plus strong />
        </button>
      {/snippet}
    </VirtualList>
    {#if next}
      <p class="muted more" aria-live="polite">Memuat lebih banyak…</p>
    {/if}
  </div>
{/if}

<Sheet bind:open={filterOpen} title="Filter">
  <div class="form">
    <Field label="Rekening">
      <select bind:value={filterAccount} onchange={reload}>
        <option value="">Semua rekening</option>
        {#each accountList as account (account.id)}
          <option value={String(account.id)}>{account.name}</option>
        {/each}
      </select>
    </Field>
    <Field label="Jenis">
      <select bind:value={filterKind} onchange={reload}>
        <option value="">Semua jenis</option>
        <option value="expense">Pengeluaran</option>
        <option value="income">Pemasukan</option>
        <option value="transfer">Transfer</option>
      </select>
    </Field>
    <Field label="Kategori">
      <select bind:value={filterCategory} onchange={reload}>
        <option value="">Semua kategori</option>
        {#each categoryList as category (category.id)}
          <option value={String(category.id)}>
            {category.parentId === null ? "" : "↳ "}{category.name}
          </option>
        {/each}
      </select>
    </Field>
    <div class="dates">
      <Field label="Dari">
        <input type="date" bind:value={filterFrom} onchange={reload} />
      </Field>
      <Field label="Sampai">
        <input type="date" bind:value={filterTo} onchange={reload} />
      </Field>
    </div>
    <Button variant="ghost" onclick={clearFilters}>Hapus semua filter</Button>
  </div>
</Sheet>

<Sheet bind:this={detailSheet} bind:open={detailOpen} title="Transfer">
  {#if transfer}
    <div class="form">
      <p>
        <strong>{describe(transfer)}</strong> · {formatDate(transfer.occurredOn)} ·
        {transfer.accountName}
      </p>
      <p><Amount minor={transfer.amountMinor} exponent={transfer.exponent} plus strong /></p>
      {#if transfer.note}<p class="muted">{transfer.note}</p>{/if}
      <p class="muted">
        Transfer tidak dapat diubah. Hapus lalu buat ulang bila ada yang salah; kedua sisinya
        terhapus bersama.
      </p>
      <Button variant="danger" block onclick={removeTransfer}>Hapus transfer</Button>
    </div>
  {/if}
</Sheet>

<style>
  .tools {
    display: flex;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }
  .search {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-3);
    min-height: var(--tap);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    color: var(--text-muted);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    outline: none;
  }
  .filter-btn {
    position: relative;
    display: grid;
    place-items: center;
    width: var(--tap);
    min-height: var(--tap);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .badge {
    position: absolute;
    top: -0.35rem;
    right: -0.35rem;
    min-width: 1.25rem;
    height: 1.25rem;
    display: grid;
    place-items: center;
    padding: 0 0.25rem;
    border-radius: 999px;
    background: var(--primary);
    color: var(--on-primary);
    font-size: var(--fs-xs);
    font-weight: 700;
  }
  .list {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }
  .tx {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    width: 100%;
    height: 100%;
    padding: 0 var(--space-4);
    border: 0;
    background: transparent;
    text-align: left;
  }
  .meta {
    display: grid;
    min-width: 0;
  }
  .meta strong,
  .meta small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  small,
  .muted {
    color: var(--text-muted);
  }
  .more {
    padding: var(--space-3);
    text-align: center;
  }
  .form {
    display: grid;
    gap: var(--space-4);
  }
  .dates {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }
  .error {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--danger-bg);
    color: var(--expense);
    margin-bottom: var(--space-3);
  }
</style>
