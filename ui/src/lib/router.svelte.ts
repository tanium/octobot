// A user or repo is named by its GitHub login or repo name, both unique.
// `new` is reserved by GitHub for both, so it can mean an unsaved record.
export type Route =
  | { page: 'users' }
  | { page: 'user'; name: string | null }
  | { page: 'repos' }
  | { page: 'repo'; name: string | null; jira: string | null }
  | { page: 'versions'; project: string | null };

// Hash routing keeps the server to serving one index page. Accepts the old
// AngularJS `#!/users` form so existing bookmarks still land.
export function parse(hash: string): Route {
  const [path, query = ''] = hash.replace(/^#!?\/?/, '').split('?', 2);
  const params = new URLSearchParams(query);
  const [page, ...rest] = path.split('/').filter((s) => s !== '');
  let name: string;
  try {
    // Repo names keep their slash, as `#/repos/org/repo`.
    name = rest.map(decodeURIComponent).join('/');
  } catch {
    // A malformed escape, as in a hand-edited or truncated link.
    return { page: 'users' };
  }

  switch (page) {
    case 'users':
      return name === '' ? { page: 'users' } : { page: 'user', name: name === 'new' ? null : name };
    case 'repos':
      return name === ''
        ? { page: 'repos' }
        : { page: 'repo', name: name === 'new' ? null : name, jira: params.get('jira') };
    case 'versions':
      return { page: 'versions', project: params.get('project') };
    default:
      return { page: 'users' };
  }
}

function path(name: string | null): string {
  return name === null ? 'new' : name.split('/').map(encodeURIComponent).join('/');
}

function withQuery(base: string, params: Record<string, string | null>): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== null) {
      q.set(k, v);
    }
  }
  const s = q.toString();
  return s === '' ? base : `${base}?${s}`;
}

export function href(route: Route): string {
  switch (route.page) {
    case 'users':
    case 'repos':
      return `#/${route.page}`;
    case 'user':
      return `#/users/${path(route.name)}`;
    case 'repo':
      return withQuery(`#/repos/${path(route.name)}`, { jira: route.jira });
    case 'versions':
      return withQuery('#/versions', { project: route.project });
  }
}

// Which page a route shows. A repo's `jira` only points within its page, so
// following one keeps the page and its unsaved edits.
export function pageKey(route: Route): string {
  return route.page === 'repo' ? href({ ...route, jira: null }) : href(route);
}

// The nav section a route belongs to.
export function section(route: Route): 'users' | 'repos' | 'versions' {
  switch (route.page) {
    case 'users':
    case 'user':
      return 'users';
    case 'repos':
    case 'repo':
      return 'repos';
    case 'versions':
      return 'versions';
  }
}

class Router {
  route = $state<Route>(parse(location.hash));

  // Set by a page with unsaved changes. Returns whether leaving is OK.
  leaveGuard: (() => boolean) | null = null;

  #hash = location.hash;

  constructor() {
    window.addEventListener('hashchange', () => {
      const leaving = pageKey(parse(location.hash)) !== pageKey(this.route);
      if (leaving && this.leaveGuard && !this.leaveGuard()) {
        // Put the address bar back without another hashchange.
        history.replaceState(null, '', this.#hash || '#/');
        return;
      }
      this.#hash = location.hash;
      this.route = parse(location.hash);
    });
  }

  go(route: Route) {
    location.hash = href(route);
  }

  // Swaps the current entry, as after saving a new or renamed record. Skips
  // the leave guard: the caller has just saved.
  replace(route: Route) {
    this.#hash = href(route);
    history.replaceState(null, '', this.#hash);
    this.route = parse(this.#hash);
  }
}

export const router = new Router();
