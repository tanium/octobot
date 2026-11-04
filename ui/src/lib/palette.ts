import type { RepoInfo } from '../bindings/RepoInfo';
import type { UserInfo } from '../bindings/UserInfo';
import type { IconName } from './Icon.svelte';
import type { Route } from './router.svelte';

export type Kind = 'Page' | 'Action' | 'User' | 'Repo' | 'Jira';

export interface Item {
  key: string;
  kind: Kind;
  icon: IconName;
  label: string;
  detail: string;
  // Searched after the label, which always ranks first.
  keywords: string[];
  // Where the item goes, or what it does.
  target: Route | (() => void);
}

// The records half of the palette. Pages and actions are added by the
// palette itself, since actions close over app state.
export function recordItems(users: UserInfo[], repos: RepoInfo[]): Item[] {
  const items: Item[] = [];
  for (const u of users) {
    items.push({
      key: `user:${u.id}`,
      kind: 'User',
      icon: 'user',
      label: u.github,
      detail: [u.slack_name && `@${u.slack_name}`, u.email].filter(Boolean).join(' · '),
      keywords: [u.slack_name, u.email],
      target: { page: 'user', name: u.github },
    });
  }
  for (const r of repos) {
    const n = r.jira_config.length;
    items.push({
      key: `repo:${r.id}`,
      kind: 'Repo',
      icon: 'repo',
      label: r.repo,
      detail: [`#${r.channel}`, n > 0 && `${n} Jira project${n === 1 ? '' : 's'}`].filter(Boolean).join(' · '),
      keywords: [r.channel],
      target: { page: 'repo', name: r.repo, jira: null },
    });
    // One per repo a project appears in, so a project shared by repos
    // offers each of them. Keyed by position: a repo can list a key twice.
    r.jira_config.forEach((c, i) => {
      if (c.jira_project === '') {
        return;
      }
      items.push({
        key: `jira:${r.id}:${i}`,
        kind: 'Jira',
        icon: 'tag',
        label: c.jira_project,
        detail: `in ${r.repo}${c.channel ? ` · #${c.channel}` : ''}`,
        keywords: [r.repo],
        target: { page: 'repo', name: r.repo, jira: c.jira_project },
      });
    });
  }
  return items;
}
