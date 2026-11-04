<script lang="ts">
  import type { MergeVersionsResp } from '../bindings/MergeVersionsResp';
  import { api } from '../lib/api';
  import Modal from '../lib/Modal.svelte';
  import { notifications, reportError } from '../lib/notifications.svelte';

  type Versions = MergeVersionsResp['versions'];

  // Kept as a string: braces in a Svelte attribute would start an expression.
  const versionPattern = String.raw`((\d+)\.){2,3}\d+`;

  let project = $state('');
  let version = $state('');
  let jiraBase = $state('');

  // Set once a dry run succeeds; the inputs lock until reset.
  let preview = $state<Versions | null>(null);
  let merged = $state<{ version: string; url: string | null; versions: Versions } | null>(null);
  let processing = $state(false);
  let confirmOpen = $state(false);

  let previewEmpty = $derived(preview !== null && Object.keys(preview).length === 0);

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
      notifications.show('success', 'Created new version successfully.');
      const result = { version, url: resp.version_url, versions: resp.versions };
      reset();
      merged = result;
    } catch (e) {
      reportError('Error creating new version', e);
    }
  }
</script>

{#snippet versionList(versions: Versions)}
  {#each Object.entries(versions) as [key, list] (key)}
    <h4><a href="{jiraBase}/browse/{key}" target="_blank" rel="noreferrer">{key}</a></h4>
    <ul>
      {#each list as v}
        <li>{v}</li>
      {/each}
    </ul>
  {/each}
{/snippet}

<h2>Versions</h2>

<p class="lead">Merge pending versions into new JIRA version</p>

{#if preview !== null && processing}
  <div class="alert alert-warning text-center"><b>Processing.</b> Please do not leave this page...</div>
{/if}

<form onsubmit={runPreview}>
  <div class="mb-3">
    <input
      type="text"
      class="form-control"
      disabled={preview !== null}
      bind:value={project}
      placeholder="JIRA project (e.g. SERVER)"
      required
    />
  </div>
  <div class="mb-3">
    <input
      type="text"
      class="form-control"
      disabled={preview !== null}
      bind:value={version}
      placeholder="New Version (e.g. 2.1.0.1000)"
      pattern={versionPattern}
      required
    />
  </div>
  <div class="mb-3">
    {#if preview === null}
      <button type="submit" disabled={processing} class="btn btn-sm btn-primary">Preview</button>
    {:else}
      <button
        type="button"
        disabled={processing || previewEmpty}
        class="btn btn-sm btn-primary"
        onclick={() => (confirmOpen = true)}
      >
        Submit
      </button>
    {/if}
    <button type="button" disabled={processing} class="btn btn-sm btn-secondary" onclick={reset}>Reset</button>
  </div>
</form>

{#if preview !== null}
  <h3>Preview</h3>
  {#if previewEmpty}
    <i>No versions found to merge</i>
  {:else}
    <p>The following JIRAs were found with pending versions to migrate:</p>
    {@render versionList(preview)}
  {/if}
{/if}

{#if merged !== null}
  <h3>Success!</h3>
  <p>
    The following JIRAs were found with pending versions were migrated to
    {#if merged.url}
      <a href={merged.url} target="_blank" rel="noreferrer">{merged.version}</a>
    {:else}
      {merged.version}
    {/if}
  </p>
  {@render versionList(merged.versions)}
{/if}

<Modal bind:open={confirmOpen} title="Confirm Merge" onsubmit={runMerge}>
  <p class="mb-0">
    Are you sure you want to merge pending versions for <strong>{project}</strong> <strong>{version}</strong>?
  </p>

  {#snippet footer()}
    <button type="button" class="btn btn-secondary" onclick={() => (confirmOpen = false)}>No</button>
    <button type="submit" class="btn btn-primary">Yes</button>
  {/snippet}
</Modal>
