import { describe, expect, it } from 'vitest';

import { formatList, parseList } from './list';

describe('parseList', () => {
  it('splits on commas and trims', () => {
    expect(parseList(' a/b,c/d ,  e ')).toEqual(['a/b', 'c/d', 'e']);
  });

  it('drops empty entries', () => {
    expect(parseList('a,, ,b,')).toEqual(['a', 'b']);
    expect(parseList('')).toEqual([]);
  });

  it('round-trips with formatList', () => {
    expect(parseList(formatList(['a/b', 'c/d']))).toEqual(['a/b', 'c/d']);
  });
});
