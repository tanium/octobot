// Subsequence fuzzy matching, in the spirit of fzf: every query character
// must appear in order. Matches score higher when they are contiguous or
// start words, so `octo` ranks `tanium/octobot` above `tanium/other-tool`.

export interface Match {
  score: number;
  // Indexes into the text of each matched character, for highlighting.
  positions: number[];
}

const boundaryChars = ' /-_.@#:';

function isBoundary(text: string, i: number): boolean {
  if (i === 0) {
    return true;
  }
  const prev = text[i - 1];
  if (boundaryChars.includes(prev)) {
    return true;
  }
  const cur = text[i];
  // camelCase hump
  return prev === prev.toLowerCase() && cur !== cur.toLowerCase();
}

// Gaps longer than this cost the same as this.
const maxGap = 8;

function isSubsequence(q: string, t: string): boolean {
  let qi = 0;
  for (let i = 0; i < t.length && qi < q.length; i++) {
    if (t[i] === q[qi]) {
      qi++;
    }
  }
  return qi === q.length;
}

// Whitespace in the query is ignored, so `tan oct` matches `tanium/octobot`.
export function fuzzy(query: string, text: string): Match | null {
  const q = query.replace(/\s+/g, '').toLowerCase();
  if (q === '') {
    return { score: 0, positions: [] };
  }
  const t = text.toLowerCase();
  if (!isSubsequence(q, t)) {
    return null;
  }

  // best[k][i]: the best score with query char k matched at text index i.
  // A greedy scan would miss word starts that come after an earlier hit.
  const n = t.length;
  const best = q.split('').map(() => new Float64Array(n).fill(-Infinity));
  const prev = q.split('').map(() => new Int32Array(n).fill(-1));
  const charScore = (i: number) => 16 + (isBoundary(text, i) ? 10 : 0);

  for (let i = 0; i < n; i++) {
    if (t[i] === q[0]) {
      best[0][i] = charScore(i) - Math.min(i, 12) * 0.5;
    }
  }
  for (let k = 1; k < q.length; k++) {
    // The best predecessor far enough back that its gap is capped.
    let far = -Infinity;
    let farAt = -1;
    for (let i = k; i < n; i++) {
      const j = i - maxGap - 1;
      if (j >= 0 && best[k - 1][j] > far) {
        far = best[k - 1][j];
        farAt = j;
      }
      if (t[i] !== q[k]) {
        continue;
      }
      let s = far - maxGap;
      let at = farAt;
      for (let j = Math.max(0, i - maxGap); j < i; j++) {
        const gap = i - j - 1;
        const v = best[k - 1][j] + (gap === 0 ? 12 : -gap);
        if (v > s) {
          s = v;
          at = j;
        }
      }
      if (at !== -1) {
        best[k][i] = s + charScore(i);
        prev[k][i] = at;
      }
    }
  }

  const last = q.length - 1;
  let end = -1;
  for (let i = 0; i < n; i++) {
    if (end === -1 || best[last][i] > best[last][end]) {
      end = i;
    }
  }
  const positions: number[] = new Array(q.length);
  for (let k = last, i = end; k >= 0; i = prev[k][i], k--) {
    positions[k] = i;
  }
  // Prefer the shorter text among otherwise equal matches.
  return { score: best[last][end] - n * 0.05, positions };
}

export interface Ranked<T> {
  item: T;
  score: number;
  // Which of the item's fields matched best, and where.
  field: number;
  positions: number[];
}

// The items that match on any field, best first. An empty query keeps the
// original order. Fields after the first score a little lower, so a match
// on the name beats the same match on, say, an email.
export function rank<T>(items: T[], query: string, fields: (item: T) => string[]): Ranked<T>[] {
  const ranked: Ranked<T>[] = [];
  for (const item of items) {
    let best: Ranked<T> | null = null;
    fields(item).forEach((text, field) => {
      const m = fuzzy(query, text);
      if (m === null) {
        return;
      }
      const s = m.score - field * 4;
      if (best === null || s > best.score) {
        best = { item, score: s, field, positions: m.positions };
      }
    });
    if (best !== null) {
      ranked.push(best);
    }
  }
  if (query.trim() !== '') {
    // Array.prototype.sort is stable, so ties keep the original order.
    ranked.sort((a, b) => b.score - a.score);
  }
  return ranked;
}
