import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { UserInfo } from '../bindings/UserInfo';
import { notifications } from '../lib/notifications.svelte';
import { router } from '../lib/router.svelte';
import { session } from '../lib/session.svelte';
import { store } from '../lib/store.svelte';
import User from './User.svelte';

const api = vi.hoisted(() => ({
  users: vi.fn(),
  verifyEmail: vi.fn(),
  createUser: vi.fn(),
  updateUser: vi.fn(),
  deleteUser: vi.fn(),
}));
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
  };
}

let component: ReturnType<typeof mount> | undefined;

beforeEach(() => {
  session.start('the-session');
  notifications.dismiss('error');
  store.users = [user(1, 'alice'), user(2, 'bob')];
  api.users.mockResolvedValue({ users: [user(1, 'alice'), user(2, 'bob')] });
  api.verifyEmail.mockResolvedValue({ id: 'U1', name: 'alice' });
  router.replace({ page: 'user', name: 'alice' });
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

function render(name: string | null) {
  component = mount(User, { target: document.body, props: { name } });
  flushSync();
}

function leave() {
  unmount(component!);
  component = undefined;
}

function input(placeholder: string) {
  return document.querySelector<HTMLInputElement>(`input[placeholder="${placeholder}"]`)!;
}

function type(el: HTMLInputElement, text: string) {
  el.value = text;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  flushSync();
}

function submit() {
  document.querySelector('form')!.requestSubmit();
}

describe('User', () => {
  it('does not move a newer page when a slow save finishes', async () => {
    let finishSave!: () => void;
    api.updateUser.mockReturnValue(new Promise<void>((r) => (finishSave = r)));

    render('alice');
    type(input('octocat'), 'alice2');
    submit();
    await settle();
    expect(api.updateUser).toHaveBeenCalledOnce();

    leave();
    router.replace({ page: 'user', name: 'bob' });

    finishSave();
    await settle();

    expect(router.route).toEqual({ page: 'user', name: 'bob' });
  });

  it('reports a failed save as a toast once the page is gone', async () => {
    let failSave!: (e: Error) => void;
    api.updateUser.mockReturnValue(new Promise<void>((_, reject) => (failSave = reject)));

    render('alice');
    submit();
    await settle();
    leave();

    failSave(new Error('database is locked'));
    await settle();

    expect(notifications.error).toBe('Error saving user: database is locked');
  });

  it('blocks edits and a second submit while saving', async () => {
    api.updateUser.mockReturnValue(new Promise<void>(() => {}));

    render('alice');
    submit();
    await settle();

    expect(document.querySelector('fieldset')!.disabled).toBe(true);
    submit();
    await settle();
    expect(api.updateUser).toHaveBeenCalledOnce();
  });

  it('moves a new user to its own page once created', async () => {
    api.createUser.mockResolvedValue(undefined);
    api.verifyEmail.mockResolvedValue({ id: 'U9', name: 'carol' });
    router.replace({ page: 'user', name: null });

    render(null);
    type(input('octocat'), 'carol');
    type(input('octocat@example.com'), 'carol@example.com');
    submit();
    await settle();

    expect(api.createUser).toHaveBeenCalledWith(expect.objectContaining({ github: 'carol', slack_id: 'U9' }));
    expect(router.route).toEqual({ page: 'user', name: 'carol' });
  });

  it('shows a failed Slack lookup on the page without saving', async () => {
    api.verifyEmail.mockResolvedValue({ id: '', name: '' });

    render('alice');
    submit();
    await settle();

    expect(api.updateUser).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('No Slack account has that email address');
  });

  it('says so when the user does not exist', () => {
    render('nobody');
    expect(document.body.textContent).toContain('No user named nobody');
  });
});
