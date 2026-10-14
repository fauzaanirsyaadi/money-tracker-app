<script lang="ts">
  import { onMount } from "svelte";

  import {
    ApiError,
    budgets as api,
    categories as categoriesApi,
    type Budget,
    type Category,
  } from "$lib/api";
  import { formatMonth, isoMonth, shiftMonth } from "$lib/dates";
  import { formatMinor, parseAmount } from "$lib/money";

  const ERROR_TEXT: Record<string, string> = {
    unavailable: "Penyimpanan tidak tersedia di perangkat ini.",
    invalid_input: "Isian tidak valid.",
    not_found: "Data tidak ditemukan.",
    archived: "Kategori sudah diarsipkan.",
    amount_out_of_range: "Jumlah terlalu besar.",
    internal: "Terjadi kesalahan.",
  };

  const STATUS_TEXT = {
    ok: "",
    warning: "Mendekati batas",
    over: "Melebihi anggaran",
  } as const;

  let month = $state(isoMonth());
  let list = $state<Budget[]>([]);
  let expenseCategories = $state<Category[]>([]);
  let loading = $state(true);
  let error = $state("");

  let categoryId = $state("");
  let limit = $state("");
  let confirmingId = $state<number | null>(null);

  const exponent = $derived(list[0]?.exponent ?? 0);
  const flagged = $derived(list.filter((b) => b.status !== "ok"));
  const totals = $derived({
    limit: list.reduce((sum, b) => sum + BigInt(b.limitMinor), 0n),
    spent: list.reduce((sum, b) => sum + BigInt(b.spentMinor), 0n),
  });

  function show(err: unknown) {
    error =
      err instanceof ApiError
        ? (ERROR_TEXT[err.code] ?? ERROR_TEXT.internal)
        : ERROR_TEXT.internal;
  }

  async function load() {
    try {
      list = await api.list(month);
      error = "";
    } catch (err) {
      show(err);
    } finally {
      loading = false;
    }
  }

  async function go(delta: number) {
    month = shiftMonth(month, delta);
    confirmingId = null;
    await load();
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const minor = parseAmount(limit, exponent);
    if (minor === null || minor.startsWith("-") || minor === "0") {
      error = "Batas harus lebih dari nol. Contoh: 1.500.000";
      return;
    }
    try {
      await api.set({ categoryId: Number(categoryId), month, limitMinor: minor });
      limit = "";
      categoryId = "";
      await load();
    } catch (err) {
      show(err);
    }
  }

  async function remove(b: Budget) {
    try {
      await api.remove(b.id);
      confirmingId = null;
      await load();
    } catch (err) {
      show(err);
    }
  }

  function startEdit(b: Budget) {
    categoryId = String(b.categoryId);
    limit = formatMinor(b.limitMinor, b.exponent);
  }

  onMount(async () => {
    try {
      expenseCategories = (await categoriesApi.list(false)).filter((c) => c.kind === "expense");
    } catch (err) {
      show(err);
    }
    await load();
  });
</script>

<main>
  <h1>Anggaran</h1>

  <div class="month" role="group" aria-label="Bulan">
    <button type="button" onclick={() => go(-1)} aria-label="Bulan sebelumnya">‹</button>
    <strong>{formatMonth(month)}</strong>
    <button type="button" onclick={() => go(1)} aria-label="Bulan berikutnya">›</button>
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if flagged.length > 0}
    <p class="warn" role="status">
      {flagged.filter((b) => b.status === "over").length > 0
        ? "Ada anggaran yang terlampaui bulan ini."
        : "Ada anggaran yang hampir habis bulan ini."}
    </p>
  {/if}

  <section aria-label="Daftar anggaran">
    {#if loading}
      <p>Memuat…</p>
    {:else if list.length === 0}
      <p>Belum ada anggaran untuk bulan ini.</p>
    {:else}
      <ul>
        {#each list as b (b.id)}
          <li class={b.status}>
            <div class="head">
              <strong>{b.categoryName}</strong>
              <span class="nums">
                {formatMinor(b.spentMinor, b.exponent)} / {formatMinor(b.limitMinor, b.exponent)}
              </span>
            </div>
            <progress max="10000" value={Math.min(b.usedBp, 10000)} aria-label="Pemakaian anggaran"
            ></progress>
            <div class="foot">
              <span>{(b.usedBp / 100).toFixed(0)}%</span>
              <span class="remaining" class:negative={b.remainingMinor.startsWith("-")}>
                {b.remainingMinor.startsWith("-")
                  ? `Lebih ${formatMinor(b.remainingMinor.slice(1), b.exponent)}`
                  : `Sisa ${formatMinor(b.remainingMinor, b.exponent)}`}
              </span>
              {#if STATUS_TEXT[b.status]}<strong class="flag">{STATUS_TEXT[b.status]}</strong>{/if}
            </div>
            <div class="actions">
              <button type="button" onclick={() => startEdit(b)}>Ubah batas</button>
              {#if confirmingId === b.id}
                <button type="button" class="danger" onclick={() => remove(b)}>Ya, hapus</button>
                <button type="button" onclick={() => (confirmingId = null)}>Batal</button>
              {:else}
                <button type="button" onclick={() => (confirmingId = b.id)}>Hapus</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
      <p class="total">
        Total: <strong>{formatMinor(totals.spent.toString(), exponent)}</strong> dari
        <strong>{formatMinor(totals.limit.toString(), exponent)}</strong>
      </p>
    {/if}
  </section>

  <form onsubmit={submit} aria-label="Form anggaran">
    <h2>Atur anggaran {formatMonth(month)}</h2>
    <label>
      Kategori
      <select bind:value={categoryId} required>
        <option value="" disabled>Pilih kategori</option>
        {#each expenseCategories as category (category.id)}
          <option value={String(category.id)}>
            {category.parentId === null ? "" : "— "}{category.name}
          </option>
        {/each}
      </select>
    </label>
    <label>
      Batas bulanan
      <input bind:value={limit} inputmode="decimal" autocomplete="off" required />
    </label>
    <button type="submit">Simpan</button>
  </form>
</main>

<style>
  main {
    max-width: 36rem;
    margin: 0 auto;
    padding: 1rem;
  }
  .month {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1rem;
  }
  ul {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  li {
    padding: 0.75rem 0;
    border-bottom: 1px solid color-mix(in srgb, currentColor 20%, transparent);
    display: grid;
    gap: 0.4rem;
  }
  .head,
  .foot {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .nums,
  .remaining {
    font-variant-numeric: tabular-nums;
  }
  progress {
    width: 100%;
    height: 0.75rem;
  }
  li.warning progress {
    accent-color: #b26a00;
  }
  li.over progress {
    accent-color: #b3261e;
  }
  .flag,
  .negative {
    color: #b3261e;
  }
  li.warning .flag {
    color: #b26a00;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .danger {
    color: #b3261e;
  }
  .total {
    text-align: right;
  }
  .warn,
  .error {
    padding: 0.5rem 0.75rem;
    border-radius: 0.5rem;
  }
  .warn {
    background: color-mix(in srgb, #b26a00 18%, transparent);
  }
  .error {
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
