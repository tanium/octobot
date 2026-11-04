<script lang="ts">
  import type { Snippet } from 'svelte';

  import Icon from './Icon.svelte';

  interface Props {
    open: boolean;
    title: string;
    // Disables the form, including submit, while a request is in flight.
    busy?: boolean;
    // The footer's submit button triggers this, after native form validation.
    onsubmit: () => void;
    children: Snippet;
    footer: Snippet;
  }

  let { open = $bindable(), title, busy = false, onsubmit, children, footer }: Props = $props();

  let dialog: HTMLDialogElement;

  $effect(() => {
    if (open && !dialog.open) {
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });
</script>

<!-- The native dialog supplies focus trapping, Escape, and the backdrop.
     Every close goes through the dialog's close event. -->
<dialog bind:this={dialog} onclose={() => (open = false)}>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      if (!busy) {
        onsubmit();
      }
    }}
  >
    <fieldset disabled={busy}>
      <header>
        <h2>{title}</h2>
        <button type="button" class="btn btn-ghost btn-icon btn-sm" aria-label="Close" onclick={() => (open = false)}>
          <Icon name="x" />
        </button>
      </header>
      <div class="body">
        {@render children()}
      </div>
      <footer>
        {@render footer()}
      </footer>
    </fieldset>
  </form>
</dialog>

<style>
  dialog {
    width: min(440px, calc(100vw - 32px));
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-lg);
  }

  dialog[open] {
    animation: pop 0.14s ease-out;
  }

  dialog::backdrop {
    background: var(--backdrop);
    backdrop-filter: blur(2px);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 14px 0 18px;
  }

  .body {
    padding: 12px 18px 18px;
    color: var(--text-2);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--border);
    background: var(--surface-2);
    border-radius: 0 0 var(--radius-lg) var(--radius-lg);
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(4px) scale(0.98);
    }
  }
</style>
