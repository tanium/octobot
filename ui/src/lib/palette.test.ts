import { describe, expect, it } from 'vitest';

import type { RepoInfo } from '../bindings/RepoInfo';
import { rank } from './fuzzy';
import { recordItems } from './palette';

function repo(name: string, projects: string[]): RepoInfo {
  return {
    id: 1,
    repo: name,
    channel: 'reviews',
    force_push_notify: true,
    use_threads: true,
    release_branch_prefix: '',
    jira_config: projects.map((p) => ({ jira_project: p, version_script: '', channel: '', release_branch_regex: '' })),
  };
}

describe('recordItems', () => {
  it('sends a Jira project to its repo, opened on that project', () => {
    const items = recordItems([], [repo('acme/mono', ['APP', 'WEB'])]);
    const [top] = rank(items, 'web', (i) => [i.label, ...i.keywords]);
    expect(top.item.target).toEqual({ page: 'repo', name: 'acme/mono', jira: 'WEB' });
  });

  it('offers a shared project once per repo', () => {
    const items = recordItems([], [repo('acme/a', ['APP']), repo('acme/b', ['APP'])]);
    expect(items.filter((i) => i.kind === 'Jira').map((i) => i.detail)).toEqual(['in acme/a', 'in acme/b']);
  });

  it('gives a key listed twice in one repo distinct items', () => {
    const items = recordItems([], [repo('acme/a', ['APP', 'APP'])]);
    const keys = items.map((i) => i.key);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it('skips blank project keys', () => {
    const items = recordItems([], [repo('acme/a', [''])]);
    expect(items.map((i) => i.kind)).toEqual(['Repo']);
  });
});
