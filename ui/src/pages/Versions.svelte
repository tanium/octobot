<script lang="ts">
  import type { MergeVersionsResp } from '../bindings/MergeVersionsResp';
  import { api } from '../lib/api';
  import Icon from '../lib/Icon.svelte';
  import Modal from '../lib/Modal.svelte';
  import { notifications, reportError } from '../lib/notifications.svelte';

  type Versions = MergeVersionsResp['versions'];

  // Kept as a string: braces in a Svelte attribute would start an expression.
  const versionPattern = String.raw`((\d+)\.){2,3}\d+`;

  // A project to start with, as linked from a repo's Jira projects.
  let { project: initialProject }: { project: string | null } = $props();

  // svelte-ignore state_referenced_locally
  let project = $state(initialProject ?? '');
  let version = $state('');
  let jiraBase = $state('');

  // Set once a dry run succeeds; the inputs lock until reset.
  let preview = $state<Versions | null>(null);
  let merged = $state<{ version: string; url: string | null; versions: Versions } | null>(null);
  let processing = $state(false);
  let confirmOpen = $state(false);

  let previewCount = $derived(preview === null ? 0 : Object.keys(preview).length);

  function reset() {
    project = '';
    version = '';
    preview = null;
    merged = null;
    processing = false;
  }

  async function mergeVersions(dryRun: boolean): Promise<MergeVersionsResp> {
    processing = true;
    try {
      const resp = await api.mergeVersions({ project, version, dry_run: dryRun });
      jiraBase = resp.jira_base;
      if (resp.error) {
        throw new Error(resp.error);
      }
      return resp;
    } finally {
      processing = false;
    }
  }

  async function runPreview(e: SubmitEvent) {
    e.preventDefault();
    merged = null;
    try {
      preview = (await mergeVersions(true)).versions;
    } catch (e) {
      reportError('Error previewing new version', e);
    }
  }

  async function runMerge() {
    confirmOpen = false;
    try {
      const resp = await mergeVersions(false);
      notifications.show('success', `Created version ${version}`);
      const result = { version, url: resp.version_url, versions: resp.versions };
      reset();
      merged = result;
    } catch (e) {
      reportError('Error creating new version', e);
    }
  }
</script>

{#snippet versionList(versions: Versions)}
  <ul class="issues">
    {#each Object.entries(versions) as [key, list] (key)}
      <li>
        <a class="mono" href="{jiraBase}/browse/{key}" target="_blank" rel="noreferrer">{key}<Icon name="external" size={12} /></a>
        <span class="vers">
          {#each list as v, i (i)}<span class="chip">{v}</span>{/each}
        </span>
      </li>
    {/each}
  </ul>
{/snippet}

<div class="page-head">
  <h1>Versions</h1>
</div>

<div class="stack">
  <section class="card">
    <div class="card-header">
      <h2>Merge pending versions</h2>
      <span class="sub">Rolls each issue's pending versions into one new Jira version.</span>
    </div>
    <form class="card-body" onsubmit={runPreview}>
      <div class="grid-2">
        <label class="field">
          <span class="field-label">Jira project</span>
          <input class="input mono" disabled={preview !== null} bind:value={project} placeholder="SERVER" required spellcheck="false" />
        </label>
        <label class="field">
          <span class="field-label">New version</span>
          <input
            class="input mono"
            disabled={preview !== null}
            bind:value={version}
            placeholder="2.1.0.1000"
            pattern={versionPattern}
            title="Three or four numbers separated by dots"
            required
          />
        </label>
      </div>
      <div class="actions">
        <span class="steps">
          <span class="step" class:current={preview === null} class:done={preview !== null}><span class="n">1</span>Preview</span>
          <span class="line"></span>
          <span class="step" class:current={preview !== null}><span class="n">2</span>Merge</span>
        </span>
        <button type="button" disabled={processing} class="btn btn-sm btn-ghost" onclick={reset}>Reset</button>
        {#if preview === null}
          <button type="submit" disabled={processing} class="btn btn-sm btn-primary">
            {#if processing}<Icon name="loader" spin size={14} />{/if}
            Preview
          </button>
        {:else}
          <button
            type="button"
            disabled={processing || previewCount === 0}
            class="btn btn-sm btn-primary"
            onclick={() => (confirmOpen = true)}
          >
            {#if processing}<Icon name="loader" spin size={14} />{/if}
            Merge into {version}
          </button>
        {/if}
      </div>
    </form>
  </section>

  {#if preview !== null && processing}
    <div class="alert alert-warning"><Icon name="alert" /><span><b>Merging.</b> Please stay on this page until it finishes.</span></div>
  {/if}

  {#if preview !== null}
    <section class="card">
      <div class="card-header">
        <h2>Preview</h2>
        <span class="sub">{previewCount} issue{previewCount === 1 ? '' : 's'} with pending versions</span>
      </div>
      {#if previewCount === 0}
        <div class="list-empty">No pending versions to merge.</div>
      {:else}
        {@render versionList(preview)}
      {/if}
    </section>
  {/if}

  {#if merged !== null}
    <section class="card">
      <div class="card-header">
        <span class="ok"><Icon name="check" /></span>
        <h2>
          Merged into
          {#if merged.url}
            <a href={merged.url} target="_blank" rel="noreferrer">{merged.version}</a>
          {:else}
            {merged.version}
          {/if}
        </h2>
      </div>
      {@render versionList(merged.versions)}
    </section>
  {/if}
</div>

<Modal bind:open={confirmOpen} title="Merge pending versions?" onsubmit={runMerge}>
  <p>
    This creates <strong>{version}</strong> in <strong>{project}</strong> and moves {previewCount} issue{previewCount === 1 ? '' : 's'} onto it.
  </p>
  {#snippet footer()}
    <button type="button" class="btn" onclick={() => (confirmOpen = false)}>Cancel</button>
    <button type="submit" class="btn btn-primary">Merge</button>
  {/snippet}
</Modal>

<style>
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .steps {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: 13px;
  }

  .step {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .n {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--surface-2);
    color: var(--text-3);
    font-size: 11px;
    font-weight: 600;
  }

  .current {
    color: var(--text);
    font-weight: 500;
  }

  .current .n {
    background: var(--accent);
    color: var(--on-accent);
  }

  .done .n {
    background: var(--success);
    color: #fff;
  }

  .line {
    width: 24px;
    height: 1px;
    background: var(--border-strong);
  }

  .issues {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .issues li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
    padding: 10px 18px;
    border-bottom: 1px solid var(--border);
  }

  .issues li:last-child {
    border-bottom: 0;
  }

  .issues a {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 110px;
    font-weight: 600;
  }

  .vers {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .ok {
    display: inline-flex;
    padding: 4px;
    border-radius: 50%;
    background: var(--success-soft);
    color: var(--success);
  }
</style>
