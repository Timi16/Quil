// Small rendering helpers shared by every screen.

import { explorerTx, walletState } from './wallet.js';

const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
export function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => ESC[c]);
}

/** Raw HTML marker: html`` output and anything wrapped in raw() is not escaped again. */
export function raw(s) {
  return { __html: String(s) };
}

function piece(v) {
  if (v === null || v === undefined || v === false) return '';
  if (Array.isArray(v)) return v.map(piece).join('');
  if (typeof v === 'object' && '__html' in v) return v.__html;
  return esc(v);
}

/** Tagged template that escapes every interpolated value. */
export function html(strings, ...vals) {
  let out = '';
  strings.forEach((s, i) => {
    out += s;
    if (i < vals.length) out += piece(vals[i]);
  });
  return raw(out);
}

export function mount(el, content) {
  el.innerHTML = piece(content);
}

// ---------- brand ----------

export function logoMark(size = 30, label = null) {
  const a = label ? `role="img" aria-label="${esc(label)}"` : 'aria-hidden="true"';
  return raw(`<svg viewBox="0 0 32 32" width="${size}" height="${size}" ${a}><rect x="1" y="1" width="30" height="30" rx="8" fill="var(--sun)" stroke="var(--ink)" stroke-width="2"></rect><path d="M6 22l4-9 4 6 4-10 4 9 3-4" fill="none" stroke="var(--ink)" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"></path><circle cx="26.2" cy="21.6" r="1.8" fill="var(--ink)"></circle></svg>`);
}

function ballPoints() {
  const pts = [];
  const n = 16;
  for (let i = 0; i < n * 2; i++) {
    const r = i % 2 === 0 ? 54 : 40;
    const a = (Math.PI * 2 * i) / (n * 2) - Math.PI / 2;
    pts.push((60 + r * Math.cos(a)).toFixed(1) + ',' + (60 + r * Math.sin(a)).toFixed(1));
  }
  return pts.join(' ');
}
const BALL = ballPoints();

/** Quill the hedgehog, standing or curled into a ball. Decorative unless `label` is given. */
export function mascot(pose = 'stand', size = 150, label = null) {
  const ball = pose === 'ball';
  const h = ball ? size : Math.round((size * 112) / 170);
  const a = label ? `role="img" aria-label="${esc(label)}"` : 'aria-hidden="true"';
  const svg = ball
    ? `<svg viewBox="0 0 120 120" ${a}><polygon points="${BALL}" fill="var(--hog)" stroke="var(--ink)" stroke-width="3" stroke-linejoin="round"></polygon><path d="M38 52l5-9 5 9M58 42l5-9 5 9M44 74l5-9 5 9M64 64l5-9 5 9M56 88l5-9 5 9" fill="none" stroke="var(--sun)" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"></path><path d="M86 62 C100 58 112 68 113 77 C104 84 92 82 84 76 Z" fill="var(--paper-2)" stroke="var(--ink)" stroke-width="3" stroke-linejoin="round"></path><path d="M93 70q3-3 6 0" fill="none" stroke="var(--ink)" stroke-width="2.2" stroke-linecap="round"></path><circle cx="111" cy="76" r="3.4" fill="var(--ink)"></circle></svg>`
    : `<svg viewBox="0 0 170 112" ${a}><ellipse cx="46" cy="102" rx="9" ry="5" fill="var(--paper-2)" stroke="var(--ink)" stroke-width="2.5"></ellipse><ellipse cx="74" cy="102" rx="9" ry="5" fill="var(--paper-2)" stroke="var(--ink)" stroke-width="2.5"></ellipse><path d="M26 98 L14 86 L27 83 L14 69 L29 68 L20 52 L36 55 L31 38 L47 44 L46 27 L60 37 L65 20 L76 34 L86 19 L93 35 L106 24 L109 41 L122 35 L121 51 L132 50 L128 64 L120 98 Z" fill="var(--hog)" stroke="var(--ink)" stroke-width="3" stroke-linejoin="round"></path><path d="M40 72l5-9 5 9M62 60l5-9 5 9M84 58l5-9 5 9M56 84l5-9 5 9M80 82l5-9 5 9M100 70l4-8 4 8" fill="none" stroke="var(--sun)" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"></path><path d="M104 98 C100 80 108 62 124 60 C136 58 148 68 158 82 C161 86 162 90 158 92 C150 96 136 98 104 98 Z" fill="var(--paper-2)" stroke="var(--ink)" stroke-width="3" stroke-linejoin="round"></path><circle cx="115" cy="62" r="6.5" fill="var(--hog-nose)" stroke="var(--ink)" stroke-width="2.5"></circle><ellipse cx="143" cy="85" rx="5" ry="3" fill="var(--hog-cheek)"></ellipse><g class="hog-eye"><circle cx="134" cy="74" r="3.6" fill="var(--ink)"></circle><circle cx="135.3" cy="72.7" r="1.1" fill="var(--white)"></circle></g><circle cx="159" cy="87" r="4.2" fill="var(--ink)"></circle><ellipse cx="114" cy="102" rx="9" ry="5" fill="var(--paper-2)" stroke="var(--ink)" stroke-width="2.5"></ellipse><ellipse cx="138" cy="102" rx="9" ry="5" fill="var(--paper-2)" stroke="var(--ink)" stroke-width="2.5"></ellipse></svg>`;
  return raw(`<span class="hog" style="width:${size}px;height:${h}px">${svg}</span>`);
}

export const arrowIcon = raw('<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14M13 6l6 6-6 6"></path></svg>');
export const warnIcon = raw('<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 9v4M12 17h.01"></path><path d="M10.3 3.9L2.4 17.5A2 2 0 004.1 20.5h15.8a2 2 0 001.7-3L13.7 3.9a2 2 0 00-3.4 0z"></path></svg>');
export const copyIcon = raw('<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="8.5" y="8.5" width="12" height="12" rx="2.5"></rect><path d="M15.5 8.5V6a2.5 2.5 0 00-2.5-2.5H6A2.5 2.5 0 003.5 6v7A2.5 2.5 0 006 15.5h2.5"></path></svg>');
export const shieldIcon = raw('<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3l7.5 3v5.5c0 4.5-3.2 8-7.5 9.5-4.3-1.5-7.5-5-7.5-9.5V6L12 3z"></path><path d="M8.5 12l2.5 2.5 4.5-5"></path></svg>');

// ---------- states ----------

export function loading(text = 'Loading…') {
  return html`<div class="empty" role="status"><span class="muted mono" style="font-size:13px">${text}</span></div>`;
}

export function emptyState(title, body = '', extra = '') {
  return html`<div class="empty"><span class="empty-title">${title}</span>${body ? html`<span class="hint">${body}</span>` : ''}${extra}</div>`;
}

/** Empty state for a failed load, with a "Try again" button handled by app.js. */
export function failState(title, err, what) {
  const transient = !err || err.status !== 404;
  return emptyState(title, errText(err, what), transient ? html`<button type="button" class="btn btn--sm" data-retry>Try again</button>` : '');
}

/** Text for an API failure, honest about routes that do not exist yet. */
export function errText(err, what = 'This data') {
  if (!err) return `${what} is not available.`;
  if (err.status === 404 && /not found/i.test(err.message)) return `${what} is not available from this server yet.`;
  if (err.status === 503) return `${what} is not available: ${err.message}`;
  return err.message || String(err);
}

// ---------- action feedback ----------

/** Render pending / success / error into a status element. */
export function statusBox(el) {
  const set = (cls, content) => {
    el.className = `status status--${cls}`;
    mount(el, content);
  };
  return {
    pending(msg) { set('pending', html`<span class="spinner" aria-hidden="true"></span><span>${msg}</span>`); },
    ok(msg, hashes = []) {
      set('ok', html`<span>${msg} ${hashes.map((h) => html`<a href="${explorerTx(h)}" target="_blank" rel="noopener">View ${h.slice(0, 6)}… on the explorer ↗</a> `)}</span>`);
    },
    err(msg, hashes = []) {
      set('err', html`<span><strong>Failed:</strong> ${msg} ${hashes.map((h) => html`<a href="${explorerTx(h)}" target="_blank" rel="noopener">Transaction ↗</a> `)}</span>`);
    },
    clear() { el.className = 'status'; el.innerHTML = ''; },
  };
}

/**
 * Run a wallet action from a button: disables it, shows pending, then the result.
 * `fn` returns `{ hashes, failure }` from wallet.send().
 */
export async function runAction(button, statusEl, label, fn, onDone) {
  const box = statusBox(statusEl);
  if (!walletState().accountId) {
    box.err('Sign in with your NEAR testnet wallet first.');
    return;
  }
  button.disabled = true;
  box.pending(`${label}: confirm in your wallet…`);
  try {
    const { hashes, failure } = await fn();
    if (failure) box.err(failure, hashes);
    else box.ok(`${label}: done.`, hashes);
    if (onDone) onDone(!failure);
  } catch (e) {
    box.err(e && e.message ? e.message : String(e));
  } finally {
    button.disabled = false;
  }
}

// ---------- toasts ----------

export function toast(content, ms = 7000) {
  const host = document.getElementById('toasts');
  if (!host) return;
  const t = document.createElement('div');
  t.className = 'toast';
  mount(t, html`<span>${content}</span><button type="button" class="btn-text close" aria-label="Dismiss message">✕</button>`);
  t.querySelector('.close').addEventListener('click', () => t.remove());
  host.appendChild(t);
  if (ms) setTimeout(() => t.remove(), ms);
}

export async function copyText(text, button) {
  try {
    await navigator.clipboard.writeText(text);
    if (button) {
      const old = button.getAttribute('aria-label');
      button.setAttribute('aria-label', 'Copied');
      toast('Copied to the clipboard.', 2500);
      setTimeout(() => button.setAttribute('aria-label', old), 2000);
    }
  } catch {
    toast('Could not copy. Select the text and copy it by hand.');
  }
}
