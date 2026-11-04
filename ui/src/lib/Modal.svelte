<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    open: boolean;
    title: string;
    large?: boolean;
    // The footer's submit button triggers this, after native form validation.
    onsubmit: () => void;
    children: Snippet;
    footer: Snippet;
  }

  let { open = $bindable(), title, large = false, onsubmit, children, footer }: Props = $props();

  let dialog: HTMLDialogElement;

  $effect(() => {
    if (open && !dialog.open) {
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });
</script>

<!-- Escape and the close button both go through the dialog's close event. -->
<dialog bind:this={dialog} class:large onclose={() => (open = false)}>
  <form
    class="modal-content"
    onsubmit={(e) => {
      e.preventDefault();
      onsubmit();
    }}
  >
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
  </form>
</dialog>

<style>
  dialog {
    width: min(500px, calc(100% - 2rem));
    padding: 0;
    border: 0;
    border-radius: var(--bs-border-radius-lg);
  }

  dialog.large {
    width: min(800px, calc(100% - 2rem));
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.5);
  }
</style>
