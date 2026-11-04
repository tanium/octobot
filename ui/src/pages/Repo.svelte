<script lang="ts">
  import { tick } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';

  import type { RepoInfo } from '../bindings/RepoInfo';
  import type { RepoJiraConfig } from '../bindings/RepoJiraConfig';
  import { api } from '../lib/api';
  import { Draft, guardUnsaved } from '../lib/draft.svelte';
  import { fuzzy } from '../lib/fuzzy';
  import Icon from '../lib/Icon.svelte';
  import { ListFilter } from '../lib/listFilter.svelte';
  import Modal from '../lib/Modal.svelte';
  import { errorText, notifications, reportError } from '../lib/notifications.svelte';
  import { href, router } from '../lib/router.svelte';
  import SaveBar from '../lib/SaveBar.svelte';
  import { session } from '../lib/session.svelte';
  import { store } from '../lib/store.svelte';
  import Switch from '../lib/Switch.svelte';

  // The repo name, or null for a new repo. `jira` opens on that project.
  let { name, jira }: { name: string | null; jira: string | null } = $props();

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

  const draft = new Draft<RepoInfo>();
  let missing = $state(false);
  let deleteOpen = $state(false);
  let deleting = $state(false);
  let alive = true;

  // Keyed by the draft's own config objects, which stay put as rows move.
  const expanded = new SvelteSet<RepoJiraConfig>();
  // svelte-ignore state_referenced_locally
  const jiraFilter = new ListFilter(jira ?? '');
  let keyInputs = new Map<RepoJiraConfig, HTMLInputElement>();

  $effect(() => () => (alive = false));

  $effect(() => {
    if (draft.value !== null) {
      return;
    }
    if (name === null) {
      draft.start(newRepo());
    } else if (store.repos !== null) {
      const found = store.repos.find((r) => r.repo === name);
      if (!found) {
        missing = true;
        return;
      }
      draft.start($state.snapshot(found));
      for (const c of draft.value!.jira_config) {
        if (c.jira_project === jira) {
          expanded.add(c);
        }
      }
    }
  });

  guardUnsaved(() => draft.dirty && !draft.saving);

  let configs = $derived(draft.value?.jira_config ?? []);

  // Filtering keeps the saved order. Open rows stay, so a row being edited
  // does not vanish when its key stops matching.
  let shown = $derived(
    configs.filter(
      (c) =>
        expanded.has(c) || fuzzy(jiraFilter.text, c.jira_project) !== null || fuzzy(jiraFilter.text, c.channel) !== null,
    ),
  );

  let duplicates = $derived.by(() => {
    const seen = new Map<string, number>();
    for (const c of configs) {
      seen.set(c.jira_project, (seen.get(c.jira_project) ?? 0) + 1);
    }
    return new Set([...seen].filter(([k, n]) => k !== '' && n > 1).map(([k]) => k));
  });

  function toggle(c: RepoJiraConfig) {
    if (expanded.has(c)) {
      expanded.delete(c);
    } else {
      expanded.add(c);
    }
  }

  async function addProject() {
    const c = newJiraConfig();
    draft.value!.jira_config.unshift(c);
    // The proxied copy is what the list holds.
    const added = draft.value!.jira_config[0];
    expanded.add(added);
    jiraFilter.text = '';
    await tick();
    keyInputs.get(added)?.focus();
  }

  // Enter in the filter: open the highlighted project and start editing it.
  async function editProject(i: number) {
    const c = shown[i];
    expanded.add(c);
    await tick();
    keyInputs.get(c)?.focus();
  }

  function removeProject(c: RepoJiraConfig) {
    const list = draft.value!.jira_config;
    list.splice(list.indexOf(c), 1);
    expanded.delete(c);
  }

  function setAll(open: boolean) {
    for (const c of shown) {
      if (open) {
        expanded.add(c);
      } else {
        expanded.delete(c);
      }
    }
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    if (draft.saving) {
      return;
    }
    const repo = draft.value!;

    // Closed rows have no inputs, so native validation cannot see them.
    const blank = repo.jira_config.find((c) => c.jira_project.trim() === '');
    if (blank) {
      jiraFilter.text = '';
      expanded.add(blank);
      draft.error = 'Every Jira project needs a key';
      await tick();
      keyInputs.get(blank)?.focus();
      return;
    }

    draft.error = '';
    draft.saving = true;
    const error = await submit(repo);
    draft.saving = false;

    if (error === null) {
      await store.refreshRepos();
    }
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
    if (repo.repo !== name) {
      router.replace({ page: 'repo', name: repo.repo, jira: null });
    }
  }

  async function submit(repo: RepoInfo): Promise<string | null> {
    try {
      if (repo.id === null) {
        await api.createRepo(repo);
        notifications.show('success', `Added ${repo.repo}`);
      } else {
        await api.updateRepo(repo);
        notifications.show('success', `Saved ${repo.repo}`);
      }
      return null;
    } catch (e) {
      return `Error saving repo: ${errorText(e)}`;
    }
  }

  async function remove() {
    const repo = draft.value!;
    deleting = true;
    try {
      await api.deleteRepo(repo.id!);
      notifications.show('success', `Removed ${repo.repo}`);
      deleteOpen = false;
      await store.refreshRepos();
      router.replace({ page: 'repos' });
    } catch (e) {
      reportError('Error removing repo', e);
    } finally {
      deleting = false;
    }
  }

  // Registers each open row's key input, for focusing.
  function keyInput(node: HTMLInputElement, c: RepoJiraConfig) {
    keyInputs.set(c, node);
    return { destroy: () => keyInputs.delete(c) };
  }
</script>

<a class="back" href={href({ page: 'repos' })}><Icon name="arrow-left" size={14} />Repos</a>

{#if missing}
  <div class="card list-empty">
    No repo named <code>{name}</code>. <a href={href({ page: 'repos' })}>Back to repos</a>
  </div>
{:else if draft.value}
  {@const r = draft.value}
  <div class="detail-head">
    <span class="tile"><Icon name="repo" size={22} /></span>
    <div class="titles">
      <h1 class="mono">{name ?? (r.repo || 'New repo')}</h1>
      <p class="muted">
        {#if name !== null}
          #{r.channel} · {configs.length} Jira project{configs.length === 1 ? '' : 's'}
        {:else}
          Octobot posts this repo's pull request activity to Slack.
        {/if}
      </p>
    </div>
    {#if r.id !== null}
      <button type="button" class="btn btn-sm" onclick={() => (deleteOpen = true)}><Icon name="trash" size={14} />Delete</button>
    {/if}
  </div>

  <form onsubmit={save}>
    <fieldset class="stack" disabled={draft.saving}>
      <section class="card">
        <div class="card-header"><h2>General</h2></div>
        <div class="card-body">
          <div class="grid-2">
            <label class="field">
              <span class="field-label">Repository</span>
              <!-- svelte-ignore a11y_autofocus -->
              <input class="input mono" bind:value={r.repo} placeholder="some-org/some-repo" required autofocus={name === null} spellcheck="false" />
              <span class="field-hint">An org name alone covers every repo in that org.</span>
            </label>
            <label class="field">
              <span class="field-label">Slack channel</span>
              <input class="input" bind:value={r.channel} placeholder="the-reviews" required />
            </label>
          </div>
        </div>
      </section>

      <section class="card">
        <div class="card-header"><h2>Git</h2></div>
        <div class="card-body">
          <Switch
            bind:checked={r.force_push_notify}
            label="Force-push notifications"
            description="Post when a pull request branch is force-pushed."
          />
          <Switch bind:checked={r.use_threads} label="Use Slack threads" description="Keep each pull request's messages in one thread." />
          <label class="field narrow">
            <span class="field-label">Release branch prefix</span>
            <input class="input mono" bind:value={r.release_branch_prefix} placeholder="release/" />
            <span class="field-hint">Branches backports can target. Defaults to <code>release/</code>.</span>
          </label>
        </div>
      </section>

      <section class="card">
        <div class="card-header jira-head">
          <h2>Jira projects <span class="count">{configs.length}</span></h2>
          {#if configs.length > 0}
            <label class="filter">
              <Icon name="search" size={14} />
              <span class="sr-only">Filter Jira projects</span>
              <input
                class="input input-sm"
                bind:value={jiraFilter.text}
                placeholder="Filter projects"
                data-page-filter
                onfocus={() => (jiraFilter.focused = true)}
                onblur={() => (jiraFilter.focused = false)}
                onkeydown={(e) => jiraFilter.keydown(e, shown.length, editProject)}
              />
              <kbd>/</kbd>
            </label>
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => setAll(expanded.size < shown.length)}>
              {expanded.size < shown.length ? 'Expand all' : 'Collapse all'}
            </button>
          {/if}
          <button type="button" class="btn btn-sm" onclick={addProject}><Icon name="plus" size={14} />Add project</button>
        </div>

        {#if configs.length === 0}
          <div class="list-empty">No Jira projects. Add one to link this repo's pull requests to Jira.</div>
        {:else}
          <ul class="projects">
            {#each shown as c, i (c)}
              {@const open = expanded.has(c)}
              <li class:open class:active={jiraFilter.isActive(i)} data-filter-active={jiraFilter.isActive(i) || undefined}>
                <div class="row">
                  <button
                    type="button"
                    class="summary"
                    aria-expanded={open}
                    onclick={() => toggle(c)}
                  >
                    <Icon name={open ? 'chevron-down' : 'chevron-right'} size={14} />
                    <span class="key mono">{c.jira_project || 'New project'}</span>
                    {#if duplicates.has(c.jira_project)}<span class="badge badge-warning">Duplicate</span>{/if}
                    <span class="meta cell">
                      {#if c.channel}<span>#{c.channel}</span>{:else}<span class="faint">#{r.channel || 'repo channel'}</span>{/if}
                      <span class="sep">·</span>
                      {#if c.release_branch_regex}<span class="mono">{c.release_branch_regex}</span>{:else}<span class="faint">all release branches</span>{/if}
                      {#if c.version_script}<span class="sep">·</span><span class="mono">{c.version_script}</span>{/if}
                    </span>
                  </button>
                  {#if c.jira_project}
                    <a
                      class="btn btn-ghost btn-icon btn-sm"
                      href={href({ page: 'versions', project: c.jira_project })}
                      title="Merge versions for {c.jira_project}"
                      aria-label="Merge versions for {c.jira_project}"
                    >
                      <Icon name="merge" size={14} />
                    </a>
                  {/if}
                  <button
                    type="button"
                    class="btn btn-ghost btn-icon btn-sm"
                    title="Remove {c.jira_project || 'project'}"
                    aria-label="Remove {c.jira_project || 'project'}"
                    onclick={() => removeProject(c)}
                  >
                    <Icon name="trash" size={14} />
                  </button>
                </div>
                {#if open}
                  <div class="edit grid-2">
                    <label class="field">
                      <span class="field-label">Project key</span>
                      <input class="input mono" bind:value={c.jira_project} use:keyInput={c} placeholder="APP" required spellcheck="false" />
                    </label>
                    <label class="field">
                      <span class="field-label">Slack channel override</span>
                      <input class="input" bind:value={c.channel} placeholder={r.channel || 'app-reviews'} />
                      <span class="field-hint">Pull requests mentioning this project go here instead.</span>
                    </label>
                    <label class="field">
                      <span class="field-label">Release branch regex</span>
                      <input class="input mono" bind:value={c.release_branch_regex} placeholder="release/app-.*" spellcheck="false" />
                      <span class="field-hint">Blank matches every release branch.</span>
                    </label>
                    <label class="field">
                      <span class="field-label">Version script</span>
                      <input class="input mono" bind:value={c.version_script} placeholder="./make-the-version.sh" spellcheck="false" />
                    </label>
                  </div>
                {/if}
              </li>
            {:else}
              <li class="list-empty">No projects match “{jiraFilter.text}”.</li>
            {/each}
          </ul>
        {/if}
      </section>
    </fieldset>

    <SaveBar
      show={draft.dirty || name === null}
      saving={draft.saving}
      isNew={name === null}
      error={draft.error}
      ondiscard={() => {
        draft.discard();
        expanded.clear();
      }}
    />
  </form>

  <Modal bind:open={deleteOpen} title="Delete {r.repo}?" busy={deleting} onsubmit={remove}>
    <p>Octobot will stop posting this repo's activity to Slack. This cannot be undone.</p>
    {#snippet footer()}
      <button type="button" class="btn" onclick={() => (deleteOpen = false)}>Cancel</button>
      <button type="submit" class="btn btn-danger">Delete repo</button>
    {/snippet}
  </Modal>
{:else}
  <div class="card list-empty"><Icon name="loader" spin /></div>
{/if}

<style>
  .tile {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    border-radius: var(--radius-lg);
    background: var(--accent-soft);
    color: var(--accent-text);
  }

  .narrow {
    max-width: 320px;
  }

  .jira-head {
    flex-wrap: wrap;
  }

  .jira-head h2 {
    display: flex;
    align-items: baseline;
    gap: 6px;
    margin-right: auto;
  }

  .count {
    color: var(--text-3);
    font-weight: 500;
  }

  .jira-head .filter {
    width: 200px;
  }

  .projects {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .projects > li {
    border-bottom: 1px solid var(--border);
  }

  .projects > li:last-child {
    border-bottom: 0;
    border-radius: 0 0 var(--radius-lg) var(--radius-lg);
  }

  .projects > li.open {
    background: var(--surface-2);
  }

  /* The project Enter opens, while the filter has focus. */
  .projects > li.active {
    background: var(--accent-soft);
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 10px 0 6px;
  }

  .summary {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 10px;
    min-width: 0;
    height: 44px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--radius);
    background: none;
    color: var(--text-3);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .summary:hover .key {
    color: var(--accent-text);
  }

  .key {
    min-width: 72px;
    color: var(--text);
    font-weight: 600;
  }

  .meta {
    display: flex;
    gap: 6px;
    color: var(--text-2);
    font-size: 13px;
  }

  .sep {
    color: var(--text-3);
  }

  .edit {
    padding: 4px 18px 18px 38px;
  }

  @media (max-width: 640px) {
    .meta {
      display: none;
    }

    .edit {
      padding-left: 18px;
    }
  }
</style>
