<script lang="ts">
  type Name =
    | "wallet"
    | "list"
    | "target"
    | "repeat"
    | "chart"
    | "plus"
    | "sun"
    | "moon"
    | "auto"
    | "close"
    | "chevron-left"
    | "chevron-right"
    | "filter"
    | "search"
    | "check";

  let { name, size = 24 }: { name: Name; size?: number } = $props();

  const PATHS: Record<Name, string> = {
    wallet: "M3 7a2 2 0 0 1 2-2h13v3 M3 7v10a2 2 0 0 0 2 2h14a1 1 0 0 0 1-1v-9a1 1 0 0 0-1-1H5a2 2 0 0 1-2-2 M16 13.5h.01",
    list: "M8 6h12 M8 12h12 M8 18h12 M4 6h.01 M4 12h.01 M4 18h.01",
    target: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8z M12 12h.01",
    repeat: "M17 2l4 4-4 4 M3 11V9a3 3 0 0 1 3-3h15 M7 22l-4-4 4-4 M21 13v2a3 3 0 0 1-3 3H3",
    chart: "M4 20V10 M10 20V4 M16 20v-7 M22 20H2",
    plus: "M12 5v14 M5 12h14",
    sun: "M12 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10z M12 1v2 M12 21v2 M4.2 4.2l1.4 1.4 M18.4 18.4l1.4 1.4 M1 12h2 M21 12h2 M4.2 19.8l1.4-1.4 M18.4 5.6l1.4-1.4",
    moon: "M21 12.8A9 9 0 1 1 11.2 3 7 7 0 0 0 21 12.8z",
    auto: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M12 3v18",
    close: "M6 6l12 12 M18 6L6 18",
    "chevron-left": "M15 6l-6 6 6 6",
    "chevron-right": "M9 6l6 6-6 6",
    filter: "M4 5h16 M7 12h10 M10 19h4",
    search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14z M21 21l-4.5-4.5",
    check: "M5 12l5 5 9-10",
  };
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  <path d={PATHS[name]} />
</svg>
