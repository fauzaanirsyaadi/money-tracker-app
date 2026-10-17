<script lang="ts">
  import { onDestroy, onMount } from "svelte";

  import { ApiError, lock } from "$lib/api";
  import Button from "$lib/components/Button.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import { biometricFailureMessage, formatCountdown } from "$lib/lock";
  import { applyLockStatus, lockState, refreshLock } from "$lib/session.svelte";

  let pin = $state("");
  let busy = $state(false);
  let error = $state("");
  let biometricTried = false;

  // The remaining wait, counted down locally from what the backend reported.
  let retryAt = $state<number | null>(null);
  let now = $state(Date.now());
  let timer: ReturnType<typeof setInterval> | undefined;

  const status = $derived(lockState.status);
  const waitMs = $derived(retryAt === null ? 0 : Math.max(0, retryAt - now));
  const waiting = $derived(waitMs > 0);

  function syncRetry() {
    const ms = lockState.status?.retryAfterMs ?? null;
    retryAt = ms === null ? null : Date.now() + ms;
    now = Date.now();
  }

  onMount(() => {
    syncRetry();
    timer = setInterval(() => {
      now = Date.now();
    }, 500);
    // One automatic prompt per lock, so the common case is a single touch.
    if (status?.biometricEnabled && status.biometric === "available" && !biometricTried) {
      biometricTried = true;
      void useBiometric();
    }
  });

  onDestroy(() => clearInterval(timer));

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (busy || waiting || pin.length === 0) return;
    busy = true;
    error = "";
    const attempt = pin;
    pin = "";
    try {
      applyLockStatus(await lock.unlockPin(attempt));
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
      error =
        err.code === "wrong_pin"
          ? "PIN salah."
          : err.code === "locked_out"
            ? "Terlalu banyak percobaan. Tunggu sebentar."
            : "Tidak dapat membuka data. Coba lagi.";
      try {
        applyLockStatus(await lock.status());
        syncRetry();
      } catch {
        // keep the message above
      }
    } finally {
      busy = false;
    }
  }

  async function useBiometric() {
    if (busy) return;
    busy = true;
    error = "";
    try {
      applyLockStatus(await lock.unlockBiometric());
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
      error = biometricFailureMessage(err.code) ?? "";
      // A changed enrolment switched biometric unlock off: learn the new state.
      if (err.code === "biometric_invalidated") void refreshLock();
    } finally {
      busy = false;
    }
  }
</script>

<main class="lock">
  <div class="panel">
    <span class="badge"><Icon name="lock" size={28} /></span>
    <h1>Money Tracker</h1>

    {#if status?.broken}
      <p class="error" role="alert">
        Data kunci aplikasi rusak, sehingga aplikasi tidak dapat dibuka dengan aman. Pulihkan dari
        cadangan terenkripsi, atau hapus data aplikasi lewat pengaturan Android untuk memulai dari awal.
      </p>
    {:else}
      <p class="sub">Masukkan PIN untuk membuka data Anda.</p>
      <form onsubmit={submit}>
        <label class="sr-only" for="lock-pin">PIN</label>
        <input
          id="lock-pin"
          type="password"
          inputmode="numeric"
          autocomplete="off"
          maxlength="16"
          placeholder="PIN"
          bind:value={pin}
          disabled={busy || waiting}
        />
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        {#if waiting}
          <p class="wait" role="status">Coba lagi dalam {formatCountdown(waitMs)}.</p>
        {/if}
        <Button type="submit" variant="primary" block disabled={busy || waiting || pin.length === 0}>
          Buka
        </Button>
      </form>
      {#if status?.biometricEnabled && status.biometric === "available"}
        <Button variant="ghost" block onclick={useBiometric} disabled={busy}>
          <Icon name="fingerprint" size={20} />
          Gunakan sidik jari atau wajah
        </Button>
      {/if}
    {/if}
  </div>
</main>

<style>
  .lock {
    min-height: 100dvh;
    display: grid;
    place-items: center;
    padding: var(--space-5) var(--space-4);
    background: var(--bg);
  }
  .panel {
    width: 100%;
    max-width: 22rem;
    display: grid;
    gap: var(--space-4);
    justify-items: center;
    text-align: center;
  }
  .badge {
    display: grid;
    place-items: center;
    width: 4rem;
    height: 4rem;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--primary);
  }
  h1 {
    margin: 0;
    font-size: var(--fs-lg);
  }
  .sub {
    margin: 0;
    color: var(--text-muted);
  }
  form {
    display: grid;
    gap: var(--space-3);
    width: 100%;
  }
  input {
    min-height: var(--tap);
    padding: 0 var(--space-3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--surface);
    text-align: center;
    font-size: var(--fs-lg);
    letter-spacing: 0.3em;
    width: 100%;
  }
  .error {
    margin: 0;
    color: var(--expense);
    font-weight: 600;
  }
  .wait {
    margin: 0;
    color: var(--text-muted);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
</style>
