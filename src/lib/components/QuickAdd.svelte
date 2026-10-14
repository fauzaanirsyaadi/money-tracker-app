<script lang="ts">
  import { tick } from "svelte";

  import {
    accounts as accountsApi,
    ApiError,
    categories as categoriesApi,
    transactions as api,
    type Account,
    type Category,
  } from "$lib/api";
  import { isoDate } from "$lib/dates";
  import { formatAmountInput, formatMinor, parseAmount } from "$lib/money";
  import { loadLastUsed, saveLastUsed } from "$lib/prefs";
  import { dataChanged, notify, ui } from "$lib/session.svelte";

  import Button from "./Button.svelte";
  import Field from "./Field.svelte";
  import Segmented from "./Segmented.svelte";
  import Sheet from "./Sheet.svelte";

  type Mode = "expense" | "income" | "transfer";

  const ERROR_TEXT: Record<string, string> = {
    unavailable: "Penyimpanan tidak tersedia di perangkat ini.",
    invalid_input: "Isian tidak valid. Periksa jumlah, kategori, dan tanggal.",
    not_found: "Data tidak ditemukan.",
    archived: "Rekening atau kategori sudah diarsipkan.",
    amount_out_of_range: "Jumlah terlalu besar.",
    internal: "Terjadi kesalahan.",
  };

  let sheet = $state<{ close: () => void }>();
  let accountList = $state<Account[]>([]);
  let categoryList = $state<Category[]>([]);

  let mode = $state<Mode>("expense");
  let amountText = $state("");
  let accountId = $state("");
  let toAccountId = $state("");
  let categoryId = $state("");
  let occurredOn = $state(isoDate());
  let note = $state("");
  let error = $state("");
  let saving = $state(false);
  let confirmDelete = $state(false);

  const editing = $derived(ui.editing);
  const exponent = $derived(accountList[0]?.exponent ?? 0);
  const categoryChoices = $derived(
    mode === "transfer" ? [] : categoryList.filter((c) => c.kind === mode),
  );
  const title = $derived(editing ? "Ubah transaksi" : "Catat transaksi");

  function firstValid(preferred: number | undefined, ids: number[]): string {
    if (preferred !== undefined && ids.includes(preferred)) return String(preferred);
    return ids[0] === undefined ? "" : String(ids[0]);
  }

  function defaultCategory(kind: "expense" | "income"): string {
    const last = loadLastUsed();
    const ids = categoryList.filter((c) => c.kind === kind && c.parentId === null).map((c) => c.id);
    return firstValid(kind === "expense" ? last.expenseCategoryId : last.incomeCategoryId, ids);
  }

  async function prepare() {
    error = "";
    confirmDelete = false;
    saving = false;
    try {
      [accountList, categoryList] = await Promise.all([
        accountsApi.list(false),
        categoriesApi.list(false),
      ]);
    } catch (err) {
      error = err instanceof ApiError ? (ERROR_TEXT[err.code] ?? ERROR_TEXT.internal) : ERROR_TEXT.internal;
      return;
    }
    const last = loadLastUsed();
    const accountIds = accountList.map((a) => a.id);
    if (editing && (editing.kind === "income" || editing.kind === "expense")) {
      mode = editing.kind;
      accountId = String(editing.accountId);
      categoryId = editing.categoryId === null ? "" : String(editing.categoryId);
      amountText = formatMinor(editing.amountMinor.replace("-", ""), editing.exponent);
      occurredOn = editing.occurredOn;
      note = editing.note;
      return;
    }
    mode = "expense";
    amountText = "";
    note = "";
    occurredOn = isoDate();
    accountId = firstValid(last.accountId, accountIds);
    toAccountId = "";
    categoryId = defaultCategory("expense");
  }

  // Load fresh data every time the sheet opens, then put the cursor in the amount.
  $effect(() => {
    if (!ui.sheetOpen) return;
    void prepare().then(async () => {
      await tick();
      document.getElementById("quick-amount")?.focus();
    });
  });

  function changeMode(next: Mode) {
    error = "";
    if (next === "transfer") {
      categoryId = "";
      toAccountId = accountList.find((a) => String(a.id) !== accountId)?.id.toString() ?? "";
    } else {
      categoryId = defaultCategory(next);
    }
  }

  function onAmountInput(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    amountText = formatAmountInput(input.value, exponent);
    input.value = amountText;
  }

  function show(err: unknown) {
    error =
      err instanceof ApiError
        ? (ERROR_TEXT[err.code] ?? ERROR_TEXT.internal)
        : ERROR_TEXT.internal;
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    const minor = parseAmount(amountText, exponent);
    if (minor === null || minor.startsWith("-") || minor === "0") {
      error = "Masukkan jumlah di atas nol.";
      return;
    }
    saving = true;
    try {
      if (mode === "transfer") {
        await api.transfer({
          fromAccountId: Number(accountId),
          toAccountId: Number(toAccountId),
          amountMinor: minor,
          note,
          occurredOn,
        });
        saveLastUsed({ accountId: Number(accountId) });
      } else {
        const input = {
          accountId: Number(accountId),
          categoryId: Number(categoryId),
          kind: mode,
          amountMinor: minor,
          note,
          occurredOn,
        };
        if (editing) await api.update(editing.id, input);
        else await api.create(input);
        saveLastUsed({
          accountId: Number(accountId),
          ...(mode === "expense"
            ? { expenseCategoryId: Number(categoryId) }
            : { incomeCategoryId: Number(categoryId) }),
        });
      }
      sheet?.close();
      dataChanged();
      notify(editing ? "Perubahan disimpan." : "Transaksi tersimpan.");
    } catch (err) {
      show(err);
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (!editing) return;
    saving = true;
    try {
      await api.remove(editing.id);
      sheet?.close();
      dataChanged();
      notify("Transaksi dihapus.");
    } catch (err) {
      show(err);
    } finally {
      saving = false;
    }
  }

  const MODE_OPTIONS = $derived([
    { value: "expense" as Mode, label: "Pengeluaran" },
    { value: "income" as Mode, label: "Pemasukan" },
    { value: "transfer" as Mode, label: "Transfer", disabled: editing !== null },
  ]);
</script>

<Sheet bind:this={sheet} bind:open={ui.sheetOpen} {title}>
  <form id="quick-add-form" onsubmit={save} class="form" aria-label={title}>
    <Segmented
      label="Jenis transaksi"
      options={MODE_OPTIONS}
      bind:value={mode}
      onchange={changeMode}
    />

    <div class="amount">
      <span class="cur" aria-hidden="true">{accountList[0]?.currency ?? "IDR"}</span>
      <input
        id="quick-amount"
        value={amountText}
        oninput={onAmountInput}
        inputmode={exponent === 0 ? "numeric" : "decimal"}
        autocomplete="off"
        placeholder="0"
        aria-label="Jumlah"
        enterkeyhint="done"
      />
    </div>

    {#if mode !== "transfer"}
      <div class="chips" role="radiogroup" aria-label="Kategori">
        {#each categoryChoices as category (category.id)}
          <button
            type="button"
            role="radio"
            aria-checked={categoryId === String(category.id)}
            class:selected={categoryId === String(category.id)}
            onclick={() => (categoryId = String(category.id))}
          >
            {category.parentId === null ? "" : "↳ "}{category.name}
          </button>
        {/each}
      </div>
    {/if}

    <div class="row">
      <Field label={mode === "transfer" ? "Dari rekening" : "Rekening"}>
        <select bind:value={accountId} required>
          {#each accountList as account (account.id)}
            <option value={String(account.id)}>{account.name}</option>
          {/each}
        </select>
      </Field>
      {#if mode === "transfer"}
        <Field label="Ke rekening">
          <select bind:value={toAccountId} required>
            <option value="" disabled>Pilih</option>
            {#each accountList.filter((a) => String(a.id) !== accountId) as account (account.id)}
              <option value={String(account.id)}>{account.name}</option>
            {/each}
          </select>
        </Field>
      {:else}
        <Field label="Tanggal">
          <input type="date" bind:value={occurredOn} required />
        </Field>
      {/if}
    </div>

    {#if mode === "transfer"}
      <Field label="Tanggal">
        <input type="date" bind:value={occurredOn} required />
      </Field>
    {/if}

    <Field label="Catatan (opsional)">
      <input bind:value={note} maxlength="500" autocomplete="off" />
    </Field>

    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

  </form>
  {#snippet footer()}
    <div class="actions">
      <Button type="submit" form="quick-add-form" variant="primary" block disabled={saving}>
        {editing ? "Simpan perubahan" : "Simpan"}
      </Button>
      {#if editing}
        {#if confirmDelete}
          <Button variant="danger" onclick={remove} disabled={saving}>Ya, hapus</Button>
          <Button variant="ghost" onclick={() => (confirmDelete = false)}>Batal</Button>
        {:else}
          <Button variant="ghost" onclick={() => (confirmDelete = true)}>Hapus</Button>
        {/if}
      {/if}
    </div>
  {/snippet}
</Sheet>

<style>
  .form {
    display: grid;
    gap: var(--space-4);
  }
  .amount {
    display: flex;
    align-items: baseline;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-3) 0;
  }
  .cur {
    font-size: var(--fs-lg);
    color: var(--text-muted);
    font-weight: 600;
  }
  .amount input {
    width: 100%;
    max-width: 14rem;
    border: 0;
    border-bottom: 2px solid var(--border);
    background: transparent;
    font-size: var(--fs-hero);
    font-weight: 700;
    text-align: center;
    padding: var(--space-1) 0;
    font-variant-numeric: tabular-nums;
    outline-offset: 4px;
  }
  .chips {
    display: flex;
    gap: var(--space-2);
    overflow-x: auto;
    padding-bottom: var(--space-1);
    scrollbar-width: none;
  }
  .chips button {
    flex: 0 0 auto;
    min-height: 2.5rem;
    padding: 0 var(--space-3);
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .chips button.selected {
    background: var(--primary);
    border-color: var(--primary);
    color: var(--on-primary);
  }
  .row {
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
  }
</style>
