const KEY = 'session';

// The server-issued session token, kept per browser tab.
class Session {
  token = $state<string | null>(sessionStorage.getItem(KEY));

  get loggedIn(): boolean {
    return this.token !== null;
  }

  start(token: string) {
    sessionStorage.setItem(KEY, token);
    this.token = token;
  }

  clear() {
    sessionStorage.removeItem(KEY);
    this.token = null;
  }
}

export const session = new Session();
