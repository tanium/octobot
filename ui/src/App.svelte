<script lang="ts">
  import { api } from './lib/api';
  import Icon from './lib/Icon.svelte';
  import Palette from './lib/Palette.svelte';
  import { href, router, section } from './lib/router.svelte';
  import { session } from './lib/session.svelte';
  import { store } from './lib/store.svelte';
  import { theme } from './lib/theme.svelte';
  import Toasts from './lib/Toasts.svelte';
  import Login from './pages/Login.svelte';
  import Repo from './pages/Repo.svelte';
  import Repos from './pages/Repos.svelte';
  import User from './pages/User.svelte';
  import Users from './pages/Users.svelte';
  import Versions from './pages/Versions.svelte';

  const sessionCheckMs = 30_000;
  const isMac = /Mac|iPhone|iPad/.test(navigator.userAgent);
  const commitUrl = 'https://github.com/tanium/octobot/commit/';

  let paletteOpen = $state(false);

  // Notices a server-side session expiry even when the user is idle; the API
  // client handles the 403. Other failures are transient, so the check just
  // runs again next interval.
  $effect(() => {
    if (!session.loggedIn) {
      return;
    }
    const timer = setInterval(() => {
      api.checkSession().catch(() => {});
    }, sessionCheckMs);
    return () => clearInterval(timer);
  });

  $effect(() => {
    if (session.loggedIn) {
      store.load();
    } else {
      store.clear();
      paletteOpen = false;
    }
  });

  async function logout() {
    try {
      await api.logout();
    } catch {
      // The local session ends either way; the server's expires on its own.
    }
    session.clear();
  }

  function typing(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLElement &&
      (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))
    );
  }

  function onkeydown(e: KeyboardEvent) {
    if (!session.loggedIn) {
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      paletteOpen = !paletteOpen;
    } else if (e.key === '/' && !paletteOpen && !typing(e.target)) {
      // List pages mark their filter box; elsewhere `/` searches everything.
      e.preventDefault();
      const filter = document.querySelector<HTMLInputElement>('[data-page-filter]');
      if (filter) {
        filter.focus();
      } else {
        paletteOpen = true;
      }
    }
  }

  const nav = [
    { section: 'users', label: 'Users', href: '#/users' },
    { section: 'repos', label: 'Repos', href: '#/repos' },
    { section: 'versions', label: 'Versions', href: '#/versions' },
  ] as const;

  const themeIcon = { system: 'monitor', light: 'sun', dark: 'moon' } as const;
</script>

<svelte:window {onkeydown} />

{#snippet logo()}
  <svg width="26" height="26" viewBox="0 0 24 24" aria-hidden="true">
    <rect width="24" height="24" rx="7" fill="var(--accent)" />
    <g fill="none" stroke="#fff" stroke-width="1.6" stroke-linecap="round">
      <path d="M6.5 12.5a5.5 5.5 0 0 1 11 0" />
      <path d="M6.5 12.5c0 2.5-1.5 3.5-1.5 5M10 12.5c0 2.5-.8 4 .2 6M14 12.5c0 2.5.8 4-.2 6M17.5 12.5c0 2.5 1.5 3.5 1.5 5" />
    </g>
    <circle cx="10" cy="10.5" r="1" fill="#fff" />
    <circle cx="14" cy="10.5" r="1" fill="#fff" />
  </svg>
{/snippet}

<Toasts />

{#if !session.loggedIn}
  <Login {logo} />
{:else}
  <header class="topbar">
    <div class="inner">
      <a class="brand" href="#/users">{@render logo()}<span>Octobot</span></a>
      <nav>
        {#each nav as n (n.section)}
          <a href={n.href} class:active={section(router.route) === n.section}>{n.label}</a>
        {/each}
      </nav>
      <button type="button" class="search" onclick={() => (paletteOpen = true)}>
        <Icon name="search" size={15} />
        <span class="search-label">Search…</span>
        <span class="keys"><kbd>{isMac ? '⌘' : 'Ctrl'}</kbd><kbd>K</kbd></span>
      </button>
      <button
        type="button"
        class="btn btn-ghost btn-icon"
        title="Theme: {theme.preference}"
        aria-label="Theme: {theme.preference}"
        onclick={() => theme.cycle()}
      >
        <Icon name={themeIcon[theme.preference]} />
      </button>
      <button type="button" class="btn btn-ghost btn-icon" title="Log out" aria-label="Log out" onclick={logout}>
        <Icon name="log-out" />
      </button>
    </div>
  </header>

  <main class="inner">
    {#key href(router.route)}
      {#if router.route.page === 'users'}
        <Users />
      {:else if router.route.page === 'user'}
        <User name={router.route.name} />
      {:else if router.route.page === 'repos'}
        <Repos />
      {:else if router.route.page === 'repo'}
        <Repo name={router.route.name} jira={router.route.jira} />
      {:else}
        <Versions project={router.route.project} />
      {/if}
    {/key}
  </main>

  <footer class="inner">
    <span>
      Octobot
      {#if store.commit}
        · <a href="{commitUrl}{store.commit}" target="_blank" rel="noreferrer" title="Deployed commit">
          <Icon name="commit" size={13} /><span class="mono">{store.commit}</span>
        </a>
      {:else}
        · <span class="mono">dev build</span>
      {/if}
    </span>
    <span><kbd>/</kbd> filter · <kbd>{isMac ? '⌘' : 'Ctrl'}</kbd><kbd>K</kbd> search</span>
  </footer>

  <Palette bind:open={paletteOpen} onlogout={logout} />
{/if}

<style>
  .inner {
    width: 100%;
    max-width: 1120px;
    margin: 0 auto;
    padding-inline: 24px;
  }

  @media (max-width: 640px) {
    .inner {
      padding-inline: 16px;
    }
  }

  .topbar {
    position: sticky;
    top: 0;
    z-index: 10;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg) 82%, transparent);
    backdrop-filter: saturate(1.6) blur(12px);
  }

  .topbar .inner {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 56px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    margin-right: 16px;
    color: var(--text);
    font-size: 15px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }

  .brand:hover {
    text-decoration: none;
  }

  nav {
    display: flex;
    gap: 2px;
    margin-right: auto;
  }

  nav a {
    padding: 6px 10px;
    border-radius: var(--radius);
    color: var(--text-2);
    font-weight: 500;
  }

  nav a:hover {
    background: var(--surface-2);
    color: var(--text);
    text-decoration: none;
  }

  nav a.active {
    background: var(--surface-2);
    color: var(--text);
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 240px;
    height: 32px;
    padding: 0 6px 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text-3);
    font: inherit;
    cursor: pointer;
    box-shadow: var(--shadow-sm);
  }

  .search:hover {
    border-color: var(--border-strong);
  }

  .search-label {
    flex: 1;
    text-align: left;
  }

  .keys {
    display: inline-flex;
    gap: 2px;
  }

  @media (max-width: 760px) {
    .brand span {
      display: none;
    }

    .brand {
      margin-right: 4px;
    }

    .search {
      width: 32px;
      padding: 0;
      justify-content: center;
    }

    .search-label,
    .keys {
      display: none;
    }
  }

  main {
    padding-top: 28px;
    padding-bottom: 48px;
    min-height: calc(100vh - 56px - 60px);
  }

  footer {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: 8px;
    padding-top: 18px;
    padding-bottom: 24px;
    border-top: 1px solid var(--border);
    color: var(--text-3);
    font-size: 12px;
  }

  footer a {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text-2);
  }

  footer > span:last-child {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }

  @media (max-width: 640px) {
    footer > span:last-child {
      display: none;
    }
  }
</style>
