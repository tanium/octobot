<script lang="ts">
  import type { UserInfo } from '../bindings/UserInfo';
  import { api } from '../lib/api';
  import Avatar from '../lib/Avatar.svelte';
  import { Draft, guardUnsaved } from '../lib/draft.svelte';
  import Icon from '../lib/Icon.svelte';
  import Modal from '../lib/Modal.svelte';
  import { errorText, notifications, reportError } from '../lib/notifications.svelte';
  import { href, router } from '../lib/router.svelte';
  import SaveBar from '../lib/SaveBar.svelte';
  import { session } from '../lib/session.svelte';
  import { store } from '../lib/store.svelte';
  import Switch from '../lib/Switch.svelte';
  import TagInput from '../lib/TagInput.svelte';

  // The user's GitHub login, or null for a new user.
  let { name }: { name: string | null } = $props();

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
      thread_reply_notifications: null,
    };
  }

  const draft = new Draft<UserInfo>();
  let missing = $state(false);
  let loadFailed = $state(false);
  let deleteOpen = $state(false);
  let deleting = $state(false);
  // A request can finish after the page is gone; its result then goes to a
  // toast, and it must not navigate.
  let alive = true;

  $effect(() => () => (alive = false));

  // Seeds the draft from a fresh fetch, since a save sends the whole record.
  async function load() {
    loadFailed = false;
    const ok = await store.refreshUsers();
    if (!alive) {
      return;
    }
    if (!ok) {
      loadFailed = true;
      return;
    }
    const found = store.users!.find((u) => u.github === name);
    if (found) {
      draft.start($state.snapshot(found));
    } else {
      missing = true;
    }
  }

  // The page remounts for each user, so name never changes under it.
  // svelte-ignore state_referenced_locally
  if (name === null) {
    draft.start(newUser());
  } else {
    load();
  }

  guardUnsaved(() => draft.dirty && !draft.saving);

  async function save(e: SubmitEvent) {
    e.preventDefault();
    if (draft.saving) {
      return;
    }
    const user = draft.value!;
    draft.error = '';
    draft.saving = true;
    const error = await submit(user);
    draft.saving = false;
    // Nothing refetches here; every page that shows users fetches on opening.
    // So markSaved follows the save directly, leaving no gap for an edit.

    if (!alive) {
      // Logged out means the session expired, which has its own toast.
      if (error !== null && session.loggedIn) {
        notifications.show('error', error);
      }
      return;
    }
    if (error !== null) {
      draft.error = error;
      return;
    }
    draft.markSaved();
    if (user.github !== name) {
      // A new or renamed user now lives at its own URL.
      router.replace({ page: 'user', name: user.github });
    }
  }

  // Returns the error to show, or null once saved.
  async function submit(user: UserInfo): Promise<string | null> {
    try {
      const slack = await api.verifyEmail(user.email);
      if (!slack.id || !slack.name) {
        return 'No Slack account has that email address';
      }
      user.slack_id = slack.id;
      user.slack_name = slack.name;
    } catch (e) {
      return `Error verifying email address: ${errorText(e)}`;
    }

    try {
      if (user.id === null) {
        await api.createUser(user);
        notifications.show('success', `Added ${user.github}`);
      } else {
        await api.updateUser(user);
        notifications.show('success', `Saved ${user.github}`);
      }
      return null;
    } catch (e) {
      return `Error saving user: ${errorText(e)}`;
    }
  }

  async function remove() {
    const user = draft.value!;
    deleting = true;
    try {
      await api.deleteUser(user.id!);
      notifications.show('success', `Removed ${user.github}`);
      deleteOpen = false;
      if (alive) {
        router.replace({ page: 'users' });
      }
    } catch (e) {
      reportError('Error removing user', e);
    } finally {
      deleting = false;
    }
  }
</script>

<a class="back" href={href({ page: 'users' })}><Icon name="arrow-left" size={14} />Users</a>

{#if loadFailed}
  <div class="card list-empty">
    Could not load this user. <button type="button" class="btn btn-sm" onclick={load}>Retry</button>
  </div>
{:else if missing}
  <div class="card list-empty">
    No user named <code>{name}</code>. <a href={href({ page: 'users' })}>Back to users</a>
  </div>
{:else if draft.value}
  {@const u = draft.value}
  <div class="detail-head">
    <Avatar name={u.github || '?'} size={48} />
    <div class="titles">
      <h1>{name ?? (u.github || 'New user')}</h1>
      {#if name !== null}
        <p class="muted">
          {#if u.slack_name}<span>@{u.slack_name}</span> ·{/if}
          <span>{u.email}</span>
        </p>
      {:else}
        <p class="muted">Octobot DMs this person about their pull requests and reviews.</p>
      {/if}
    </div>
    {#if u.id !== null}
      <button type="button" class="btn btn-sm" onclick={() => (deleteOpen = true)}><Icon name="trash" size={14} />Delete</button>
    {/if}
  </div>

  <form onsubmit={save}>
    {#key draft.generation}
    <fieldset class="stack" disabled={draft.saving}>
      <section class="card">
        <div class="card-header"><h2>Identity</h2></div>
        <div class="card-body">
          <div class="grid-2">
            <label class="field">
              <span class="field-label">GitHub username</span>
              <!-- svelte-ignore a11y_autofocus -->
              <input class="input mono" bind:value={u.github} placeholder="octocat" required autofocus={name === null} spellcheck="false" />
            </label>
            <label class="field">
              <span class="field-label">Email</span>
              <input class="input" type="email" bind:value={u.email} placeholder="octocat@example.com" required />
              <span class="field-hint">Finds the Slack account. Checked when you save.</span>
            </label>
          </div>
          <div class="slack">
            <Icon name="hash" />
            {#if u.slack_id}
              <span>Slack <strong>@{u.slack_name}</strong> <span class="faint mono">{u.slack_id}</span></span>
            {:else}
              <span class="muted">The Slack account is looked up from the email when you save.</span>
            {/if}
          </div>
        </div>
      </section>

      <section class="card">
        <div class="card-header"><h2>Direct messages</h2></div>
        <div class="card-body">
          <Switch
            bind:checked={u.mute_direct_messages}
            label="Mute all direct messages"
            description="Octobot sends this person no DMs at all."
          />
          <Switch
            bind:checked={u.mute_team_direct_messages}
            label="Mute team direct messages"
            description="Skip DMs that reach this person only because a team they are on was requested."
          />
          <Switch
            bind:checked={() => u.thread_direct_messages ?? false, (v) => (u.thread_direct_messages = v)}
            label="Thread DMs per pull request"
            description="Keep each pull request's DMs together in one Slack thread."
          />
          <Switch
            bind:checked={() => u.thread_reply_notifications ?? false, (v) => (u.thread_reply_notifications = v)}
            label="Thread reply notifications"
            description="Mention this person in each new DM thread, so Slack notifies them about every reply. Only applies when DMs are threaded per pull request."
          />
          <Switch
            bind:checked={() => u.group_review_direct_messages ?? false, (v) => (u.group_review_direct_messages = v)}
            label="Group review comments"
            description="Send all comments from one review as a single DM."
          />
        </div>
      </section>

      <section class="card">
        <div class="card-header">
          <h2>Muting</h2>
          <span class="sub">Fine-grained mutes, applied on top of the switches above.</span>
        </div>
        <div class="card-body">
          <div class="field">
            <label class="field-label" for="muted-repos">Muted repos</label>
            <TagInput id="muted-repos" bind:value={u.muted_repos} placeholder="some-org/some-repo, some-org/other-repo" />
            <span class="field-hint">No DMs for activity in these repos. Press Enter or comma to add.</span>
          </div>
          <div class="field">
            <label class="field-label" for="muted-teams">Muted requested teams</label>
            <TagInput id="muted-teams" bind:value={u.muted_teams} placeholder="some-org/some-team" />
            <span class="field-hint">No DMs when review is requested only from these teams, as <code>org/team-slug</code>.</span>
          </div>
        </div>
      </section>
    </fieldset>
    {/key}

    <SaveBar
      show={draft.dirty || name === null}
      saving={draft.saving}
      isNew={name === null}
      error={draft.error}
      ondiscard={() => draft.discard()}
    />
  </form>

  <Modal bind:open={deleteOpen} title="Delete {u.github}?" busy={deleting} onsubmit={remove}>
    <p>Octobot will stop sending this person DMs. This cannot be undone.</p>
    {#snippet footer()}
      <button type="button" class="btn" onclick={() => (deleteOpen = false)}>Cancel</button>
      <button type="submit" class="btn btn-danger">Delete user</button>
    {/snippet}
  </Modal>
{:else}
  <div class="card list-empty"><Icon name="loader" spin /></div>
{/if}

<style>
  .slack {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-radius: var(--radius);
    background: var(--surface-2);
    font-size: 13px;
    color: var(--text-2);
  }

  .slack strong {
    color: var(--text);
  }
</style>
