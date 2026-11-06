<script lang="ts">
  import Avatar from '../lib/Avatar.svelte';
  import { rank } from '../lib/fuzzy';
  import Highlight from '../lib/Highlight.svelte';
  import Icon from '../lib/Icon.svelte';
  import { ListFilter } from '../lib/listFilter.svelte';
  import { href, router } from '../lib/router.svelte';
  import { store } from '../lib/store.svelte';

  const filter = new ListFilter();

  // Whether the latest fetch failed. Without an earlier copy to show, the
  // page offers a retry.
  let failed = $state(false);
  async function refresh() {
    failed = !(await store.refreshUsers());
  }
  refresh();
  let visible = $derived(rank(store.users ?? [], filter.text, (u) => [u.github, u.slack_name, u.email]));

  // Highlights go only on the field that matched.
  const at = (field: number, r: { field: number; positions: number[] }) => (r.field === field ? r.positions : []);
</script>

<div class="page-head">
  <h1>Users {#if store.users}<span class="count">{store.users.length}</span>{/if}</h1>
  <label class="filter">
    <Icon name="search" size={14} />
    <span class="sr-only">Filter users</span>
    <input
      class="input input-sm"
      bind:value={filter.text}
      placeholder="Filter users"
      data-page-filter
      onfocus={() => (filter.focused = true)}
      onblur={() => (filter.focused = false)}
      onkeydown={(e) =>
        filter.keydown(e, visible.length, (i) => router.go({ page: 'user', name: visible[i].item.github }))}
    />
    <kbd>/</kbd>
  </label>
  <a class="btn btn-primary btn-sm" href={href({ page: 'user', name: null })}><Icon name="plus" size={14} />Add user</a>
</div>

<div class="card list">
  <div class="list-head cols">
    <span>User</span>
    <span>Slack</span>
    <span>Notifications</span>
    <span></span>
  </div>
  {#if store.users === null && failed}
    <div class="list-empty">
      Could not load users. <button type="button" class="btn btn-sm" onclick={refresh}>Retry</button>
    </div>
  {:else if store.users === null}
    {#each [1, 2, 3, 4] as i (i)}
      <div class="list-row cols skeleton"><span></span><span></span><span></span><span></span></div>
    {/each}
  {:else}
    {#each visible as r, i (r.item.id)}
      {@const u = r.item}
      <a
        class="list-row cols"
        class:active={filter.isActive(i)}
        data-filter-active={filter.isActive(i) || undefined}
        href={href({ page: 'user', name: u.github })}
      >
        <span class="who">
          <Avatar name={u.github} />
          <span class="cell">
            <span class="name"><Highlight text={u.github} positions={at(0, r)} /></span>
            <span class="email faint"><Highlight text={u.email} positions={at(2, r)} /></span>
          </span>
        </span>
        <span class="cell muted">
          {#if u.slack_name}@<Highlight text={u.slack_name} positions={at(1, r)} />{/if}
        </span>
        <span class="badges">
          {#if u.mute_direct_messages}<span class="badge badge-warning"><Icon name="bell-off" size={12} />DMs muted</span>{/if}
          {#if u.mute_team_direct_messages}<span class="badge badge-warning">Team DMs muted</span>{/if}
          {#if u.muted_repos.length > 0}<span class="badge">{u.muted_repos.length} repo{u.muted_repos.length === 1 ? '' : 's'} muted</span>{/if}
          {#if u.muted_teams.length > 0}<span class="badge">{u.muted_teams.length} team{u.muted_teams.length === 1 ? '' : 's'} muted</span>{/if}
          {#if u.thread_direct_messages}<span class="badge badge-accent">Threaded</span>{/if}
          {#if u.thread_reply_notifications}<span class="badge badge-accent">Thread reply notifications</span>{/if}
          {#if u.group_review_direct_messages}<span class="badge badge-accent">Grouped reviews</span>{/if}
        </span>
        <span class="chev faint"><Icon name="chevron-right" /></span>
      </a>
    {:else}
      <div class="list-empty">
        {#if filter.text}
          No users match “{filter.text}”.
        {:else}
          No users yet. <a href={href({ page: 'user', name: null })}>Add the first one</a>.
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

  .who {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }

  .who .cell {
    display: flex;
    flex-direction: column;
  }

  .name {
    font-weight: 550;
  }

  .email {
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .chev {
    display: inline-flex;
  }
</style>
