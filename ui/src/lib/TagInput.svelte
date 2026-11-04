<script lang="ts">
  import Icon from './Icon.svelte';
  import { parseList } from './list';

  interface Props {
    value: string[];
    placeholder: string;
    id?: string;
  }

  let { value = $bindable(), placeholder, id }: Props = $props();

  // Chips are committed entries. Text not yet turned into a chip still
  // counts toward value, so saving mid-entry keeps it.
  let tags = $state([...value]);
  let text = $state('');
  let input: HTMLInputElement;

  function sync() {
    value = [...tags, ...parseList(text).filter((t) => !tags.includes(t))];
  }

  function commit(raw: string) {
    for (const t of parseList(raw)) {
      if (!tags.includes(t)) {
        tags.push(t);
      }
    }
  }

  function oninput() {
    // A comma, typed or pasted, ends an entry.
    if (text.includes(',')) {
      const cut = text.lastIndexOf(',');
      commit(text.slice(0, cut));
      text = text.slice(cut + 1).trimStart();
    }
    sync();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && text.trim() !== '') {
      // Enter makes a chip instead of submitting the form.
      e.preventDefault();
      commit(text);
      text = '';
      sync();
    } else if (e.key === 'Backspace' && text === '' && tags.length > 0) {
      tags.pop();
      sync();
    }
  }

  function remove(i: number) {
    tags.splice(i, 1);
    sync();
    input.focus();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="tags" onclick={() => input.focus()}>
  {#each tags as tag, i (tag)}
    <span class="tag">
      {tag}
      <button type="button" aria-label="Remove {tag}" onclick={() => remove(i)}><Icon name="x" size={12} /></button>
    </span>
  {/each}
  <input
    {id}
    bind:this={input}
    bind:value={text}
    {oninput}
    {onkeydown}
    onblur={() => {
      commit(text);
      text = '';
      sync();
    }}
    placeholder={tags.length === 0 ? placeholder : ''}
  />
</div>

<style>
  .tags {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    min-height: 34px;
    padding: 4px 6px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--surface);
    cursor: text;
  }

  .tags:focus-within {
    border-color: var(--accent);
    box-shadow: var(--ring);
  }

  .tag {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    height: 24px;
    padding: 0 4px 0 8px;
    border-radius: var(--radius-sm);
    background: var(--accent-soft);
    color: var(--accent-text);
    font-family: var(--mono);
    font-size: 12px;
  }

  .tag button {
    display: inline-flex;
    padding: 2px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: inherit;
    cursor: pointer;
  }

  .tag button:hover {
    background: color-mix(in srgb, currentColor 15%, transparent);
  }

  input {
    flex: 1;
    min-width: 120px;
    height: 24px;
    padding: 0 4px;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
  }

  input:focus {
    outline: none;
    box-shadow: none;
  }

  input::placeholder {
    color: var(--text-3);
  }
</style>
