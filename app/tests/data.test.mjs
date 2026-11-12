// Shaping API data: evidence, problems, history order, payoff rows, sparklines.
import test from 'node:test';
import assert from 'node:assert/strict';

import { parseEvidence, problemsFor, liveValue, chronological, sparkline, trendWords, payoffRows } from '../js/data.js';
import { microText } from '../js/format.js';

test('evidence lines from the oracle are parsed, junk skipped', () => {
  const rows = parseEvidence([
    'H100|akash|2560000|56',
    'H100|vast|2700000|31',
    'INF_OUT|near-ai|1200000|',
    'H100|mystery|1000000|2',
    'broken line',
    'H100|runpod|not-a-number|3',
    'H100|runpod||3',
  ]);
  assert.deepEqual(rows.map((r) => [r.index, r.name, r.micro, r.listings]), [
    ['H100', 'Akash', 2_560_000n, '56'],
    ['H100', 'Vast.ai', 2_700_000n, '31'],
    ['INF_OUT', 'NEAR AI Cloud', 1_200_000n, ''],
    ['H100', 'mystery', 1_000_000n, '2'],
  ]);
  assert.deepEqual(parseEvidence(), []);
  assert.deepEqual(parseEvidence(undefined), []);
});

test('problems are filtered by index prefix, not substring', () => {
  const problems = ['B200: akash: model not listed', 'H100: vast: timeout', 'H1000: nonsense'];
  assert.deepEqual(problemsFor(problems, 'H100'), ['vast: timeout']);
  assert.deepEqual(problemsFor(problems, 'B200'), ['akash: model not listed']);
  assert.deepEqual(problemsFor(undefined, 'H100'), []);
});

test('live values are found by index or null', () => {
  const live = { values: [{ index: 'H100', value: '2.643750', sources: 4 }] };
  assert.equal(liveValue(live, 'H100').sources, 4);
  assert.equal(liveValue(live, 'B200'), null);
  assert.equal(liveValue(null, 'H100'), null);
  assert.equal(liveValue({}, 'H100'), null);
});

test('history is put oldest first without changing the input', () => {
  const newestFirst = [{ slot: 3 }, { slot: 2 }, { slot: 1 }];
  assert.deepEqual(chronological(newestFirst).map((f) => f.slot), [1, 2, 3]);
  assert.deepEqual(newestFirst.map((f) => f.slot), [3, 2, 1], 'the API answer is not mutated');
  assert.deepEqual(chronological(), []);
});

test('payoff rows for the worked band: LONG + SHORT always equals the width', () => {
  const [floor, cap] = [1_500_000n, 5_000_000n];
  const rows = payoffRows(floor, cap, 3_060_000n);
  assert.deepEqual(rows.map((r) => microText(r.v)), ['1.50', '2.37', '3.06', '4.12', '5.00']);
  for (const r of rows) assert.equal(r.long + r.short, cap - floor, microText(r.v));
  const today = rows.find((r) => r.today);
  assert.equal(microText(today.long), '1.56');
  assert.equal(microText(today.short), '1.94');
  assert.equal(rows[0].edge, 'floor');
  assert.equal(rows.at(-1).edge, 'cap');
});

test('a fixing on a payoff point merges into that row instead of duplicating it', () => {
  const rows = payoffRows(1_500_000n, 5_000_000n, 5_000_000n);
  assert.equal(rows.length, 4);
  const cap = rows.at(-1);
  assert.ok(cap.today);
  assert.equal(cap.note, 'at the cap, latest fixing');
});

test('a fixing outside the band is still shown, clamped', () => {
  const below = payoffRows(1_500_000n, 5_000_000n, 900_000n);
  assert.equal(below[0].today, true);
  assert.equal(below[0].long, 0n);
  const above = payoffRows(1_500_000n, 5_000_000n, 7_000_000n);
  assert.equal(above.at(-1).today, true);
  assert.equal(above.at(-1).long, 3_500_000n);
});

test('no fixing yet: only the band rows', () => {
  assert.equal(payoffRows(1_500_000n, 5_000_000n, null).length, 4);
  assert.ok(!payoffRows(1_500_000n, 5_000_000n, undefined).some((r) => r.today));
});

test('sparklines need two points, survive a flat line, and cannot inject markup', () => {
  assert.match(sparkline([]).__html, /building/);
  assert.match(sparkline([2.6]).__html, /building/);
  const flat = sparkline([2.6, 2.6, 2.6]).__html;
  assert.doesNotMatch(flat, /NaN|Infinity/);
  const hostile = sparkline([1, 2], { label: 'x" onload="alert(1)' }).__html;
  assert.doesNotMatch(hostile, /onload="/);
});

test('trend words', () => {
  assert.equal(trendWords([1, 2], 48), 'Rising over the last 48 hours');
  assert.equal(trendWords([2, 1], 6), 'Falling over the last 6 hours');
  assert.equal(trendWords([2, 2], 6), 'Flat over the last 6 hours');
  assert.equal(trendWords([2], 6), 'Not enough fixings yet for a trend');
});
