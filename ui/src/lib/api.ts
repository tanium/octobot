import type { LoginRequest } from '../bindings/LoginRequest';
import type { LoginResp } from '../bindings/LoginResp';
import type { MergeVersionsReq } from '../bindings/MergeVersionsReq';
import type { MergeVersionsResp } from '../bindings/MergeVersionsResp';
import type { RepoInfo } from '../bindings/RepoInfo';
import type { ReposResp } from '../bindings/ReposResp';
import type { SlackUserResp } from '../bindings/SlackUserResp';
import type { UserInfo } from '../bindings/UserInfo';
import type { UsersResp } from '../bindings/UsersResp';
import { HttpError, SessionExpiredError } from './errors';
import { notifications } from './notifications.svelte';
import { session } from './session.svelte';

async function request<T>(method: string, url: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (session.token !== null) {
    headers.session = session.token;
  }

  const resp = await fetch(url, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });

  if (resp.status === 403) {
    session.clear();
    notifications.show('error', 'Session expired: Logged out');
    throw new SessionExpiredError();
  }

  const text = await resp.text();
  if (!resp.ok) {
    throw new HttpError(resp.status, text || `HTTP ${resp.status}`);
  }
  return (text ? JSON.parse(text) : undefined) as T;
}

export const api = {
  login: (req: LoginRequest) => request<LoginResp>('POST', '/auth/login', req),
  logout: () => request<void>('POST', '/auth/logout'),
  checkSession: () => request<void>('POST', '/auth/check'),

  users: () => request<UsersResp>('GET', '/api/users'),
  verifyEmail: (email: string) =>
    request<SlackUserResp>('POST', `/api/users/verify?email=${encodeURIComponent(email)}`),
  createUser: (user: UserInfo) => request<void>('POST', '/api/users', user),
  updateUser: (user: UserInfo) => request<void>('PUT', '/api/user', user),
  deleteUser: (id: number) => request<void>('DELETE', `/api/user?id=${id}`),

  repos: () => request<ReposResp>('GET', '/api/repos'),
  createRepo: (repo: RepoInfo) => request<void>('POST', '/api/repos', repo),
  updateRepo: (repo: RepoInfo) => request<void>('PUT', '/api/repo', repo),
  deleteRepo: (id: number) => request<void>('DELETE', `/api/repo?id=${id}`),

  mergeVersions: (req: MergeVersionsReq) =>
    request<MergeVersionsResp>('POST', '/api/merge-versions', req),
};
