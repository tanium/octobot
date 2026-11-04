<script lang="ts">
  import type { Snippet } from 'svelte';

  import { api } from '../lib/api';
  import Icon from '../lib/Icon.svelte';
  import { notifications } from '../lib/notifications.svelte';
  import { session } from '../lib/session.svelte';

  let { logo }: { logo: Snippet } = $props();

  let username = $state('');
  let password = $state('');
  let busy = $state(false);

  async function login(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    try {
      const resp = await api.login({ username, password });
      session.start(resp.session);
    } catch {
      notifications.show('error', 'Login failed');
    } finally {
      busy = false;
    }
  }
</script>

<div class="wrap">
  <form class="card" onsubmit={login}>
    <div class="head">
      {@render logo()}
      <h1>Sign in to Octobot</h1>
    </div>
    <fieldset disabled={busy}>
      <label class="field">
        <span class="field-label">Username</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input" type="text" bind:value={username} autocomplete="username" required autofocus />
      </label>
      <label class="field">
        <span class="field-label">Password</span>
        <input class="input" type="password" bind:value={password} autocomplete="current-password" required />
      </label>
      <button class="btn btn-primary" type="submit">
        {#if busy}<Icon name="loader" spin />{/if}
        Sign in
      </button>
    </fieldset>
  </form>
</div>

<style>
  .wrap {
    display: grid;
    place-items: center;
    min-height: 100vh;
    padding: 16px;
    background:
      radial-gradient(60rem 30rem at 50% -10%, var(--accent-soft), transparent 70%),
      var(--bg);
  }

  form {
    width: min(360px, 100%);
    padding: 28px;
  }

  .head {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    margin-bottom: 24px;
    text-align: center;
  }

  .head :global(svg) {
    width: 40px;
    height: 40px;
    margin-bottom: 4px;
  }

  h1 {
    font-size: 18px;
  }

  fieldset {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .btn {
    height: 36px;
    margin-top: 6px;
  }
</style>
