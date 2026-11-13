// Trade (design/Trade.dc.html): chart, payoff, and a ticket that mints, sends and redeems.

import { api, settle, viewCached } from '../api.js';
import { html, mount, raw, loading, runAction, failState } from '../ui.js';
import { walletState, signIn, call, ONE_YOCTO } from '../wallet.js';
import { amt, microText, toMicro, unitsOf, times, indexLabel, indexMeta, utcShort, utcFull, timeLeft, marketName, deltaText, wholeUnits, isAccountId } from '../format.js';
import { chronological, payoffRows } from '../data.js';
import { rolling, rollNumbers } from '../motion.js';

const MODES = [
  { id: 'mint', label: 'Mint pair' },
  { id: 'long', label: 'Send LONG' },
  { id: 'short', label: 'Send SHORT' },
  { id: 'redeem', label: 'Redeem' },
];

export default async function trade(ctx) {
  const { root, cfg } = ctx;
  const id = Number(ctx.params[0]);
  mount(root, html`<div class="main">${loading('Loading market…')}</div>`);

  const mk = await settle(api.market(id));
  if (!ctx.isCurrent()) return;
  if (!mk.ok) {
    mount(root, html`<div class="main"><a class="back-link" href="#/markets">← Markets</a><div class="card">${failState(`Market #${id} is not available`, mk.error, 'This market')}</div></div>`);
    return;
  }
  const m = mk.value;
  const meta = indexMeta(m.index);
  const floor = toMicro(m.floor);
  const cap = toMicro(m.cap);
  const width = toMicro(m.width) ?? cap - floor;
  const sym = cfg.usdc_symbol || 'tUSDC';

  const [latestR, histR] = await Promise.all([settle(api.latest(m.index)), settle(api.history(m.index, 48))]);
  if (!ctx.isCurrent()) return;
  const latest = latestR.ok ? latestR.value : null;
  const hist = histR.ok ? chronological(histR.value) : null;
  const nowText = latest ? latest.value : m.index_value;
  let change = null;
  if (hist && hist.length >= 2) change = toMicro(hist[hist.length - 1].value) - toMicro(hist[0].value);

  const left = timeLeft(m.expires_at);
  const statusChip = m.status === 'open' ? (left ? `EXPIRES IN ${left.toUpperCase()}` : 'EXPIRING') : m.status === 'expired' ? 'EXPIRED · AWAITING SETTLE' : String(m.status).toUpperCase();

  mount(root, html`
    <div class="main" style="padding-top:28px;gap:22px">
      <div class="page-head">
        <div style="gap:8px">
          <a class="back-link" href="#/markets">← Markets</a>
          <div style="display:flex;flex-wrap:wrap;align-items:center;gap:12px">
            <h1 class="h1" style="font-size:clamp(30px,6vw,40px)">${marketName(m)}</h1>
            <span class="chip chip--cobalt">${meta.kind === 'inf' ? 'INFERENCE' : 'GPU-HOUR'}</span>
            <span class="chip">${statusChip}</span>
          </div>
        </div>
        <div style="display:flex;align-items:baseline;gap:12px">
          <span class="big">${nowText ? rolling(toMicro(nowText), { key: `trade-${m.id}` }) : '—'}</span>
          <span class="mono" style="display:flex;flex-direction:column;gap:2px;font-size:12px">
            <span class="muted">${meta.unit}</span>
            ${change === null ? html`<span class="muted">latest fixing</span>` : html`<span style="font-weight:600;color:${change > 0n ? 'var(--leaf)' : change < 0n ? 'var(--ember-text)' : 'var(--ink-soft)'}">${deltaText(change)} in 48 h</span>`}
          </span>
        </div>
      </div>

      ${m.status !== 'open' ? html`<div class="status status--pending">This market is ${m.status}. Minting and sending are closed. <a href="#/settlement/${m.id}">Open the settlement receipt →</a></div>` : ''}

      <div class="row">
        <div class="col-main">
          <section class="card" aria-label="Index chart" style="padding:18px 20px 12px;display:flex;flex-direction:column;gap:8px">
            <div style="display:flex;flex-wrap:wrap;justify-content:space-between;align-items:center;gap:10px">
              <h2 class="h2 h2--sm">Quill ${indexLabel(m.index)} index, last 48 hours</h2>
              <a href="#/fixing/${m.index}" class="mono" style="display:flex;align-items:center;gap:8px;font-size:12px;text-decoration:none;padding:12px 0">
                <span class="live-dot ${latest ? '' : 'live-dot--still'}"></span>${latest ? html`Fixing #${latest.slot} <span class="leaf-text">${latest.accepted ? 'stored' : 'flagged'}</span> →` : 'No fixing yet →'}
              </a>
            </div>
            ${chart(m, floor, cap, hist, histR.ok ? null : histR.error)}
            <span class="mono muted" style="font-size:11px">${meta.unit} · ${meta.kind === 'inf' ? 'NEAR AI Cloud output price' : 'median of Akash, RunPod, Vast.ai and Shadeform'} · shaded area is this market's band</span>
          </section>

          <section class="card" aria-label="What each side gets" style="overflow:hidden">
            <h2 class="h2 h2--sm" style="padding:16px 20px">What each side gets at expiry, per ${meta.unitWord}</h2>
            <div class="table-wrap">
              <table class="table" style="min-width:560px">
                <thead><tr><th scope="col">IF ${indexLabel(m.index)} SETTLES AT</th><th scope="col" class="r">LONG GETS</th><th scope="col" class="r">SHORT GETS</th><th scope="col" class="r">TOTAL</th></tr></thead>
                <tbody>
                  ${payoffRows(floor, cap, nowText ? toMicro(nowText) : null).map((p) => html`
                    <tr class="${p.today ? 'is-today' : ''}">
                      <th scope="row" style="font-weight:500">$${microText(p.v)}${p.edge === 'floor' ? ' or less' : p.edge === 'cap' ? ' or more' : ''} <span class="muted" style="font-family:var(--font-body);font-size:12px">${p.note}</span></th>
                      <td class="r long-text">${microText(p.long)}</td>
                      <td class="r short-text">${microText(p.short)}</td>
                      <td class="r">${microText(width)}</td>
                    </tr>`)}
                </tbody>
              </table>
            </div>
          </section>

          <section class="card card--flat" aria-label="Market details" style="padding:6px 20px">
            <dl class="dl">
              <div><dt>SETTLES ON</dt><dd>Mean of 24 hourly fixings before ${utcFull(m.expires_at)}</dd></div>
              <div><dt>COLLATERAL</dt><dd>${amt(m.width)} ${sym} per pair</dd></div>
              <div><dt>LOCKED NOW</dt><dd>${amt(m.collateral_locked)} ${sym} · ${amt(m.long_supply, 0)} pairs</dd></div>
              <div><dt>CONTRACT</dt><dd>${cfg.claim_market || '—'}</dd></div>
              <div><dt>MARKET ID</dt><dd>#${m.id} · tokens ${m.id}:L and ${m.id}:S</dd></div>
              <div><dt>LIQUIDATIONS</dt><dd class="leaf-text">None. Fully backed.</dd></div>
            </dl>
          </section>
        </div>

        <aside class="col-side">
          <section class="card" aria-label="Trade ticket" style="overflow:hidden" data-slot="ticket"></section>
          <section class="card card--quiet" aria-label="How to buy one side" style="padding:18px 20px;display:flex;flex-direction:column;gap:8px">
            <h2 class="h2" style="font-size:18px">No order book on testnet</h2>
            <p class="hint" style="color:var(--ink-mid)">LONG and SHORT live inside the contract and cannot be listed on an exchange yet. To buy a side, someone who minted sends it to you. To sell yours, send it to the buyer once they have paid you.</p>
          </section>
        </aside>
      </div>
    </div>`);

  rollNumbers(root);
  mountTicket(root.querySelector('[data-slot="ticket"]'), ctx, m, { width, sym, meta });
}

// ---------- chart ----------

function chart(m, floor, cap, hist, error) {
  const X0 = 48;
  const X1 = 680;
  const values = (hist || []).map((f) => Number(f.value));
  const fl = Number(m.floor);
  const cp = Number(m.cap);
  let lo = Math.min(fl, ...values);
  let hi = Math.max(cp, ...values);
  const pad = (hi - lo) * 0.12 || 0.5;
  lo = Math.max(0, lo - pad);
  hi += pad;
  const Y = (v) => 290 - ((v - lo) / (hi - lo)) * 270;
  const step = niceStep((hi - lo) / 4);
  const ticks = [];
  for (let t = Math.ceil(lo / step) * step; t <= hi; t += step) ticks.push(t);
  const dp = step < 0.01 ? 3 : 2;
  const n = values.length;
  const X = (i) => (n <= 1 ? X1 : X0 + ((X1 - X0) * i) / (n - 1));
  const path = values.map((v, i) => `${i === 0 ? 'M' : 'L'}${X(i).toFixed(1)} ${Y(v).toFixed(1)}`).join(' ');
  const last = n ? values[n - 1] : null;
  const desc = n
    ? `Line chart of the ${indexLabel(m.index)} index over ${n} hourly fixings, from ${values[0]} to ${last}, against the band ${m.floor} to ${m.cap}.`
    : `The band ${m.floor} to ${m.cap}. No fixings in the last 48 hours.`;
  const xl = (i) => utcShort(hist[i].at);
  const svg = `
    <svg class="chart" viewBox="0 0 760 320" role="img" aria-label="${desc}">
      <rect x="${X0}" y="${Y(cp).toFixed(1)}" width="${X1 - X0}" height="${(Y(fl) - Y(cp)).toFixed(1)}" fill="var(--sun-soft)" fill-opacity="0.75"></rect>
      ${ticks.map((t) => `<line x1="${X0}" x2="${X1}" y1="${Y(t).toFixed(1)}" y2="${Y(t).toFixed(1)}" stroke="var(--rule)" stroke-width="1"></line><text x="40" y="${(Y(t) + 4).toFixed(1)}" text-anchor="end" font-size="11" fill="var(--ink-soft)">${t.toFixed(dp)}</text>`).join('')}
      <line x1="${X0}" x2="${X1}" y1="${Y(cp).toFixed(1)}" y2="${Y(cp).toFixed(1)}" stroke="var(--ink)" stroke-width="1.6" stroke-dasharray="6 5"></line>
      <text x="688" y="${(Y(cp) + 4).toFixed(1)}" font-size="11" font-weight="600" fill="var(--ink)">cap ${esc(amt(m.cap))}</text>
      <line x1="${X0}" x2="${X1}" y1="${Y(fl).toFixed(1)}" y2="${Y(fl).toFixed(1)}" stroke="var(--ink)" stroke-width="1.6" stroke-dasharray="6 5"></line>
      <text x="688" y="${(Y(fl) + 4).toFixed(1)}" font-size="11" font-weight="600" fill="var(--ink)">floor ${esc(amt(m.floor))}</text>
      ${n > 1 ? `<path class="chart-line" d="${path}" fill="none" stroke="var(--cobalt)" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"></path>` : ''}
      ${n ? `<circle cx="${X1}" cy="${Y(last).toFixed(1)}" r="6" fill="var(--cobalt)" stroke="var(--ink)" stroke-width="2"></circle><text x="692" y="${(Y(last) - 12).toFixed(1)}" font-size="13" font-weight="600" fill="var(--cobalt-text)">${esc(amt(hist[n - 1].value))}</text>` : ''}
      ${n ? `<text x="${X0}" y="312" font-size="11" fill="var(--ink-soft)">${esc(xl(0))}</text>` : ''}
      ${n > 2 ? `<text x="${(X0 + X1) / 2}" y="312" text-anchor="middle" font-size="11" fill="var(--ink-soft)">${esc(xl(Math.floor((n - 1) / 2)))}</text>` : ''}
      ${n > 1 ? `<text x="${X1}" y="312" text-anchor="end" font-size="11" fill="var(--ink-soft)">${esc(xl(n - 1))}</text>` : ''}
      ${!n ? `<text x="${(X0 + X1) / 2}" y="${((Y(cp) + Y(fl)) / 2).toFixed(1)}" text-anchor="middle" font-size="14" fill="var(--ink-soft)">${error ? 'Fixing history is not available from this server yet' : 'No fixings in the last 48 hours'}</text>` : ''}
    </svg>`;
  return raw(svg);
}

function esc(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
}

function niceStep(raw) {
  const p = Math.pow(10, Math.floor(Math.log10(raw || 1)));
  const f = raw / p;
  return (f < 1.5 ? 1 : f < 3 ? 2 : f < 7 ? 5 : 10) * p;
}

// ---------- ticket ----------

function mountTicket(el, ctx, m, { width, sym, meta }) {
  const { cfg } = ctx;
  let mode = 'mint';
  let size = '10';
  let receiver = '';
  let hold = { long: 0n, short: 0n, loaded: false };
  let balance = null;

  async function loadHoldings() {
    const w = walletState();
    if (!w.accountId) {
      hold = { long: 0n, short: 0n, loaded: false };
      balance = null;
      return;
    }
    const [pf, bal] = await Promise.all([
      settle(api.portfolio(w.accountId)),
      cfg.usdc ? settle(viewCached(cfg.rpc, cfg.usdc, 'ft_balance_of', { account_id: w.accountId })) : Promise.resolve({ ok: false }),
    ]);
    const pos = pf.ok ? pf.value.find((p) => p.market === m.id) : null;
    hold = { long: pos ? toMicro(pos.long) : 0n, short: pos ? toMicro(pos.short) : 0n, loaded: pf.ok };
    balance = bal.ok ? BigInt(bal.value || '0') : null;
  }

  function draw() {
    const w = walletState();
    const open = m.status === 'open';
    const blurb = {
      mint: `Lock ${sym} and receive both tokens. Keep the side you want and send the other, or hold both and get your ${sym} back at expiry.`,
      long: `LONG pays more the higher ${indexLabel(m.index)} settles. Send LONG to someone who wants cover against a rise.`,
      short: `SHORT pays more the lower ${indexLabel(m.index)} settles. Send SHORT to someone who wants cover against a fall.`,
      redeem: `Burn equal LONG and SHORT before settlement and get the full collateral back.`,
    }[mode];
    mount(el, html`
      <div class="ticket-tabs" role="group" aria-label="Choose an action">
        ${MODES.map((x) => html`<button type="button" data-mode="${x.id}" data-tone="${x.id}" aria-pressed="${x.id === mode ? 'true' : 'false'}">${x.label}</button>`)}
      </div>
      <div class="ticket-body">
        <p class="hint" style="font-size:14px">${blurb}</p>
        <div class="field">
          <label for="size" class="label">SIZE · ${mode === 'mint' || mode === 'redeem' ? 'PAIRS' : mode.toUpperCase()} · ${meta.unitWords}</label>
          <div class="field-box">
            <input id="size" type="text" inputmode="numeric" pattern="[0-9]*" autocomplete="off" value="${size}" aria-describedby="size-help">
            <div class="presets">${[1, 10, 100].map((n) => html`<button type="button" class="preset" data-preset="${n}" aria-label="Set size to ${n}">${n}</button>`)}</div>
          </div>
          <span class="hint" id="size-help" style="font-size:12px">Whole units. One unit is one ${meta.unitWord}.</span>
        </div>
        ${mode === 'long' || mode === 'short' ? html`
          <div class="field">
            <label for="recv" class="label">SEND TO · NEAR TESTNET ACCOUNT</label>
            <input id="recv" class="text-input" type="text" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="buyer.testnet" value="${receiver}">
          </div>` : ''}
        <div class="lines double-top" data-slot="lines"></div>
        <button type="button" class="btn btn--lg btn--block ${mode === 'mint' ? 'btn--sun' : mode === 'long' ? 'btn--long' : mode === 'short' ? 'btn--short' : 'btn--ink'}" data-slot="cta"></button>
        <div class="status" data-slot="status" aria-live="polite"></div>
        <span class="hint" style="font-size:12px;text-align:center" data-slot="foot"></span>
      </div>`);
    el.querySelectorAll('[data-mode]').forEach((b) => b.addEventListener('click', () => { mode = b.dataset.mode; draw(); }));
    el.querySelectorAll('[data-preset]').forEach((b) => b.addEventListener('click', () => { size = b.dataset.preset; el.querySelector('#size').value = size; update(); }));
    el.querySelector('#size').addEventListener('input', (e) => { size = e.target.value; update(); });
    el.querySelector('#recv')?.addEventListener('input', (e) => { receiver = e.target.value.trim(); update(); });
    el.querySelector('[data-slot="cta"]').addEventListener('click', act);
    update();

    function update() {
      const n = wholeUnits(size);
      const units = n === null ? null : unitsOf(n);
      const lines = el.querySelector('[data-slot="lines"]');
      const cta = el.querySelector('[data-slot="cta"]');
      const foot = el.querySelector('[data-slot="foot"]');
      const nTxt = n === null ? '—' : n.toString();
      let rows = [];
      let label = '';
      let problem = null;
      const longV = toMicro(m.long_value);
      const shortV = toMicro(m.short_value);
      if (mode === 'mint') {
        const coll = n === null ? null : n * width;
        rows = [
          ['You lock', coll === null ? '—' : `${microText(coll)} ${sym}`, true],
          ['You get', `${nTxt} LONG + ${nTxt} SHORT`, true],
          ['Back at expiry, holding both', coll === null ? '—' : `${microText(coll)} ${sym}`],
          ['Fee', `0.00 on testnet`],
        ];
        label = `Mint ${nTxt} pair${n === 1n ? '' : 's'}`;
        if (balance !== null && coll !== null && coll > balance) problem = `You have ${microText(balance)} ${sym}. Use "Get test USDC" in the nav first.`;
        if (!cfg.usdc) problem = 'The test USDC contract is not configured on this server.';
      } else if (mode === 'long' || mode === 'short') {
        const have = mode === 'long' ? hold.long : hold.short;
        const per = mode === 'long' ? longV : shortV;
        rows = [
          ['You send', `${nTxt} ${mode.toUpperCase()}`, true],
          ['Worth at the latest fixing', units === null || per === null ? '—' : `${microText(times(units, per))} ${sym}`],
          ['Most it can pay at expiry', units === null ? '—' : `${microText(times(units, width))} ${sym}`],
          ['You hold', w.accountId ? (hold.loaded ? `${microText(have, 0)} ${mode.toUpperCase()}` : '—') : 'sign in to see'],
        ];
        label = `Send ${nTxt} ${mode.toUpperCase()}`;
        if (w.accountId && hold.loaded && units !== null && units > have) problem = `You hold ${microText(have, 0)} ${mode.toUpperCase()}.`;
        else if (receiver && !isAccountId(receiver)) problem = 'That is not a valid NEAR account name.';
        else if (receiver && receiver === w.accountId) problem = 'That is your own account.';
      } else {
        const pairs = hold.long < hold.short ? hold.long : hold.short;
        rows = [
          ['You burn', `${nTxt} LONG + ${nTxt} SHORT`, true],
          ['You get back', units === null ? '—' : `${microText(times(units, width))} ${sym}`, true],
          ['Pairs you can redeem', w.accountId ? (hold.loaded ? microText(pairs, 0) : '—') : 'sign in to see'],
        ];
        label = `Redeem ${nTxt} pair${n === 1n ? '' : 's'}`;
        if (w.accountId && hold.loaded && units !== null && units > pairs) problem = `You can redeem at most ${microText(pairs, 0)} pairs.`;
      }
      mount(lines, rows.map(([k, v, strong]) => html`<div><span class="k">${k}</span><span class="v ${strong ? 'strong' : ''}">${v}</span></div>`));
      if (!w.accountId) {
        cta.textContent = w.status === 'loading' ? 'Wallet loading…' : w.status === 'error' ? 'Wallet unavailable' : 'Sign in to continue';
        cta.disabled = w.status !== 'ready';
        cta.dataset.go = 'signin';
      } else {
        cta.textContent = label;
        cta.dataset.go = 'act';
        cta.disabled = !open || n === null || !!problem || ((mode === 'long' || mode === 'short') && !receiver);
      }
      foot.textContent = !open ? `This market is ${m.status}; actions are closed.` : problem || (w.accountId ? `Balance ${balance === null ? '—' : microText(balance)} ${sym}` : 'You need a NEAR testnet wallet.');
    }
  }

  async function act(e) {
    const btn = e.currentTarget;
    if (btn.dataset.go === 'signin') {
      signIn();
      return;
    }
    const n = wholeUnits(size);
    if (n === null) return;
    const units = unitsOf(n);
    const status = el.querySelector('[data-slot="status"]');
    let label;
    let fn;
    if (mode === 'mint') {
      const coll = n * width;
      label = `Mint ${n} pairs`;
      fn = () => call(cfg.usdc, 'ft_transfer_call', { receiver_id: cfg.claim_market, amount: coll.toString(), msg: JSON.stringify({ mint: m.id }) }, ONE_YOCTO);
    } else if (mode === 'long' || mode === 'short') {
      label = `Send ${n} ${mode.toUpperCase()} to ${receiver}`;
      const token = `${m.id}:${mode === 'long' ? 'L' : 'S'}`;
      fn = () => call(cfg.claim_market, 'mt_transfer', { receiver_id: receiver, token_id: token, amount: units.toString(), approval: null, memo: null }, ONE_YOCTO);
    } else {
      label = `Redeem ${n} pairs`;
      fn = () => call(cfg.claim_market, 'redeem_pair', { market_id: m.id, units: units.toString() });
    }
    await runAction(btn, status, label, fn, async () => {
      window.dispatchEvent(new Event('quill:balances'));
      const saved = status.innerHTML;
      const cls = status.className;
      await loadHoldings();
      draw();
      const s2 = el.querySelector('[data-slot="status"]');
      s2.className = cls;
      s2.innerHTML = saved;
    });
  }

  draw();
  loadHoldings().then(() => { if (ctx.isCurrent()) draw(); });
}

