<script lang="ts">
  import { onMount } from "svelte";

  import {
    accounts as api,
    ApiError,
    type Account,
    type AccountKind,
  } from "$lib/api";
  import Amount from "$lib/components/Amount.svelte";
  import Button from "$lib/components/Button.svelte";
  import Card from "$lib/components/Card.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import Field from "$lib/components/Field.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import Sheet from "$lib/components/Sheet.svelte";
  import { formatAmountInput, formatMinor, parseAmount } from "$lib/money";
  import { dataChanged, notify, ui } from "$lib/session.svelte";

  const KIND_LABEL: Record<AccountKind, string> = {
    cash: "Tunai",
    bank: "Bank",
    ewallet: "Dompet digital",
  };

  const ERROR_TEXT: Record<string, string> = {
    unavailable: "Penyimpanan tidak tersedia di perangkat ini.",
    invalid_input: "Isian tidak valid.",
    not_found: "Rekening tidak ditemukan.",
    name_in_use: "Nama sudah dipakai rekening aktif lain.",
    amount_out_of_range: "Jumlah terlalu besar.",
    internal: "Terjadi kesalahan.",
  };

  let list = $state<Account[]>([]);
  let showArchived = $state(false);
  let loading = $state(true);
  let error = $state("");

  let sheetOpen = $state(false);
  let sheet = $state<{ close: () => void }>();
  let editing = $state<Account | null>(null);
  let name = $state("");
  let kind = $state<AccountKind>("cash");
  let opening = $state("");
  let formError = $state("");

  const exponent = $derived(list[0]?.exponent ?? 0);
  const active = $derived(list.filter((a) => !a.archived));
  const total = $derived(active.reduce((sum, a) => sum + BigInt(a.balanceMinor), 0n));

  function message(err: unknown): string {
    return err instanceof ApiError
      ? (ERROR_TEXT[err.code] ?? ERROR_TEXT.internal)
      : ERROR_TEXT.internal;
  }

  async function load() {
    try {
      list = await api.list(showArchived);
      error = "";
    } catch (err) {
      error = message(err);
    } finally {
      loading = false;
    }
  }

  function openNew() {
    editing = null;
    name = "";
    kind = "cash";
    opening = "";
    formError = "";
    sheetOpen = true;
  }

  function openEdit(account: Account) {
    editing = account;
    name = account.name;
    kind = account.kind;
    opening = formatMinor(account.openingBalanceMinor, account.exponent);
    formError = "";
    sheetOpen = true;
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const minor = parseAmount(opening === "" ? "0" : opening, exponent);
    if (minor === null) {
      formError = "Saldo awal tidak dapat dibaca. Contoh: 1.500.000";
      return;
    }
    const input = { name, kind, openingBalanceMinor: minor };
    try {
      if (editing) await api.update(editing.id, input);
      else await api.create(input);
      sheet?.close();
      notify(editing ? "Rekening diperbarui." : "Rekening ditambahkan.");
      dataChanged();
    } catch (err) {
      formError = message(err);
    }
  }

  async function toggleArchive() {
    if (!editing) return;
    try {
      await api.archive(editing.id, !editing.archived);
      sheet?.close();
      notify(editing.archived ? "Rekening dipulihkan." : "Rekening diarsipkan.");
      dataChanged();
    } catch (err) {
      formError = message(err);
    }
  }

  onMount(load);

  // Reload after any write elsewhere (quick-add, recurring catch-up) and when
  // the archived filter changes.
  $effect(() => {
    void ui.version;
    void showArchived;
    void load();
  });
</script>

<PageHeader title="Rekening">
  <Button variant="primary" onclick={openNew}>Tambah</Button>
</PageHeader>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

{#if loading}
  <p class="muted">Memuat…</p>
{:else if active.length === 0 && !showArchived}
  <EmptyState
    title="Belum ada rekening"
    text="Tambahkan rekening tunai, bank, atau dompet digital untuk mulai mencatat."
  >
    <Button variant="primary" onclick={openNew}>Tambah rekening</Button>
  </EmptyState>
{:else}
  <Card>
    <p class="muted label">Total saldo aktif</p>
    <p class="total">
      <Amount minor={total.toString()} {exponent} currency={list[0]?.currency} strong />
    </p>
  </Card>

  <ul class="accounts" aria-label="Daftar rekening">
    {#each list as account (account.id)}
      <li>
        <button type="button" class="row" class:archived={account.archived} onclick={() => openEdit(account)}>
          <span class="meta">
            <strong>{account.name}</strong>
            <small>{KIND_LABEL[account.kind]}{account.archived ? " · diarsipkan" : ""}</small>
          </span>
          <Amount minor={account.balanceMinor} exponent={account.exponent} strong />
        </button>
      </li>
    {/each}
  </ul>

  <label class="check">
    <input type="checkbox" bind:checked={showArchived} />
    Tampilkan yang diarsipkan
  </label>
{/if}

<Sheet bind:this={sheet} bind:open={sheetOpen} title={editing ? "Ubah rekening" : "Tambah rekening"}>
  <form id="account-form" onsubmit={submit} class="form" aria-label="Form rekening">
    <Field label="Nama">
      <input bind:value={name} maxlength="80" required autocomplete="off" />
    </Field>
    <div>
      <p class="seg-label">Jenis</p>
      <Segmented
        label="Jenis rekening"
        bind:value={kind}
        options={[
          { value: "cash", label: "Tunai" },
          { value: "bank", label: "Bank" },
          { value: "ewallet", label: "E-wallet" },
        ]}
      />
    </div>
    <Field label="Saldo awal" hint="Boleh negatif, misalnya untuk kartu kredit.">
      <input
        value={opening}
        oninput={(e) => {
          const input = e.currentTarget as HTMLInputElement;
          const negative = input.value.trimStart().startsWith("-");
          const body = formatAmountInput(input.value.replace("-", ""), exponent);
          opening = negative ? `-${body}` : body;
          input.value = opening;
        }}
        inputmode="text"
        autocomplete="off"
        placeholder="0"
      />
    </Field>
    {#if formError}
      <p class="error" role="alert">{formError}</p>
    {/if}
  </form>
  {#snippet footer()}
    <div class="actions">
      <Button type="submit" form="account-form" variant="primary" block>{editing ? "Simpan" : "Tambah"}</Button>
      {#if editing}
        <Button variant="ghost" onclick={toggleArchive}>
          {editing.archived ? "Pulihkan" : "Arsipkan"}
        </Button>
      {/if}
    </div>
  {/snippet}
</Sheet>

<style>
  .muted {
    color: var(--text-muted);
  }
  .label {
    font-size: var(--fs-sm);
  }
  .total {
    font-size: var(--fs-2xl);
    margin-top: var(--space-1);
  }
  .accounts {
    list-style: none;
    margin: var(--space-4) 0 0;
    padding: 0;
    display: grid;
    gap: var(--space-2);
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    min-height: 4rem;
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    text-align: left;
  }
  .row.archived {
    opacity: 0.6;
  }
  .meta {
    display: grid;
    min-width: 0;
  }
  .meta strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  small {
    color: var(--text-muted);
  }
  .check {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    margin-top: var(--space-4);
    min-height: var(--tap);
  }
  .form {
    display: grid;
    gap: var(--space-4);
  }
  .seg-label {
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--text-muted);
    margin-bottom: var(--space-1);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
  }
  .error {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--danger-bg);
    color: var(--expense);
  }
</style>
