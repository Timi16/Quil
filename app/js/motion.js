// Motion that carries meaning: prices that roll up to their exact value, and a pipeline that
// shows a price travelling from the marketplaces, through the TEE, onto NEAR and into LONG / SHORT.
// Everything here stops under prefers-reduced-motion; the final state is always exact.

import { html, raw } from './ui.js';
import { microText } from './format.js';

export const reducedMotion = () =>
  typeof window !== 'undefined' && window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

/** Ease-out cubic, 0..1 -> 0..1. */
export function easeOut(t) {
  const c = Math.min(1, Math.max(0, t));
  return 1 - (1 - c) ** 3;
}

/**
 * The text a rolling counter shows at progress `t` (0..1) on its way to `toMicro`.
 * Integer maths only, so the last frame is the exact value, never a float approximation.
 */
export function rollText(toMicro, t, minDp = 2) {
  if (t >= 1) return microText(toMicro, minDp);
  const scaled = BigInt(Math.floor(easeOut(t) * 1_000_000));
  return microText((toMicro * scaled) / 1_000_000n, minDp);
}

const played = new Set();

/**
 * Roll every `[data-roll]` element under `root` up to its value (micro-units in the attribute).
 * Each (key, value) rolls once per page load, so a re-render with the same number stays still.
 */
export function rollNumbers(root, ms = 1100) {
  for (const el of root.querySelectorAll('[data-roll]')) {
    const to = BigInt(el.dataset.roll);
    const dp = Number(el.dataset.rollDp || 2);
    const prefix = el.dataset.rollPrefix || '';
    const key = `${el.dataset.rollKey || ''}:${to}`;
    if (reducedMotion() || played.has(key)) {
      el.firstChild.nodeValue = prefix + microText(to, dp);
      continue;
    }
    played.add(key);
    const start = performance.now();
    const step = (now) => {
      const t = (now - start) / ms;
      el.firstChild.nodeValue = prefix + rollText(to, t, dp);
      if (t < 1 && el.isConnected) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }
}

/** A number that rolls up: `$2.64375`. `micro` is a BigInt. */
export function rolling(micro, { prefix = '$', dp = 2, key = '' } = {}) {
  if (micro === null || micro === undefined) return '—';
  return raw(`<span data-roll="${micro}" data-roll-dp="${dp}" data-roll-prefix="${prefix}" data-roll-key="${key}">${prefix}${microText(micro, dp)}</span>`);
}

/**
 * The live pipeline: four marketplaces feed the enclave, the enclave writes a fixing on NEAR,
 * the fixing splits a pair into LONG and SHORT. Packets loop along the wires.
 */
export function pipeline({ price = null, sources = 4, long = null, short = null } = {}) {
  const src = ['Akash', 'RunPod', 'Vast.ai', 'Shadeform'];
  const wire = (n, delay = 0) => html`<div class="pipe-wire" aria-hidden="true">${Array.from({ length: n }, (_, i) => raw(`<i class="pipe-packet" style="animation-delay:${(delay + i * (2.4 / n)).toFixed(2)}s"></i>`))}</div>`;
  return html`
  <div class="pipe" role="img" aria-label="Prices from Akash, RunPod, Vast.ai and Shadeform flow into OutLayer's TEE, become an hourly fixing on NEAR, and settle LONG and SHORT">
    <div class="pipe-node pipe-node--src">
      <span class="pipe-step">01 · READ</span>
      <div class="pipe-srcs">${src.map((s, i) => html`<span class="pipe-src" style="animation-delay:${(i * 0.6).toFixed(1)}s">${s}</span>`)}</div>
      <span class="pipe-cap">live GPU listings</span>
    </div>
    ${wire(3, 0)}
    <div class="pipe-node pipe-node--tee">
      <span class="pipe-step">02 · COMPUTE IN A TEE</span>
      <div class="pipe-tee"><span class="pipe-scan" aria-hidden="true"></span><strong>median of medians</strong><span class="mono">Intel TDX · code hash pinned</span></div>
      <span class="pipe-cap">OutLayer</span>
    </div>
    ${wire(3, 0.8)}
    <div class="pipe-node pipe-node--near">
      <span class="pipe-step">03 · FIX ON NEAR</span>
      <div class="pipe-price">${price === null ? '—' : rolling(price, { key: 'flow' })}<span class="pipe-unit">/ GPU-hr</span></div>
      <span class="pipe-cap">${sources} sources · every hour</span>
    </div>
    ${wire(3, 1.6)}
    <div class="pipe-node pipe-node--split">
      <span class="pipe-step">04 · SETTLE</span>
      <div class="pipe-sides">
        <span class="pipe-side pipe-side--long">LONG <b>${long === null ? '—' : microText(long)}</b></span>
        <span class="pipe-side pipe-side--short">SHORT <b>${short === null ? '—' : microText(short)}</b></span>
      </div>
      <span class="pipe-cap">per pair · always sums to the collateral</span>
    </div>
  </div>`;
}

/** A price line that spikes, drawn behind the hero headline. Decorative. */
export function heroSpike() {
  return raw(`<svg class="hero-bg" viewBox="0 0 1200 420" preserveAspectRatio="none" aria-hidden="true">
    <path class="hero-bg-area" d="M0 330 L120 318 L220 326 L330 300 L430 310 L540 286 L640 296 L720 270 L800 282 L880 236 L940 250 L1000 150 L1040 190 L1090 92 L1130 120 L1200 60 L1200 420 L0 420 Z"></path>
    <path class="hero-bg-line" d="M0 330 L120 318 L220 326 L330 300 L430 310 L540 286 L640 296 L720 270 L800 282 L880 236 L940 250 L1000 150 L1040 190 L1090 92 L1130 120 L1200 60"></path>
    <circle class="hero-bg-dot" cx="1194" cy="62" r="7"></circle>
  </svg>`);
}
