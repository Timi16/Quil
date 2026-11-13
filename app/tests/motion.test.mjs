// Motion must never change a figure: a rolling price ends on the exact value.
import test from 'node:test';
import assert from 'node:assert/strict';

import { easeOut, rollText, rolling, pipeline, heroSpike, reducedMotion } from '../js/motion.js';

test('easing is clamped and monotonic', () => {
  assert.equal(easeOut(-1), 0);
  assert.equal(easeOut(0), 0);
  assert.equal(easeOut(1), 1);
  assert.equal(easeOut(5), 1);
  let last = -1;
  for (let i = 0; i <= 100; i++) {
    const v = easeOut(i / 100);
    assert.ok(v >= last, `not monotonic at ${i}`);
    last = v;
  }
});

test('a rolling price lands on the exact value, never a float approximation', () => {
  assert.equal(rollText(2_643_750n, 1), '2.64375');
  assert.equal(rollText(2_643_750n, 7), '2.64375');
  assert.equal(rollText(340282366920938463463374607431768211455n, 1), '340,282,366,920,938,463,463,374,607,431,768.211455');
  assert.equal(rollText(2_643_750n, 0), '0.00');
  assert.equal(rollText(5n, 1, 6), '0.000005');
});

test('in between, it only ever counts up towards the target', () => {
  const to = 2_697_176n;
  let last = -1n;
  for (let i = 0; i <= 50; i++) {
    const shown = BigInt(rollText(to, i / 50, 6).replace(/[.,]/g, ''));
    assert.ok(shown >= last && shown <= to, `frame ${i}: ${shown}`);
    last = shown;
  }
});

test('the rolling element carries its exact value and shows it without script', () => {
  const html = rolling(2_643_750n, { key: 'x' }).__html;
  assert.match(html, /data-roll="2643750"/);
  assert.match(html, />\$2\.64375</, 'the final value is in the markup, so it is right even if the animation never runs');
  assert.equal(rolling(null), '—');
});

test('the pipeline shows live figures and is labelled for screen readers', () => {
  const html = pipeline({ price: 2_643_750n, sources: 4, long: 1_143_750n, short: 2_356_250n }).__html;
  assert.match(html, /role="img" aria-label="Prices from Akash/);
  assert.match(html, /\$2\.64375/);
  assert.match(html, /LONG <b>1\.14375<\/b>/);
  assert.match(html, /SHORT <b>2\.35625<\/b>/);
  for (const source of ['Akash', 'RunPod', 'Vast.ai', 'Shadeform']) assert.match(html, new RegExp(source));
});

test('with no data yet the pipeline shows dashes, not zeros', () => {
  const html = pipeline().__html;
  assert.doesNotMatch(html, /\$0\.00/);
  assert.match(html, /LONG <b>—<\/b>/);
});

test('the hero line is decorative and hidden from assistive tech', () => {
  assert.match(heroSpike().__html, /aria-hidden="true"/);
});

test('no window means no reduced-motion preference to read', () => {
  assert.equal(reducedMotion(), false);
});
