<script lang="ts">
  interface Props {
    checked: boolean;
    label: string;
    description?: string;
  }

  let { checked = $bindable(), label, description }: Props = $props();
</script>

<label class="switch">
  <span class="text">
    <span class="label">{label}</span>
    {#if description}
      <span class="description">{description}</span>
    {/if}
  </span>
  <input type="checkbox" role="switch" bind:checked />
  <span class="track" aria-hidden="true"><span class="thumb"></span></span>
</label>

<style>
  .switch {
    position: relative;
    display: flex;
    align-items: center;
    gap: 16px;
    cursor: pointer;
  }

  .text {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
  }

  .label {
    font-weight: 500;
  }

  .description {
    font-size: 13px;
    color: var(--text-2);
  }

  input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .track {
    flex: none;
    width: 34px;
    height: 20px;
    padding: 2px;
    border-radius: 999px;
    background: var(--border-strong);
    transition: background 0.15s;
  }

  .thumb {
    display: block;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.25);
    transition: transform 0.15s;
  }

  input:checked + .track {
    background: var(--accent);
  }

  input:checked + .track .thumb {
    transform: translateX(14px);
  }

  input:focus-visible + .track {
    box-shadow: var(--ring);
  }

  input:disabled + .track {
    opacity: 0.5;
  }
</style>
