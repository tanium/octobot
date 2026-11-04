import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { HttpError, SessionExpiredError, api } from './api';
import { session } from './session.svelte';

function stubFetch(status: number, body = '') {
  const fetch = vi.fn(async () => new Response(body || null, { status }));
  vi.stubGlobal('fetch', fetch);
  return fetch;
}

beforeEach(() => {
  const store = new Map<string, string>();
  vi.stubGlobal('sessionStorage', {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => store.set(k, v),
    removeItem: (k: string) => store.delete(k),
  });
  session.start('the-session');
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('api', () => {
  it('sends the session header', async () => {
    const fetch = stubFetch(200, '{"users":[]}');

    await api.users();

    expect(fetch).toHaveBeenCalledWith(
      '/api/users',
      expect.objectContaining({ headers: expect.objectContaining({ session: 'the-session' }) }),
    );
  });

  it('ends the session on 403', async () => {
    stubFetch(403);

    await expect(api.users()).rejects.toBeInstanceOf(SessionExpiredError);
    expect(session.loggedIn).toBe(false);
  });

  it('reports the server message on other errors', async () => {
    stubFetch(500, 'database is locked');

    await expect(api.users()).rejects.toEqual(new HttpError(500, 'database is locked'));
    expect(session.loggedIn).toBe(true);
  });

  it('accepts an empty success body', async () => {
    stubFetch(200);

    await expect(api.deleteUser(3)).resolves.toBeUndefined();
  });

  it('encodes the email to verify', async () => {
    const fetch = stubFetch(200, '{"id":"U1","name":"a"}');

    await api.verifyEmail('a+b@example.com');

    expect(fetch).toHaveBeenCalledWith('/api/users/verify?email=a%2Bb%40example.com', expect.anything());
  });
});
