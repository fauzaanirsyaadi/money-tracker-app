<script lang="ts">
  import { onMount } from "svelte";

  import {
    accounts as api,
    ApiError,
    type Account,
    type AccountKind,
  } from "$lib/api";
  import { formatMinor, parseAmount } from "$lib/money";

  const KIND_LABEL: Record<AccountKind, string> = {
    cash: "Tunai",
    bank: "Bank",
    ewallet: "Dompet digital",
  };

  const ERROR_TEXT: Record<string, string> = {
    unavailable: "Penyimpanan tidak tersedia di perangkat ini.",
    invalid_input: "Isian tidak valid.",
    not_found: "Rekening tidak ditemukan.",
    name_in_use: "Nama rekening sudah dipakai rekening aktif lain.",
    currency_mismatch: "Mata uang tidak sesuai.",
    amount_out_of_range: "Jumlah terlalu besar.",
    internal: "Terjadi kesalahan.",
  };

  let list = $state<Account[]>([]);
  let showArchived = $state(false);
  let loading = $state(true);
  let error = $state("");

  let editingId = $state<number | null>(null);
  let name = $state("");
  let kind = $state<AccountKind>("cash");
  let opening = $state("0");

  const total = $derived(
    list
      .filter((a) => !a.archived)
      .reduce((sum, a) => sum + BigInt(a.balanceMinor), 0n),
  );
  const exponent = $derived(list[0]?.exponent ?? 0);

  function show(err: unknown) {
    error =
      err instanceof ApiError
        ? (ERROR_TEXT[err.code] ?? ERROR_TEXT.internal)
        : ERROR_TEXT.internal;
  }

  async function load() {
    try {
      list = await api.list(showArchived);
      error = "";
    } catch (err) {
      show(err);
    } finally {
      loading = false;
    }
  }

  function reset() {
    editingId = null;
    name = "";
    kind = "cash";
    opening = "0";
  }

  function edit(account: Account) {
    editingId = account.id;
    name = account.name;
    kind = account.kind;
    opening = formatMinor(account.openingBalanceMinor, account.exponent);
    error = "";
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const minor = parseAmount(opening, exponent);
    if (minor === null) {
      error = "Saldo awal tidak dapat dibaca. Contoh: 1.500.000";
      return;
    }
    const input = { name, kind, openingBalanceMinor: minor };
    try {
      if (editingId === null) await api.create(input);
      else await api.update(editingId, input);
      reset();
      await load();
    } catch (err) {
      show(err);
    }
  }

  async function toggleArchive(account: Account) {
    try {
      await api.archive(account.id, !account.archived);
      await load();
    } catch (err) {
      show(err);
    }
  }

  onMount(load);
</script>

<main>
  <h1>Rekening</h1>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <section aria-label="Daftar rekening">
    {#if loading}
      <p>Memuat…</p>
    {:else if list.length === 0}
      <p>Belum ada rekening. Tambahkan satu di bawah.</p>
    {:else}
      <ul>
        {#each list as account (account.id)}
          <li class:archived={account.archived}>
            <div>
              <strong>{account.name}</strong>
              <small>{KIND_LABEL[account.kind]}{account.archived ? " · diarsipkan" : ""}</small>
            </div>
            <span class="balance" class:negative={account.balanceMinor.startsWith("-")}>
              {account.currency}
              {formatMinor(account.balanceMinor, account.exponent)}
            </span>
            <div class="actions">
              <button type="button" onclick={() => edit(account)}>Ubah</button>
              <button type="button" onclick={() => toggleArchive(account)}>
                {account.archived ? "Pulihkan" : "Arsipkan"}
              </button>
            </div>
          </li>
        {/each}
      </ul>
      <p class="total">
        Total aktif: <strong>{formatMinor(total.toString(), exponent)}</strong>
      </p>
    {/if}

    <label class="check">
      <input type="checkbox" bind:checked={showArchived} onchange={load} />
      Tampilkan yang diarsipkan
    </label>
  </section>

  <form onsubmit={submit} aria-label="Form rekening">
    <h2>{editingId === null ? "Tambah rekening" : "Ubah rekening"}</h2>
    <label>
      Nama
      <input bind:value={name} maxlength="80" required autocomplete="off" />
    </label>
    <label>
      Jenis
      <select bind:value={kind}>
        {#each Object.entries(KIND_LABEL) as [value, label]}
          <option {value}>{label}</option>
        {/each}
      </select>
    </label>
    <label>
      Saldo awal
      <input bind:value={opening} inputmode="decimal" autocomplete="off" />
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
  :global(:root) {
    color-scheme: light dark;
    font-family: system-ui, sans-serif;
  }
  main {
    max-width: 36rem;
    margin: 0 auto;
    padding: 1rem;
  }
  ul {
    list-style: none;
    padding: 0;
    margin: 0 0 1rem;
  }
  li {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0.25rem 1rem;
    padding: 0.75rem 0;
    border-bottom: 1px solid color-mix(in srgb, currentColor 20%, transparent);
  }
  li.archived {
    opacity: 0.6;
  }
  small {
    display: block;
    opacity: 0.7;
  }
  .balance {
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
  }
  .total {
    text-align: right;
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
  .check {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    margin-top: 0.5rem;
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
