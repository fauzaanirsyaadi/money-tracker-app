<script lang="ts">
  import { onMount, untrack } from "svelte";

  import "../app.css";

  import { page } from "$app/state";
  import { ApiError, lock, platform, recurring } from "$lib/api";
  import Icon from "$lib/components/Icon.svelte";
  import LockScreen from "$lib/components/LockScreen.svelte";
  import QuickAdd from "$lib/components/QuickAdd.svelte";
  import RecoveryScreen from "$lib/components/RecoveryScreen.svelte";
  import StepUpSheet from "$lib/components/StepUpSheet.svelte";
  import { isoDate } from "$lib/dates";
  import { isDarkTheme, loadTheme, nextTheme, saveTheme, type ThemePref } from "$lib/prefs";
  import {
    applyLockStatus,
    dataChanged,
    lockState,
    notify,
    backupState,
    openQuickAdd,
    refreshBackupStatus,
    refreshLock,
    ui,
  } from "$lib/session.svelte";
  import { backupBanner } from "$lib/backup";

  let { children } = $props();

  const tabs = [
    { href: "/", label: "Rekening", icon: "wallet" },
    { href: "/transactions", label: "Transaksi", icon: "list" },
    { href: "/budgets", label: "Anggaran", icon: "target" },
    { href: "/recurring", label: "Berulang", icon: "repeat" },
    { href: "/reports", label: "Laporan", icon: "chart" },
  ] as const;

  const THEME_LABEL: Record<ThemePref, string> = {
    auto: "Otomatis",
    light: "Terang",
    dark: "Gelap",
  };

  let theme = $state<ThemePref>("auto");

  function applyTheme(value: ThemePref) {
    const root = document.documentElement;
    if (value === "auto") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", value);
    syncSystemBars(value);
  }

  // The OS draws the status/navigation bar icons; tell it which theme is shown
  // so they stay readable when the app theme differs from the system theme.
  function syncSystemBars(value: ThemePref) {
    const systemDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    platform.systemBars(isDarkTheme(value, systemDark)).catch(() => {
      // Cosmetic only: a failure leaves the bars as the system drew them.
    });
  }

  function cycleTheme() {
    theme = nextTheme(theme);
    saveTheme(theme);
    applyTheme(theme);
  }

  async function lockNow() {
    try {
      applyLockStatus(await lock.lockNow());
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
    }
  }

  // Recurring transactions are computed when the app opens and after every
  // unlock: no background service exists. A failure must not block the app.
  async function catchUp() {
    try {
      const report = await recurring.catchUp(isoDate());
      if (report.transactionsCreated > 0) {
        notify(`${report.transactionsCreated} transaksi berulang dicatat.`, 6000);
        dataChanged();
      }
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
    }
  }

  $effect(() => {
    if (lockState.unlocks > 0) {
      untrack(() => {
        void catchUp();
        void refreshBackupStatus();
      });
    }
  });

  const backupReminder = $derived(
    backupState.status ? backupBanner(backupState.status, Date.now()) : null,
  );

  // Cover the content the moment the app is hidden, before the OS takes its
  // task-switcher snapshot, and re-check the lock when it comes back.
  function onVisibility() {
    if (document.visibilityState === "hidden") {
      if (lockState.status?.configured) lockState.covered = true;
    } else {
      void refreshLock().finally(() => {
        lockState.covered = false;
      });
    }
  }

  onMount(() => {
    theme = loadTheme();
    applyTheme(theme);
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onSystemChange = () => syncSystemBars(theme);
    media.addEventListener("change", onSystemChange);
    document.addEventListener("visibilitychange", onVisibility);
    void refreshLock();
    return () => {
      media.removeEventListener("change", onSystemChange);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  });

  // The data must be usable and the lock open before any screen shows it.
  const unlocked = $derived(
    lockState.status !== null && lockState.status.storage === "ok" && !lockState.status.locked,
  );
</script>

{#if lockState.status === null}
  <div class="cover" aria-hidden="true"></div>
{:else if lockState.status.storage !== "ok"}
  <RecoveryScreen />
{:else if lockState.status.locked}
  <LockScreen />
{/if}

{#if unlocked}
  <div class="shell">
    <header class="topbar">
      <span class="brand">Money Tracker</span>
      <span class="actions">
        {#if lockState.status?.configured}
          <button type="button" class="icon-btn" onclick={lockNow} aria-label="Kunci sekarang">
            <Icon name="lock" />
          </button>
        {/if}
        <a
          class="icon-btn"
          href="/settings"
          aria-label="Pengaturan"
          aria-current={page.url.pathname === "/settings" ? "page" : undefined}
        >
          <Icon name="settings" />
        </a>
        <button
          type="button"
          class="icon-btn"
          onclick={cycleTheme}
          aria-label={`Tema: ${THEME_LABEL[theme]}. Ketuk untuk mengganti.`}
        >
          <Icon name={theme === "auto" ? "auto" : theme === "light" ? "sun" : "moon"} />
        </button>
      </span>
    </header>

    <nav class="tabs" aria-label="Navigasi utama">
      {#each tabs as tab (tab.href)}
        <a
          href={tab.href}
          aria-current={page.url.pathname === tab.href ? "page" : undefined}
        >
          <Icon name={tab.icon} size={22} />
          <span>{tab.label}</span>
        </a>
      {/each}
    </nav>

    <div class="content">
      {#if ui.notice}
        <p class="notice" role="status">{ui.notice}</p>
      {/if}
      {#if backupReminder && page.url.pathname !== "/settings"}
        <p class="reminder" role="status"><a href="/settings">{backupReminder}</a></p>
      {/if}
      {@render children()}
    </div>

    <button
      type="button"
      class="fab"
      onclick={() => openQuickAdd()}
      aria-label="Catat transaksi"
    >
      <Icon name="plus" size={28} />
    </button>
  </div>

  <QuickAdd />
{/if}

<StepUpSheet />

{#if lockState.covered && unlocked}
  <div class="cover" aria-hidden="true"></div>
{/if}

<style>
  .shell {
    min-height: 100dvh;
    display: grid;
    grid-template-rows: auto 1fr;
  }
  .topbar {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-1) var(--space-4);
    /* Solid, not translucent: backdrop-filter is costly on low-end GPUs. */
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    min-height: 3.25rem;
  }
  .actions {
    display: flex;
    align-items: center;
  }
  .cover {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: var(--bg);
  }
  .brand {
    font-weight: 700;
    letter-spacing: 0.01em;
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: var(--tap);
    height: var(--tap);
    border: 0;
    border-radius: 999px;
    background: transparent;
    color: inherit;
  }
  .tabs {
    position: fixed;
    z-index: 6;
    left: 0;
    right: 0;
    bottom: 0;
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    background: var(--surface);
    border-top: 1px solid var(--border);
    padding-bottom: env(safe-area-inset-bottom);
  }
  .tabs a {
    display: grid;
    justify-items: center;
    align-content: center;
    gap: 2px;
    min-height: var(--nav-height);
    padding: var(--space-1);
    color: var(--text-muted);
    text-decoration: none;
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .tabs a[aria-current="page"] {
    color: var(--primary);
  }
  .content {
    width: 100%;
    max-width: var(--content-max);
    margin: 0 auto;
    padding: var(--space-4) var(--space-4) calc(var(--nav-height) + 6rem);
  }
  .reminder {
    margin: 0 0 var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--warning-bg);
    font-weight: 600;
  }
  .reminder a {
    color: var(--warning);
    text-decoration: none;
  }
  .notice {
    margin-bottom: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--success-bg);
  }
  .fab {
    position: fixed;
    z-index: 7;
    right: var(--space-4);
    bottom: calc(var(--nav-height) + var(--space-4) + env(safe-area-inset-bottom));
    display: grid;
    place-items: center;
    width: 3.5rem;
    height: 3.5rem;
    border: 0;
    border-radius: 999px;
    background: var(--primary);
    color: var(--on-primary);
    box-shadow: var(--shadow);
  }

  /* Wide screens: a side rail instead of the bottom bar. */
  @media (min-width: 720px) {
    .shell {
      grid-template-columns: 13rem 1fr;
      grid-template-rows: auto 1fr;
    }
    .topbar {
      grid-column: 1 / -1;
    }
    .tabs {
      position: sticky;
      top: 3.25rem;
      align-self: start;
      grid-template-columns: 1fr;
      height: calc(100dvh - 3.25rem);
      border-top: 0;
      border-right: 1px solid var(--border);
      padding: var(--space-3);
      gap: var(--space-1);
      align-content: start;
    }
    .tabs a {
      grid-auto-flow: column;
      justify-content: start;
      justify-items: start;
      gap: var(--space-3);
      min-height: var(--tap);
      padding: 0 var(--space-3);
      border-radius: var(--radius-md);
      font-size: var(--fs-md);
    }
    .tabs a[aria-current="page"] {
      background: var(--surface-2);
    }
    .content {
      padding-bottom: 6rem;
    }
    .fab {
      bottom: var(--space-5);
      right: var(--space-5);
    }
  }
</style>
