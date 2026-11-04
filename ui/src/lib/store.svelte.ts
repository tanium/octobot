import type { RepoInfo } from '../bindings/RepoInfo';
import type { UserInfo } from '../bindings/UserInfo';
import { api } from './api';
import { reportError } from './notifications.svelte';

const byName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'base' });

// The users and repos, shared by the list pages, the record pages, and the
// command palette. Null until the first load finishes.
class Store {
  users = $state<UserInfo[] | null>(null);
  repos = $state<RepoInfo[] | null>(null);
  // The deployed commit, or null for a build without one.
  commit = $state<string | null>(null);

  async refreshUsers() {
    try {
      this.users = (await api.users()).users.sort((a, b) => byName(a.github, b.github));
    } catch (e) {
      reportError('Error getting users', e);
    }
  }

  async refreshRepos() {
    try {
      this.repos = (await api.repos()).repos.sort((a, b) => byName(a.repo, b.repo));
    } catch (e) {
      reportError('Error getting repos', e);
    }
  }

  async load() {
    api
      .version()
      .then((v) => (this.commit = v.commit))
      .catch(() => {});
    await Promise.all([this.refreshUsers(), this.refreshRepos()]);
  }

  clear() {
    this.users = null;
    this.repos = null;
    this.commit = null;
  }
}

export const store = new Store();
