<script lang="ts">
  import type { UserInfo } from '../bindings/UserInfo';
  import { api } from '../lib/api';
  import { matches } from '../lib/filter';
  import ListInput from '../lib/ListInput.svelte';
  import Modal from '../lib/Modal.svelte';
  import { errorText, notifications, reportError } from '../lib/notifications.svelte';

  function newUser(): UserInfo {
    return {
      id: null,
      github: '',
      slack_name: '',
      slack_id: '',
      email: '',
      mute_direct_messages: false,
      mute_team_direct_messages: false,
      muted_repos: [],
      muted_teams: [],
      // null applies the server default until the user picks a value.
      thread_direct_messages: null,
      group_review_direct_messages: null,
    };
  }

  let users = $state<UserInfo[]>([]);
  let needle = $state('');
  let visible = $derived(users.filter((u) => matches(needle, u.github, u.slack_name, u.email)));

  // A copy of the user being edited, so Cancel leaves the list untouched.
  let editing = $state<UserInfo | null>(null);
  let modalOpen = $state(false);
  let formError = $state('');
  let saving = $state(false);

  async function refresh() {
    try {
      users = (await api.users()).users;
    } catch (e) {
      reportError('Error getting users', e);
    }
  }

  function edit(user: UserInfo) {
    editing = structuredClone($state.snapshot(user));
    formError = '';
    modalOpen = true;
  }

  async function save() {
    const user = editing!;
    formError = '';
    saving = true;
    const error = await submit(user);
    saving = false;

    // Escape can close the dialog mid-save and another edit can open, so
    // only the dialog that started this save takes its result.
    if (editing === user) {
      if (error) {
        formError = error;
      } else {
        modalOpen = false;
      }
    }
    if (!error) {
      await refresh();
    }
  }

  // Returns the error to show in the form, or null once saved.
  async function submit(user: UserInfo): Promise<string | null> {
    try {
      const slack = await api.verifyEmail(user.email);
      if (!slack.id || !slack.name) {
        return 'Failed to lookup slack username for given email address';
      }
      user.slack_id = slack.id;
      user.slack_name = slack.name;
    } catch (e) {
      return `Error verifying email address: ${errorText(e)}`;
    }

    try {
      if (user.id === null) {
        await api.createUser(user);
        notifications.show('success', 'Added user successfully');
      } else {
        await api.updateUser(user);
        notifications.show('success', 'Edited user successfully');
      }
      return null;
    } catch (e) {
      return `Error saving user: ${errorText(e)}`;
    }
  }

  async function remove(user: UserInfo) {
    if (!confirm(`Are you sure you want to delete user ${user.github}?`)) {
      return;
    }
    try {
      await api.deleteUser(user.id!);
      notifications.show('success', 'Removed user successfully');
    } catch (e) {
      reportError('Error removing user', e);
    }
    await refresh();
  }

  refresh();
</script>

<div class="d-flex flex-wrap gap-2 align-items-center mb-2">
  <h3 class="me-auto mb-0">Users</h3>
  <input type="text" class="form-control form-control-sm w-auto" bind:value={needle} placeholder="Filter" />
  <button type="button" class="btn btn-sm btn-primary" onclick={() => edit(newUser())}>Add user</button>
</div>

<table class="table">
  <thead>
    <tr>
      <th>GitHub Username</th>
      <th>Slack Username</th>
      <th></th>
    </tr>
  </thead>
  <tbody>
    {#each visible as user (user.id)}
      <tr>
        <td>{user.github}</td>
        <td>{user.slack_name}</td>
        <td class="text-end text-nowrap">
          <button type="button" class="btn btn-link p-0 me-2" title="Edit" aria-label="Edit" onclick={() => edit(user)}>
            <i class="bi bi-pencil"></i>
          </button>
          <button type="button" class="btn btn-link p-0" title="Remove" aria-label="Remove" onclick={() => remove(user)}>
            <i class="bi bi-trash"></i>
          </button>
        </td>
      </tr>
    {/each}
  </tbody>
</table>

<Modal bind:open={modalOpen} title={editing?.id ? 'Edit user' : 'Add user'} busy={saving} onsubmit={save}>
  <!-- Remounted per open, so ListInput reseeds from the new user. -->
  {#if modalOpen && editing}
    <div class="mb-3">
      <!-- svelte-ignore a11y_autofocus -->
      <input type="text" class="form-control" bind:value={editing.github} placeholder="GitHub username" required autofocus />
    </div>
    <div class="mb-3">
      <input type="email" class="form-control" bind:value={editing.email} placeholder="email address" required />
    </div>
    <div class="form-check">
      <label class="form-check-label">
        <input type="checkbox" class="form-check-input" bind:checked={editing.mute_direct_messages} />
        Mute Direct Messages
      </label>
    </div>
    <div class="form-check mb-3">
      <label class="form-check-label">
        <input type="checkbox" class="form-check-input" bind:checked={editing.mute_team_direct_messages} />
        Mute Team Direct Messages
      </label>
    </div>
    <div class="mb-3">
      <label class="form-label w-100">
        Muted repos (comma separated)
        <ListInput bind:value={editing.muted_repos} placeholder="some-org/some-repo, some-org/other-repo" />
      </label>
    </div>
    <div class="mb-3">
      <label class="form-label w-100">
        Muted requested teams (comma separated)
        <ListInput bind:value={editing.muted_teams} placeholder="some-org/some-team, some-org/other-team" />
      </label>
    </div>
    <div class="form-check">
      <label class="form-check-label">
        <input
          type="checkbox"
          class="form-check-input"
          bind:checked={() => editing!.thread_direct_messages ?? false, (v) => (editing!.thread_direct_messages = v)}
        />
        Thread Direct Messages per Pull Request
      </label>
    </div>
    <div class="form-check">
      <label class="form-check-label">
        <input
          type="checkbox"
          class="form-check-input"
          bind:checked={
            () => editing!.group_review_direct_messages ?? false,
            (v) => (editing!.group_review_direct_messages = v)
          }
        />
        Group Review Comments into One Direct Message
      </label>
    </div>
    {#if formError}
      <div class="alert alert-danger mt-3 mb-0" role="alert">{formError}</div>
    {/if}
  {/if}

  {#snippet footer()}
    <button type="button" class="btn btn-secondary" onclick={() => (modalOpen = false)}>Cancel</button>
    <button type="submit" class="btn btn-primary">Save</button>
  {/snippet}
</Modal>
