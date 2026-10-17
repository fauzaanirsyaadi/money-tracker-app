<script lang="ts">
  import { ApiError, backup } from "$lib/api";
  import { backupAgeText, backupErrorMessage, backupFileName, passphraseProblem } from "$lib/backup";
  import Button from "$lib/components/Button.svelte";
  import Card from "$lib/components/Card.svelte";
  import Field from "$lib/components/Field.svelte";
  import { isoDate } from "$lib/dates";
  import RestoreCard from "$lib/components/RestoreCard.svelte";
  import { backupState, guarded, notify, refreshBackupStatus } from "$lib/session.svelte";

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
      const result = await guarded(() => backup.export(secret, backupFileName(isoDate())));
      if (result.saved) {
        notify(
          `Cadangan disimpan: ${result.counts.accounts} rekening, ${result.counts.transactions} transaksi.`,
          6000,
        );
        passphrase = "";
        repeat = "";
        understood = false;
        void refreshBackupStatus();
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

</script>

<Card>
  <h2>Cadangan terenkripsi</h2>
  <p class="muted">
    Simpan salinan data Anda sebagai satu berkas terenkripsi, lalu pulihkan di perangkat ini atau
    perangkat lain. PIN dan kunci perangkat tidak ikut di dalamnya.
  </p>

  {#if backupState.status}
    <p class="age" role="status">{backupAgeText(backupState.status, Date.now())}</p>
  {/if}

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

<RestoreCard />

<style>
  h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--fs-lg);
  }
  .age {
    margin: 0 0 var(--space-3);
    font-weight: 600;
  }
  .stack {
    display: grid;
    gap: var(--space-3);
  }
  .muted {
    color: var(--text-muted);
    margin: 0 0 var(--space-3);
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
</style>
