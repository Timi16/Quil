// Exact money and time formatting. Amounts travel as decimal strings ("3.060000")
// and are turned into BigInt micro-units for any arithmetic; never into floats.

export const MICRO = 1000000n;

/** "3.06" or "3.060000" -> 3060000n. Returns null if the text is not a plain decimal with at most 6 places. */
export function toMicro(text) {
  if (text === null || text === undefined) return null;
  const t = String(text).trim().replace(/,/g, '');
  const m = /^(-)?(\d*)(?:\.(\d*))?$/.exec(t);
  if (!m || (m[2] === '' && !m[3])) return null;
  const frac = m[3] || '';
  if (frac.length > 6) return null;
  const v = BigInt(m[2] || '0') * MICRO + BigInt((frac + '000000').slice(0, 6));
  return m[1] ? -v : v;
}

function group(intStr) {
  return intStr.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

/** Exact decimal text of a micro amount, trailing zeros trimmed down to `minDp` places, with thousands separators. */
export function microText(v, minDp = 2) {
  if (v === null || v === undefined) return '—';
  const neg = v < 0n;
  const a = neg ? -v : v;
  const int = (a / MICRO).toString();
  let frac = (a % MICRO).toString().padStart(6, '0');
  while (frac.length > minDp && frac.endsWith('0')) frac = frac.slice(0, -1);
  return (neg ? '−' : '') + group(int) + (frac ? '.' + frac : '');
}

/** Same as microText but from an API decimal string. */
export function amt(text, minDp = 2) {
  const v = toMicro(text);
  return v === null ? '—' : microText(v, minDp);
}

/** Base units (6 decimals) of a whole number of tokens. */
export function unitsOf(n) {
  return BigInt(n) * MICRO;
}

/** Whole-token count text from base units, exact. */
export function unitsText(v) {
  return microText(v, 0);
}

/** Signed change text with a real minus sign. */
export function deltaText(v, minDp = 2) {
  if (v === 0n) return 'no change';
  return (v > 0n ? '+' : '') + microText(v, minDp);
}

/** LONG payout per unit for a settlement index: clamp(index - floor, 0, width). All micro. */
export function longPerUnit(indexM, floorM, capM) {
  const width = capM - floorM;
  let v = indexM - floorM;
  if (v < 0n) v = 0n;
  if (v > width) v = width;
  return v;
}

/** Micro amount for `units` base units at `perUnit` micro-USDC each, rounded down like the contract. */
export function times(unitsBase, perUnitM) {
  return (unitsBase * perUnitM) / MICRO;
}

/** Floating value for charts only. */
export function chartNum(text) {
  const n = Number(text);
  return Number.isFinite(n) ? n : null;
}

/** Display name of an index id. */
export function indexLabel(id) {
  return id === 'INF_OUT' ? 'INF-OUT' : id;
}

const INDEX_META = {
  H100: { kind: 'gpu', unit: 'USD / GPU-hr', short: 'USD/hr', unitWord: 'GPU-hour', unitWords: 'GPU-HOURS', desc: 'NVIDIA H100 rental, per GPU-hour', glyph: 'glyph--gpu' },
  B200: { kind: 'gpu', unit: 'USD / GPU-hr', short: 'USD/hr', unitWord: 'GPU-hour', unitWords: 'GPU-HOURS', desc: 'NVIDIA B200 rental, per GPU-hour', glyph: 'glyph--b200' },
  INF_OUT: { kind: 'inf', unit: 'USD / 1M tokens', short: 'USD/1M', unitWord: 'million tokens', unitWords: '1M TOKENS', desc: 'NEAR AI Cloud, per 1M output tokens', glyph: 'glyph--inf' },
};

export function indexMeta(id) {
  return INDEX_META[id] || { kind: 'other', unit: 'USD', short: 'USD', unitWord: 'unit', unitWords: 'UNITS', desc: id, glyph: 'glyph--gpu' };
}

export const INDEXES = ['H100', 'B200', 'INF_OUT'];

// ---------- time ----------
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const pad = (n) => String(n).padStart(2, '0');

export function slotStart(slot) {
  return Number(slot) * 3600;
}

/** "12 Nov 2026 15:00 UTC" */
export function utcFull(unixSec) {
  const d = new Date(unixSec * 1000);
  return `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]} ${d.getUTCFullYear()} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())} UTC`;
}

/** "12 Nov 15:00" */
export function utcShort(unixSec) {
  const d = new Date(unixSec * 1000);
  return `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}`;
}

/** "14 Nov" */
export function utcDay(unixSec) {
  const d = new Date(unixSec * 1000);
  return `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]}`;
}

/** "15:00 UTC" */
export function utcHour(unixSec) {
  const d = new Date(unixSec * 1000);
  return `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())} UTC`;
}

/** Human time left until a Unix time: "1 day 9 h", "5 h 12 min", "12 min", or null if past. */
export function timeLeft(unixSec, now = Date.now() / 1000) {
  let s = Math.floor(unixSec - now);
  if (s <= 0) return null;
  const d = Math.floor(s / 86400); s -= d * 86400;
  const h = Math.floor(s / 3600); s -= h * 3600;
  const m = Math.floor(s / 60);
  if (d > 0) return `${d} day${d === 1 ? '' : 's'} ${h} h`;
  if (h > 0) return `${h} h ${m} min`;
  return `${Math.max(1, m)} min`;
}

/** Seconds until the next whole UTC hour. */
export function secsToNextHour(now = Date.now() / 1000) {
  return 3600 - (Math.floor(now) % 3600);
}

export function mmss(secs) {
  return `${pad(Math.floor(secs / 60))}:${pad(secs % 60)}`;
}

/** Next whole UTC hour as Unix seconds. */
export function nextHour(now = Date.now() / 1000) {
  return Math.floor(now / 3600) * 3600 + 3600;
}

/** Short form of a long hash or account: "c163a26f…0d54ab6". */
export function shortHash(h, head = 8, tail = 7) {
  if (!h) return '—';
  const s = String(h).replace(/^sha256:/, '');
  return s.length <= head + tail + 1 ? s : `${s.slice(0, head)}…${s.slice(-tail)}`;
}

/** Name of a market: "H100 · 14 Nov". */
export function marketName(m) {
  return `${indexLabel(m.index)} · ${utcDay(m.expires_at)}`;
}

/** Is this a plausible NEAR account id? */
export function isAccountId(s) {
  return typeof s === 'string' && s.length >= 2 && s.length <= 64 && /^(([a-z\d]+[-_])*[a-z\d]+\.)*([a-z\d]+[-_])*[a-z\d]+$/.test(s);
}

/** Parse a whole positive number of tokens from user input; null if invalid. */
export function wholeUnits(text) {
  const t = String(text ?? '').trim();
  if (!/^\d{1,9}$/.test(t)) return null;
  const n = BigInt(t);
  return n > 0n ? n : null;
}
