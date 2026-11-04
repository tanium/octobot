<script lang="ts">
  import { api } from './lib/api';
  import { notifications, reportError } from './lib/notifications.svelte';
  import { router } from './lib/router.svelte';
  import { session } from './lib/session.svelte';
  import Login from './pages/Login.svelte';
  import Repos from './pages/Repos.svelte';
  import Users from './pages/Users.svelte';
  import Versions from './pages/Versions.svelte';

  const sessionCheckMs = 30_000;

  // Notices a server-side session expiry even when the user is idle. Other
  // failures are transient, so the check just runs again next interval.
  $effect(() => {
    if (!session.loggedIn) {
      return;
    }
    const timer = setInterval(() => {
      api.checkSession().catch((e) => reportError('Error checking session', e));
    }, sessionCheckMs);
    return () => clearInterval(timer);
  });

  async function logout() {
    try {
      await api.logout();
    } finally {
      session.clear();
    }
  }
</script>

{#if notifications.error}
  <div class="alert alert-danger alert-dismissible mt-3" role="alert">
    {notifications.error}
    <button type="button" class="btn-close" aria-label="Close" onclick={() => notifications.dismiss('error')}></button>
  </div>
{/if}
{#if notifications.success}
  <div class="alert alert-success alert-dismissible mt-3" role="alert">
    {notifications.success}
    <button type="button" class="btn-close" aria-label="Close" onclick={() => notifications.dismiss('success')}></button>
  </div>
{/if}

<header class="d-flex flex-wrap align-items-end justify-content-between pb-2 mt-4 mb-3 border-bottom">
  <h1 class="mb-0">Octobot</h1>
  {#if session.loggedIn}
    <nav class="d-flex gap-3">
      <a href="#/users" class:fw-bold={router.page === 'users'}>Users</a>
      <a href="#/repos" class:fw-bold={router.page === 'repos'}>Repos</a>
      <a href="#/versions" class:fw-bold={router.page === 'versions'}>Versions</a>
      <button type="button" class="btn btn-link p-0" onclick={logout}>Logout</button>
    </nav>
  {/if}
</header>

{#if !session.loggedIn}
  <Login />
{:else if router.page === 'users'}
  <Users />
{:else if router.page === 'repos'}
  <Repos />
{:else}
  <Versions />
{/if}
