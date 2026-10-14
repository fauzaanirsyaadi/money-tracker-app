<script lang="ts">
  import { onMount } from "svelte";

  import {
    accounts as accountsApi,
    ApiError,
    categories as categoriesApi,
    recurring as api,
    type Account,
    type Category,
    type FlowKind,
    type Frequency,
    type RecurringRule,
  } from "$lib/api";
  import { formatDate, isoDate } from "$lib/dates";
  import { formatMinor, parseAmount } from "$lib/money";

  const FREQUENCY_LABEL: Record<Frequency, string> = {
    daily: "hari",
    weekly: "minggu",
    monthly: "bulan",
    yearly: "tahun",
  };

  const ERROR_TEXT: Record<string, string> = {
    unavailable: "Penyimpanan tidak tersedia di perangkat ini.",
    invalid_input: "Isian tidak valid. Periksa jumlah, interval, dan tanggal.",
    not_found: "Data tidak ditemukan.",
    archived: "Rekening atau kategori sudah diarsipkan.",
    amount_out_of_range: "Jumlah terlalu besar.",
    internal: "Terjadi kesalahan.",
  };

  let rules = $state<RecurringRule[]>([]);
  let accountList = $state<Account[]>([]);
  let categoryList = $state<Category[]>([]);
  let loading = $state(true);
  let error = $state("");

  let editingId = $state<number | null>(null);
  let kind = $state<FlowKind>("expense");
  let accountId = $state("");
  let categoryId = $state("");
  let amount = $state("");
  let note = $state("");
  let frequency = $state<Frequency>("monthly");
  let every = $state(1);
  let startOn = $state(isoDate());
  let endOn = $state("");
  let confirmingId = $state<number | null>(null);

  const exponent = $derived(accountList[0]?.exponent ?? 0);
  const formCategories = $derived(categoryList.filter((c) => c.kind === kind));

  function show(err: unknown) {
    error =
      err instanceof ApiError
        ? (ERROR_TEXT[err.code] ?? ERROR_TEXT.internal)
        : ERROR_TEXT.internal;
  }

  function schedule(rule: RecurringRule): string {
    const unit = FREQUENCY_LABEL[rule.frequency];
    return rule.every === 1 ? `Setiap ${unit}` : `Setiap ${rule.every} ${unit}`;
  }

  async function load() {
    try {
      rules = await api.list();
      error = "";
    } catch (err) {
      show(err);
    } finally {
      loading = false;
    }
  }

  function reset() {
    editingId = null;
    kind = "expense";
    categoryId = "";
    amount = "";
    note = "";
    frequency = "monthly";
    every = 1;
    startOn = isoDate();
    endOn = "";
  }

  function startEdit(rule: RecurringRule) {
    editingId = rule.id;
    kind = rule.kind;
    accountId = String(rule.accountId);
    categoryId = String(rule.categoryId);
    amount = formatMinor(rule.amountMinor, rule.exponent);
    note = rule.note;
    frequency = rule.frequency;
    every = rule.every;
    startOn = rule.startOn;
    endOn = rule.endOn ?? "";
    error = "";
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const minor = parseAmount(amount, exponent);
    if (minor === null || minor.startsWith("-") || minor === "0") {
      error = "Jumlah harus lebih dari nol. Contoh: 150.000";
      return;
    }
    try {
      if (editingId === null) {
        await api.create({
          accountId: Number(accountId),
          categoryId: Number(categoryId),
          kind,
          amountMinor: minor,
          note,
          frequency,
          every,
          startOn,
          endOn: endOn || null,
        });
      } else {
        await api.update(editingId, {
          accountId: Number(accountId),
          categoryId: Number(categoryId),
          amountMinor: minor,
          note,
          endOn: endOn || null,
        });
      }
      reset();
      await load();
    } catch (err) {
      show(err);
    }
  }

  async function toggle(rule: RecurringRule) {
    try {
      await api.setActive(rule.id, !rule.active, isoDate());
      await load();
    } catch (err) {
      show(err);
    }
  }

  async function remove(rule: RecurringRule) {
    try {
      await api.remove(rule.id);
      confirmingId = null;
      await load();
    } catch (err) {
      show(err);
    }
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
    await load();
  });
</script>

<main>
  <h1>Transaksi berulang</h1>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <section aria-label="Daftar aturan">
    {#if loading}
      <p>Memuat…</p>
    {:else if rules.length === 0}
      <p>Belum ada transaksi berulang.</p>
    {:else}
      <ul>
        {#each rules as rule (rule.id)}
          <li class:paused={!rule.active}>
            <div class="main">
              <strong>{rule.categoryName}</strong>
              <small>
                {schedule(rule)} · {rule.accountName}{rule.note ? ` · ${rule.note}` : ""}
              </small>
              <small>
                {rule.active
                  ? `Berikutnya ${formatDate(rule.nextDue)}`
                  : "Dijeda"}{rule.endOn ? ` · sampai ${formatDate(rule.endOn)}` : ""}
              </small>
            </div>
            <span class="amount" class:negative={rule.kind === "expense"}>
              {rule.kind === "expense" ? "-" : ""}{formatMinor(rule.amountMinor, rule.exponent)}
            </span>
            <div class="actions">
              <button type="button" onclick={() => startEdit(rule)}>Ubah</button>
              <button type="button" onclick={() => toggle(rule)}>
                {rule.active ? "Jeda" : "Lanjutkan"}
              </button>
              {#if confirmingId === rule.id}
                <button type="button" class="danger" onclick={() => remove(rule)}>Ya, hapus</button>
                <button type="button" onclick={() => (confirmingId = null)}>Batal</button>
              {:else}
                <button type="button" onclick={() => (confirmingId = rule.id)}>Hapus</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <form onsubmit={submit} aria-label="Form aturan berulang">
    <h2>{editingId === null ? "Tambah aturan" : "Ubah aturan"}</h2>
    {#if editingId !== null}
      <p class="hint">
        Jadwal dan jenis tidak dapat diubah. Untuk mengubahnya, hapus aturan lalu buat yang baru;
        transaksi yang sudah tercatat tetap ada.
      </p>
    {/if}

    <label>
      Jenis
      <select bind:value={kind} disabled={editingId !== null} onchange={() => (categoryId = "")}>
        <option value="expense">Pengeluaran</option>
        <option value="income">Pemasukan</option>
      </select>
    </label>
    <label>
      Rekening
      <select bind:value={accountId} required>
        {#each accountList as account (account.id)}
          <option value={String(account.id)}>{account.name}</option>
        {/each}
      </select>
    </label>
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
    <label>
      Jumlah
      <input bind:value={amount} inputmode="decimal" autocomplete="off" required />
    </label>
    <label>
      Catatan
      <input bind:value={note} maxlength="500" autocomplete="off" />
    </label>

    <div class="row">
      <label>
        Setiap
        <input type="number" min="1" max="999" bind:value={every} disabled={editingId !== null} required />
      </label>
      <label>
        Satuan
        <select bind:value={frequency} disabled={editingId !== null}>
          <option value="daily">Hari</option>
          <option value="weekly">Minggu</option>
          <option value="monthly">Bulan</option>
          <option value="yearly">Tahun</option>
        </select>
      </label>
    </div>
    <label>
      Mulai
      <input type="date" bind:value={startOn} disabled={editingId !== null} required />
    </label>
    <label>
      Berakhir (opsional)
      <input type="date" bind:value={endOn} />
    </label>

    <div class="actions">
      <button type="submit">{editingId === null ? "Simpan" : "Perbarui"}</button>
      {#if editingId !== null}
        <button type="button" onclick={reset}>Batal</button>
      {/if}
    </div>
  </form>
</main>

<style>
  main {
    max-width: 36rem;
    margin: 0 auto;
    padding: 1rem;
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
  li.paused {
    opacity: 0.65;
  }
  small {
    display: block;
    opacity: 0.75;
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
  .row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.75rem;
  }
  .hint {
    margin: 0;
    opacity: 0.8;
  }
  .error {
    padding: 0.5rem 0.75rem;
    border-radius: 0.5rem;
    background: color-mix(in srgb, #b3261e 15%, transparent);
  }
  form {
    display: grid;
    gap: 0.75rem;
    margin-top: 1.5rem;
  }
  label {
    display: grid;
    gap: 0.25rem;
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
