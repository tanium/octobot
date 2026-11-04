<script lang="ts">
  import { api } from '../lib/api';
  import { notifications } from '../lib/notifications.svelte';
  import { session } from '../lib/session.svelte';

  let username = $state('');
  let password = $state('');

  async function login(e: SubmitEvent) {
    e.preventDefault();
    try {
      const resp = await api.login({ username, password });
      session.start(resp.session);
      notifications.show('success', 'Logged in successfully');
    } catch {
      notifications.show('error', 'Login failed');
    }
  }
</script>

<form style="width: 300px" onsubmit={login}>
  <h2>Login</h2>
  <div class="mb-3">
    <label class="form-label" for="username">Username</label>
    <!-- svelte-ignore a11y_autofocus -->
    <input id="username" type="text" class="form-control" bind:value={username} required autofocus />
  </div>
  <div class="mb-3">
    <label class="form-label" for="password">Password</label>
    <input id="password" type="password" class="form-control" bind:value={password} required />
  </div>
  <button class="btn btn-primary" type="submit">Sign in</button>
</form>
