<script lang="ts">
  import { onMount } from "svelte";

  import "../app.css";

  import { page } from "$app/state";
  import { ApiError, platform, recurring } from "$lib/api";
  import Icon from "$lib/components/Icon.svelte";
  import QuickAdd from "$lib/components/QuickAdd.svelte";
  import { isoDate } from "$lib/dates";
  import { isDarkTheme, loadTheme, nextTheme, saveTheme, type ThemePref } from "$lib/prefs";
  import { dataChanged, notify, openQuickAdd, ui } from "$lib/session.svelte";

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

  // Recurring transactions are computed when the app opens: no background
  // service exists. A failure here must not block the app.
  onMount(async () => {
    theme = loadTheme();
    applyTheme(theme);
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onSystemChange = () => syncSystemBars(theme);
    media.addEventListener("change", onSystemChange);
    try {
      const report = await recurring.catchUp(isoDate());
      if (report.transactionsCreated > 0) {
        notify(`${report.transactionsCreated} transaksi berulang dicatat.`, 6000);
        dataChanged();
      }
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
    }
  });
</script>

<div class="shell">
  <header class="topbar">
    <span class="brand">Money Tracker</span>
    <button
      type="button"
      class="icon-btn"
      onclick={cycleTheme}
      aria-label={`Tema: ${THEME_LABEL[theme]}. Ketuk untuk mengganti.`}
    >
      <Icon name={theme === "auto" ? "auto" : theme === "light" ? "sun" : "moon"} />
    </button>
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
