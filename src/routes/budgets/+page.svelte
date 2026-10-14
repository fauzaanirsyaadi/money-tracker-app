<script lang="ts">
  import { onMount } from "svelte";

  import {
    ApiError,
    budgets as api,
    categories as categoriesApi,
    type Budget,
    type Category,
  } from "$lib/api";
  import { percentOf } from "$lib/charts";
  import Amount from "$lib/components/Amount.svelte";
  import Button from "$lib/components/Button.svelte";
  import Card from "$lib/components/Card.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import Field from "$lib/components/Field.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import Sheet from "$lib/components/Sheet.svelte";
  import { formatMonth, isoMonth, shiftMonth } from "$lib/dates";
  import { formatAmountInput, formatMinor, parseAmount } from "$lib/money";
  import { dataChanged, notify, ui } from "$lib/session.svelte";

  const ERROR_TEXT: Record<string, string> = {
    unavailable: "Penyimpanan tidak tersedia di perangkat ini.",
    invalid_input: "Isian tidak valid.",
    not_found: "Data tidak ditemukan.",
    archived: "Kategori sudah diarsipkan.",
    amount_out_of_range: "Jumlah terlalu besar.",
    internal: "Terjadi kesalahan.",
  };

  const STATUS_TEXT = { ok: "", warning: "Mendekati batas", over: "Melebihi anggaran" } as const;

  let month = $state(isoMonth());
  let list = $state<Budget[]>([]);
  let expenseCategories = $state<Category[]>([]);
  let loading = $state(true);
  let error = $state("");

  let sheetOpen = $state(false);
  let sheet = $state<{ close: () => void }>();
  let categoryId = $state("");
  let limit = $state("");
  let editing = $state<Budget | null>(null);
  let formError = $state("");
  let confirmDelete = $state(false);

  const exponent = $derived(list[0]?.exponent ?? 0);
  const over = $derived(list.filter((b) => b.status === "over"));
  const near = $derived(list.filter((b) => b.status === "warning"));
  const totals = $derived({
    limit: list.reduce((sum, b) => sum + BigInt(b.limitMinor), 0n).toString(),
    spent: list.reduce((sum, b) => sum + BigInt(b.spentMinor), 0n).toString(),
  });

  function message(err: unknown): string {
    return err instanceof ApiError
      ? (ERROR_TEXT[err.code] ?? ERROR_TEXT.internal)
      : ERROR_TEXT.internal;
  }

  async function load() {
    try {
      list = await api.list(month);
      error = "";
    } catch (err) {
      error = message(err);
    } finally {
      loading = false;
    }
  }

  function go(delta: number) {
    month = shiftMonth(month, delta);
    void load();
  }

  function tone(b: Budget): "primary" | "warning" | "expense" {
    return b.status === "over" ? "expense" : b.status === "warning" ? "warning" : "primary";
  }

  function openNew() {
    editing = null;
    categoryId = "";
    limit = "";
    formError = "";
    confirmDelete = false;
    sheetOpen = true;
  }

  function openEdit(b: Budget) {
    editing = b;
    categoryId = String(b.categoryId);
    limit = formatMinor(b.limitMinor, b.exponent);
    formError = "";
    confirmDelete = false;
    sheetOpen = true;
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const minor = parseAmount(limit, exponent);
    if (minor === null || minor.startsWith("-") || minor === "0") {
      formError = "Batas harus lebih dari nol.";
      return;
    }
    try {
      await api.set({ categoryId: Number(categoryId), month, limitMinor: minor });
      sheet?.close();
      notify("Anggaran disimpan.");
      await load();
    } catch (err) {
      formError = message(err);
    }
  }

  async function remove() {
    if (!editing) return;
    try {
      await api.remove(editing.id);
      sheet?.close();
      notify("Anggaran dihapus.");
      await load();
    } catch (err) {
      formError = message(err);
    }
  }

  onMount(async () => {
    try {
      expenseCategories = (await categoriesApi.list(false)).filter((c) => c.kind === "expense");
    } catch (err) {
      error = message(err);
    }
  });

  // Spending changes the progress; reload after any write elsewhere.
  $effect(() => {
    void ui.version;
    void load();
  });
</script>

<PageHeader title="Anggaran">
  <Button variant="primary" onclick={openNew}>Atur</Button>
</PageHeader>

<div class="month" role="group" aria-label="Bulan">
  <button type="button" onclick={() => go(-1)} aria-label="Bulan sebelumnya">
    <Icon name="chevron-left" />
  </button>
  <strong>{formatMonth(month)}</strong>
  <button type="button" onclick={() => go(1)} aria-label="Bulan berikutnya">
    <Icon name="chevron-right" />
  </button>
</div>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

{#if loading}
  <p class="muted">Memuat…</p>
{:else if list.length === 0}
  <EmptyState title="Belum ada anggaran" text="Tetapkan batas bulanan untuk kategori pengeluaran.">
    <Button variant="primary" onclick={openNew}>Atur anggaran</Button>
  </EmptyState>
{:else}
  {#if over.length > 0 || near.length > 0}
    <Card tone={over.length > 0 ? "danger" : "warning"}>
      <p role="status">
        {#if over.length > 0}
          <strong>{over.length} anggaran terlampaui</strong>{near.length > 0
            ? `, ${near.length} hampir habis`
            : ""} bulan ini.
        {:else}
          <strong>{near.length} anggaran hampir habis</strong> bulan ini.
        {/if}
      </p>
    </Card>
  {/if}

  <Card>
    <p class="muted label">Total terpakai</p>
    <p class="big">
      <Amount minor={totals.spent} {exponent} neutral strong />
      <span class="of">dari <Amount minor={totals.limit} {exponent} neutral /></span>
    </p>
    <ProgressBar
      percent={percentOf(totals.spent, totals.limit)}
      label="Total pemakaian anggaran"
    />
  </Card>

  <ul class="budgets" aria-label="Daftar anggaran">
    {#each list as b (b.id)}
      <li>
        <button type="button" class="item" onclick={() => openEdit(b)}>
          <span class="top">
            <strong>{b.categoryName}</strong>
            <span class="num">
              {formatMinor(b.spentMinor, b.exponent)} / {formatMinor(b.limitMinor, b.exponent)}
            </span>
          </span>
          <ProgressBar
            percent={Math.min(100, b.usedBp / 100)}
            tone={tone(b)}
            label={`Pemakaian ${b.categoryName}`}
          />
          <span class="bottom">
            <span>{Math.round(b.usedBp / 100)}%</span>
            <span class:neg={b.remainingMinor.startsWith("-")}>
              {b.remainingMinor.startsWith("-")
                ? `Lebih ${formatMinor(b.remainingMinor.slice(1), b.exponent)}`
                : `Sisa ${formatMinor(b.remainingMinor, b.exponent)}`}
            </span>
            {#if STATUS_TEXT[b.status]}
              <strong class="flag {b.status}">{STATUS_TEXT[b.status]}</strong>
            {/if}
          </span>
        </button>
      </li>
    {/each}
  </ul>
{/if}

<Sheet bind:this={sheet} bind:open={sheetOpen} title={editing ? "Ubah anggaran" : "Atur anggaran"}>
  <form id="budget-form" onsubmit={submit} class="form" aria-label="Form anggaran">
    <p class="muted">{formatMonth(month)}</p>
    <Field label="Kategori">
      <select bind:value={categoryId} required disabled={editing !== null}>
        <option value="" disabled>Pilih kategori</option>
        {#each expenseCategories as category (category.id)}
          <option value={String(category.id)}>
            {category.parentId === null ? "" : "↳ "}{category.name}
          </option>
        {/each}
      </select>
    </Field>
    <Field label="Batas bulanan" hint="Anggaran kategori induk juga menghitung sub-kategorinya.">
      <input
        value={limit}
        oninput={(e) => {
          const input = e.currentTarget as HTMLInputElement;
          limit = formatAmountInput(input.value, exponent);
          input.value = limit;
        }}
        inputmode={exponent === 0 ? "numeric" : "decimal"}
        autocomplete="off"
        placeholder="0"
        required
      />
    </Field>
    {#if formError}<p class="error" role="alert">{formError}</p>{/if}
  </form>
  {#snippet footer()}
    <div class="actions">
      <Button type="submit" form="budget-form" variant="primary" block>Simpan</Button>
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
  .month {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-4);
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
  .muted {
    color: var(--text-muted);
  }
  .label {
    font-size: var(--fs-sm);
  }
  .big {
    font-size: var(--fs-xl);
    margin: var(--space-1) 0 var(--space-3);
  }
  .of {
    font-size: var(--fs-sm);
    color: var(--text-muted);
    margin-left: var(--space-2);
  }
  .budgets {
    list-style: none;
    margin: var(--space-4) 0 0;
    padding: 0;
    display: grid;
    gap: var(--space-2);
  }
  .item {
    display: grid;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    text-align: left;
  }
  .top,
  .bottom {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .bottom {
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .neg {
    color: var(--expense);
  }
  .flag.warning {
    color: var(--warning);
  }
  .flag.over {
    color: var(--expense);
  }
  .form {
    display: grid;
    gap: var(--space-4);
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
