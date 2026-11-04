import { describe, expect, it } from 'vitest';

import { parse } from './router.svelte';

describe('parse', () => {
  it('reads the page from the hash', () => {
    expect(parse('#/repos')).toBe('repos');
    expect(parse('#versions')).toBe('versions');
  });

  it('accepts old AngularJS bookmarks', () => {
    expect(parse('#!/repos')).toBe('repos');
  });

  it('falls back to users', () => {
    expect(parse('')).toBe('users');
    expect(parse('#/nope')).toBe('users');
  });
});
