// Money, units and time formatting. Run: node --test app/tests
import test from 'node:test';
import assert from 'node:assert/strict';

import {
  MICRO, toMicro, microText, amt, unitsOf, unitsText, deltaText, longPerUnit, times,
  chartNum, indexLabel, indexMeta, utcFull, utcShort, utcDay, utcHour, timeLeft,
  secsToNextHour, mmss, nextHour, shortHash, marketName, isAccountId, wholeUnits,
} from '../js/format.js';

test('decimal text becomes exact micro-units', () => {
  assert.equal(toMicro('3.06'), 3_060_000n);
  assert.equal(toMicro('3.060000'), 3_060_000n);
  assert.equal(toMicro('0.000001'), 1n);
  assert.equal(toMicro('350'), 350_000_000n);
  assert.equal(toMicro('.5'), 500_000n);
  assert.equal(toMicro('1.'), 1_000_000n);
  assert.equal(toMicro(' 1,234.5 '), 1_234_500_000n, 'grouping and spaces are tolerated');
  assert.equal(toMicro('-2.5'), -2_500_000n);
  // Far beyond Number precision stays exact.
  assert.equal(toMicro('340282366920938463463374607431768.211455'), 340282366920938463463374607431768211455n);
});

test('anything that is not a plain decimal is refused, never guessed', () => {
  for (const bad of [null, undefined, '', '.', '-', 'abc', '1e6', '0x10', '1.2.3', '1.0000001', 'NaN', 'Infinity', '--1', '1-']) {
    assert.equal(toMicro(bad), null, JSON.stringify(bad));
  }
});

test('micro-units print exactly, with grouping and a real minus sign', () => {
  assert.equal(microText(156_000_000n), '156.00');
  assert.equal(microText(3_060_000n), '3.06');
  assert.equal(microText(2_643_750n), '2.64375');
  assert.equal(microText(1n), '0.000001');
  assert.equal(microText(0n), '0.00');
  assert.equal(microText(1_234_567_000_000n), '1,234,567.00');
  assert.equal(microText(-1_500_000n), '−1.50');
  assert.equal(microText(5_000_000n, 0), '5');
  assert.equal(microText(null), '—');
  assert.equal(microText(undefined), '—');
});

test('API strings round-trip through amt without loss', () => {
  for (const s of ['0.000000', '3.060000', '2.643750', '280.000000', '0.010000', '999999999.999999']) {
    assert.equal(toMicro(amt(s, 6).replace(/,/g, '')), toMicro(s), s);
  }
  assert.equal(amt('garbage'), '—');
});

test('units and deltas', () => {
  assert.equal(unitsOf(100), 100_000_000n);
  assert.equal(unitsOf('7'), 7_000_000n);
  assert.equal(unitsText(78_000_000n), '78');
  assert.equal(unitsText(2_500_000n), '2.5');
  assert.equal(deltaText(0n), 'no change');
  assert.equal(deltaText(1_500_000n), '+1.50');
  assert.equal(deltaText(-1_500_000n), '−1.50');
});

test('the worked example pays LONG 156.00 and SHORT 194.00', () => {
  const [floor, cap, index, units] = [1_500_000n, 5_000_000n, 3_060_000n, unitsOf(100)];
  const long = times(units, longPerUnit(index, floor, cap));
  const short = times(units, cap - floor - longPerUnit(index, floor, cap));
  assert.equal(microText(long), '156.00');
  assert.equal(microText(short), '194.00');
  assert.equal(long + short, times(units, cap - floor), 'the two sides always sum to the collateral');
});

test('LONG per unit is clamped to the band', () => {
  const [floor, cap] = [1_500_000n, 5_000_000n];
  assert.equal(longPerUnit(0n, floor, cap), 0n, 'below the floor');
  assert.equal(longPerUnit(floor, floor, cap), 0n, 'at the floor');
  assert.equal(longPerUnit(cap, floor, cap), 3_500_000n, 'at the cap');
  assert.equal(longPerUnit(99_000_000n, floor, cap), 3_500_000n, 'far above the cap');
});

test('payouts round down like the contract', () => {
  // 1 base unit at 1.999999 per unit is 0.000001999999 USDC: rounds down to 1 micro.
  assert.equal(times(1n, 1_999_999n), 1n);
  assert.equal(times(3n, 333_333n), 0n);
  assert.equal(times(unitsOf(3), 333_333n), 999_999n);
});

test('chart numbers are for drawing only and refuse junk', () => {
  assert.equal(chartNum('2.64375'), 2.64375);
  assert.equal(chartNum('nope'), null);
  assert.equal(chartNum('Infinity'), null);
});

test('index names and metadata', () => {
  assert.equal(indexLabel('INF_OUT'), 'INF-OUT');
  assert.equal(indexLabel('H100'), 'H100');
  assert.equal(indexMeta('H100').unitWord, 'GPU-hour');
  assert.equal(indexMeta('INF_OUT').unit, 'USD / 1M tokens');
  assert.equal(indexMeta('DOGE').kind, 'other', 'an unknown index still renders');
});

test('times are UTC, whatever the local zone', () => {
  const expiry = 498505 * 3600; // market 0
  assert.equal(utcFull(expiry), '14 Nov 2026 01:00 UTC');
  assert.equal(utcShort(expiry), '14 Nov 01:00');
  assert.equal(utcDay(expiry), '14 Nov');
  assert.equal(utcHour(expiry), '01:00 UTC');
  assert.equal(marketName({ index: 'H100', expires_at: expiry }), 'H100 · 14 Nov');
});

test('time left', () => {
  const now = 1_000_000;
  assert.equal(timeLeft(now, now), null, 'now is not in the future');
  assert.equal(timeLeft(now - 5, now), null, 'past');
  assert.equal(timeLeft(now + 20, now), '1 min', 'under a minute shows 1 min, never 0');
  assert.equal(timeLeft(now + 12 * 60, now), '12 min');
  assert.equal(timeLeft(now + 5 * 3600 + 12 * 60, now), '5 h 12 min');
  assert.equal(timeLeft(now + 86400 + 9 * 3600, now), '1 day 9 h');
  assert.equal(timeLeft(now + 2 * 86400, now), '2 days 0 h');
});

test('the hour clock', () => {
  assert.equal(secsToNextHour(3600 * 10), 3600, 'exactly on the hour: a full hour to go');
  assert.equal(secsToNextHour(3600 * 10 + 1), 3599);
  assert.equal(nextHour(3600 * 10 + 59), 3600 * 11);
  assert.equal(mmss(0), '00:00');
  assert.equal(mmss(3599), '59:59');
});

test('short hashes', () => {
  assert.equal(shortHash('sha256:c163a26f9867a3afe597de2e8abc21c157f81a9e95737a50509ad262f0d54ab6'), 'c163a26f…0d54ab6');
  assert.equal(shortHash('abc'), 'abc');
  assert.equal(shortHash(''), '—');
  assert.equal(shortHash(null), '—');
});

test('NEAR account ids', () => {
  for (const good of ['ab', 'alice.near', 'agent.quill-1112-f33adb.testnet', 'a_b-c.d', 'f'.repeat(64)]) {
    assert.ok(isAccountId(good), good);
  }
  for (const bad of ['', 'a', 'Alice.near', '.alice', 'alice.', 'al..ice', 'a--b', 'a b', 'alice@near', 'x'.repeat(65), null, 42]) {
    assert.ok(!isAccountId(bad), String(bad));
  }
});

test('whole units from user input', () => {
  assert.equal(wholeUnits('100'), 100n);
  assert.equal(wholeUnits(' 7 '), 7n);
  for (const bad of ['0', '', '-1', '1.5', '1e3', 'abc', '1234567890', null, undefined]) {
    assert.equal(wholeUnits(bad), null, String(bad));
  }
});

test('MICRO is one token', () => {
  assert.equal(MICRO, 1_000_000n);
});
