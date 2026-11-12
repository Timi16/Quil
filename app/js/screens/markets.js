// Markets (design/Markets.dc.html).

import { api, settle } from '../api.js';
import { html, mount, mascot, loading, emptyState, failState } from '../ui.js';
import { walletState } from '../wallet.js';
import { amt, microText, toMicro, indexLabel, indexMeta, utcDay, utcHour, timeLeft, nextHour, marketName } from '../format.js';
import { sparkline, chronological, trendWords } from '../data.js';

const FILTERS = [
  { id: 'all', label: 'All' },
  { id: 'gpu', label: 'GPU-hours' },
  { id: 'inf', label: 'Inference' },
];

export default async function markets(ctx) {
  const { root } = ctx;
  let filter = 'all';
  mount(root, html`
    <div class="main">
      <div class="page-head">
        <div>
          <h1 class="h1">Markets</h1>
          <p class="lede">Pick a price to hedge. Every market is fully collateralized and settles on the mean of 24 hourly fixings.</p>
        </div>
        <a class="btn mono" href="#/fixing" data-slot="fixchip" style="font-family:var(--font-mono);font-size:12px;font-weight:500"><span class="live-dot live-dot--sun"></span>Reading the latest fixing…</a>
      </div>
      <section class="card stats" aria-label="Summary" data-slot="stats">${loading()}</section>
      <section class="card" aria-label="All markets" style="overflow:hidden">
        <div class="card-head">
          <h2 class="h2">Markets on testnet</h2>
          <div class="chips" role="group" aria-label="Filter markets" data-slot="filters"></div>
        </div>
        <div data-slot="table">${loading('Loading markets…')}</div>
        <p class="card-note">LONG and SHORT are worth, per unit, what each would get if the market settled at the latest fixing. They always add up to cap minus floor.</p>
      </section>
      <section aria-label="Get started" class="grid-auto" data-slot="start"></section>
    </div>`);
  const $ = (s) => root.querySelector(`[data-slot="${s}"]`);

  const [mk, h100] = await Promise.all([settle(api.markets()), settle(api.latest('H100'))]);
  if (!ctx.isCurrent()) return;

  const fix = h100.ok ? h100.value : null;
  mount($('fixchip'), fix
    ? html`<span class="live-dot"></span>Fixing #${fix.slot} · ${utcHour(fix.at)} <span class="badge ${fix.accepted ? 'badge--leaf' : 'badge--warn'}">${fix.accepted ? 'STORED' : 'FLAGGED'}</span>`
    : html`<span class="live-dot live-dot--still"></span>No fixing yet`);

  if (!mk.ok) {
    mount($('stats'), failState('Markets unavailable', mk.error, 'The market list'));
    mount($('table'), emptyState('No markets to show', 'See above.'));
    renderStart($('start'), []);
    return;
  }
  const list = mk.value;
  const indexes = [...new Set(list.map((m) => m.index))];
  const [latests, histories] = await Promise.all([
    Promise.all(indexes.map((i) => settle(api.latest(i)))),
    Promise.all(indexes.map((i) => settle(api.history(i, 48)))),
  ]);
  if (!ctx.isCurrent()) return;
  const latestBy = {};
  const histBy = {};
  indexes.forEach((i, n) => {
    latestBy[i] = latests[n].ok ? latests[n].value : null;
    histBy[i] = histories[n].ok ? chronological(histories[n].value).map((f) => Number(f.value)) : null;
  });

  // Summary
  const locked = list.reduce((s, m) => s + (toMicro(m.collateral_locked) || 0n), 0n);
  const pairs = list.filter((m) => m.status === 'open').reduce((s, m) => s + (toMicro(m.long_supply) || 0n), 0n);
  const open = list.filter((m) => m.status === 'open').length;
  mount($('stats'), [
    stat('COLLATERAL LOCKED', microText(locked), `${cfgSym(ctx)} across ${list.length} market${list.length === 1 ? '' : 's'}`),
    stat('OPEN PAIRS', microText(pairs, 0), `in ${open} open market${open === 1 ? '' : 's'}, each backed 100%`),
    stat('NEXT FIXING', utcHour(nextHour()), 'hourly, 24h mean at expiry'),
    stat('PRICE SOURCES', fix ? `${fix.sources}` : '—', fix ? 'behind the latest H100 fixing' : 'no H100 fixing yet'),
  ]);

  const drawFilters = () => mount($('filters'), FILTERS.map((f) => html`<button type="button" class="fchip" data-f="${f.id}" aria-pressed="${f.id === filter ? 'true' : 'false'}">${f.label}</button>`));
  const drawTable = () => {
    const rows = list.filter((m) => filter === 'all' || indexMeta(m.index).kind === filter);
    if (!list.length) {
      mount($('table'), emptyState('No markets yet', 'Nobody has created a market on this contract yet.'));
      return;
    }
    if (!rows.length) {
      mount($('table'), emptyState('Nothing in this filter', filter === 'inf' ? 'There is no inference market yet. The INF-OUT index is fixed hourly; see the Fixing page.' : 'No market matches.'));
      return;
    }
    mount($('table'), html`
      <div class="table-wrap">
        <table class="table" style="min-width:1020px">
          <thead><tr>
            <th scope="col">MARKET</th><th scope="col" class="r">INDEX NOW</th><th scope="col">48 HOURS</th><th scope="col">BAND</th><th scope="col">EXPIRES</th>
            <th scope="col" class="r">LONG</th><th scope="col" class="r">SHORT</th><th scope="col" class="r">COLLATERAL</th><th scope="col"><span class="sr-only">Action</span></th>
          </tr></thead>
          <tbody>${rows.map((m, i) => row(m, i, latestBy[m.index], histBy[m.index], ctx))}</tbody>
        </table>
      </div>`);
  };
  drawFilters();
  drawTable();
  $('filters').addEventListener('click', (e) => {
    const b = e.target.closest('[data-f]');
    if (!b) return;
    filter = b.dataset.f;
    drawFilters();
    drawTable();
  });
  renderStart($('start'), list);
}

function cfgSym(ctx) {
  return (ctx.cfg && ctx.cfg.usdc_symbol) || 'tUSDC';
}

function stat(label, value, note) {
  return html`<div class="stat"><span class="label">${label}</span><span class="stat-value">${value}</span><span class="stat-note">${note}</span></div>`;
}

function statusCell(m) {
  const left = timeLeft(m.expires_at);
  if (m.status === 'open') return html`${utcDay(m.expires_at)} <span class="muted">· ${left ? `${left} left` : 'expiring'}</span>`;
  if (m.status === 'expired') return html`${utcDay(m.expires_at)} <span class="badge badge--warn">EXPIRED · NOT SETTLED</span>`;
  if (m.status === 'settled') return html`${utcDay(m.expires_at)} <span class="badge badge--leaf">SETTLED</span>`;
  return html`${utcDay(m.expires_at)} <span class="badge">${String(m.status).toUpperCase()}</span>`;
}

function row(m, i, latest, hist, ctx) {
  const meta = indexMeta(m.index);
  const now = latest ? latest.value : m.index_value;
  const stroke = meta.kind === 'inf' ? 'var(--leaf)' : 'var(--cobalt)';
  const action = m.status === 'open'
    ? html`<a class="btn btn--sun" href="#/trade/${m.id}" aria-label="Trade ${marketName(m)}">Trade</a>`
    : html`<a class="btn" href="#/settlement/${m.id}" aria-label="${m.status === 'expired' ? 'Settle' : 'Receipt for'} ${marketName(m)}">${m.status === 'expired' ? 'Settle' : 'Receipt'}</a>`;
  return html`
    <tr class="qrow row-in" style="animation-delay:${(0.1 + i * 0.09).toFixed(2)}s">
      <td><span class="mkt-cell"><span class="glyph ${meta.glyph}">${indexLabel(m.index).slice(0, 4)}</span><span style="display:flex;flex-direction:column;gap:2px"><span class="sym">${marketName(m).toUpperCase()}</span><span class="desc">${meta.desc} · #${m.id}</span></span></span></td>
      <td class="r nowrap" style="font-weight:600;font-size:15px">${now ? amt(now) : '—'} <span class="muted" style="font-weight:400;font-size:11px">${meta.short}</span></td>
      <td>${sparkline(hist, { stroke, label: trendWords(hist, 48) })}</td>
      <td class="nowrap">${amt(m.floor)} – ${amt(m.cap)}</td>
      <td class="nowrap">${statusCell(m)}</td>
      <td class="r nowrap long-text">${m.long_value ? amt(m.long_value) : '—'}</td>
      <td class="r nowrap short-text">${m.short_value ? amt(m.short_value) : '—'}</td>
      <td class="r nowrap">${amt(m.collateral_locked)} ${cfgSym(ctx)}</td>
      <td class="r">${action}</td>
    </tr>`;
}

async function renderStart(el, list) {
  const w = walletState();
  const first = list.find((m) => m.status === 'open');
  let headline = 'Sign in to mint your first pair.';
  let body = 'Use a NEAR testnet wallet. Get test USDC from the faucet in the nav, then mint a pair. It takes about two minutes.';
  let held = [];
  if (w.accountId) {
    const pf = await settle(api.portfolio(w.accountId));
    held = pf.ok ? pf.value.filter((p) => toMicro(p.long) > 0n || toMicro(p.short) > 0n) : [];
    headline = held.length ? `You hold ${held.length} position${held.length === 1 ? '' : 's'}.` : 'You hold no positions yet.';
    body = held.length ? 'See what they are worth now and what is ready to claim.' : 'Get test USDC from the nav, then mint a pair. It takes about two minutes.';
  }
  const link = held.length
    ? html`<a class="link-strong" href="#/portfolio">Open your portfolio →</a>`
    : first ? html`<a class="link-strong" href="#/trade/${first.id}">Mint your first pair →</a>` : '';
  mount(el, html`
    <div class="card card--quiet" style="padding:22px;display:flex;align-items:center;gap:20px;flex-wrap:wrap">
      <div class="float">${mascot('stand', 120)}</div>
      <div style="display:flex;flex-direction:column;gap:8px;align-items:flex-start;flex:1 1 200px;min-width:0">
        <span class="empty-title" style="font-size:22px">${headline}</span>
        <span class="hint" style="color:var(--ink-mid)">${body}</span>
        ${link}
      </div>
    </div>
    <div class="card card--flat" style="padding:22px;display:flex;flex-direction:column;gap:10px;align-items:flex-start">
      <span class="label">COLLATERAL FROM ANY CHAIN</span>
      <span class="empty-title" style="font-size:22px">USDC on Base, Ethereum or Solana?</span>
      <span class="hint">NEAR Intents can route it to a NEAR account. That leg runs on mainnet only, with real funds.</span>
      <a class="link-strong" href="#/collateral">Get a quote →</a>
    </div>`);
}

