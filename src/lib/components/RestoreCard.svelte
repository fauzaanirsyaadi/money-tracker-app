<script lang="ts">
  import { ApiError, backup, type BackupCounts, type RestorePreview } from "$lib/api";
  import { backupErrorMessage } from "$lib/backup";
  import Button from "$lib/components/Button.svelte";
  import Card from "$lib/components/Card.svelte";
  import Field from "$lib/components/Field.svelte";
  import { dataChanged, notify, refreshLock } from "$lib/session.svelte";

  /**
   * On the recovery screen the current data is unreadable: the restore builds a
   * new key and replaces it, and there is nothing to compare against.
   */
  let { recovering = false }: { recovering?: boolean } = $props();

  type Step = "idle" | "picked" | "preview" | "working";
  let step = $state<Step>("idle");
  let restorePass = $state("");
  let restoreError = $state("");
  let preview = $state<RestorePreview | null>(null);
  let confirmed = $state(false);

  function fail(err: unknown) {
    if (!(err instanceof ApiError)) throw err;
    restoreError = backupErrorMessage(err.code);
    // The state may have changed under us (PIN confirmation expired, or the data
    // turned out to be readable after all): let the screen follow the truth.
    if (recovering) void refreshLock();
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
      reset();
      // The storage problem (if any) is gone and the lock was reset: learn the new state.
      await refreshLock();
      dataChanged();
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
  <h2>{recovering ? "Pulihkan data dari cadangan" : "Pulihkan dari cadangan"}</h2>
  {#if step === "idle"}
    <p class="muted">
      Pilih berkas cadangan (.mtbak). Tidak ada yang diubah sampai Anda mengonfirmasi.
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
      {#if recovering}
        <p class="warn">
          Data lama tidak dapat dibaca dan akan <strong>dibuang</strong>, digantikan oleh isi cadangan.
          Kunci aplikasi (PIN) diatur ulang; Anda perlu memasangnya lagi.
        </p>
      {:else}
        <p class="warn">
          Data saat ini ({preview.current.accounts} rekening, {preview.current.transactions} transaksi)
          akan <strong>diganti seluruhnya</strong>. Ini tidak dapat dibatalkan.
        </p>
      {/if}
      <label class="check">
        <input type="checkbox" bind:checked={confirmed} />
        <span>{recovering ? "Buang data lama dan pakai isi cadangan." : "Ganti data saat ini dengan isi cadangan."}</span>
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
