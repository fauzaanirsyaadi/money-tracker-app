<script lang="ts">
  import { ApiError, lock } from "$lib/api";
  import Button from "$lib/components/Button.svelte";
  import Field from "$lib/components/Field.svelte";
  import Sheet from "$lib/components/Sheet.svelte";
  import { applyLockStatus, settleStepUp, stepUpUi } from "$lib/session.svelte";

  let pin = $state("");
  let busy = $state(false);
  let message = $state("");

  // Closing the sheet any other way (Back, the X, the backdrop) declines.
  $effect(() => {
    if (!stepUpUi.open) {
      pin = "";
      message = "";
      settleStepUp(false);
    }
  });

  async function confirm(event: SubmitEvent) {
    event.preventDefault();
    if (busy || pin === "") return;
    busy = true;
    message = "";
    const attempt = pin;
    pin = "";
    try {
      applyLockStatus(await lock.stepUp(attempt));
      settleStepUp(true);
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
      message =
        err.code === "wrong_pin"
          ? "PIN salah."
          : err.code === "locked_out"
            ? "Terlalu banyak percobaan. Tunggu sebentar."
            : "Tidak berhasil. Tidak ada yang diubah.";
    } finally {
      busy = false;
    }
  }
</script>

<Sheet bind:open={stepUpUi.open} title="Masukkan PIN">
  <form onsubmit={confirm} class="stack">
    <p class="muted">
      Tindakan ini mengambil atau mengubah data Anda, jadi PIN dibutuhkan lagi. Percobaan salah
      dihitung sama seperti saat membuka kunci.
    </p>
    <Field label="PIN">
      <input
        type="password"
        inputmode="numeric"
        autocomplete="off"
        maxlength="16"
        bind:value={pin}
        disabled={busy}
      />
    </Field>
    {#if message}<p class="error" role="alert">{message}</p>{/if}
    <Button type="submit" variant="primary" block disabled={busy || pin === ""}>
      {busy ? "Memeriksa…" : "Lanjutkan"}
    </Button>
  </form>
</Sheet>

<style>
  .stack {
    display: grid;
    gap: var(--space-3);
  }
  .muted {
    color: var(--text-muted);
    margin: 0;
  }
  .error {
    color: var(--expense);
    margin: 0;
  }
</style>
