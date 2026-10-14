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
  import Amount from "$lib/components/Amount.svelte";
  import Button from "$lib/components/Button.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import Field from "$lib/components/Field.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import Sheet from "$lib/components/Sheet.svelte";
  import { formatDate, isoDate } from "$lib/dates";
  import { formatAmountInput, formatMinor, parseAmount } from "$lib/money";
  import { dataChanged, notify, ui } from "$lib/session.svelte";

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

  let sheetOpen = $state(false);
  let sheet = $state<{ close: () => void }>();
  let editing = $state<RecurringRule | null>(null);
  let kind = $state<FlowKind>("expense");
  let accountId = $state("");
  let categoryId = $state("");
  let amount = $state("");
  let note = $state("");
  let frequency = $state<Frequency>("monthly");
  let every = $state(1);
  let startOn = $state(isoDate());
  let endOn = $state("");
  let formError = $state("");
  let confirmDelete = $state(false);

  const exponent = $derived(accountList[0]?.exponent ?? 0);
  const formCategories = $derived(categoryList.filter((c) => c.kind === kind));

  function message(err: unknown): string {
    return err instanceof ApiError
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
      error = message(err);
    } finally {
      loading = false;
    }
  }

  function openNew() {
    editing = null;
    kind = "expense";
    accountId = accountList[0] ? String(accountList[0].id) : "";
    categoryId = "";
    amount = "";
    note = "";
    frequency = "monthly";
    every = 1;
    startOn = isoDate();
    endOn = "";
    formError = "";
    confirmDelete = false;
    sheetOpen = true;
  }

  function openEdit(rule: RecurringRule) {
    editing = rule;
    kind = rule.kind;
    accountId = String(rule.accountId);
    categoryId = String(rule.categoryId);
    amount = formatMinor(rule.amountMinor, rule.exponent);
    note = rule.note;
    frequency = rule.frequency;
    every = rule.every;
    startOn = rule.startOn;
    endOn = rule.endOn ?? "";
    formError = "";
    confirmDelete = false;
    sheetOpen = true;
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const minor = parseAmount(amount, exponent);
    if (minor === null || minor.startsWith("-") || minor === "0") {
      formError = "Jumlah harus lebih dari nol.";
      return;
    }
    try {
      if (editing) {
        await api.update(editing.id, {
          accountId: Number(accountId),
          categoryId: Number(categoryId),
          amountMinor: minor,
          note,
          endOn: endOn || null,
        });
      } else {
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
      }
      sheet?.close();
      notify(editing ? "Aturan diperbarui." : "Aturan ditambahkan.");
      await load();
    } catch (err) {
      formError = message(err);
    }
  }

  async function toggle(rule: RecurringRule) {
    try {
      await api.setActive(rule.id, !rule.active, isoDate());
      notify(rule.active ? "Aturan dijeda." : "Aturan dilanjutkan.");
      await load();
    } catch (err) {
      error = message(err);
    }
  }

  async function remove() {
    if (!editing) return;
    try {
      await api.remove(editing.id);
      sheet?.close();
      notify("Aturan dihapus. Transaksi yang sudah tercatat tetap ada.");
      await load();
    } catch (err) {
      formError = message(err);
    }
  }

  onMount(async () => {
    try {
      [accountList, categoryList] = await Promise.all([
        accountsApi.list(false),
        categoriesApi.list(false),
      ]);
    } catch (err) {
      error = message(err);
    }
  });

  $effect(() => {
    void ui.version;
    void load();
  });
</script>

<PageHeader title="Berulang">
  <Button variant="primary" onclick={openNew}>Tambah</Button>
</PageHeader>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

{#if loading}
  <p class="muted">Memuat…</p>
{:else if rules.length === 0}
  <EmptyState
    title="Belum ada transaksi berulang"
    text="Cocok untuk sewa, langganan, atau gaji. Dicatat otomatis saat aplikasi dibuka."
  >
    <Button variant="primary" onclick={openNew}>Tambah aturan</Button>
  </EmptyState>
{:else}
  <ul class="rules" aria-label="Daftar aturan">
    {#each rules as rule (rule.id)}
      <li class:paused={!rule.active}>
        <button type="button" class="item" onclick={() => openEdit(rule)}>
          <span class="meta">
            <strong>{rule.categoryName}</strong>
            <small>{schedule(rule)} · {rule.accountName}{rule.note ? ` · ${rule.note}` : ""}</small>
            <small>
              {rule.active ? `Berikutnya ${formatDate(rule.nextDue)}` : "Dijeda"}{rule.endOn
                ? ` · sampai ${formatDate(rule.endOn)}`
                : ""}
            </small>
          </span>
          <Amount
            minor={rule.kind === "expense" ? `-${rule.amountMinor}` : rule.amountMinor}
            exponent={rule.exponent}
            plus
            strong
          />
        </button>
        <div class="row-actions">
          <Button variant="ghost" onclick={() => toggle(rule)}>
            {rule.active ? "Jeda" : "Lanjutkan"}
          </Button>
        </div>
      </li>
    {/each}
  </ul>
{/if}

<Sheet bind:this={sheet} bind:open={sheetOpen} title={editing ? "Ubah aturan" : "Tambah aturan"}>
  <form id="recurring-form" onsubmit={submit} class="form" aria-label="Form aturan berulang">
    {#if editing}
      <p class="muted">
        Jadwal dan jenis tidak dapat diubah. Hapus aturan lalu buat yang baru bila perlu;
        transaksi yang sudah tercatat tetap ada.
      </p>
    {:else}
      <Segmented
        label="Jenis"
        bind:value={kind}
        onchange={() => (categoryId = "")}
        options={[
          { value: "expense", label: "Pengeluaran" },
          { value: "income", label: "Pemasukan" },
        ]}
      />
    {/if}

    <Field label="Jumlah">
      <input
        value={amount}
        oninput={(e) => {
          const input = e.currentTarget as HTMLInputElement;
          amount = formatAmountInput(input.value, exponent);
          input.value = amount;
        }}
        inputmode={exponent === 0 ? "numeric" : "decimal"}
        autocomplete="off"
        placeholder="0"
        required
      />
    </Field>
    <div class="two">
      <Field label="Rekening">
        <select bind:value={accountId} required>
          {#each accountList as account (account.id)}
            <option value={String(account.id)}>{account.name}</option>
          {/each}
        </select>
      </Field>
      <Field label="Kategori">
        <select bind:value={categoryId} required>
          <option value="" disabled>Pilih</option>
          {#each formCategories as category (category.id)}
            <option value={String(category.id)}>
              {category.parentId === null ? "" : "↳ "}{category.name}
            </option>
          {/each}
        </select>
      </Field>
    </div>
    <Field label="Catatan (opsional)">
      <input bind:value={note} maxlength="500" autocomplete="off" />
    </Field>

    <div class="two">
      <Field label="Setiap">
        <input type="number" min="1" max="999" bind:value={every} disabled={editing !== null} required />
      </Field>
      <Field label="Satuan">
        <select bind:value={frequency} disabled={editing !== null}>
          <option value="daily">Hari</option>
          <option value="weekly">Minggu</option>
          <option value="monthly">Bulan</option>
          <option value="yearly">Tahun</option>
        </select>
      </Field>
    </div>
    <div class="two">
      <Field label="Mulai">
        <input type="date" bind:value={startOn} disabled={editing !== null} required />
      </Field>
      <Field label="Berakhir (opsional)">
        <input type="date" bind:value={endOn} />
      </Field>
    </div>

    {#if formError}<p class="error" role="alert">{formError}</p>{/if}
  </form>
  {#snippet footer()}
    <div class="actions">
      <Button type="submit" form="recurring-form" variant="primary" block>{editing ? "Simpan" : "Tambah"}</Button>
      {#if editing}
        {#if confirmDelete}
          <Button variant="danger" onclick={remove}>Ya, hapus</Button>
          <Button variant="ghost" onclick={() => (confirmDelete = false)}>Batal</Button>
        {:else}
          <Button variant="ghost" onclick={() => (confirmDelete = true)}>Hapus</Button>
        {/if}
      {/if}
    </div>
  {/snippet}
</Sheet>

<style>
  .muted,
  small {
    color: var(--text-muted);
  }
  .rules {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-2);
  }
  .rules li {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }
  .rules li.paused .item {
    opacity: 0.6;
  }
  .item {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-3) var(--space-4);
    border: 0;
    background: transparent;
    text-align: left;
  }
  .meta {
    display: grid;
    min-width: 0;
  }
  .meta small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-actions {
    display: flex;
    justify-content: flex-end;
    padding: 0 var(--space-2) var(--space-2);
  }
  .form {
    display: grid;
    gap: var(--space-4);
  }
  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .error {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--danger-bg);
    color: var(--expense);
    margin-bottom: var(--space-3);
  }
</style>
