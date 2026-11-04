<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    open: boolean;
    title: string;
    large?: boolean;
    // Disables the form, including submit, while a save is in flight.
    busy?: boolean;
    // The footer's submit button triggers this, after native form validation.
    onsubmit: () => void;
    children: Snippet;
    footer: Snippet;
  }

  let { open = $bindable(), title, large = false, busy = false, onsubmit, children, footer }: Props = $props();

  let dialog: HTMLDialogElement;

  $effect(() => {
    if (open && !dialog.open) {
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });
</script>

<!-- The native dialog supplies focus trapping, Escape, and the backdrop;
     Bootstrap's .modal classes supply the layout. Every close goes through
     the dialog's close event. -->
<dialog bind:this={dialog} class="modal" onclose={() => (open = false)}>
  <div class="modal-dialog" class:modal-lg={large}>
    <form
      class="modal-content"
      onsubmit={(e) => {
        e.preventDefault();
        if (!busy) {
          onsubmit();
        }
      }}
    >
      <fieldset disabled={busy}>
        <div class="modal-header">
          <h4 class="modal-title">{title}</h4>
          <button type="button" class="btn-close" aria-label="Close" onclick={() => (open = false)}></button>
        </div>
        <div class="modal-body">
          {@render children()}
        </div>
        <div class="modal-footer">
          {@render footer()}
        </div>
      </fieldset>
    </form>
  </div>
</dialog>

<style>
  /* Undo the user agent's dialog box so .modal can fill the viewport. */
  dialog {
    max-width: none;
    max-height: none;
    padding: 0;
    border: 0;
    background: transparent;
  }

  dialog[open] {
    display: block;
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.5);
  }
</style>
