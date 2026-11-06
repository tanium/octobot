import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

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
    reply_notifications: null,
  };
}

let component: ReturnType<typeof mount> | undefined;

// jsdom has no dialog behavior; this is the part Modal relies on.
beforeAll(() => {
  Object.defineProperty(HTMLDialogElement.prototype, 'open', {
    configurable: true,
    get(this: HTMLDialogElement) {
      return this.hasAttribute('open');
    },
  });
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute('open', '');
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute('open');
    this.dispatchEvent(new Event('close'));
  };
});

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

// Waits out the fresh fetch the page seeds its draft from.
async function render(name: string | null) {
  component = mount(User, { target: document.body, props: { name } });
  await settle();
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

    await render('alice');
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

    await render('alice');
    submit();
    await settle();
    leave();

    failSave(new Error('database is locked'));
    await settle();

    expect(notifications.error).toBe('Error saving user: database is locked');
  });

  it('blocks edits and a second submit while saving', async () => {
    api.updateUser.mockReturnValue(new Promise<void>(() => {}));

    await render('alice');
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

    await render(null);
    type(input('octocat'), 'carol');
    type(input('octocat@example.com'), 'carol@example.com');
    submit();
    await settle();

    expect(api.createUser).toHaveBeenCalledWith(expect.objectContaining({ github: 'carol', slack_id: 'U9' }));
    expect(router.route).toEqual({ page: 'user', name: 'carol' });
  });

  it('shows a failed Slack lookup on the page without saving', async () => {
    api.verifyEmail.mockResolvedValue({ id: '', name: '' });

    await render('alice');
    submit();
    await settle();

    expect(api.updateUser).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('No Slack account has that email address');
  });

  it('says so when the user does not exist', async () => {
    await render('nobody');
    expect(document.body.textContent).toContain('No user named nobody');
  });

  it('edits a fresh copy of the user, not the cached one', async () => {
    store.users = [{ ...user(1, 'alice'), email: 'stale@example.com' }];

    await render('alice');

    expect(input('octocat@example.com').value).toBe('alice@example.com');
  });

  it('offers a retry instead of editing when the fetch fails', async () => {
    api.users.mockRejectedValue(new Error('down'));

    await render('alice');

    expect(document.body.textContent).toContain('Could not load this user');
    expect(document.querySelector('form')).toBeNull();
  });

  it('comes back clean from a save without fetching again', async () => {
    api.updateUser.mockResolvedValue(undefined);
    await render('alice');
    type(input('octocat@example.com'), 'alice2@example.com');
    expect(document.querySelector('.bar')).not.toBeNull();

    submit();
    await settle();

    expect(api.updateUser).toHaveBeenCalledOnce();
    expect(api.users).toHaveBeenCalledOnce();
    expect(document.querySelector('fieldset')!.disabled).toBe(false);
    expect(document.querySelector('.bar')).toBeNull();
  });

  it('puts discarded mute chips back to the saved list', async () => {
    store.users = null;
    api.users.mockResolvedValue({ users: [{ ...user(1, 'alice'), muted_repos: ['acme/saved'] }] });
    await render('alice');
    const tags = () => [...document.querySelectorAll('.tags .tag')].map((t) => t.textContent!.trim());
    const muted = document.querySelector<HTMLInputElement>('#muted-repos')!;
    type(muted, 'acme/extra,');
    expect(tags()).toContain('acme/extra');

    (document.querySelector('.bar button') as HTMLButtonElement).click();
    flushSync();
    // A blur used to write the stale chips back.
    const fresh = document.querySelector<HTMLInputElement>('#muted-repos')!;
    fresh.dispatchEvent(new FocusEvent('blur'));
    flushSync();

    expect(tags()).toEqual(['acme/saved']);
    expect(document.querySelector('.bar')).toBeNull();
  });

  it('does not navigate after a delete finishes on a page already left', async () => {
    let finishDelete!: () => void;
    api.deleteUser.mockReturnValue(new Promise<void>((r) => (finishDelete = r)));
    await render('alice');
    (document.querySelector('.detail-head button') as HTMLButtonElement).click();
    flushSync();
    document.querySelectorAll('form')[1].requestSubmit();
    await settle();
    expect(api.deleteUser).toHaveBeenCalledOnce();

    leave();
    router.replace({ page: 'user', name: 'bob' });
    finishDelete();
    await settle();

    expect(router.route).toEqual({ page: 'user', name: 'bob' });
  });
});
