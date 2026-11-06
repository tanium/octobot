import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { RepoInfo } from '../bindings/RepoInfo';
import type { UserInfo } from '../bindings/UserInfo';
import { router } from '../lib/router.svelte';
import { store } from '../lib/store.svelte';
import Repos from './Repos.svelte';
import Users from './Users.svelte';

const api = vi.hoisted(() => ({ users: vi.fn(), repos: vi.fn() }));
vi.mock('../lib/api', () => ({ api }));

function user(id: number, github: string): UserInfo {
  return {
    id,
    github,
    slack_name: github,
    slack_id: `U${id}`,
    email: `${github}@example.com`,
    mute_direct_messages: false,
    mute_team_direct_messages: false,
    muted_repos: [],
    muted_teams: [],
    thread_direct_messages: null,
    group_review_direct_messages: null,
    thread_reply_notifications: null,
  };
}

function repo(id: number, name: string, projects: string[]): RepoInfo {
  return {
    id,
    repo: name,
    channel: 'reviews',
    force_push_notify: true,
    use_threads: true,
    release_branch_prefix: '',
    jira_config: projects.map((p) => ({ jira_project: p, version_script: '', channel: '', release_branch_regex: '' })),
  };
}

let component: ReturnType<typeof mount> | undefined;

// jsdom has no layout, so no scrollIntoView.
beforeAll(() => {
  Element.prototype.scrollIntoView = () => {};
});

beforeEach(() => {
  api.users.mockResolvedValue({ users: [user(1, 'alice'), user(2, 'albert'), user(3, 'bob')] });
  api.repos.mockResolvedValue({ repos: [repo(1, 'acme/web', ['WEB']), repo(2, 'acme/mono', ['APP', 'PAY'])] });
  router.replace({ page: 'users' });
});

afterEach(() => {
  if (component) {
    unmount(component);
    component = undefined;
  }
  document.body.innerHTML = '';
});

// Mounts a list page and waits out the fetch it makes on opening.
async function show(page: typeof Users | typeof Repos) {
  component = mount(page, { target: document.body });
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

function filterBox() {
  return document.querySelector<HTMLInputElement>('[data-page-filter]')!;
}

function type(text: string) {
  const box = filterBox();
  box.focus();
  box.value = text;
  box.dispatchEvent(new Event('input', { bubbles: true }));
  flushSync();
}

async function press(key: string) {
  filterBox().dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

describe('Users loading', () => {
  it('shows what it fetched on opening', async () => {
    store.users = null;
    await show(Users);
    expect(document.querySelectorAll('a.list-row')).toHaveLength(3);
  });

  it('offers a retry only when there is nothing to show', async () => {
    api.users.mockRejectedValue(new Error('down'));

    store.users = null;
    await show(Users);
    expect(document.body.textContent).toContain('Could not load users');

    unmount(component!);
    store.users = [user(1, 'alice')];
    await show(Users);
    expect(document.body.textContent).not.toContain('Could not load users');
    expect(document.querySelectorAll('a.list-row')).toHaveLength(1);
  });
});

describe('Users filter', () => {
  it('opens the top match on Enter', async () => {
    await show(Users);

    type('bob');
    await press('Enter');

    expect(router.route).toEqual({ page: 'user', name: 'bob' });
  });

  it('opens the highlighted match after arrowing down', async () => {
    await show(Users);

    type('al');
    await press('ArrowDown');
    const highlighted = document.querySelector('.list-row.active')!.textContent!;
    await press('Enter');

    expect(router.route.page).toBe('user');
    expect(highlighted).toContain((router.route as { name: string }).name);
  });
});

describe('Repos filter', () => {
  it('opens a repo on the Jira project that matched', async () => {
    await show(Repos);

    type('PAY');
    await press('Enter');

    expect(router.route).toEqual({ page: 'repo', name: 'acme/mono', jira: 'PAY' });
  });

  it('lists a repo that names a Jira project twice', async () => {
    api.repos.mockResolvedValue({ repos: [repo(1, 'acme/web', ['WEB', 'WEB'])] });
    await show(Repos);

    expect([...document.querySelectorAll('.chip')].map((c) => c.textContent)).toEqual(['WEB', 'WEB']);
  });

  it('opens a repo plainly when its name matched', async () => {
    await show(Repos);

    type('acme/web');
    await press('Enter');

    expect(router.route).toEqual({ page: 'repo', name: 'acme/web', jira: null });
  });
});
