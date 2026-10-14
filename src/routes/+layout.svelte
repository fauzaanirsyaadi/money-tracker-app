<script lang="ts">
  import { page } from "$app/state";

  let { children } = $props();

  const links = [
    { href: "/", label: "Rekening" },
    { href: "/transactions", label: "Transaksi" },
  ];
</script>

<nav aria-label="Navigasi utama">
  {#each links as link}
    <a href={link.href} aria-current={page.url.pathname === link.href ? "page" : undefined}>
      {link.label}
    </a>
  {/each}
</nav>

{@render children()}

<style>
  nav {
    display: flex;
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
</style>
