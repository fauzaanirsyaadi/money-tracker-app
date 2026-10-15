<script lang="ts">
  import { ApiError, backup, type BackupCounts, type RestorePreview } from "$lib/api";
  import { backupErrorMessage, backupFileName, passphraseProblem } from "$lib/backup";
  import Button from "$lib/components/Button.svelte";
  import Card from "$lib/components/Card.svelte";
  import Field from "$lib/components/Field.svelte";
  import { isoDate } from "$lib/dates";
  import { dataChanged, notify } from "$lib/session.svelte";

  // --- export ---
  let passphrase = $state("");
  let repeat = $state("");
  let understood = $state(false);
  let exporting = $state(false);
  let exportError = $state("");

  const problem = $derived(passphrase === "" ? null : passphraseProblem(passphrase));
  const mismatch = $derived(repeat !== "" && repeat !== passphrase);
  const canExport = $derived(
    passphrase !== "" && problem === null && repeat === passphrase && understood && !exporting,
  );

  async function exportBackup(event: SubmitEvent) {
    event.preventDefault();
    if (!canExport) return;
    exporting = true;
    exportError = "";
    const secret = passphrase;
    try {
      const result = await backup.export(secret, backupFileName(isoDate()));
      if (result.saved) {
        notify(
          `Cadangan disimpan: ${result.counts.accounts} rekening, ${result.counts.transactions} transaksi.`,
          6000,
        );
        passphrase = "";
        repeat = "";
        understood = false;
      } else {
        notify("Pembuatan cadangan dibatalkan.");
      }
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
      exportError = backupErrorMessage(err.code);
    } finally {
      exporting = false;
    }
  }

  // --- restore ---
  type Step = "idle" | "picked" | "preview" | "working";
  let step = $state<Step>("idle");
  let restorePass = $state("");
  let restoreError = $state("");
  let preview = $state<RestorePreview | null>(null);
  let confirmed = $state(false);

  function fail(err: unknown) {
    if (!(err instanceof ApiError)) throw err;
    restoreError = backupErrorMessage(err.code);
  }

  async function pickFile() {
    restoreError = "";
    step = "working";
    try {
      step = (await backup.restorePick()) ? "picked" : "idle";
    } catch (err) {
      step = "idle";
      fail(err);
    }
  }

  async function check(event: SubmitEvent) {
    event.preventDefault();
    if (restorePass === "" || step !== "picked") return;
    restoreError = "";
    step = "working";
    const secret = restorePass;
    try {
      preview = await backup.restorePrepare(secret);
      restorePass = "";
      confirmed = false;
      step = "preview";
    } catch (err) {
      fail(err);
      // A wrong passphrase keeps the chosen file; anything else discards it.
      step = err instanceof ApiError && err.code === "wrong_passphrase" ? "picked" : "idle";
    }
  }

  async function commit() {
    if (!confirmed || step !== "preview") return;
    restoreError = "";
    step = "working";
    try {
      const counts: BackupCounts = await backup.restoreCommit();
      notify(`Data dipulihkan: ${counts.accounts} rekening, ${counts.transactions} transaksi.`, 7000);
      dataChanged();
      reset();
    } catch (err) {
      fail(err);
      step = "idle";
      preview = null;
    }
  }

  async function cancel() {
    try {
      await backup.restoreCancel();
    } catch {
      // nothing to clean up on our side
    }
    reset();
  }

  function reset() {
    step = "idle";
    preview = null;
    restorePass = "";
    confirmed = false;
  }
</script>

<Card>
  <h2>Cadangan terenkripsi</h2>
  <p class="muted">
    Simpan salinan data Anda sebagai satu berkas terenkripsi, lalu pulihkan di perangkat ini atau
    perangkat lain. PIN dan kunci perangkat tidak ikut di dalamnya.
  </p>

  <form class="stack" onsubmit={exportBackup}>
    <Field label="Frasa sandi cadangan" hint="Minimal 12 karakter. Kalimat panjang lebih kuat daripada kata acak pendek.">
      <input type="password" autocomplete="off" maxlength="128" bind:value={passphrase} disabled={exporting} />
    </Field>
    {#if problem}<p class="error" role="alert">{problem}</p>{/if}
    <Field label="Ulangi frasa sandi">
      <input type="password" autocomplete="off" maxlength="128" bind:value={repeat} disabled={exporting} />
    </Field>
    {#if mismatch}<p class="error" role="alert">Frasa sandi tidak sama.</p>{/if}
    <label class="check">
      <input type="checkbox" bind:checked={understood} disabled={exporting} />
      <span>
        Saya mengerti: <strong>frasa sandi tidak dapat dipulihkan</strong>. Jika hilang, cadangan ini
        tidak bisa dibuka oleh siapa pun.
      </span>
    </label>
    {#if exportError}<p class="error" role="alert">{exportError}</p>{/if}
    <Button type="submit" variant="primary" block disabled={!canExport}>
      {exporting ? "Mengenkripsi…" : "Buat cadangan"}
    </Button>
  </form>
</Card>

<Card>
  <h2>Pulihkan dari cadangan</h2>
  {#if step === "idle"}
    <p class="muted">
      Pilih berkas cadangan (.mtbak). Data Anda saat ini tidak diubah sampai Anda mengonfirmasi.
    </p>
    {#if restoreError}<p class="error" role="alert">{restoreError}</p>{/if}
    <Button block onclick={pickFile}>Pilih berkas cadangan…</Button>
  {:else if step === "picked"}
    <form class="stack" onsubmit={check}>
      <Field label="Frasa sandi cadangan">
        <input type="password" autocomplete="off" maxlength="128" bind:value={restorePass} />
      </Field>
      {#if restoreError}<p class="error" role="alert">{restoreError}</p>{/if}
      <div class="actions">
        <Button onclick={cancel}>Batal</Button>
        <Button type="submit" variant="primary" disabled={restorePass === ""}>Periksa cadangan</Button>
      </div>
    </form>
  {:else if step === "preview" && preview}
    <div class="stack">
      <p>
        Cadangan ini berisi <strong>{preview.backup.accounts} rekening</strong>,
        <strong>{preview.backup.transactions} transaksi</strong>,
        {preview.backup.categories} kategori, {preview.backup.budgets} anggaran, dan
        {preview.backup.recurringRules} transaksi berulang.
      </p>
      <p class="warn">
        Data saat ini ({preview.current.accounts} rekening, {preview.current.transactions} transaksi)
        akan <strong>diganti seluruhnya</strong>. Ini tidak dapat dibatalkan.
      </p>
      <label class="check">
        <input type="checkbox" bind:checked={confirmed} />
        <span>Ganti data saat ini dengan isi cadangan.</span>
      </label>
      {#if restoreError}<p class="error" role="alert">{restoreError}</p>{/if}
      <div class="actions">
        <Button onclick={cancel}>Batal</Button>
        <Button variant="danger" disabled={!confirmed} onclick={commit}>Ganti data saya</Button>
      </div>
    </div>
  {:else}
    <p class="muted" role="status">Memproses… jangan tutup aplikasi.</p>
  {/if}
</Card>

<style>
  h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--fs-lg);
  }
  .stack {
    display: grid;
    gap: var(--space-3);
  }
  .muted {
    color: var(--text-muted);
    margin: 0 0 var(--space-3);
  }
  .warn {
    margin: 0;
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--warning-bg);
    font-size: var(--fs-sm);
  }
  .error {
    margin: 0;
    color: var(--expense);
    font-weight: 600;
    font-size: var(--fs-sm);
  }
  .check {
    display: flex;
    gap: var(--space-3);
    align-items: flex-start;
    font-size: var(--fs-sm);
  }
  .check input {
    width: 1.5rem;
    height: 1.5rem;
    flex: none;
  }
  .actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
</style>
