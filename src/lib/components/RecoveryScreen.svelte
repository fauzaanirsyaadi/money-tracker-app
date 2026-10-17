<script lang="ts">
  import { ApiError, lock, recovery } from "$lib/api";
  import Button from "$lib/components/Button.svelte";
  import Card from "$lib/components/Card.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import RestoreCard from "$lib/components/RestoreCard.svelte";
  import Field from "$lib/components/Field.svelte";
  import {
    confirmationMatches,
    needsPin,
    recoveryText,
    retryAdvice,
    START_FRESH_WORD,
  } from "$lib/recovery";
  import {
    applyLockStatus,
    dataChanged,
    guarded,
    lockState,
    notify,
    refreshLock,
  } from "$lib/session.svelte";

  const text = $derived(recoveryText(lockState.status?.storage ?? "ok"));
  // A valid lock guards the destructive steps: the PIN comes first.
  const askPin = $derived(lockState.status !== null && needsPin(lockState.status));

  let busy = $state(false);
  let message = $state("");
  // How many times in a row "try again" ended in the same failure.
  let failedRetries = $state(0);
  const advice = $derived(retryAdvice(failedRetries));
  let typed = $state("");
  let pin = $state("");

  async function confirmPin(event: SubmitEvent) {
    event.preventDefault();
    if (busy || pin === "") return;
    busy = true;
    message = "";
    const attempt = pin;
    pin = "";
    try {
      applyLockStatus(await lock.stepUp(attempt));
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
      message =
        err.code === "wrong_pin"
          ? "PIN salah."
          : err.code === "locked_out"
            ? "Terlalu banyak percobaan. Tunggu sebentar."
            : "Tidak berhasil. Tidak ada yang diubah.";
      void refreshLock();
    } finally {
      busy = false;
    }
  }

  async function retry() {
    if (busy) return;
    busy = true;
    message = "";
    try {
      applyLockStatus(await recovery.retry());
      if (lockState.status?.storage !== "ok") {
        message = "Masih belum dapat dibuka.";
        failedRetries += 1;
      }
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
      message = "Masih belum dapat dibuka.";
      failedRetries += 1;
    } finally {
      busy = false;
    }
  }

  async function startFresh() {
    if (busy || !confirmationMatches(typed)) return;
    busy = true;
    message = "";
    try {
      applyLockStatus(await guarded(() => recovery.startFresh(typed)));
      notify("Database baru dibuat. Data lama dibuang.", 7000);
      dataChanged();
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
      message =
        err.code === "pin_required"
          ? "Masukkan PIN lagi untuk melanjutkan."
          : err.code === "invalid_input"
            ? "Tidak dilanjutkan: data ternyata bisa dibuka lagi, atau konfirmasi tidak sesuai. Tidak ada yang diubah."
            : "Tidak berhasil memulai dari awal. Tidak ada yang diubah.";
      // The truth may have changed (data readable again, PIN confirmation expired).
      void refreshLock();
    } finally {
      busy = false;
    }
  }
</script>

<main class="recovery">
  <div class="panel">
    <span class="badge"><Icon name="lock" size={28} /></span>
    <h1>{text.title}</h1>
    <p class="explain">{text.explanation}</p>

    {#if text.canRecover && askPin}
      <Card>
        <h2>Masukkan PIN</h2>
        <p class="muted">
          Pemulihan dapat membuang data lama. Karena kunci aplikasi aktif, PIN dibutuhkan lebih dulu.
        </p>
        <form onsubmit={confirmPin} class="pinform">
          <label class="sr-only" for="recovery-pin">PIN</label>
          <input
            id="recovery-pin"
            type="password"
            inputmode="numeric"
            autocomplete="off"
            maxlength="16"
            placeholder="PIN"
            bind:value={pin}
            disabled={busy}
          />
          {#if message}<p class="error" role="alert">{message}</p>{/if}
          <Button type="submit" variant="primary" block disabled={busy || pin === ""}>
            {busy ? "Memeriksa…" : "Lanjutkan"}
          </Button>
        </form>
      </Card>
    {:else if text.canRecover}
      <RestoreCard recovering />

      <Card tone="danger">
        <h2>Mulai dari awal</h2>
        <p class="muted">
          Membuat database kosong baru. <strong>Semua data lama akan hilang</strong> dan tidak dapat
          dipulihkan, dan PIN diatur ulang. Pakai ini hanya bila tidak punya cadangan.
        </p>
        <Field label={`Ketik ${START_FRESH_WORD} untuk mengonfirmasi`}>
          <input
            type="text"
            autocomplete="off"
            autocapitalize="characters"
            bind:value={typed}
            disabled={busy}
          />
        </Field>
        {#if message}<p class="error" role="alert">{message}</p>{/if}
        <Button
          variant="danger"
          block
          disabled={busy || !confirmationMatches(typed)}
          onclick={startFresh}
        >
          {busy ? "Memproses…" : "Buang data lama dan mulai baru"}
        </Button>
      </Card>
    {:else}
      {#if message}<p class="error" role="alert">{message}</p>{/if}
      <Button variant="primary" block disabled={busy} onclick={retry}>
        {busy ? "Memeriksa…" : "Coba lagi"}
      </Button>
      {#if advice}
        <Card tone="warning">
          <h2>{advice.title}</h2>
          <p class="muted">{advice.explanation}</p>
          <ol class="steps">
            {#each advice.steps as step (step)}
              <li>{step}</li>
            {/each}
          </ol>
        </Card>
      {/if}
    {/if}
  </div>
</main>

<style>
  .recovery {
    min-height: 100dvh;
    display: grid;
    justify-items: center;
    padding: var(--space-5) var(--space-4);
    background: var(--bg);
  }
  .panel {
    width: 100%;
    max-width: 30rem;
    display: grid;
    gap: var(--space-4);
    align-content: start;
  }
  .badge {
    display: grid;
    place-items: center;
    width: 4rem;
    height: 4rem;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--primary);
    justify-self: center;
  }
  h1 {
    margin: 0;
    font-size: var(--fs-xl);
    text-align: center;
  }
  h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--fs-lg);
  }
  .explain {
    margin: 0;
    color: var(--text-muted);
    text-align: center;
  }
  .muted {
    margin: 0 0 var(--space-3);
  }
  .steps {
    margin: 0;
    padding-left: var(--space-5);
    display: grid;
    gap: var(--space-2);
    font-size: var(--fs-sm);
  }
  .pinform {
    display: grid;
    gap: var(--space-3);
  }
  .pinform input {
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
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .error {
    margin: 0 0 var(--space-3);
    color: var(--expense);
    font-weight: 600;
    font-size: var(--fs-sm);
  }
</style>
