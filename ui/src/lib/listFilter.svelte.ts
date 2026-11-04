import { tick } from 'svelte';

// A list's filter box, driven from the keyboard like the palette: arrows
// move a highlight through the matches, Enter opens the highlighted one,
// and Escape clears the text, then leaves the box.
export class ListFilter {
  active = $state(0);
  // The highlight shows only while the box has focus.
  focused = $state(false);

  #text = $state('');

  constructor(text = '') {
    this.#text = text;
  }

  get text() {
    return this.#text;
  }

  // New text starts the highlight back at the top match.
  set text(v: string) {
    this.#text = v;
    this.active = 0;
  }

  isActive(i: number): boolean {
    return this.focused && i === this.active;
  }

  // `open` gets the index of the highlighted match among `count`.
  async keydown(e: KeyboardEvent, count: number, open: (i: number) => void) {
    switch (e.key) {
      case 'ArrowDown':
      case 'ArrowUp': {
        e.preventDefault();
        if (count === 0) {
          return;
        }
        const step = e.key === 'ArrowDown' ? 1 : -1;
        this.active = (this.active + step + count) % count;
        await tick();
        document.querySelector('[data-filter-active]')?.scrollIntoView({ block: 'nearest' });
        return;
      }
      case 'Enter':
        // Always taken: a filter inside a form must not submit it.
        e.preventDefault();
        if (count > 0) {
          open(Math.min(this.active, count - 1));
        }
        return;
      case 'Escape':
        e.preventDefault();
        if (this.#text !== '') {
          this.text = '';
        } else {
          (e.currentTarget as HTMLElement).blur();
        }
        return;
    }
  }
}
