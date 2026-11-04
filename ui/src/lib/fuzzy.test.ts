import { describe, expect, it } from 'vitest';

import { fuzzy, rank } from './fuzzy';

describe('fuzzy', () => {
  it('matches characters in order, ignoring case and spaces', () => {
    expect(fuzzy('TAN oct', 'tanium/octobot')?.positions).toEqual([0, 1, 2, 7, 8, 9]);
  });

  it('rejects text missing a character or with them out of order', () => {
    expect(fuzzy('octoz', 'tanium/octobot')).toBeNull();
    expect(fuzzy('to', 'ot')).toBeNull();
  });

  it('matches everything on an empty query', () => {
    expect(fuzzy('  ', 'anything')).toEqual({ score: 0, positions: [] });
  });

  it('picks the contiguous run over an earlier scattered one', () => {
    // o-c-t appear scattered in "o_c_t" before the run in "octo".
    expect(fuzzy('oct', 'o_c_t/octo')?.positions).toEqual([6, 7, 8]);
  });

  it('scores word starts above mid-word hits', () => {
    const start = fuzzy('ob', 'org/octo-bot')!;
    const mid = fuzzy('ob', 'org/jobs')!;
    expect(start.score).toBeGreaterThan(mid.score);
  });
});

describe('rank', () => {
  const repos = ['tanium/cocoa-tools', 'tanium/octobot', 'acme/robot'];

  it('orders matches best first and drops non-matches', () => {
    expect(rank(repos, 'octo', (r) => [r]).map((r) => r.item)).toEqual(['tanium/octobot', 'tanium/cocoa-tools']);
  });

  it('keeps the original order on an empty query', () => {
    expect(rank(repos, '', (r) => [r]).map((r) => r.item)).toEqual(repos);
  });

  it('reports which field matched', () => {
    const users = [{ github: 'alice', email: 'al@example.com' }];
    const [r] = rank(users, 'example', (u) => [u.github, u.email]);
    expect(r.field).toBe(1);
  });

  it('prefers an equal match on an earlier field', () => {
    const users = [
      { github: 'zed', email: 'bob@example.com' },
      { github: 'bob', email: 'zed@example.com' },
    ];
    expect(rank(users, 'bob', (u) => [u.github, u.email])[0].item.github).toBe('bob');
  });
});
