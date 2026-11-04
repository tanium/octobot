<script lang="ts">
  import type { RepoInfo } from '../bindings/RepoInfo';
  import { fuzzy, rank } from '../lib/fuzzy';
  import Highlight from '../lib/Highlight.svelte';
  import Icon from '../lib/Icon.svelte';
  import { ListFilter } from '../lib/listFilter.svelte';
  import { href, router } from '../lib/router.svelte';
  import { store } from '../lib/store.svelte';

  const maxChips = 5;

  const filter = new ListFilter();
  let needle = $derived(filter.text);
  // Fields: the repo, its channel, then each Jira project key.
  let visible = $derived(
    rank(store.repos ?? [], needle, (r) => [r.repo, r.channel, ...r.jira_config.map((c) => c.jira_project)]),
  );

  const at = (field: number, r: { field: number; positions: number[] }) => (r.field === field ? r.positions : []);

  // A match on a Jira key opens the repo on that project, as the palette does.
  function open(i: number) {
    const { item, field } = visible[i];
    const jira = field >= 2 ? item.jira_config[field - 2].jira_project : null;
    router.go({ page: 'repo', name: item.repo, jira });
  }

  // Jira keys that match the filter come first, so a monorepo's long list
  // shows why it matched.
  function chips(repo: RepoInfo): { key: string; hit: boolean }[] {
    const keys = repo.jira_config.map((c) => c.jira_project).filter((k) => k !== '');
    if (needle.trim() === '') {
      return keys.map((key) => ({ key, hit: false }));
    }
    const all = keys.map((key) => ({ key, hit: fuzzy(needle, key) !== null }));
    return [...all.filter((c) => c.hit), ...all.filter((c) => !c.hit)];
  }

  // "org/repo" splits so the org can be quieter; a bare org stays whole.
  function parts(name: string): [string, string] {
    const i = name.indexOf('/');
    return i === -1 ? ['', name] : [name.slice(0, i + 1), name.slice(i + 1)];
  }
</script>

<div class="page-head">
  <h1>Repos {#if store.repos}<span class="count">{store.repos.length}</span>{/if}</h1>
  <label class="filter">
    <Icon name="search" size={14} />
    <span class="sr-only">Filter repos</span>
    <input
      class="input input-sm"
      bind:value={filter.text}
      placeholder="Filter repos, channels, Jira"
      data-page-filter
      onfocus={() => (filter.focused = true)}
      onblur={() => (filter.focused = false)}
      onkeydown={(e) => filter.keydown(e, visible.length, open)}
    />
    <kbd>/</kbd>
  </label>
  <a class="btn btn-primary btn-sm" href={href({ page: 'repo', name: null, jira: null })}><Icon name="plus" size={14} />Add repo</a>
</div>

<div class="card list">
  <div class="list-head cols">
    <span>Repository</span>
    <span>Slack channel</span>
    <span>Jira projects</span>
    <span></span>
  </div>
  {#if store.repos === null}
    {#each [1, 2, 3, 4] as i (i)}
      <div class="list-row cols skeleton"><span></span><span></span><span></span><span></span></div>
    {/each}
  {:else}
    {#each visible as r, i (r.item.id)}
      {@const repo = r.item}
      {@const [org, rest] = parts(repo.repo)}
      {@const cs = chips(repo)}
      <a
        class="list-row cols"
        class:active={filter.isActive(i)}
        data-filter-active={filter.isActive(i) || undefined}
        href={href({ page: 'repo', name: repo.repo, jira: null })}
      >
        <span class="repo cell">
          <span class="icon faint"><Icon name="repo" /></span>
          <span class="mono cell">
            {#if r.field === 0 && r.positions.length > 0}
              <Highlight text={repo.repo} positions={r.positions} />
            {:else}
              <span class="faint">{org}</span><strong>{rest}</strong>
            {/if}
          </span>
          {#if !org}<span class="badge">org-wide</span>{/if}
        </span>
        <span class="cell muted">#<Highlight text={repo.channel} positions={at(1, r)} /></span>
        <span class="chips">
          {#each cs.slice(0, maxChips) as c (c.key)}
            <span class="chip" class:hit={c.hit}>{c.key}</span>
          {:else}
            <span class="faint">None</span>
          {/each}
          {#if cs.length > maxChips}
            <span class="more faint">+{cs.length - maxChips}</span>
          {/if}
        </span>
        <span class="chev faint"><Icon name="chevron-right" /></span>
      </a>
    {:else}
      <div class="list-empty">
        {#if needle}
          No repos match “{needle}”.
        {:else}
          No repos yet. <a href={href({ page: 'repo', name: null, jira: null })}>Add the first one</a>.
        {/if}
      </div>
    {/each}
  {/if}
</div>

<style>
  .cols {
    grid-template-columns: minmax(0, 2.2fr) minmax(0, 1.2fr) minmax(0, 2fr) 16px;
  }

  @media (max-width: 760px) {
    .cols {
      grid-template-columns: minmax(0, 1fr) 16px;
    }

    .cols > :nth-child(2),
    .cols > :nth-child(3) {
      display: none;
    }
  }

  .repo {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .repo strong {
    font-weight: 600;
  }

  .icon {
    display: inline-flex;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
  }

  .chip.hit {
    border-color: transparent;
    background: var(--accent-soft);
    color: var(--accent-text);
  }

  .more {
    font-size: 12px;
    font-weight: 500;
  }

  .chev {
    display: inline-flex;
  }
</style>
