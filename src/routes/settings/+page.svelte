<script lang="ts">
  import { ApiError, AUTO_LOCK_CHOICES, lock, type LockStatus } from "$lib/api";
  import Button from "$lib/components/Button.svelte";
  import BackupPanel from "$lib/components/BackupPanel.svelte";
  import Card from "$lib/components/Card.svelte";
  import Field from "$lib/components/Field.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import { autoLockLabel, newPinProblem } from "$lib/lock";
  import { applyLockStatus, lockState, notify } from "$lib/session.svelte";

  const status = $derived(lockState.status as LockStatus);

  let newPin = $state("");
  let repeatPin = $state("");
  let currentPin = $state("");
  let busy = $state(false);
  let error = $state("");
  let mode = $state<"none" | "change" | "remove">("none");

  const problem = $derived(newPin === "" ? null : newPinProblem(newPin));
  const mismatch = $derived(repeatPin !== "" && repeatPin !== newPin);
  const canSetPin = $derived(newPin !== "" && problem === null && repeatPin === newPin);

  function clearForm() {
    newPin = "";
    repeatPin = "";
    currentPin = "";
    error = "";
  }

  function describe(err: unknown): string {
    if (!(err instanceof ApiError)) throw err;
    switch (err.code) {
      case "wrong_pin":
        return "PIN saat ini salah.";
      case "locked_out":
        return "Terlalu banyak percobaan. Coba lagi nanti.";
      case "invalid_input":
        return "PIN ditolak. Pakai 6 sampai 16 angka yang tidak berurutan atau berulang.";
      case "cancelled":
        return "Dibatalkan.";
      default:
        return "Tidak berhasil. Coba lagi.";
    }
  }

  async function run(work: () => Promise<LockStatus>, done: string) {
    if (busy) return;
    busy = true;
    error = "";
    try {
      applyLockStatus(await work());
      clearForm();
      mode = "none";
      notify(done);
    } catch (err) {
      error = describe(err);
    } finally {
      busy = false;
    }
  }

  const enable = (event: SubmitEvent) => {
    event.preventDefault();
    if (canSetPin) void run(() => lock.setup(newPin), "Kunci aplikasi diaktifkan.");
  };
  const change = (event: SubmitEvent) => {
    event.preventDefault();
    if (canSetPin && currentPin) void run(() => lock.changePin(currentPin, newPin), "PIN diubah.");
  };
  const remove = (event: SubmitEvent) => {
    event.preventDefault();
    if (currentPin) void run(() => lock.remove(currentPin), "Kunci aplikasi dimatikan.");
  };
  const toggleBiometric = (event: Event) => {
    const enabled = (event.currentTarget as HTMLInputElement).checked;
    void run(
      () => lock.setBiometric(enabled),
      enabled ? "Buka dengan biometrik diaktifkan." : "Buka dengan biometrik dimatikan.",
    );
  };

  const choices = AUTO_LOCK_CHOICES.map((seconds) => ({
    value: String(seconds),
    label: autoLockLabel(seconds),
  }));
  let autoLockText = $state("0");
  $effect(() => {
    autoLockText = String(status?.autoLockSeconds ?? 0);
  });
</script>

<PageHeader title="Pengaturan" />

<Card>
  <h2>Kunci aplikasi</h2>

  {#if !status.configured}
    <p class="muted">
      Atur PIN agar data keuangan tidak terbuka oleh siapa pun yang memegang ponsel Anda. Aplikasi
      terkunci setiap kali Anda meninggalkannya.
    </p>
    <form onsubmit={enable} class="stack">
      <Field label="PIN baru" hint="6 sampai 16 angka, tidak berurutan atau berulang.">
        <input type="password" inputmode="numeric" autocomplete="off" maxlength="16" bind:value={newPin} />
      </Field>
      {#if problem}<p class="error" role="alert">{problem}</p>{/if}
      <Field label="Ulangi PIN">
        <input type="password" inputmode="numeric" autocomplete="off" maxlength="16" bind:value={repeatPin} />
      </Field>
      {#if mismatch}<p class="error" role="alert">PIN tidak sama.</p>{/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <Button type="submit" variant="primary" block disabled={busy || !canSetPin}>
        Aktifkan kunci
      </Button>
    </form>
    <p class="warn">
      Catat PIN Anda. Tidak ada cara memulihkannya di dalam aplikasi; jika lupa, satu-satunya jalan
      adalah memulihkan dari cadangan terenkripsi (lihat bagian Cadangan di bawah). Buat cadangan
      sekarang.
    </p>
  {:else}
    <div class="stack">
      <label class="row">
        <span>
          <strong>Buka dengan sidik jari atau wajah</strong>
          <span class="hint">
            {#if status.biometric === "none_enrolled"}
              Belum ada sidik jari atau wajah yang terdaftar di perangkat.
            {:else if status.biometric === "unavailable"}
              Perangkat ini tidak mendukung biometrik yang kuat.
            {:else}
              PIN tetap bisa dipakai kapan saja.
            {/if}
          </span>
        </span>
        <input
          type="checkbox"
          checked={status.biometricEnabled}
          disabled={busy || (status.biometric !== "available" && !status.biometricEnabled)}
          onchange={toggleBiometric}
        />
      </label>

      <div class="stack tight">
        <span class="label">Kunci otomatis setelah ditinggalkan</span>
        <Segmented
          label="Kunci otomatis"
          options={choices}
          bind:value={autoLockText}
          onchange={(value) =>
            run(() => lock.setAutoLock(Number(value)), "Pengaturan kunci otomatis disimpan.")}
        />
      </div>

      {#if mode === "none"}
        <div class="actions">
          <Button onclick={() => ((mode = "change"), clearForm())}>Ubah PIN</Button>
          <Button variant="danger" onclick={() => ((mode = "remove"), clearForm())}>
            Matikan kunci
          </Button>
        </div>
      {:else if mode === "change"}
        <form onsubmit={change} class="stack">
          <Field label="PIN saat ini">
            <input type="password" inputmode="numeric" autocomplete="off" maxlength="16" bind:value={currentPin} />
          </Field>
          <Field label="PIN baru" hint="6 sampai 16 angka, tidak berurutan atau berulang.">
            <input type="password" inputmode="numeric" autocomplete="off" maxlength="16" bind:value={newPin} />
          </Field>
          {#if problem}<p class="error" role="alert">{problem}</p>{/if}
          <Field label="Ulangi PIN baru">
            <input type="password" inputmode="numeric" autocomplete="off" maxlength="16" bind:value={repeatPin} />
          </Field>
          {#if mismatch}<p class="error" role="alert">PIN tidak sama.</p>{/if}
          {#if error}<p class="error" role="alert">{error}</p>{/if}
          <div class="actions">
            <Button onclick={() => ((mode = "none"), clearForm())}>Batal</Button>
            <Button type="submit" variant="primary" disabled={busy || !canSetPin || !currentPin}>
              Simpan PIN
            </Button>
          </div>
        </form>
      {:else}
        <form onsubmit={remove} class="stack">
          <p class="muted">Masukkan PIN untuk mematikan kunci. Data akan terbuka tanpa PIN.</p>
          <Field label="PIN saat ini">
            <input type="password" inputmode="numeric" autocomplete="off" maxlength="16" bind:value={currentPin} />
          </Field>
          {#if error}<p class="error" role="alert">{error}</p>{/if}
          <div class="actions">
            <Button onclick={() => ((mode = "none"), clearForm())}>Batal</Button>
            <Button type="submit" variant="danger" disabled={busy || !currentPin}>Matikan kunci</Button>
          </div>
        </form>
      {/if}
    </div>
  {/if}
</Card>

<div class="gap"></div>
<BackupPanel />

<style>
  .gap {
    height: var(--space-4);
  }
  h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--fs-lg);
  }
  .stack {
    display: grid;
    gap: var(--space-3);
  }
  .tight {
    gap: var(--space-1);
  }
  .muted {
    color: var(--text-muted);
    margin: 0 0 var(--space-3);
  }
  .warn {
    margin: var(--space-3) 0 0;
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
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-height: var(--tap);
  }
  .row .hint {
    display: block;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .row input[type="checkbox"] {
    width: 1.5rem;
    height: 1.5rem;
    flex: none;
  }
  .label {
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
</style>
