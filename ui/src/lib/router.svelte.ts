export const pages = ['users', 'repos', 'versions'] as const;
export type Page = (typeof pages)[number];

// Hash routing keeps the server to serving one index page. Accepts the old
// AngularJS `#!/users` form so existing bookmarks still land.
export function parse(hash: string): Page {
  const name = hash.replace(/^#!?\/?/, '');
  return pages.find((p) => p === name) ?? 'users';
}

class Router {
  page = $state<Page>(parse(location.hash));

  constructor() {
    window.addEventListener('hashchange', () => (this.page = parse(location.hash)));
  }
}

export const router = new Router();
