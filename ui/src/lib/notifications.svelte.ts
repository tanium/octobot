import { SessionExpiredError } from './errors';

type Kind = 'error' | 'success';

const durationMs: Record<Kind, number> = { error: 5000, success: 3000 };

// The page-level banners. Each kind shows one message at a time; a newer
// message replaces the older one and restarts its timer.
class Notifications {
  error = $state<string | null>(null);
  success = $state<string | null>(null);

  #timers: Partial<Record<Kind, ReturnType<typeof setTimeout>>> = {};

  show(kind: Kind, msg: string) {
    clearTimeout(this.#timers[kind]);
    this[kind] = msg;
    this.#timers[kind] = setTimeout(() => (this[kind] = null), durationMs[kind]);
  }

  dismiss(kind: Kind) {
    clearTimeout(this.#timers[kind]);
    this[kind] = null;
  }
}

export const notifications = new Notifications();

export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

// Session expiry already has its own banner.
export function reportError(prefix: string, e: unknown) {
  if (!(e instanceof SessionExpiredError)) {
    notifications.show('error', `${prefix}: ${errorText(e)}`);
  }
}
