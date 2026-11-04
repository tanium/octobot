import type { RepoInfo } from '../bindings/RepoInfo';
import type { UserInfo } from '../bindings/UserInfo';
import { api } from './api';
import { reportError } from './notifications.svelte';

const byName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'base' });

// The users and repos, shared by the list pages, the record pages, and the
// command palette. Null until the first fetch succeeds. Each page refreshes
// what it shows when it opens: saves send whole records, so editing a stale
// copy would undo other people's changes.
class Store {
  users = $state<UserInfo[] | null>(null);
  repos = $state<RepoInfo[] | null>(null);
  // The deployed commit, or null for a build without one.
  commit = $state<string | null>(null);

  // Resolves to whether the fetch succeeded. A failure is the asking page's
  // to show: the store keeps whatever it last had.
  async refreshUsers(): Promise<boolean> {
    try {
      this.users = (await api.users()).users.sort((a, b) => byName(a.github, b.github));
      return true;
    } catch (e) {
      reportError('Error getting users', e);
      return false;
    }
  }

  async refreshRepos(): Promise<boolean> {
    try {
      this.repos = (await api.repos()).repos.sort((a, b) => byName(a.repo, b.repo));
      return true;
    } catch (e) {
      reportError('Error getting repos', e);
      return false;
    }
  }

  async loadCommit() {
    try {
      this.commit = (await api.version()).commit;
    } catch {
      // The footer just shows no commit.
    }
  }

  clear() {
    this.users = null;
    this.repos = null;
    this.commit = null;
  }
}

export const store = new Store();
