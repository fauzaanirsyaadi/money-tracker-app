<script lang="ts">
  import type { Snippet } from "svelte";

  import { pushState } from "$app/navigation";
  import { page } from "$app/state";

  import Icon from "./Icon.svelte";

  let {
    open = $bindable(false),
    title,
    children,
    footer,
  }: { open: boolean; title: string; children: Snippet; footer?: Snippet } = $props();

  let dialog = $state<HTMLDialogElement>();
  // True while we own a history entry pushed for this sheet.
  let pushed = false;

  $effect(() => {
    if (!dialog) return;
    if (open && !dialog.open) {
      dialog.showModal();
      pushState("", { sheet: true });
      pushed = true;
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });

  // The hardware Back button pops our entry: close instead of leaving the screen.
  $effect(() => {
    if (open && pushed && !page.state.sheet) {
      pushed = false;
      open = false;
    }
  });

  export function close(): void {
    if (pushed) {
      pushed = false;
      history.back();
    }
    open = false;
  }

  function onBackdrop(event: MouseEvent) {
    if (event.target === dialog) close();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  aria-labelledby="sheet-title"
  onclick={onBackdrop}
  oncancel={(event) => {
    event.preventDefault();
    close();
  }}
>
  <div class="panel">
    <header>
      <h2 id="sheet-title">{title}</h2>
      <button type="button" class="x" aria-label="Tutup" onclick={close}>
        <Icon name="close" />
      </button>
    </header>
    <div class="body">
      {@render children()}
    </div>
    {#if footer}
      <!-- Pinned outside the scroll area so the primary action stays visible
           above the on-screen keyboard. -->
      <footer>
        {@render footer()}
      </footer>
    {/if}
  </div>
</dialog>

<style>
  dialog {
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    max-width: none;
    max-height: none;
    width: 100%;
    height: 100%;
    margin: 0;
    overflow: hidden;
  }
  dialog::backdrop {
    background: var(--scrim);
  }
  .panel {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    max-height: 92dvh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border-radius: var(--radius-lg) var(--radius-lg) 0 0;
    box-shadow: var(--shadow);
    animation: rise 0.18s ease-out;
    padding-bottom: env(safe-area-inset-bottom);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);
  }
  .x {
    display: grid;
    place-items: center;
    width: var(--tap);
    height: var(--tap);
    margin-right: calc(var(--space-2) * -1);
    border: 0;
    border-radius: 999px;
    background: transparent;
  }
  footer {
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--border);
    background: var(--surface);
  }
  .body {
    overflow-y: auto;
    flex: 1 1 auto;
    min-height: 0;
    padding: var(--space-4);
    overscroll-behavior: contain;
  }
  @keyframes rise {
    from {
      transform: translateY(2rem);
      opacity: 0;
    }
  }
  @media (min-width: 720px) {
    .panel {
      left: 50%;
      right: auto;
      bottom: auto;
      top: 50%;
      width: min(34rem, 92vw);
      transform: translate(-50%, -50%);
      border-radius: var(--radius-lg);
      animation: none;
    }
  }
</style>
