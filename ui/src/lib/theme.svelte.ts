export type Preference = 'system' | 'light' | 'dark';

const KEY = 'theme';
const order: Preference[] = ['system', 'light', 'dark'];

function load(): Preference {
  try {
    const v = localStorage.getItem(KEY);
    return v === 'light' || v === 'dark' ? v : 'system';
  } catch {
    // Storage can be blocked; the theme then just follows the system.
    return 'system';
  }
}

// The color theme. The resolved theme lives on <html data-theme>, which
// index.html also sets before first paint.
class Theme {
  preference = $state<Preference>(load());

  #media = matchMedia('(prefers-color-scheme: dark)');

  constructor() {
    this.#media.addEventListener('change', () => this.#apply());
    this.#apply();
  }

  set(p: Preference) {
    this.preference = p;
    try {
      if (p === 'system') {
        localStorage.removeItem(KEY);
      } else {
        localStorage.setItem(KEY, p);
      }
    } catch {
      // Applies for this page load only.
    }
    this.#apply();
  }

  cycle() {
    this.set(order[(order.indexOf(this.preference) + 1) % order.length]);
  }

  #apply() {
    const dark = this.preference === 'dark' || (this.preference === 'system' && this.#media.matches);
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  }
}

export const theme = new Theme();
