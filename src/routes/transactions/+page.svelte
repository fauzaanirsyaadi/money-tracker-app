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
    type FlowKind,
    type Transaction,
    type TransactionFilter,
  } from "$lib/api";
  import { formatDate, isoDate } from "$lib/dates";
  import { formatMinor, parseAmount } from "$lib/money";

  type Mode = FlowKind | "transfer";

  const MODE_LABEL: Record<Mode, string> = {
    expense: "Pengeluaran",
    income: "Pemasukan",
    transfer: "Transfer",
  };

  const ERROR_TEXT: Record<string, string> = {
    unavailable: "Penyimpanan tidak tersedia di perangkat ini.",
    invalid_input: "Isian tidak valid.",
    not_found: "Data tidak ditemukan.",
    name_in_use: "Nama sudah dipakai.",
    archived: "Rekening atau kategori sudah diarsipkan.",
    currency_mismatch: "Mata uang tidak sesuai.",
    amount_out_of_range: "Jumlah terlalu besar.",
    internal: "Terjadi kesalahan.",
  };

  let accountList = $state<Account[]>([]);
  let categoryList = $state<Category[]>([]);
  let items = $state<Transaction[]>([]);
  let next = $state<Cursor | null>(null);
  let loading = $state(true);
  let loadingMore = $state(false);
  let error = $state("");

  // filters
  let search = $state("");
  let filterAccount = $state("");
  let filterCategory = $state("");
  let filterKind = $state("");
  let filterFrom = $state("");
  let filterTo = $state("");

  // form
  let editingId = $state<number | null>(null);
  let mode = $state<Mode>("expense");
  let accountId = $state("");
  let toAccountId = $state("");
  let categoryId = $state("");
  let amount = $state("");
  let occurredOn = $state(isoDate());
  let note = $state("");

  // two-step delete: the id waiting for confirmation
  let confirmingId = $state<number | null>(null);

  const exponent = $derived(accountList[0]?.exponent ?? 0);
  const formCategories = $derived(
    mode === "transfer" ? [] : categoryList.filter((c) => c.kind === mode),
  );

  function show(err: unknown) {
    error =
      err instanceof ApiError
        ? (ERROR_TEXT[err.code] ?? ERROR_TEXT.internal)
        : ERROR_TEXT.internal;
  }

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

  async function reload() {
    try {
      const page = await api.list(currentFilter(), null);
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
      const page = await api.list(currentFilter(), next);
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

  function resetForm() {
    editingId = null;
    mode = "expense";
    toAccountId = "";
    categoryId = "";
    amount = "";
    note = "";
    occurredOn = isoDate();
  }

  function startEdit(t: Transaction) {
    if (t.kind === "transfer_out" || t.kind === "transfer_in") return;
    editingId = t.id;
    mode = t.kind;
    accountId = String(t.accountId);
    categoryId = t.categoryId === null ? "" : String(t.categoryId);
    amount = formatMinor(t.amountMinor.replace("-", ""), t.exponent);
    occurredOn = t.occurredOn;
    note = t.note;
    error = "";
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const minor = parseAmount(amount, exponent);
    if (minor === null || minor.startsWith("-") || minor === "0") {
      error = "Jumlah harus lebih dari nol. Contoh: 25.000";
      return;
    }
    try {
      if (mode === "transfer") {
        await api.transfer({
          fromAccountId: Number(accountId),
          toAccountId: Number(toAccountId),
          amountMinor: minor,
          note,
          occurredOn,
        });
      } else {
        const input = {
          accountId: Number(accountId),
          categoryId: Number(categoryId),
          kind: mode,
          amountMinor: minor,
          note,
          occurredOn,
        };
        if (editingId === null) await api.create(input);
        else await api.update(editingId, input);
      }
      resetForm();
      await reload();
    } catch (err) {
      show(err);
    }
  }

  async function remove(t: Transaction) {
    try {
      await api.remove(t.id);
      confirmingId = null;
      await reload();
    } catch (err) {
      show(err);
    }
  }

  function describe(t: Transaction): string {
    if (t.kind === "transfer_out") return "Transfer keluar";
    if (t.kind === "transfer_in") return "Transfer masuk";
    return t.categoryName ?? "";
  }

  onMount(async () => {
    try {
      [accountList, categoryList] = await Promise.all([
        accountsApi.list(false),
        categoriesApi.list(false),
      ]);
      accountId = accountList[0] ? String(accountList[0].id) : "";
    } catch (err) {
      show(err);
    }
    await reload();
  });
</script>

<main>
  <h1>Transaksi</h1>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <form onsubmit={submit} aria-label="Form transaksi">
    <h2>{editingId === null ? "Catat transaksi" : "Ubah transaksi"}</h2>

    <div class="modes" role="radiogroup" aria-label="Jenis">
      {#each Object.entries(MODE_LABEL) as [value, label]}
        <button
          type="button"
          role="radio"
          aria-checked={mode === value}
          disabled={editingId !== null && value === "transfer"}
          onclick={() => {
            mode = value as Mode;
            categoryId = "";
          }}>{label}</button
        >
      {/each}
    </div>

    <label>
      {mode === "transfer" ? "Dari rekening" : "Rekening"}
      <select bind:value={accountId} required>
        {#each accountList as account (account.id)}
          <option value={String(account.id)}>{account.name}</option>
        {/each}
      </select>
    </label>

    {#if mode === "transfer"}
      <label>
        Ke rekening
        <select bind:value={toAccountId} required>
          <option value="" disabled>Pilih rekening</option>
          {#each accountList.filter((a) => String(a.id) !== accountId) as account (account.id)}
            <option value={String(account.id)}>{account.name}</option>
          {/each}
        </select>
      </label>
    {:else}
      <label>
        Kategori
        <select bind:value={categoryId} required>
          <option value="" disabled>Pilih kategori</option>
          {#each formCategories as category (category.id)}
            <option value={String(category.id)}>
              {category.parentId === null ? "" : "— "}{category.name}
            </option>
          {/each}
        </select>
      </label>
    {/if}

    <label>
      Jumlah
      <input bind:value={amount} inputmode="decimal" autocomplete="off" required />
    </label>
    <label>
      Tanggal
      <input type="date" bind:value={occurredOn} required />
    </label>
    <label>
      Catatan
      <input bind:value={note} maxlength="500" autocomplete="off" />
    </label>

    <div class="actions">
      <button type="submit">{editingId === null ? "Simpan" : "Perbarui"}</button>
      {#if editingId !== null}
        <button type="button" onclick={resetForm}>Batal</button>
      {/if}
    </div>
  </form>

  <section aria-label="Filter">
    <h2>Riwayat</h2>
    <div class="filters">
      <input
        type="search"
        placeholder="Cari catatan atau kategori"
        bind:value={search}
        oninput={searchChanged}
        aria-label="Cari"
      />
      <select bind:value={filterAccount} onchange={reload} aria-label="Rekening">
        <option value="">Semua rekening</option>
        {#each accountList as account (account.id)}
          <option value={String(account.id)}>{account.name}</option>
        {/each}
      </select>
      <select bind:value={filterKind} onchange={reload} aria-label="Jenis">
        <option value="">Semua jenis</option>
        <option value="expense">Pengeluaran</option>
        <option value="income">Pemasukan</option>
        <option value="transfer">Transfer</option>
      </select>
      <select bind:value={filterCategory} onchange={reload} aria-label="Kategori">
        <option value="">Semua kategori</option>
        {#each categoryList as category (category.id)}
          <option value={String(category.id)}>
            {category.parentId === null ? "" : "— "}{category.name}
          </option>
        {/each}
      </select>
      <label>
        Dari
        <input type="date" bind:value={filterFrom} onchange={reload} />
      </label>
      <label>
        Sampai
        <input type="date" bind:value={filterTo} onchange={reload} />
      </label>
    </div>
  </section>

  <section aria-label="Daftar transaksi">
    {#if loading}
      <p>Memuat…</p>
    {:else if items.length === 0}
      <p>Belum ada transaksi yang cocok.</p>
    {:else}
      <ul>
        {#each items as t (t.id)}
          <li>
            <div class="main">
              <strong>{describe(t)}</strong>
              <small>
                {formatDate(t.occurredOn)} · {t.accountName}{t.note ? ` · ${t.note}` : ""}
              </small>
            </div>
            <span class="amount" class:negative={t.amountMinor.startsWith("-")}>
              {formatMinor(t.amountMinor, t.exponent)}
            </span>
            <div class="actions">
              {#if t.kind === "income" || t.kind === "expense"}
                <button type="button" onclick={() => startEdit(t)}>Ubah</button>
              {/if}
              {#if confirmingId === t.id}
                <button type="button" class="danger" onclick={() => remove(t)}>
                  Ya, hapus{t.transferGroupId !== null ? " kedua sisi" : ""}
                </button>
                <button type="button" onclick={() => (confirmingId = null)}>Batal</button>
              {:else}
                <button type="button" onclick={() => (confirmingId = t.id)}>Hapus</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
      {#if next}
        <button type="button" class="more" onclick={more} disabled={loadingMore}>
          {loadingMore ? "Memuat…" : "Muat lebih banyak"}
        </button>
      {/if}
    {/if}
  </section>
</main>

<style>
  main {
    max-width: 36rem;
    margin: 0 auto;
    padding: 1rem;
  }
  form,
  .filters {
    display: grid;
    gap: 0.75rem;
  }
  .filters {
    grid-template-columns: 1fr 1fr;
  }
  .filters input[type="search"] {
    grid-column: 1 / -1;
  }
  label {
    display: grid;
    gap: 0.25rem;
  }
  .modes {
    display: flex;
    gap: 0.5rem;
  }
  .modes button {
    flex: 1;
  }
  .modes button[aria-checked="true"] {
    font-weight: 600;
    outline: 2px solid currentColor;
  }
  ul {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  li {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0.25rem 1rem;
    padding: 0.75rem 0;
    border-bottom: 1px solid color-mix(in srgb, currentColor 20%, transparent);
  }
  small {
    display: block;
    opacity: 0.7;
  }
  .amount {
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .negative {
    color: #b3261e;
  }
  .actions {
    grid-column: 1 / -1;
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .danger {
    color: #b3261e;
  }
  .more {
    width: 100%;
    margin-top: 1rem;
  }
  .error {
    padding: 0.5rem 0.75rem;
    border-radius: 0.5rem;
    background: color-mix(in srgb, #b3261e 15%, transparent);
  }
  input,
  select,
  button {
    font: inherit;
    padding: 0.5rem 0.75rem;
  }
  button {
    min-height: 2.75rem;
  }
</style>
