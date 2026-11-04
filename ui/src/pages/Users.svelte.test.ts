import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { UserInfo } from '../bindings/UserInfo';
import Users from './Users.svelte';

const api = vi.hoisted(() => ({
  users: vi.fn(),
  verifyEmail: vi.fn(),
  createUser: vi.fn(),
  updateUser: vi.fn(),
  deleteUser: vi.fn(),
}));
vi.mock('../lib/api', () => ({ api }));

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
  api.users.mockResolvedValue({ users: [user(1, 'alice'), user(2, 'bob')] });
  api.verifyEmail.mockResolvedValue({ id: 'U1', name: 'alice' });
});

afterEach(() => {
  if (component) {
    unmount(component);
  }
  document.body.innerHTML = '';
  vi.clearAllMocks();
});

async function settle() {
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

function click(el: Element) {
  (el as HTMLElement).click();
  flushSync();
}

function editButton(row: number) {
  return document.querySelectorAll('button[title="Edit"]')[row];
}

function dialog() {
  return document.querySelector('dialog')!;
}

describe('Users', () => {
  it('does not let a slow save close a newer edit', async () => {
    let finishSave!: () => void;
    api.updateUser.mockReturnValue(new Promise<void>((r) => (finishSave = r)));

    component = mount(Users, { target: document.body });
    await settle();

    click(editButton(0));
    dialog().querySelector('form')!.requestSubmit();
    await settle();
    expect(api.updateUser).toHaveBeenCalledOnce();

    // Escape: the form's own buttons are disabled mid-save.
    dialog().close();
    flushSync();
    expect(dialog().open).toBe(false);
    click(editButton(1));
    expect(dialog().open).toBe(true);

    finishSave();
    await settle();

    expect(dialog().open).toBe(true);
    expect(dialog().querySelector<HTMLInputElement>('input[placeholder="GitHub username"]')!.value).toBe('bob');
  });

  it('blocks edits and a second submit while saving', async () => {
    api.updateUser.mockReturnValue(new Promise<void>(() => {}));

    component = mount(Users, { target: document.body });
    await settle();

    click(editButton(0));
    dialog().querySelector('form')!.requestSubmit();
    await settle();

    expect(dialog().querySelector('fieldset')!.disabled).toBe(true);
    dialog().querySelector('form')!.requestSubmit();
    await settle();
    expect(api.updateUser).toHaveBeenCalledOnce();
  });
});
