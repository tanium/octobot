import { describe, expect, it } from 'vitest';

import { href, parse, type Route } from './router.svelte';

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
