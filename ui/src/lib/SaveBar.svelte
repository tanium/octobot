<script lang="ts">
  import Icon from './Icon.svelte';

  interface Props {
    // Shown while there is something to save: edits, or a new record.
    show: boolean;
    saving: boolean;
    isNew: boolean;
    error: string;
    ondiscard: () => void;
  }

  let { show, saving, isNew, error, ondiscard }: Props = $props();
</script>

{#if show || error}
  <div class="bar" class:error={!!error}>
    <span class="msg">
      {#if error}
        <Icon name="alert" />{error}
      {:else if isNew}
        Not saved yet
      {:else}
        <span class="dot"></span>Unsaved changes
      {/if}
    </span>
    {#if !isNew}
      <button type="button" class="btn btn-ghost btn-sm" disabled={saving} onclick={ondiscard}>Discard</button>
    {/if}
    <!-- Outside the form's disabled fieldset, so it shows its own state. -->
    <button type="submit" class="btn btn-primary btn-sm" disabled={saving}>
      {#if saving}<Icon name="loader" spin size={14} />{/if}
      {isNew ? 'Create' : 'Save changes'}
    </button>
  </div>
{/if}

<style>
  .bar {
    position: sticky;
    bottom: 16px;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 20px;
    padding: 10px 10px 10px 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: var(--shadow-lg);
    animation: rise 0.16s ease-out;
  }

  .msg {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-weight: 500;
  }

  .error .msg {
    color: var(--danger);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--warning);
  }

  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
</style>
