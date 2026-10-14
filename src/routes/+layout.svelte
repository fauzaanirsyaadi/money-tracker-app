<script lang="ts">
  import { onMount } from "svelte";

  import { page } from "$app/state";
  import { ApiError, recurring } from "$lib/api";
  import { isoDate } from "$lib/dates";

  let { children } = $props();

  const links = [
    { href: "/", label: "Rekening" },
    { href: "/transactions", label: "Transaksi" },
    { href: "/budgets", label: "Anggaran" },
    { href: "/recurring", label: "Berulang" },
    { href: "/reports", label: "Laporan" },
  ];

  let notice = $state("");

  // Recurring transactions are computed when the app opens: no background
  // service exists. Failures here must not block the app.
  onMount(async () => {
    try {
      const report = await recurring.catchUp(isoDate());
      if (report.transactionsCreated > 0) {
        notice = `${report.transactionsCreated} transaksi berulang dicatat.`;
      }
    } catch (err) {
      if (!(err instanceof ApiError)) throw err;
    }
  });
</script>

<nav aria-label="Navigasi utama">
  {#each links as link}
    <a href={link.href} aria-current={page.url.pathname === link.href ? "page" : undefined}>
      {link.label}
    </a>
  {/each}
</nav>

{#if notice}
  <p class="notice" role="status">
    {notice}
    <button type="button" onclick={() => (notice = "")}>Tutup</button>
  </p>
{/if}

{@render children()}

<style>
  nav {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    padding: 0.5rem 1rem 0;
    max-width: 36rem;
    margin: 0 auto;
  }
  a {
    padding: 0.5rem 0.75rem;
    border-radius: 0.5rem;
    color: inherit;
    text-decoration: none;
  }
  a[aria-current="page"] {
    font-weight: 600;
    background: color-mix(in srgb, currentColor 12%, transparent);
  }
  .notice {
    max-width: 36rem;
    margin: 0.5rem auto 0;
    padding: 0.5rem 0.75rem;
    border-radius: 0.5rem;
    background: color-mix(in srgb, #2e7d32 15%, transparent);
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
</style>
