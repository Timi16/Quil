// Shared shaping of API data: evidence lines, source names, sparklines, payoff rows.

import { toMicro, longPerUnit, MICRO } from './format.js';
import { raw } from './ui.js';

export const SOURCE_NAMES = {
  akash: 'Akash',
  runpod: 'RunPod',
  vast: 'Vast.ai',
  shadeform: 'Shadeform',
  'near-ai': 'NEAR AI Cloud',
};

export const SOURCE_API = {
  akash: 'public API',
  runpod: 'public API',
  vast: '/api/v0/bundles',
  shadeform: 'public API',
  'near-ai': '/v1/model/list',
};

/** "H100|akash|2560000|56" -> { index, source, name, micro, listings } */
export function parseEvidence(lines = []) {
  const out = [];
  for (const line of lines) {
    const [index, source, value, listings] = String(line).split('|');
    if (!index || !source || !/^\d+$/.test(value || '')) continue;
    out.push({ index, source, name: SOURCE_NAMES[source] || source, api: SOURCE_API[source] || '', micro: BigInt(value), listings: listings || '' });
  }
  return out;
}

/** Problems that mention an index: "B200: akash: model not listed". */
export function problemsFor(problems = [], index) {
  return problems.filter((p) => String(p).startsWith(`${index}:`)).map((p) => String(p).slice(index.length + 1).trim());
}

export function liveValue(live, index) {
  const v = live && (live.values || []).find((x) => x.index === index);
  return v || null;
}

/** History newest-first -> oldest-first numbers for charts. */
export function chronological(history = []) {
  return [...history].sort((a, b) => a.slot - b.slot);
}

/** Inline SVG sparkline; values are display-only numbers. */
export function sparkline(values, { w = 120, h = 28, stroke = 'var(--cobalt)', label = 'Trend', cls = 'spark' } = {}) {
  if (!values || values.length < 2) return raw('<span class="muted mono" style="font-size:11px">building</span>');
  const lo = Math.min(...values);
  const hi = Math.max(...values);
  const span = hi - lo || 1;
  const pts = values.map((v, i) => `${((w * i) / (values.length - 1)).toFixed(1)},${(h - 3 - ((v - lo) / span) * (h - 6)).toFixed(1)}`).join(' ');
  return raw(`<svg class="${cls}" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}" role="img" aria-label="${label.replace(/"/g, '')}"><polyline points="${pts}" fill="none" stroke="${stroke}" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"></polyline></svg>`);
}

export function trendWords(values, hours) {
  if (!values || values.length < 2) return 'Not enough fixings yet for a trend';
  const a = values[0];
  const b = values[values.length - 1];
  const word = b > a ? 'Rising' : b < a ? 'Falling' : 'Flat';
  return `${word} over the last ${hours} hours`;
}

/** Payoff rows for a band: floor, quarter points, today's value, cap. All micro. */
export function payoffRows(floorM, capM, todayM) {
  const width = capM - floorM;
  const cent = 10000n;
  const q1 = ((floorM + width / 4n) / cent) * cent;
  const q3 = ((floorM + (3n * width) / 4n) / cent) * cent;
  const pts = [{ v: floorM, note: 'at the floor', edge: 'floor' }, { v: q1, note: '' }, { v: q3, note: '' }, { v: capM, note: 'at the cap', edge: 'cap' }];
  if (todayM !== null && todayM !== undefined) pts.push({ v: todayM, note: 'latest fixing', today: true });
  pts.sort((a, b) => (a.v < b.v ? -1 : a.v > b.v ? 1 : (a.today ? 1 : -1)));
  const seen = new Set();
  const rows = [];
  for (const p of pts) {
    const key = p.v.toString();
    if (seen.has(key)) {
      const prev = rows.find((r) => r.v === p.v);
      if (p.today && prev) { prev.today = true; prev.note = prev.note ? `${prev.note}, latest fixing` : 'latest fixing'; }
      continue;
    }
    seen.add(key);
    const long = longPerUnit(p.v, floorM, capM);
    rows.push({ ...p, long, short: width - long });
  }
  return rows;
}

export function micro(text) {
  const v = toMicro(text);
  return v === null ? null : v;
}

export { MICRO };
