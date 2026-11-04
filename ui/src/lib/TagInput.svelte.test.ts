import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';

import TagInput from './TagInput.svelte';

let component: ReturnType<typeof mount> | undefined;

afterEach(() => {
  if (component) {
    unmount(component);
  }
  document.body.innerHTML = '';
});

function render(initial: string[]) {
  const props = $state({ value: initial, placeholder: '' });
  component = mount(TagInput, { target: document.body, props });
  flushSync();
  return { props, input: document.querySelector('input')! };
}

function type(input: HTMLInputElement, text: string) {
  input.value = text;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  flushSync();
}

function key(input: HTMLInputElement, key: string) {
  const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
  input.dispatchEvent(e);
  flushSync();
  return e;
}

function chips() {
  return [...document.querySelectorAll('.tag')].map((t) => t.textContent!.trim());
}

describe('TagInput', () => {
  it('shows the initial list as chips', () => {
    render(['a/b', 'c/d']);
    expect(chips()).toEqual(['a/b', 'c/d']);
  });

  it('counts text not yet made a chip', () => {
    const { props, input } = render(['a/b']);

    type(input, 'org/repo');

    expect(props.value).toEqual(['a/b', 'org/repo']);
  });

  it('makes a chip on a comma, keeping what follows it', () => {
    const { props, input } = render([]);

    type(input, 'a/b, c/');

    expect(chips()).toEqual(['a/b']);
    expect(input.value).toBe('c/');
    expect(props.value).toEqual(['a/b', 'c/']);
  });

  it('splits a pasted list into chips', () => {
    const { props, input } = render([]);

    type(input, 'a, b, c,');

    expect(chips()).toEqual(['a', 'b', 'c']);
    expect(props.value).toEqual(['a', 'b', 'c']);
  });

  it('makes a chip on Enter without submitting', () => {
    const { input } = render([]);

    type(input, 'a/b');
    const e = key(input, 'Enter');

    expect(e.defaultPrevented).toBe(true);
    expect(chips()).toEqual(['a/b']);
  });

  it('lets Enter submit when there is no text', () => {
    const { input } = render(['a/b']);

    expect(key(input, 'Enter').defaultPrevented).toBe(false);
  });

  it('removes the last chip on Backspace in empty text', () => {
    const { props, input } = render(['a/b', 'c/d']);

    key(input, 'Backspace');

    expect(props.value).toEqual(['a/b']);
  });

  it('removes a chip by its button', () => {
    const { props } = render(['a/b', 'c/d']);

    (document.querySelector('button[aria-label="Remove a/b"]') as HTMLElement).click();
    flushSync();

    expect(props.value).toEqual(['c/d']);
  });

  it('ignores duplicates', () => {
    const { props, input } = render(['a/b']);

    type(input, 'a/b,');

    expect(chips()).toEqual(['a/b']);
    expect(props.value).toEqual(['a/b']);
  });
});
