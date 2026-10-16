<script lang="ts">
  import { ApiError, recovery } from "$lib/api";
  import Button from "$lib/components/Button.svelte";
  import Card from "$lib/components/Card.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import RestoreCard from "$lib/components/RestoreCard.svelte";
  import { recoveryText } from "$lib/recovery";
  import { applyLockStatus, dataChanged, lockState, notify } from "$lib/session.svelte";

  const text = $derived(recoveryText(lockState.status?.storage ?? "ok"));

  let busy = $state(false);
  let message = $state("");
  let understood = $state(false);

  async function retry() {
    if (busy) return;
    busy = true;
    message = "";
    try {
      applyLockStatus(await recovery.retry());
      if (lockState.status?.storage !== "ok") message = "Masih belum dapat dibuka.";
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
      message = "Masih belum dapat dibuka.";
    } finally {
      busy = false;
    }
  }

  async function startFresh() {
    if (busy || !understood) return;
    busy = true;
    message = "";
    try {
      applyLockStatus(await recovery.startFresh(true));
      notify("Database baru dibuat. Data lama dibuang.", 7000);
      dataChanged();
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
      message = "Tidak berhasil memulai dari awal. Tidak ada yang diubah.";
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

    {#if text.canRecover}
      <RestoreCard recovering />

      <Card tone="danger">
        <h2>Mulai dari awal</h2>
        <p class="muted">
          Membuat database kosong baru. <strong>Semua data lama akan hilang</strong> dan tidak dapat
          dipulihkan, dan PIN diatur ulang. Pakai ini hanya bila tidak punya cadangan.
        </p>
        <label class="check">
          <input type="checkbox" bind:checked={understood} disabled={busy} />
          <span>Saya mengerti: data lama akan hilang permanen.</span>
        </label>
        {#if message}<p class="error" role="alert">{message}</p>{/if}
        <Button variant="danger" block disabled={busy || !understood} onclick={startFresh}>
          {busy ? "Memproses…" : "Buang data lama dan mulai baru"}
        </Button>
      </Card>
    {:else}
      {#if message}<p class="error" role="alert">{message}</p>{/if}
      <Button variant="primary" block disabled={busy} onclick={retry}>
        {busy ? "Memeriksa…" : "Coba lagi"}
      </Button>
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
  .check {
    display: flex;
    gap: var(--space-3);
    align-items: flex-start;
    font-size: var(--fs-sm);
    margin-bottom: var(--space-3);
  }
  .check input {
    width: 1.5rem;
    height: 1.5rem;
    flex: none;
  }
  .error {
    margin: 0 0 var(--space-3);
    color: var(--expense);
    font-weight: 600;
    font-size: var(--fs-sm);
  }
</style>
