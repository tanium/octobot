<script lang="ts">
  import type { RepoInfo } from '../bindings/RepoInfo';
  import type { RepoJiraConfig } from '../bindings/RepoJiraConfig';
  import { api } from '../lib/api';
  import { matches } from '../lib/filter';
  import Modal from '../lib/Modal.svelte';
  import { errorText, notifications, reportError } from '../lib/notifications.svelte';
  import { session } from '../lib/session.svelte';

  function newRepo(): RepoInfo {
    return {
      id: null,
      repo: '',
      channel: '',
      force_push_notify: true,
      use_threads: true,
      jira_config: [],
      release_branch_prefix: '',
    };
  }

  function newJiraConfig(): RepoJiraConfig {
    return { jira_project: '', version_script: '', channel: '', release_branch_regex: '' };
  }

  function jiraProjects(repo: RepoInfo): string {
    return repo.jira_config.map((c) => c.jira_project).join(', ');
  }

  let repos = $state<RepoInfo[]>([]);
  let needle = $state('');
  let visible = $derived(repos.filter((r) => matches(needle, r.repo, r.channel, jiraProjects(r))));

  // A copy of the repo being edited, so Cancel leaves the list untouched.
  let editing = $state<RepoInfo | null>(null);
  let modalOpen = $state(false);
  let formError = $state('');
  let saving = $state(false);

  async function refresh() {
    try {
      repos = (await api.repos()).repos;
    } catch (e) {
      reportError('Error getting repos', e);
    }
  }

  function edit(repo: RepoInfo) {
    editing = structuredClone($state.snapshot(repo));
    formError = '';
    modalOpen = true;
  }

  async function save() {
    const repo = editing!;
    formError = '';
    saving = true;
    const error = await submit(repo);
    saving = false;

    // Escape can close the dialog mid-save and another edit can open, so the
    // result goes to its own dialog only while that is still showing.
    const ownDialog = modalOpen && editing === repo;
    if (error === null) {
      if (ownDialog) {
        modalOpen = false;
      }
      await refresh();
    } else if (ownDialog) {
      formError = error;
    } else if (session.loggedIn) {
      // Logged out means the session expired, which has its own banner.
      notifications.show('error', error);
    }
  }

  // Returns the error to show in the form, or null once saved.
  async function submit(repo: RepoInfo): Promise<string | null> {
    try {
      if (repo.id === null) {
        await api.createRepo(repo);
        notifications.show('success', 'Added repo successfully');
      } else {
        await api.updateRepo(repo);
        notifications.show('success', 'Edited repo successfully');
      }
      return null;
    } catch (e) {
      return `Error saving repo: ${errorText(e)}`;
    }
  }

  async function remove(repo: RepoInfo) {
    if (!confirm(`Are you sure you want to delete repo ${repo.repo}?`)) {
      return;
    }
    try {
      await api.deleteRepo(repo.id!);
      notifications.show('success', 'Removed repo successfully');
    } catch (e) {
      reportError('Error removing repo', e);
    }
    await refresh();
  }

  refresh();
</script>

<div class="d-flex flex-wrap gap-2 align-items-center mb-2">
  <h3 class="me-auto mb-0">Repos</h3>
  <input type="text" class="form-control form-control-sm w-auto" bind:value={needle} placeholder="Filter" />
  <button type="button" class="btn btn-sm btn-primary" onclick={() => edit(newRepo())}>Add repo</button>
</div>

<table class="table">
  <thead>
    <tr>
      <th>Repo</th>
      <th>Slack Channel</th>
      <th>JIRA Project(s)</th>
      <th></th>
    </tr>
  </thead>
  <tbody>
    {#each visible as repo (repo.id)}
      <tr>
        <td>{repo.repo}</td>
        <td>{repo.channel}</td>
        <td>{jiraProjects(repo) || '(none)'}</td>
        <td class="text-end text-nowrap">
          <button type="button" class="btn btn-link p-0 me-2" title="Edit" aria-label="Edit" onclick={() => edit(repo)}>
            <i class="bi bi-pencil"></i>
          </button>
          <button type="button" class="btn btn-link p-0" title="Delete" aria-label="Delete" onclick={() => remove(repo)}>
            <i class="bi bi-trash"></i>
          </button>
        </td>
      </tr>
    {/each}
  </tbody>
</table>

<Modal bind:open={modalOpen} large title={editing?.id ? 'Edit repo' : 'Add repo'} busy={saving} onsubmit={save}>
  {#if modalOpen && editing}
    <h4>Repo</h4>
    <div class="mb-3">
      <label class="form-label w-100">
        Repository
        <!-- svelte-ignore a11y_autofocus -->
        <input type="text" class="form-control" bind:value={editing.repo} placeholder="some-org/some-repo" required autofocus />
      </label>
    </div>
    <div class="mb-3">
      <label class="form-label w-100">
        Slack channel
        <input type="text" class="form-control" bind:value={editing.channel} placeholder="the-reviews" required />
      </label>
    </div>

    <h4>Git</h4>
    <div class="form-check">
      <label class="form-check-label">
        <input type="checkbox" class="form-check-input" bind:checked={editing.force_push_notify} />
        Force-push notification
      </label>
    </div>
    <div class="form-check mb-3">
      <label class="form-check-label">
        <input type="checkbox" class="form-check-input" bind:checked={editing.use_threads} />
        Use slack threads
      </label>
    </div>
    <div class="mb-3">
      <label class="form-label w-100">
        Release branch prefix
        <input type="text" class="form-control" bind:value={editing.release_branch_prefix} placeholder="release/" />
      </label>
    </div>

    <h4>JIRA</h4>
    <button type="button" class="btn btn-sm btn-primary mb-2" onclick={() => editing!.jira_config.unshift(newJiraConfig())}>
      Add JIRA
    </button>

    {#each editing.jira_config as config, i}
      <div class="d-flex gap-2 mb-2">
        <div class="border rounded p-2 flex-grow-1">
          <div class="mb-2">
            <label class="form-label w-100">
              Project
              <input type="text" class="form-control" bind:value={config.jira_project} placeholder="APP" required />
            </label>
          </div>
          <div class="mb-2">
            <label class="form-label w-100">
              Release branch regex
              <input type="text" class="form-control" bind:value={config.release_branch_regex} placeholder="release/app-.*" />
            </label>
          </div>
          <div class="mb-2">
            <label class="form-label w-100">
              Version script
              <input type="text" class="form-control" bind:value={config.version_script} placeholder="./make-the-version.sh" />
            </label>
          </div>
          <div>
            <label class="form-label w-100">
              Slack channel override
              <input type="text" class="form-control" bind:value={config.channel} placeholder="app-reviews" />
            </label>
          </div>
        </div>
        <div>
          <button
            type="button"
            class="btn btn-sm btn-secondary"
            title="Remove JIRA"
            aria-label="Remove JIRA"
            onclick={() => editing!.jira_config.splice(i, 1)}
          >
            <i class="bi bi-trash"></i>
          </button>
        </div>
      </div>
    {/each}

    {#if formError}
      <div class="alert alert-danger mt-3 mb-0" role="alert">{formError}</div>
    {/if}
  {/if}

  {#snippet footer()}
    <button type="button" class="btn btn-secondary" onclick={() => (modalOpen = false)}>Cancel</button>
    <button type="submit" class="btn btn-primary">Save</button>
  {/snippet}
</Modal>
