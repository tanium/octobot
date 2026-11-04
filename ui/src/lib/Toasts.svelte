<script lang="ts">
  import Icon from './Icon.svelte';
  import { notifications } from './notifications.svelte';
</script>

<div class="toasts">
  {#if notifications.error}
    <div class="toast error" role="alert">
      <Icon name="alert" />
      <span class="msg">{notifications.error}</span>
      <button type="button" aria-label="Dismiss" onclick={() => notifications.dismiss('error')}><Icon name="x" size={14} /></button>
    </div>
  {/if}
  {#if notifications.success}
    <div class="toast success" role="status">
      <Icon name="check" />
      <span class="msg">{notifications.success}</span>
      <button type="button" aria-label="Dismiss" onclick={() => notifications.dismiss('success')}><Icon name="x" size={14} /></button>
    </div>
  {/if}
</div>

<style>
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 16px;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: min(380px, calc(100vw - 32px));
  }

  .toast {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px 12px 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-lg);
    animation: slide 0.18s ease-out;
  }

  .toast :global(svg) {
    margin-top: 2px;
  }

  .error > :global(svg:first-child) {
    color: var(--danger);
  }

  .success > :global(svg:first-child) {
    color: var(--success);
  }

  .msg {
    flex: 1;
    overflow-wrap: anywhere;
  }

  button {
    display: inline-flex;
    padding: 3px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--text-3);
    cursor: pointer;
  }

  button:hover {
    background: var(--surface-2);
    color: var(--text);
  }

  @keyframes slide {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
</style>
