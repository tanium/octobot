import { afterEach, describe, expect, it, vi } from 'vitest';

import { href, pageKey, parse, router, type Route } from './router.svelte';

describe('parse', () => {
  it('reads list pages from the hash', () => {
    expect(parse('#/repos')).toEqual({ page: 'repos' });
    expect(parse('#versions')).toEqual({ page: 'versions', project: null });
  });

  it('accepts old AngularJS bookmarks', () => {
    expect(parse('#!/repos')).toEqual({ page: 'repos' });
  });

  it('falls back to users', () => {
    expect(parse('')).toEqual({ page: 'users' });
    expect(parse('#/nope')).toEqual({ page: 'users' });
  });

  it('falls back to users on a malformed escape', () => {
    expect(parse('#/repos/foo%E0')).toEqual({ page: 'users' });
  });

  it('reads records, keeping the slash in repo names', () => {
    expect(parse('#/users/alice')).toEqual({ page: 'user', name: 'alice' });
    expect(parse('#/repos/acme/mono?jira=APP')).toEqual({ page: 'repo', name: 'acme/mono', jira: 'APP' });
    expect(parse('#/repos/acme')).toEqual({ page: 'repo', name: 'acme', jira: null });
  });

  it('reads `new` as an unsaved record', () => {
    expect(parse('#/users/new')).toEqual({ page: 'user', name: null });
    expect(parse('#/repos/new')).toEqual({ page: 'repo', name: null, jira: null });
  });

  it('reads the versions project', () => {
    expect(parse('#/versions?project=APP')).toEqual({ page: 'versions', project: 'APP' });
  });
});

describe('href', () => {
  const routes: Route[] = [
    { page: 'users' },
    { page: 'user', name: 'alice' },
    { page: 'user', name: null },
    { page: 'repos' },
    { page: 'repo', name: 'acme/mono', jira: null },
    { page: 'repo', name: 'acme/odd name', jira: 'A&B' },
    { page: 'versions', project: null },
    { page: 'versions', project: 'APP' },
  ];

  it('round-trips through parse', () => {
    for (const r of routes) {
      expect(parse(href(r))).toEqual(r);
    }
  });

  it('keeps repo slashes readable', () => {
    expect(href({ page: 'repo', name: 'acme/mono', jira: null })).toBe('#/repos/acme/mono');
  });
});

describe('pageKey', () => {
  it('ignores the Jira project within a repo page', () => {
    expect(pageKey({ page: 'repo', name: 'acme/mono', jira: 'APP' })).toBe(
      pageKey({ page: 'repo', name: 'acme/mono', jira: null }),
    );
  });
});

describe('leave guard', () => {
  afterEach(() => {
    router.leaveGuard = null;
  });

  function navigate(hash: string) {
    location.hash = hash;
    window.dispatchEvent(new HashChangeEvent('hashchange'));
  }

  it('asks before leaving the page, and stays when refused', () => {
    router.replace({ page: 'repo', name: 'acme/mono', jira: null });
    const guard = vi.fn(() => false);
    router.leaveGuard = guard;

    navigate('#/users');

    expect(guard).toHaveBeenCalled();
    expect(router.route).toEqual({ page: 'repo', name: 'acme/mono', jira: null });
  });

  it('does not ask when only the Jira project changes', () => {
    router.replace({ page: 'repo', name: 'acme/mono', jira: null });
    const guard = vi.fn(() => false);
    router.leaveGuard = guard;

    navigate('#/repos/acme/mono?jira=APP');

    expect(guard).not.toHaveBeenCalled();
    expect(router.route).toEqual({ page: 'repo', name: 'acme/mono', jira: 'APP' });
  });
});
