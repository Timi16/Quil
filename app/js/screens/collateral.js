// Collateral from any chain through NEAR Intents 1Click (design/Collateral.dc.html).
// Mainnet only: real funds. A dry quote previews; a real quote returns a one-time deposit address.

import { api, settle } from '../api.js';
import { html, mount, loading, emptyState, errText, warnIcon, copyIcon, copyText, statusBox, failState } from '../ui.js';
import { walletState } from '../wallet.js';
import { toMicro, microText, utcFull, isAccountId } from '../format.js';

const CHAINS = { eth: 'Ethereum', base: 'Base', arb: 'Arbitrum', sol: 'Solana', near: 'NEAR', btc: 'Bitcoin', op: 'Optimism', pol: 'Polygon', bsc: 'BNB Chain', avax: 'Avalanche', gnosis: 'Gnosis', ton: 'TON', tron: 'Tron', sui: 'Sui', xrp: 'XRP Ledger', doge: 'Dogecoin', zec: 'Zcash', bera: 'Berachain', aptos: 'Aptos', stellar: 'Stellar', cardano: 'Cardano' };
const FEATURED_SYMBOLS = ['USDC', 'USDT', 'ETH', 'SOL', 'wNEAR', 'NEAR', 'BTC'];
const FEATURED_CHAINS = ['base', 'eth', 'arb', 'sol', 'near', 'btc'];
const TERMINAL = ['SUCCESS', 'REFUNDED', 'FAILED', 'INCOMPLETE_DEPOSIT'];

const chainName = (c) => CHAINS[c] || c;

function featured(tokens) {
  const pick = [];
  for (const sym of FEATURED_SYMBOLS) {
    for (const ch of FEATURED_CHAINS) {
      const t = tokens.find((x) => x.symbol === sym && x.blockchain === ch);
      if (t && !pick.includes(t)) pick.push(t);
    }
  }
  return pick.slice(0, 8);
}

export default async function collateral(ctx) {
  const { root } = ctx;
  const w = walletState();
  const testnetAcct = w.accountId && /\.testnet$/.test(w.accountId);
  const state = {
    tokens: [],
    query: '',
    asset: null,
    amount: '100',
    recipient: w.accountId && isMainnetAccount(w.accountId) ? w.accountId : '',
    refund: '',
    dry: null,
    real: null,
    status: null,
    pollTimer: null,
    deadlineTimer: null,
    quoteSeq: 0,
  };
  ctx.onCleanup(() => { clearTimeout(state.pollTimer); clearInterval(state.deadlineTimer); clearTimeout(state.debounce); });

  mount(root, html`
    <div class="main main--narrow">
      <div class="page-head"><div>
        <h1 class="h1">Add collateral from <span class="mark">any chain</span></h1>
        <p class="lede">Pay with what you already hold. NEAR Intents swaps it and delivers USDC to a NEAR account. You never bridge by hand.</p>
      </div></div>

      <div class="warn-box" role="note">${warnIcon}<span><strong>This step uses real funds.</strong> NEAR Intents runs on mainnet only, so USDC arrives in a <em>mainnet</em> NEAR account. Quill's markets run on testnet with test USDC from the faucet (in the nav when signed in). Keep any real deposit small.</span></div>

      <div class="row" style="gap:26px">
        <div class="col-main" style="flex-basis:520px">
          <section class="card card-pad" aria-label="Amount" style="display:flex;flex-direction:column;gap:10px">
            <label for="amt" class="label">01 · YOU WANT TO RECEIVE</label>
            <div class="field-box field-box--lg">
              <input id="amt" type="text" inputmode="decimal" autocomplete="off" value="${state.amount}" aria-describedby="amt-help">
              <span class="field-suffix">USDC on NEAR</span>
            </div>
            <span class="hint" id="amt-help">Exact output: the quote tells you what to send so that this much USDC arrives. Up to 6 decimal places.</span>
          </section>

          <fieldset class="card" style="margin:0;padding:0;overflow:hidden">
            <legend class="label" style="float:left;width:100%;padding:16px 20px 12px">02 · PAY WITH</legend>
            <div style="clear:both;padding:0 20px 12px">
              <label for="tok-q" class="sr-only">Search tokens</label>
              <input id="tok-q" class="text-input" type="search" placeholder="Search: USDC, ETH, Solana…" autocomplete="off">
            </div>
            <div class="token-list" data-slot="tokens">${loading('Loading tokens NEAR Intents can route…')}</div>
          </fieldset>

          <section class="card card-pad" aria-label="Accounts" style="display:flex;flex-direction:column;gap:14px">
            <div class="field">
              <label for="recv" class="label">03 · CREDIT THIS MAINNET NEAR ACCOUNT</label>
              <input id="recv" class="text-input" type="text" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="you.near" value="${state.recipient}">
              <span class="hint">${testnetAcct ? `You are signed in as ${w.accountId}, a testnet account. Intents cannot pay a testnet account, so enter your mainnet one.` : w.accountId ? 'Filled from your signed-in account. Change it if you want USDC elsewhere.' : 'Sign in, or type the mainnet account that should receive the USDC.'}</span>
            </div>
            <div class="field">
              <label for="refund" class="label">REFUND ADDRESS ON THE CHAIN YOU PAY FROM</label>
              <input id="refund" class="text-input" type="text" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="0x… or your address on that chain">
              <span class="hint" data-out="refundhint">Your own address on the chain you pay from. If the swap cannot complete, 1Click sends your deposit back here. 1Click checks it even for a preview.</span>
            </div>
          </section>
        </div>

        <aside class="col-side" style="flex-basis:400px;gap:14px">
          <section class="card card--deep" aria-label="Deposit slip" data-slot="slip"></section>
          <div class="status" data-slot="qstatus" aria-live="polite"></div>
          <span class="mono muted" style="font-size:11px;text-align:center">Routed by NEAR Intents · 1Click</span>
        </aside>
      </div>
    </div>`);

  const $ = (s) => root.querySelector(`[data-slot="${s}"]`);
  const qbox = statusBox($('qstatus'));

  root.querySelector('#amt').addEventListener('input', (e) => { state.amount = e.target.value; resetReal(); scheduleDry(); });
  root.querySelector('#tok-q').addEventListener('input', (e) => { state.query = e.target.value.trim().toLowerCase(); drawTokens(); });
  root.querySelector('#recv').addEventListener('input', (e) => { state.recipient = e.target.value.trim(); resetReal(); scheduleDry(); });
  root.querySelector('#refund').addEventListener('input', (e) => { state.refund = e.target.value.trim(); resetReal(); scheduleDry(); });
  $('tokens').addEventListener('change', (e) => {
    if (e.target.name !== 'token') return;
    state.asset = e.target.value;
    setRefundPlaceholder();
    resetReal();
    scheduleDry(0);
  });

  drawSlip();
  const tk = await settle(api.tokens());
  if (!ctx.isCurrent()) return;
  if (!tk.ok) {
    mount($('tokens'), failState('Token list unavailable', tk.error, 'The NEAR Intents token list'));
    return;
  }
  state.tokens = (tk.value || []).filter((t) => t && t.asset_id && t.symbol);
  const first = featured(state.tokens)[0];
  if (first) state.asset = first.asset_id;
  drawTokens();
  setRefundPlaceholder();
  scheduleDry(0);

  function setRefundPlaceholder() {
    const t = selected();
    const ch = t ? t.blockchain : '';
    const ph = ['eth', 'base', 'arb', 'op', 'pol', 'bsc', 'avax', 'gnosis', 'bera', 'scroll', 'monad', 'abs', 'plasma', 'xlayer'].includes(ch) ? '0x…' : ch === 'near' ? 'you.near' : ch === 'sol' ? 'Your Solana address' : ch === 'btc' ? 'Your Bitcoin address' : 'Your address on that chain';
    root.querySelector('#refund').placeholder = ph;
    root.querySelector('label[for="refund"]').textContent = t ? `REFUND ADDRESS ON ${chainName(ch).toUpperCase()}` : 'REFUND ADDRESS ON THE CHAIN YOU PAY FROM';
  }

  function selected() {
    return state.tokens.find((t) => t.asset_id === state.asset) || null;
  }

  function drawTokens() {
    const q = state.query;
    let list = q
      ? state.tokens.filter((t) => t.symbol.toLowerCase().includes(q) || chainName(t.blockchain).toLowerCase().includes(q) || String(t.blockchain).toLowerCase().includes(q)).slice(0, 30)
      : featured(state.tokens);
    const sel = selected();
    if (sel && !list.includes(sel)) list = [sel, ...list];
    if (!list.length) {
      mount($('tokens'), emptyState('No token matches', `Nothing called "${state.query}". Try a symbol like USDC or a chain like Base.`));
      return;
    }
    mount($('tokens'), html`<div role="radiogroup" aria-label="Token to pay with">${list.map((t) => html`
      <label class="token-row">
        <input type="radio" name="token" value="${t.asset_id}" ${t.asset_id === state.asset ? html`checked` : ''}>
        <span class="token-chip ${/ETH/.test(t.symbol) ? 'token-chip--eth' : /USD/.test(t.symbol) ? '' : 'token-chip--other'}" aria-hidden="true">${t.symbol.slice(0, 5)}</span>
        <span style="flex:1;display:flex;flex-direction:column;gap:2px;min-width:0">
          <span style="font-weight:700;font-size:15px">${t.symbol} <span class="muted" style="font-weight:500">on ${chainName(t.blockchain)}</span></span>
          <span class="mono muted" style="font-size:12px">${t.price !== null && t.price !== undefined ? `about $${String(t.price)} each` : 'no price listed'}</span>
        </span>
        <span class="radio-dot" aria-hidden="true"></span>
      </label>`)}</div>
      ${!q ? html`<p class="card-note">Showing common tokens. Search to see all ${state.tokens.length}.</p>` : ''}`);
  }

  function amountMicro() {
    const v = toMicro(state.amount);
    return v !== null && v > 0n ? v : null;
  }

  function inputsProblem() {
    if (!selected()) return 'Pick a token to pay with.';
    if (amountMicro() === null) return 'Enter how much USDC you want, like 25 or 10.5.';
    if (!state.recipient) return 'Enter the mainnet NEAR account to credit.';
    if (!state.refund) return `Enter your refund address on ${chainName(selected().blockchain)}. 1Click needs it even for a preview.`;
    if (/\.testnet$/.test(state.recipient)) return 'That is a testnet account. 1Click pays mainnet accounts only (*.near or a 64-character implicit account).';
    if (!isMainnetAccount(state.recipient)) return 'Enter a mainnet NEAR account: a name ending in .near, or a 64-character implicit account.';
    return null;
  }

  function scheduleDry(ms = 600) {
    clearTimeout(state.debounce);
    state.debounce = setTimeout(runDry, ms);
  }

  async function runDry() {
    const problem = inputsProblem();
    state.dry = null;
    if (problem) {
      qbox.clear();
      drawSlip(problem);
      return;
    }
    const seq = ++state.quoteSeq;
    drawSlip(null, true);
    const r = await settle(api.quote({ origin_asset: state.asset, amount_out: amountMicro().toString(), recipient: state.recipient, refund_to: state.refund, dry: true }));
    if (!ctx.isCurrent() || seq !== state.quoteSeq) return;
    if (!r.ok) {
      drawSlip(errText(r.error, 'Quotes'));
      return;
    }
    state.dry = r.value;
    drawSlip();
  }

  function resetReal() {
    if (!state.real) return;
    state.real = null;
    state.status = null;
    clearTimeout(state.pollTimer);
    clearInterval(state.deadlineTimer);
  }

  async function runReal(btn) {
    const problem = inputsProblem();
    if (problem) { qbox.err(problem); return; }
    btn.disabled = true;
    qbox.pending('Asking 1Click for a real quote and a one-time deposit address…');
    const r = await settle(api.quote({ origin_asset: state.asset, amount_out: amountMicro().toString(), recipient: state.recipient, refund_to: state.refund, dry: false }));
    btn.disabled = false;
    if (!ctx.isCurrent()) return;
    if (!r.ok) { qbox.err(errText(r.error, 'Real quotes')); return; }
    if (!r.value.deposit_address) { qbox.err('The quote came back without a deposit address.'); return; }
    qbox.ok('Quote ready. Send the exact amount to the address below before the deadline.');
    state.real = r.value;
    state.status = { status: 'PENDING_DEPOSIT', tx_hashes: [] };
    drawSlip();
    poll();
    clearInterval(state.deadlineTimer);
    state.deadlineTimer = setInterval(() => {
      const el = root.querySelector('[data-out="deadline"]');
      if (el) el.textContent = deadlineText(state.real && state.real.deadline);
    }, 1000);
  }

  async function poll() {
    if (!state.real || !ctx.isCurrent()) return;
    const r = await settle(api.depositStatus(state.real.deposit_address));
    if (!ctx.isCurrent() || !state.real) return;
    if (r.ok) {
      state.status = r.value;
      drawProgress();
    } else {
      const el = root.querySelector('[data-out="pollerr"]');
      if (el) el.textContent = `Status check failed: ${errText(r.error, 'Deposit status')} Retrying.`;
    }
    if (!r.ok || !TERMINAL.includes(state.status.status)) state.pollTimer = setTimeout(poll, 5000);
  }

  function drawSlip(problem = null, busy = false) {
    const t = selected();
    const q = state.real || state.dry;
    const real = !!state.real;
    let body;
    if (problem) body = html`<div class="empty"><span class="hint">${problem}</span></div>`;
    else if (busy) body = html`<div class="empty" role="status"><span class="hint"><span class="spinner" style="display:inline-block;vertical-align:middle;margin-right:8px" aria-hidden="true"></span>Getting a dry quote from 1Click. This can take up to 40 seconds…</span></div>`;
    else if (!q) body = html`<div class="empty"><span class="hint">Pick a token and an amount to see what you would send.</span></div>`;
    else {
      body = html`
        <div style="padding:18px 20px;display:flex;flex-direction:column;gap:14px">
          <div style="display:flex;flex-direction:column;gap:4px">
            <span class="label">${real ? 'SEND EXACTLY' : 'YOU WOULD SEND ABOUT'}</span>
            <span class="big big--md" style="overflow-wrap:anywhere">${q.amount_in_formatted || q.amount_in} ${t ? t.symbol : ''}</span>
            <span class="mono muted" style="font-size:12px">on ${t ? chainName(t.blockchain) : '—'} · exact-output quote${real ? '' : ' · preview only'}</span>
          </div>
          ${real ? html`
            <div class="addr-box">
              <span style="flex:1;min-width:0;display:flex;flex-direction:column;gap:2px">
                <span class="label" style="font-size:10px">ONE-TIME DEPOSIT ADDRESS</span>
                <span class="mono break" style="font-size:12px;line-height:1.4" data-out="addr">${q.deposit_address}</span>
              </span>
              <button type="button" class="icon-btn" data-act="copy" aria-label="Copy deposit address">${copyIcon}</button>
            </div>` : ''}
          <div class="lines double-top">
            <div><span class="k">You receive</span><span class="v strong">${q.amount_out_formatted || microText(BigInt(q.amount_out || '0'))} USDC</span></div>
            <div><span class="k">Credited to</span><span class="v">${state.recipient} (mainnet)</span></div>
            <div><span class="k">Takes about</span><span class="v">${q.time_estimate_secs !== null && q.time_estimate_secs !== undefined ? `${q.time_estimate_secs} s` : '—'}</span></div>
            <div><span class="k">If it fails, refund to</span><span class="v">${state.refund || '—'}</span></div>
            ${q.deadline ? html`<div><span class="k">Quote ${real ? 'expires' : 'deadline'}</span><span class="v">${utcFull(Math.floor(Date.parse(q.deadline) / 1000))}</span></div>` : ''}
          </div>
        </div>
        ${real ? html`<ol class="progress" aria-label="Progress" data-slot="progress"></ol><p class="card-note" data-out="pollerr" aria-live="polite"></p>` : ''}`;
    }
    const head = real
      ? html`<span>DEPOSIT SLIP · LIVE</span><span class="slip-tag"><span class="live-dot live-dot--sun"></span><span data-out="deadline">${deadlineText(q && q.deadline)}</span></span>`
      : html`<span>DEPOSIT SLIP · PREVIEW</span><span class="slip-tag">DRY QUOTE · NO ADDRESS</span>`;
    mount($('slip'), html`
      <div class="slip-head">${head}</div>
      ${body}
      ${!real ? html`
        <div style="padding:14px 20px 18px;border-top:2px solid var(--ink);background:var(--paper-2);border-radius:0 0 12px 12px;display:flex;flex-direction:column;gap:10px">
          <label class="check"><input type="checkbox" data-act="ack"> <span>I understand a real quote is on <strong>mainnet</strong> and anything I send to its address is real money.</span></label>
          <button type="button" class="btn btn--sun btn--lg btn--block" data-act="real" disabled>Get a real deposit address</button>
        </div>` : html`
        <div style="padding:14px 20px 18px;display:flex;flex-direction:column;gap:10px">
          <button type="button" class="btn btn--block" data-act="new">Start a new quote</button>
        </div>`}`);
    const ack = $('slip').querySelector('[data-act="ack"]');
    const realBtn = $('slip').querySelector('[data-act="real"]');
    if (ack && realBtn) {
      ack.addEventListener('change', () => { realBtn.disabled = !ack.checked; });
      realBtn.addEventListener('click', () => runReal(realBtn));
    }
    $('slip').querySelector('[data-act="copy"]')?.addEventListener('click', (e) => copyText(state.real.deposit_address, e.currentTarget));
    $('slip').querySelector('[data-act="new"]')?.addEventListener('click', () => { resetReal(); qbox.clear(); drawSlip(); scheduleDry(0); });
    if (real) drawProgress();
  }

  function drawProgress() {
    const el = $('slip').querySelector('[data-slot="progress"]');
    if (!el || !state.status) return;
    const s = state.status.status;
    const steps = ['Waiting for your deposit', 'Swapped by NEAR Intents', 'USDC delivered to the NEAR account'];
    let at = 0;
    if (s === 'PROCESSING') at = 1;
    if (s === 'SUCCESS') at = 3;
    const bad = ['REFUNDED', 'FAILED', 'INCOMPLETE_DEPOSIT'].includes(s);
    const badText = { REFUNDED: 'Refunded to your refund address', FAILED: 'The swap failed', INCOMPLETE_DEPOSIT: 'Deposit was less than the quote; send the rest or wait for a refund' }[s];
    mount(el, html`
      ${steps.map((label, i) => {
        const cls = bad && i === at ? 'bad' : i < at ? 'done' : i === at ? 'now' : '';
        const mark = cls === 'done' ? '✓' : cls === 'bad' ? '!' : '';
        return html`<li class="${cls}"><span class="step-ring" aria-hidden="true">${mark}</span>${cls === 'bad' ? badText : label}<span class="sr-only">${cls === 'done' ? ' (done)' : cls === 'now' ? ' (in progress)' : cls === 'bad' ? '' : ' (not yet)'}</span></li>`;
      })}
      <li class="mono" style="font-size:11px;color:var(--ink-soft)">Status ${s}${(state.status.tx_hashes || []).length ? ` · tx ${state.status.tx_hashes.join(', ')}` : ''}</li>`);
  }
}

function isMainnetAccount(a) {
  return /^[0-9a-f]{64}$/.test(a) || (isAccountId(a) && /\.near$/.test(a));
}

function deadlineText(deadline) {
  if (!deadline) return 'QUOTE LIVE';
  const left = Math.floor((Date.parse(deadline) - Date.now()) / 1000);
  if (!Number.isFinite(left)) return 'QUOTE LIVE';
  if (left <= 0) return 'QUOTE EXPIRED';
  const h = Math.floor(left / 3600);
  const m = Math.floor((left % 3600) / 60);
  const s = left % 60;
  return `QUOTE LIVE · ${h ? `${h}:` : ''}${String(m).padStart(h ? 2 : 1, '0')}:${String(s).padStart(2, '0')}`;
}
