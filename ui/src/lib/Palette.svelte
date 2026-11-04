<script lang="ts">
  import { tick, untrack } from 'svelte';

  import { rank } from './fuzzy';
  import Highlight from './Highlight.svelte';
  import Icon from './Icon.svelte';
  import { recordItems, type Item } from './palette';
  import { router } from './router.svelte';
  import { store } from './store.svelte';
  import { theme } from './theme.svelte';

  interface Props {
    open: boolean;
    onlogout: () => void;
  }

  let { open = $bindable(), onlogout }: Props = $props();

  const limit = 50;

  let dialog: HTMLDialogElement;
  let input = $state<HTMLInputElement>();
  let list = $state<HTMLElement>();
  let query = $state('');
  let active = $state(0);

  const pages: Item[] = [
    { key: 'p:users', kind: 'Page', icon: 'users', label: 'Users', detail: '', keywords: [], target: { page: 'users' } },
    { key: 'p:repos', kind: 'Page', icon: 'repo', label: 'Repos', detail: '', keywords: [], target: { page: 'repos' } },
    {
      key: 'p:versions',
      kind: 'Page',
      icon: 'merge',
      label: 'Versions',
      detail: 'Merge pending Jira versions',
      keywords: ['merge', 'release'],
      target: { page: 'versions', project: null },
    },
  ];

  let actions = $derived<Item[]>([
    { key: 'a:user', kind: 'Action', icon: 'plus', label: 'Add user', detail: '', keywords: ['new', 'create'], target: { page: 'user', name: null } },
    {
      key: 'a:repo',
      kind: 'Action',
      icon: 'plus',
      label: 'Add repo',
      detail: '',
      keywords: ['new', 'create'],
      target: { page: 'repo', name: null, jira: null },
    },
    ...(['light', 'dark', 'system'] as const)
      .filter((p) => p !== theme.preference)
      .map(
        (p): Item => ({
          key: `a:theme:${p}`,
          kind: 'Action',
          icon: p === 'light' ? 'sun' : p === 'dark' ? 'moon' : 'monitor',
          label: p === 'system' ? 'Use system theme' : `Switch to ${p} theme`,
          detail: '',
          keywords: ['theme', 'appearance', 'mode'],
          target: () => theme.set(p),
        }),
      ),
    { key: 'a:logout', kind: 'Action', icon: 'log-out', label: 'Log out', detail: '', keywords: ['sign out'], target: onlogout },
  ]);

  let items = $derived([...pages, ...actions, ...recordItems(store.users ?? [], store.repos ?? [])]);

  // With no query, only pages and actions show; records need a search.
  let results = $derived(
    query.trim() === ''
      ? rank([...pages, ...actions], '', () => [''])
      : rank(items, query, (i) => [i.label, ...i.keywords]).slice(0, limit),
  );

  $effect(() => {
    if (open && !dialog.open) {
      query = '';
      active = 0;
      dialog.showModal();
      input?.focus();
      // Searches what is cached right away; fresh results replace it.
      untrack(() => {
        store.refreshUsers();
        store.refreshRepos();
      });
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });

  // A new query starts back at the top.
  $effect(() => {
    void query;
    active = 0;
  });

  async function move(delta: number) {
    if (results.length === 0) {
      return;
    }
    active = (active + delta + results.length) % results.length;
    await tick();
    list?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
  }

  function run(item: Item) {
    open = false;
    if (typeof item.target === 'function') {
      item.target();
    } else {
      router.go(item.target);
    }
  }

  function onkeydown(e: KeyboardEvent) {
    const ctrl = e.ctrlKey && !e.metaKey;
    if (e.key === 'ArrowDown' || (ctrl && e.key === 'n')) {
      e.preventDefault();
      move(1);
    } else if (e.key === 'ArrowUp' || (ctrl && e.key === 'p')) {
      e.preventDefault();
      move(-1);
    } else if (e.key === 'Enter' && results[active]) {
      e.preventDefault();
      run(results[active].item);
    }
  }
</script>

<dialog
  bind:this={dialog}
  onclose={() => (open = false)}
  onclick={(e) => {
    // A click on the backdrop lands on the dialog itself.
    if (e.target === dialog) {
      open = false;
    }
  }}
  aria-label="Search"
>
  <div class="panel">
    <div class="search">
      <Icon name="search" size={18} />
      <input
        bind:this={input}
        bind:value={query}
        {onkeydown}
        placeholder="Search users, repos, Jira projects…"
        role="combobox"
        aria-expanded="true"
        aria-controls="palette-results"
        aria-activedescendant={results[active] ? `palette-${results[active].item.key}` : undefined}
        spellcheck="false"
        autocomplete="off"
      />
      <kbd>esc</kbd>
    </div>

    <ul id="palette-results" role="listbox" bind:this={list}>
      {#each results as r, i (r.item.key)}
        <li
          id="palette-{r.item.key}"
          role="option"
          aria-selected={i === active}
          onmousemove={() => (active = i)}
          onclick={() => run(r.item)}
          onkeydown={() => {}}
        >
          <span class="icon"><Icon name={r.item.icon} /></span>
          <span class="label">
            {#if r.field === 0}<Highlight text={r.item.label} positions={r.positions} />{:else}{r.item.label}{/if}
          </span>
          <span class="detail">
            {#if r.field > 0}
              <Highlight text={r.item.keywords[r.field - 1]} positions={r.positions} />
            {:else}
              {r.item.detail}
            {/if}
          </span>
          <span class="kind">{r.item.kind}</span>
        </li>
      {:else}
        <li class="empty">No matches for “{query}”</li>
      {/each}
    </ul>

    <footer>
      <span><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
      <span><kbd>↵</kbd> open</span>
      {#if query.trim() === ''}
        <span class="hint">Type to search {store.users?.length ?? 0} users and {store.repos?.length ?? 0} repos</span>
      {/if}
    </footer>
  </div>
</dialog>

<style>
  dialog {
    width: min(640px, calc(100vw - 32px));
    max-height: none;
    margin: 12vh auto auto;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
  }

  dialog::backdrop {
    background: var(--backdrop);
    backdrop-filter: blur(2px);
  }

  dialog[open] .panel {
    animation: pop 0.12s ease-out;
  }

  .panel {
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: var(--shadow-lg);
  }

  .search {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border);
    color: var(--text-3);
  }

  .search input {
    flex: 1;
    height: 52px;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: 16px;
  }

  .search input:focus {
    outline: none;
    box-shadow: none;
  }

  .search input::placeholder {
    color: var(--text-3);
  }

  ul {
    max-height: min(420px, 55vh);
    margin: 0;
    padding: 6px;
    overflow-y: auto;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: 20px auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 10px;
    height: 40px;
    padding: 0 10px;
    border-radius: var(--radius);
    cursor: pointer;
  }

  li[aria-selected='true'] {
    background: var(--accent-soft);
  }

  li[aria-selected='true'] .icon {
    color: var(--accent-text);
  }

  .icon {
    display: inline-flex;
    color: var(--text-3);
  }

  .label {
    font-weight: 500;
    white-space: nowrap;
  }

  .detail {
    overflow: hidden;
    color: var(--text-3);
    font-size: 13px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kind {
    color: var(--text-3);
    font-size: 12px;
  }

  li.empty {
    display: block;
    height: auto;
    padding: 28px 10px;
    text-align: center;
    color: var(--text-2);
    cursor: default;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 8px 14px;
    border-top: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-3);
    font-size: 12px;
  }

  footer span {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  .hint {
    margin-left: auto;
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.98);
    }
  }
</style>
