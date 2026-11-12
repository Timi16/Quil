// Settlement receipt (design/Settlement.dc.html), with Settle and Claim.

import { api, settle, viewCached } from '../api.js';
import { html, mount, mascot, loading, emptyState, runAction, failState } from '../ui.js';
import { walletState, call, send, STORAGE_DEPOSIT } from '../wallet.js';
import { amt, microText, toMicro, times, indexLabel, indexMeta, utcFull, marketName, timeLeft } from '../format.js';

export default async function settlement(ctx) {
  const { root, cfg } = ctx;
  const id = Number(ctx.params[0]);
  const sym = cfg.usdc_symbol || 'tUSDC';
  mount(root, html`<div class="main">${loading('Loading the receipt…')}</div>`);

  const w = walletState();
  const [mk, pf, reg] = await Promise.all([
    settle(api.market(id)),
    w.accountId ? settle(api.portfolio(w.accountId)) : Promise.resolve({ ok: false }),
    w.accountId && cfg.usdc ? settle(viewCached(cfg.rpc, cfg.usdc, 'storage_balance_of', { account_id: w.accountId })) : Promise.resolve({ ok: false }),
  ]);
  if (!ctx.isCurrent()) return;
  if (!mk.ok) {
    mount(root, html`<div class="main"><a class="back-link" href="#/portfolio">← Portfolio</a><div class="card">${failState(`Market #${id} is not available`, mk.error, 'This market')}</div></div>`);
    return;
  }
  const m = mk.value;
  const meta = indexMeta(m.index);
  const settled = m.status === 'settled' || m.status === 'void';
  const expired = m.status === 'expired';
  const width = toMicro(m.width);
  const longV = toMicro(m.long_value);
  const shortV = toMicro(m.short_value);
  const supply = toMicro(m.long_supply) || 0n;
  const locked = toMicro(m.collateral_locked) || 0n;
  const pos = pf.ok ? pf.value.find((p) => p.market === id) : null;
  const myLong = pos ? toMicro(pos.long) : 0n;
  const myShort = pos ? toMicro(pos.short) : 0n;
  const holds = myLong > 0n || myShort > 0n;
  const mine = pos && pos.worth_now ? toMicro(pos.worth_now) : null;
  const registered = reg.ok ? reg.value !== null : true;

  const longTotal = longV !== null ? times(supply, longV) : null;
  const shortTotal = shortV !== null ? times(supply, shortV) : null;
  const pct = longV !== null && width ? Number((longV * 100n) / width) : null;
  const sumOk = longV !== null && shortV !== null && longV + shortV === width;
  const backed = width !== null && locked >= times(supply, width);

  const title = settled
    ? html`${marketName(m)} settled at <span class="mark">$${amt(m.index_value)}.</span>`
    : expired
      ? html`${marketName(m)} has expired. <span class="mark">Settle it.</span>`
      : html`${marketName(m)} settles in <span class="mark">${timeLeft(m.expires_at) || 'minutes'}.</span>`;
  const stamp = settled
    ? html`<span class="stamp stamp--in" style="transform:rotate(-6deg)">${m.status === 'void' ? 'VOID · HALF EACH' : 'SETTLED · ON-CHAIN'}</span>`
    : expired ? html`<span class="stamp stamp--warn stamp--in" style="transform:rotate(-6deg)">EXPIRED · NOT SETTLED</span>`
      : html`<span class="stamp stamp--ink stamp--in" style="transform:rotate(-6deg)">OPEN · NOT FINAL</span>`;
  const stampLine = settled
    ? (holds && mine !== null ? `*** ${pos.claimable ? 'CLAIMABLE' : 'PAID'} · ${microText(mine)} ${sym} ***` : '*** FINAL ***')
    : expired ? '*** AWAITING SETTLE ***' : `*** SETTLES AFTER ${utcFull(m.expires_at).toUpperCase()} ***`;

  mount(root, html`
    <div class="main main--narrow" style="flex-direction:row;flex-wrap:wrap;gap:48px 64px;align-items:flex-start;padding-top:32px">
      <div class="receipt-col">
        <article class="receipt" aria-label="Settlement receipt">
          <div class="receipt-body">
            <div style="text-align:center;display:flex;flex-direction:column;gap:4px">
              <span style="font-family:var(--font-head);font-weight:800;font-size:22px;letter-spacing:.04em">QUILL</span>
              <span style="font-size:11px;letter-spacing:.1em">SETTLEMENT RECEIPT · MARKET #${m.id}</span>
              <span style="font-size:11px;letter-spacing:.1em">${indexLabel(m.index)} · ${utcFull(m.expires_at).toUpperCase()} · NEAR ${String(cfg.network || 'testnet').toUpperCase()}</span>
            </div>
            <div class="receipt-sec">
              <div><span class="k">BAND</span><span>${amt(m.floor)} – ${amt(m.cap)}</span></div>
              <div><span class="k">WINDOW</span><span>24h before expiry</span></div>
              <div><span class="k">PAIRS</span><span>${microText(supply, 0)}</span></div>
              <div><span class="k">STATUS</span><span>${String(m.status).toUpperCase()}</span></div>
            </div>
            <div class="receipt-sec">
              ${check(0, 'LONG + SHORT = BAND WIDTH', longV === null ? null : sumOk)}
              ${check(1, 'COLLATERAL COVERS EVERY PAIR', backed)}
              ${check(2, 'OUTCOME WRITTEN ONCE', settled ? true : null)}
            </div>
            <div style="border-top:3px double var(--ink);padding-top:12px;display:flex;justify-content:space-between;align-items:baseline;gap:12px;flex-wrap:wrap">
              <span style="font-family:var(--font-head);font-weight:800;font-size:20px">INDEX</span>
              <span style="font-family:var(--font-head);font-weight:800;font-size:30px">${m.index_value ? amt(m.index_value) : '—'} <span class="mono" style="font-size:11px;font-weight:500">${meta.short.toUpperCase()}${settled ? '' : ' · LATEST'}</span></span>
            </div>
            <div style="display:flex;flex-direction:column;gap:7px">
              <div style="display:flex;justify-content:space-between"><span><span class="long-text">LONG</span> per unit</span><span>${longV !== null ? microText(longV) : '—'}</span></div>
              <div style="display:flex;justify-content:space-between"><span><span class="short-text">SHORT</span> per unit</span><span>${shortV !== null ? microText(shortV) : '—'}</span></div>
              ${holds ? html`<div style="display:flex;justify-content:space-between;border-top:1px dashed var(--rule-dash);padding-top:7px;gap:8px"><span>Yours · ${[myLong > 0n ? `${microText(myLong, 0)} LONG` : '', myShort > 0n ? `${microText(myShort, 0)} SHORT` : ''].filter(Boolean).join(' + ')}</span><span style="font-weight:600">${mine !== null ? `${microText(mine)} ${sym}` : '—'}</span></div>` : ''}
            </div>
            <div class="receipt-stamp">${stampLine}</div>
            <span style="text-align:center;font-size:11px;color:var(--ink-soft)">contract ${cfg.claim_market || '—'}</span>
          </div>
          <div class="receipt-edge" aria-hidden="true"></div>
        </article>
      </div>

      <div style="flex:1 1 440px;min-width:0;display:flex;flex-direction:column;gap:22px;align-items:flex-start">
        <div style="display:flex;flex-direction:column;gap:10px;align-items:flex-start">
          <a class="back-link" href="#/portfolio">← Portfolio</a>
          ${stamp}
          <h1 class="h1" style="font-size:clamp(34px,4.6vw,58px)">${title}</h1>
          <p class="lede" style="font-size:17px;line-height:1.55;max-width:520px">${settled
            ? `The contract took the mean of the hourly fixings before expiry, wrote the outcome once, and split every pair's ${amt(m.width)} ${sym} between the two sides.`
            : expired
              ? 'Expiry has passed. Anyone can call settle: the contract reads the mean of the last 24 hourly fixings from the oracle and writes the outcome once.'
              : `At ${utcFull(m.expires_at)} the contract will take the mean of the 24 hourly fixings before it. The split below uses the latest fixing and will change until then.`}</p>
        </div>

        <section class="card card-pad" aria-label="How the collateral is split" style="width:100%;display:flex;flex-direction:column;gap:14px">
          <span class="label">WHOLE MARKET · ${microText(supply, 0)} PAIRS · ${amt(m.collateral_locked)} ${sym} LOCKED${settled ? '' : ' · AT THE LATEST FIXING, NOT FINAL'}</span>
          ${pct !== null ? html`
            <div class="payout-bar" role="img" aria-label="LONG holders ${settled ? 'received' : 'would receive'} ${microText(longTotal)} ${sym}, ${pct} percent. SHORT holders ${settled ? 'received' : 'would receive'} ${microText(shortTotal)} ${sym}, ${100 - pct} percent.">
              <div class="l grow" style="width:${pct}%">${pct >= 14 ? `LONG ${pct}%` : ''}</div>
              <div class="s ${pct > 0 && pct < 100 ? 'edge' : ''}">${pct <= 86 ? `SHORT ${100 - pct}%` : ''}</div>
            </div>
            <div class="lines double-top">
              <div><span class="k long-text">LONG holders</span><span class="v">${microText(longTotal)} ${sym}</span></div>
              <div><span class="k short-text">SHORT holders</span><span class="v">${microText(shortTotal)} ${sym}</span></div>
              <div><span class="k" style="color:var(--ink)">Paid out vs locked</span><span class="v strong">${microText(longTotal + shortTotal)} of ${microText(locked)} ${sym}${longTotal + shortTotal === locked ? html` <span class="leaf-text">· exact</span>` : ''}</span></div>
            </div>` : emptyState('No fixing to split on yet', 'The split appears once the oracle has a value for this index.')}
        </section>

        <div style="width:100%;display:flex;flex-direction:column;gap:12px" data-slot="actions"></div>
      </div>
    </div>`);

  const actions = root.querySelector('[data-slot="actions"]');
  if (expired) {
    mount(actions, html`
      <div style="display:flex;flex-wrap:wrap;align-items:center;gap:16px">
        <button type="button" class="btn btn--sun btn--lg" data-act="settle" style="min-height:60px;font-size:18px">Settle market #${m.id}</button>
        <span class="hint" style="font-size:14px">Anyone can settle. It costs only gas.</span>
      </div>
      <div class="status" data-slot="status" aria-live="polite"></div>`);
    actions.querySelector('[data-act="settle"]').addEventListener('click', (e) => {
      runAction(e.currentTarget, actions.querySelector('[data-slot="status"]'), `Settle market #${m.id}`, () => call(cfg.claim_market, 'settle', { market_id: m.id }), (ok) => ok && setTimeout(() => ctx.isCurrent() && ctx.rerender(), 2500));
    });
  } else if (settled && holds && pos && pos.claimable) {
    mount(actions, html`
      <div style="display:flex;flex-wrap:wrap;align-items:center;gap:16px">
        <button type="button" class="btn btn--sun btn--lg" data-act="claim" style="min-height:60px;font-size:18px">Claim ${mine !== null ? `${microText(mine)} ${sym}` : ''}</button>
        <span class="hint" style="font-size:14px">Goes to ${w.accountId}.${registered ? '' : ' Registers you on the test USDC token first (0.00125 NEAR).'}</span>
      </div>
      <div class="status" data-slot="status" aria-live="polite"></div>`);
    actions.querySelector('[data-act="claim"]').addEventListener('click', (e) => {
      const txs = [];
      if (!registered) txs.push({ receiverId: cfg.usdc, calls: [{ method: 'storage_deposit', args: { account_id: w.accountId, registration_only: true }, deposit: STORAGE_DEPOSIT }] });
      txs.push({ receiverId: cfg.claim_market, calls: [{ method: 'claim', args: { market_id: m.id } }] });
      const btn = e.currentTarget;
      runAction(btn, actions.querySelector('[data-slot="status"]'), 'Claim', () => send(txs), (ok) => {
        if (!ok) return;
        window.dispatchEvent(new Event('quill:balances'));
        mount(actions, html`
          <div class="card card--leaf" style="display:flex;align-items:center;gap:16px;padding:16px 18px;flex-wrap:wrap">
            <div class="float">${mascot('stand', 96)}</div>
            <div style="display:flex;flex-direction:column;gap:4px">
              <span class="empty-title" style="font-size:22px">${mine !== null ? `${microText(mine)} ${sym}` : 'Payout'} claimed.</span>
              <span class="hint" style="color:var(--leaf-ink);font-size:14px">It's in your wallet. <a class="link-strong" href="#/markets">Hedge the next one →</a></span>
            </div>
          </div>`);
      });
    });
  } else if (settled && holds) {
    mount(actions, html`<div class="status status--ok">Nothing left to claim from this market for ${w.accountId}.</div>`);
  } else if (!settled && holds) {
    mount(actions, html`<p class="hint">You hold ${[myLong > 0n ? `${microText(myLong, 0)} LONG` : '', myShort > 0n ? `${microText(myShort, 0)} SHORT` : ''].filter(Boolean).join(' and ')} here. Claim opens once the market settles. <a class="link-strong" href="#/trade/${m.id}">Trade →</a></p>`);
  } else if (!w.accountId) {
    mount(actions, html`<p class="hint">Sign in to see your share and claim it.</p>`);
  }
}

function check(i, label, ok) {
  const word = ok === null ? 'PENDING' : ok ? 'PASS' : 'FAIL';
  const color = ok === null ? 'var(--ink-soft)' : ok ? 'var(--leaf)' : 'var(--ember-text)';
  return html`<div class="receipt-check" style="animation-delay:${(0.5 + i * 0.2).toFixed(1)}s"><span style="font-size:11px;letter-spacing:.08em">${label}</span><span style="color:${color};font-weight:600">${word}</span></div>`;
}

