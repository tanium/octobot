import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';

import type { RepoInfo } from '../bindings/RepoInfo';
import type { UserInfo } from '../bindings/UserInfo';
import { router } from '../lib/router.svelte';
import { store } from '../lib/store.svelte';
import Repos from './Repos.svelte';
import Users from './Users.svelte';

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
  store.users = [user(1, 'alice'), user(2, 'albert'), user(3, 'bob')];
  store.repos = [repo(1, 'acme/web', ['WEB']), repo(2, 'acme/mono', ['APP', 'PAY'])];
  router.replace({ page: 'users' });
});

afterEach(() => {
  if (component) {
    unmount(component);
    component = undefined;
  }
  document.body.innerHTML = '';
});

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

describe('Users filter', () => {
  it('opens the top match on Enter', async () => {
    component = mount(Users, { target: document.body });
    flushSync();

    type('bob');
    await press('Enter');

    expect(router.route).toEqual({ page: 'user', name: 'bob' });
  });

  it('opens the highlighted match after arrowing down', async () => {
    component = mount(Users, { target: document.body });
    flushSync();

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
    component = mount(Repos, { target: document.body });
    flushSync();

    type('PAY');
    await press('Enter');

    expect(router.route).toEqual({ page: 'repo', name: 'acme/mono', jira: 'PAY' });
  });

  it('opens a repo plainly when its name matched', async () => {
    component = mount(Repos, { target: document.body });
    flushSync();

    type('acme/web');
    await press('Enter');

    expect(router.route).toEqual({ page: 'repo', name: 'acme/web', jira: null });
  });
});
