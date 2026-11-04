import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';

import ListInput from './ListInput.svelte';

let component: ReturnType<typeof mount> | undefined;

afterEach(() => {
  if (component) {
    unmount(component);
  }
  document.body.innerHTML = '';
});

function render(initial: string[]) {
  const props = $state({ value: initial, placeholder: '' });
  component = mount(ListInput, { target: document.body, props });
  return { props, input: document.querySelector('input')! };
}

function type(input: HTMLInputElement, text: string) {
  input.value = text;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  flushSync();
}

describe('ListInput', () => {
  it('shows the initial list', () => {
    const { input } = render(['a/b', 'c/d']);
    expect(input.value).toBe('a/b, c/d');
  });

  it('updates the list with the latest edit', () => {
    const { props, input } = render([]);

    type(input, 'org/repo');

    expect(props.value).toEqual(['org/repo']);
  });

  it('clears the list when the text is cleared', () => {
    const { props, input } = render(['a/b']);

    type(input, '');

    expect(props.value).toEqual([]);
  });
});
