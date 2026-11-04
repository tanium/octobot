import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { RepoInfo } from '../bindings/RepoInfo';
import { session } from '../lib/session.svelte';
import { store } from '../lib/store.svelte';
import Repo from './Repo.svelte';

const api = vi.hoisted(() => ({
  repos: vi.fn(),
  createRepo: vi.fn(),
  updateRepo: vi.fn(),
  deleteRepo: vi.fn(),
}));
vi.mock('../lib/api', () => ({ api }));

function repo(projects: string[]): RepoInfo {
  return {
    id: 1,
    repo: 'acme/mono',
    channel: 'reviews',
    force_push_notify: true,
    use_threads: true,
    release_branch_prefix: '',
    jira_config: projects.map((p) => ({ jira_project: p, version_script: '', channel: '', release_branch_regex: '' })),
  };
}

let component: ReturnType<typeof mount> | undefined;

beforeEach(() => {
  session.start('the-session');
  api.updateRepo.mockResolvedValue(undefined);
});

afterEach(() => {
  if (component) {
    unmount(component);
    component = undefined;
  }
  document.body.innerHTML = '';
  vi.clearAllMocks();
});

async function settle() {
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

// Props stay reactive, so a test can follow a new Jira link on the open page.
async function render(projects: string[], jira: string | null = null) {
  api.repos.mockResolvedValue({ repos: [repo(projects)] });
  const props = $state({ name: 'acme/mono', jira });
  component = mount(Repo, { target: document.body, props });
  await settle();
  return props;
}

function keyInputs() {
  return [...document.querySelectorAll<HTMLInputElement>('input[placeholder="APP"]')];
}

describe('Repo', () => {
  it('opens on the project named in the link', async () => {
    await render(['APP', 'WEB', 'API'], 'WEB');

    expect(keyInputs().map((i) => i.value)).toEqual(['WEB']);
  });

  it('opens a project linked while the page is open, keeping edits', async () => {
    const props = await render(['APP', 'WEB', 'API']);
    const channel = document.querySelector<HTMLInputElement>('input[placeholder="the-reviews"]')!;
    channel.value = 'edited';
    channel.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    props.jira = 'API';
    flushSync();

    expect(keyInputs().map((i) => i.value)).toEqual(['API']);
    expect(channel.isConnected).toBe(true);
    expect(channel.value).toBe('edited');
  });

  it('edits a fresh copy of the repo, not the cached one', async () => {
    store.repos = [repo(['STALE'])];
    await render(['FRESH'], 'FRESH');

    expect(keyInputs().map((i) => i.value)).toEqual(['FRESH']);
  });

  it('offers a retry instead of editing when the fetch fails', async () => {
    store.repos = [repo(['STALE'])];
    api.repos.mockRejectedValue(new Error('down'));
    component = mount(Repo, { target: document.body, props: { name: 'acme/mono', jira: null } });
    await settle();

    expect(document.body.textContent).toContain('Could not load this repo');
    expect(document.querySelector('form')).toBeNull();
  });

  it('refuses to save a project without a key, opening it', async () => {
    await render(['APP', '', 'API']);
    expect(keyInputs()).toEqual([]);

    document.querySelector('form')!.requestSubmit();
    await settle();

    expect(api.updateRepo).not.toHaveBeenCalled();
    expect(keyInputs().map((i) => i.value)).toEqual(['']);
    expect(document.activeElement).toBe(keyInputs()[0]);
    expect(document.body.textContent).toContain('Every Jira project needs a key');
  });

  it('keeps an open project visible while its key stops matching the filter', async () => {
    await render(['APP', 'WEB'], 'APP');
    const [key] = keyInputs();

    key.value = 'ZZZ';
    key.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    expect(keyInputs().map((i) => i.value)).toEqual(['ZZZ']);
  });

  it('opens and focuses the top project on Enter in the filter, without saving', async () => {
    await render(['APP', 'WEB', 'API']);
    const box = document.querySelector<HTMLInputElement>('[data-page-filter]')!;
    box.focus();
    box.value = 'web';
    box.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await settle();

    expect(keyInputs().map((i) => i.value)).toEqual(['WEB']);
    expect(document.activeElement).toBe(keyInputs()[0]);
    expect(api.updateRepo).not.toHaveBeenCalled();
  });

  it('saves the edited projects in their order', async () => {
    await render(['APP', 'WEB'], 'WEB');
    const [key] = keyInputs();
    key.value = 'WEB2';
    key.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    document.querySelector('form')!.requestSubmit();
    await settle();

    expect(api.updateRepo.mock.calls[0][0].jira_config.map((c: { jira_project: string }) => c.jira_project)).toEqual([
      'APP',
      'WEB2',
    ]);
  });
});
