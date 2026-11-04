import { router } from './router.svelte';

// An editable copy of a record, so leaving without saving changes nothing.
export class Draft<T> {
  value = $state<T | null>(null);
  saving = $state(false);
  error = $state('');
  // Bumped whenever value is replaced wholesale. Key form fields on it so
  // inputs that keep their own state, like TagInput, start over.
  generation = $state(0);

  #saved = $state('');

  dirty = $derived(this.value !== null && JSON.stringify($state.snapshot(this.value)) !== this.#saved);

  start(v: T) {
    this.#saved = JSON.stringify(v);
    this.value = JSON.parse(this.#saved);
    this.generation++;
  }

  markSaved() {
    this.#saved = JSON.stringify($state.snapshot(this.value));
  }

  discard() {
    this.value = JSON.parse(this.#saved);
    this.error = '';
    this.generation++;
  }
}

// Asks before leaving a page with unsaved changes, by link or by closing
// the tab. Call during component init.
export function guardUnsaved(dirty: () => boolean) {
  $effect(() => {
    const guard = () => !dirty() || confirm('Discard unsaved changes?');
    const onbeforeunload = (e: BeforeUnloadEvent) => {
      if (dirty()) {
        e.preventDefault();
      }
    };
    router.leaveGuard = guard;
    window.addEventListener('beforeunload', onbeforeunload);
    return () => {
      if (router.leaveGuard === guard) {
        router.leaveGuard = null;
      }
      window.removeEventListener('beforeunload', onbeforeunload);
    };
  });
}
