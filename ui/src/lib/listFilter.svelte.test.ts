import { describe, expect, it, vi } from 'vitest';

import { ListFilter } from './listFilter.svelte';

function key(k: string, target: HTMLElement = document.createElement('input')) {
  const e = new KeyboardEvent('keydown', { key: k, cancelable: true });
  Object.defineProperty(e, 'currentTarget', { value: target });
  return e;
}

describe('ListFilter', () => {
  it('opens the top match on Enter', async () => {
    const f = new ListFilter();
    const open = vi.fn();

    await f.keydown(key('Enter'), 3, open);

    expect(open).toHaveBeenCalledWith(0);
  });

  it('moves the highlight with the arrows, wrapping at the ends', async () => {
    const f = new ListFilter();

    await f.keydown(key('ArrowUp'), 3, () => {});
    expect(f.active).toBe(2);
    await f.keydown(key('ArrowDown'), 3, () => {});
    expect(f.active).toBe(0);
  });

  it('starts back at the top when the text changes', async () => {
    const f = new ListFilter();
    await f.keydown(key('ArrowDown'), 3, () => {});

    f.text = 'a';

    expect(f.active).toBe(0);
  });

  it('opens the last match if the list shrank under the highlight', async () => {
    const f = new ListFilter();
    f.active = 4;
    const open = vi.fn();

    await f.keydown(key('Enter'), 2, open);

    expect(open).toHaveBeenCalledWith(1);
  });

  it('takes Enter even with no matches, so a form does not submit', async () => {
    const f = new ListFilter('zzz');
    const open = vi.fn();
    const e = key('Enter');

    await f.keydown(e, 0, open);

    expect(e.defaultPrevented).toBe(true);
    expect(open).not.toHaveBeenCalled();
  });

  it('clears on the first Escape and leaves the box on the second', async () => {
    const input = document.createElement('input');
    document.body.append(input);
    input.focus();
    const f = new ListFilter('ali');

    await f.keydown(key('Escape', input), 1, () => {});
    expect(f.text).toBe('');
    expect(document.activeElement).toBe(input);

    await f.keydown(key('Escape', input), 1, () => {});
    expect(document.activeElement).not.toBe(input);
    input.remove();
  });

  it('shows the highlight only while focused', () => {
    const f = new ListFilter();
    expect(f.isActive(0)).toBe(false);
    f.focused = true;
    expect(f.isActive(0)).toBe(true);
  });
});
