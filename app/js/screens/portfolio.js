// Portfolio (design/Portfolio.dc.html) for the signed-in account or any account typed in.

import { api, settle, viewCached } from '../api.js';
import { html, mount, mascot, loading, emptyState, runAction, failState } from '../ui.js';
import { walletState, signIn, send, STORAGE_DEPOSIT } from '../wallet.js';
import { amt, microText, toMicro, indexLabel, utcDay, timeLeft, isAccountId } from '../format.js';

export default async function portfolio(ctx) {
  const { root, cfg } = ctx;
  const w = walletState();
  const typed = ctx.params[0] ? ctx.params[0].trim() : '';
  const account = typed || w.accountId || '';
  const own = !!account && account === w.accountId;
  const sym = cfg.usdc_symbol || 'tUSDC';

  mount(root, html`
    <div class="main">
      <div class="page-head">
        <div>
          <h1 class="h1">Portfolio</h1>
          <p class="lede">What ${own ? 'you hold' : account ? `${account} holds` : 'an account holds'}, what it's worth now, and what's ready to claim.</p>
        </div>
        <form class="field" data-slot="lookup" style="flex:0 1 380px;min-width:0">
          <label for="acct" class="label">${own ? 'LOOK UP ANOTHER ACCOUNT' : 'NEAR TESTNET ACCOUNT'}</label>
          <div style="display:flex;gap:8px">
            <input id="acct" class="text-input" type="text" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="someone.testnet" value="${typed}">
            <button type="submit" class="btn">Show</button>
          </div>
        </form>
      </div>
      <div data-slot="body">${account ? loading('Loading positions…') : ''}</div>
    </div>`);
  root.querySelector('[data-slot="lookup"]').addEventListener('submit', (e) => {
    e.preventDefault();
    const v = root.querySelector('#acct').value.trim();
    location.hash = v ? `#/portfolio/${encodeURIComponent(v)}` : '#/portfolio';
  });
  const body = root.querySelector('[data-slot="body"]');

  if (!account) {
    mount(body, html`
      <div class="card card--quiet" style="padding:22px;display:flex;align-items:center;gap:20px;flex-wrap:wrap">
        <div class="float">${mascot('stand', 120)}</div>
        <div style="display:flex;flex-direction:column;gap:10px;align-items:flex-start;flex:1 1 240px">
          <span class="empty-title">Sign in to see your positions.</span>
          <span class="hint">Or type any testnet account above to see what it holds.</span>
          <button type="button" class="btn btn--sun" data-act="signin" ${w.status !== 'ready' ? html`disabled` : ''}>${w.status === 'loading' ? 'Wallet loading…' : 'Sign in'}</button>
        </div>
      </div>`);
    body.querySelector('[data-act="signin"]').addEventListener('click', signIn);
    return;
  }
  if (!isAccountId(account)) {
    mount(body, html`<div class="card">${emptyState('That is not a NEAR account name', 'Account names look like alice.testnet.')}</div>`);
    return;
  }

  const [pf, mk, bal, reg] = await Promise.all([
    settle(api.portfolio(account)),
    settle(api.markets()),
    cfg.usdc ? settle(viewCached(cfg.rpc, cfg.usdc, 'ft_balance_of', { account_id: account })) : Promise.resolve({ ok: false }),
    cfg.usdc ? settle(viewCached(cfg.rpc, cfg.usdc, 'storage_balance_of', { account_id: account })) : Promise.resolve({ ok: false }),
  ]);
  if (!ctx.isCurrent()) return;
  if (!pf.ok) {
    mount(body, html`<div class="card">${failState('Positions unavailable', pf.error, 'The portfolio')}</div>`);
    return;
  }
  const marketsById = new Map((mk.ok ? mk.value : []).map((m) => [m.id, m]));
  const positions = pf.value.filter((p) => toMicro(p.long) > 0n || toMicro(p.short) > 0n);
  const registered = reg.ok ? reg.value !== null : true;

  const sum = (f) => positions.reduce((s, p) => s + (f(p) || 0n), 0n);
  const worthOpen = sum((p) => (p.status === 'open' ? toMicro(p.worth_now) : 0n));
  const claimable = positions.filter((p) => p.claimable);
  const claimTotal = claimable.reduce((s, p) => s + (toMicro(p.worth_now) || 0n), 0n);
  const maxOpen = sum((p) => (p.status === 'open' ? toMicro(p.max_payout) : 0n));

  mount(body, html`
    <div style="display:flex;flex-direction:column;gap:24px">
      <section class="card stats" aria-label="Summary">
        ${stat('FREE BALANCE', bal.ok ? microText(BigInt(bal.value || '0')) : '—', sym, `${sym} in ${own ? 'your' : 'this'} wallet, ready to mint with`)}
        ${stat('OPEN POSITIONS', microText(worthOpen), sym, 'worth at the latest fixing')}
        ${stat('READY TO CLAIM', microText(claimTotal), sym, claimable.length ? `from ${claimable.length} settled market${claimable.length === 1 ? '' : 's'}` : 'nothing settled yet')}
        ${stat('MOST YOU CAN STILL RECEIVE', microText(maxOpen), sym, 'if every open position pays in full')}
      </section>

      <section class="card" aria-label="Positions" style="overflow:hidden">
        <h2 class="h2" style="padding:16px 20px">Positions</h2>
        ${positions.length ? html`
          <div class="table-wrap">
            <table class="table" style="min-width:900px">
              <thead><tr><th scope="col">MARKET</th><th scope="col" class="r">LONG</th><th scope="col" class="r">SHORT</th><th scope="col" class="r">WORTH NOW</th><th scope="col" class="r">MOST AT EXPIRY</th><th scope="col">STATUS</th><th scope="col"><span class="sr-only">Action</span></th></tr></thead>
              <tbody>${positions.map((p) => row(p, marketsById.get(p.market), own, sym))}</tbody>
            </table>
          </div>
          <div class="status" data-slot="status" aria-live="polite" style="margin:0 20px 16px"></div>`
        : emptyState(own ? 'You hold no positions yet.' : 'This account holds no positions.', own ? 'Get test USDC from the nav, then mint a pair on a market.' : '', own ? html`<a class="link-strong" href="#/markets">Go to the markets →</a>` : '')}
        <p class="card-note">"Most at expiry" is the most a position can pay. It can never go below zero.${!registered && own ? ' Your account is not registered on the test USDC token yet; claiming will register it first (0.00125 NEAR).' : ''}</p>
      </section>

      <aside class="card card--quiet" style="padding:22px;display:flex;align-items:center;gap:20px;flex-wrap:wrap">
        <div class="float float--slow">${mascot('ball', 92, 'Quill the hedgehog curled into a ball')}</div>
        <div style="display:flex;flex-direction:column;gap:8px;flex:1 1 260px;min-width:0">
          <span class="empty-title" style="font-size:24px">Nothing here can be liquidated.</span>
          <span class="hint" style="color:var(--ink-mid);font-size:14px">You paid in full up front. The worst a position can do is expire at zero. Nobody can close it for you, and there is no margin to top up.</span>
        </div>
      </aside>
    </div>`);

  body.querySelectorAll('[data-claim]').forEach((btn) => {
    btn.addEventListener('click', () => {
      const id = Number(btn.dataset.claim);
      const status = body.querySelector('[data-slot="status"]');
      const txs = [];
      if (!registered) txs.push({ receiverId: cfg.usdc, calls: [{ method: 'storage_deposit', args: { account_id: account, registration_only: true }, deposit: STORAGE_DEPOSIT }] });
      txs.push({ receiverId: cfg.claim_market, calls: [{ method: 'claim', args: { market_id: id } }] });
      runAction(btn, status, `Claim market #${id}`, () => send(txs), (ok) => {
        if (ok) {
          window.dispatchEvent(new Event('quill:balances'));
          setTimeout(() => ctx.isCurrent() && ctx.rerender(), 2500);
        }
      });
    });
  });
}

function stat(label, value, unit, note) {
  return html`<div class="stat"><span class="label">${label}</span><span class="stat-value">${value} <span class="mono muted" style="font-size:12px;font-weight:500">${unit}</span></span><span class="stat-note">${note}</span></div>`;
}

function row(p, m, own, sym) {
  const name = m ? `${indexLabel(m.index)} · ${utcDay(m.expires_at)}` : `${indexLabel(p.index)} · #${p.market}`;
  const sub = m ? (m.index_value ? `${m.status === 'settled' ? 'settled at' : 'index now'} ${amt(m.index_value)}` : '') : '';
  const left = m ? timeLeft(m.expires_at) : null;
  const statusEl = p.status === 'settled'
    ? html`<span class="badge badge--leaf">SETTLED</span>`
    : p.status === 'expired'
      ? html`<span class="badge badge--warn">EXPIRED</span> <span class="muted">awaiting settle</span>`
      : html`<span class="badge">${String(p.status).toUpperCase()}</span> <span class="muted">${left ? `${left} left` : ''}</span>`;
  let action;
  if (p.claimable && own) action = html`<button type="button" class="btn btn--sun" data-claim="${p.market}" aria-label="Claim ${amt(p.worth_now)} ${sym} from ${name}">Claim ${amt(p.worth_now)}</button>`;
  else if (p.status === 'open') action = html`<a class="btn" href="#/trade/${p.market}" aria-label="Trade ${name}">Trade</a>`;
  else action = html`<a class="btn" href="#/settlement/${p.market}" aria-label="Receipt for ${name}">${p.status === 'expired' ? 'Settle' : 'Receipt'}</a>`;
  return html`
    <tr class="qrow">
      <th scope="row" class="nowrap" style="font-size:14px">${name}<br><span class="muted" style="font-family:var(--font-body);font-weight:400;font-size:12px">${sub}</span></th>
      <td class="r">${toMicro(p.long) > 0n ? html`<span class="side side--long">${amt(p.long, 0)} LONG</span>` : html`<span class="muted">0</span>`}</td>
      <td class="r">${toMicro(p.short) > 0n ? html`<span class="side side--short">${amt(p.short, 0)} SHORT</span>` : html`<span class="muted">0</span>`}</td>
      <td class="r nowrap" style="font-weight:600;font-size:15px">${p.worth_now ? amt(p.worth_now) : '—'}</td>
      <td class="r nowrap muted">${amt(p.max_payout)}</td>
      <td class="nowrap">${statusEl}</td>
      <td class="r">${action}</td>
    </tr>`;
}
