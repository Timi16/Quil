// Boot, hash routing, header and footer.

import { loadConfig, viewCached, forget } from './api.js';
import { initWallet, onWalletChange, walletState, signIn, signOut, call, explorerTx } from './wallet.js';
import { html, raw, mount, logoMark, toast, esc } from './ui.js';
import { microText } from './format.js';

import landing from './screens/landing.js';
import markets from './screens/markets.js';
import trade from './screens/trade.js';
import collateral from './screens/collateral.js';
import fixing from './screens/fixing.js';
import portfolio from './screens/portfolio.js';
import settlement from './screens/settlement.js';
import feed from './screens/feed.js';

const ROUTES = [
  { re: /^#\/?$/, screen: landing, nav: 'landing', title: 'Quill: hedge the price of compute' },
  { re: /^#\/markets\/?$/, screen: markets, nav: 'markets', title: 'Markets · Quill' },
  { re: /^#\/trade\/(\d+)\/?$/, screen: trade, nav: 'markets', title: 'Trade · Quill' },
  { re: /^#\/collateral\/?$/, screen: collateral, nav: 'collateral', title: 'Add collateral · Quill' },
  { re: /^#\/fixing(?:\/([A-Z0-9_]+))?\/?$/, screen: fixing, nav: 'fixing', title: 'Fixing · Quill' },
  { re: /^#\/portfolio(?:\/([^/]+))?\/?$/, screen: portfolio, nav: 'portfolio', title: 'Portfolio · Quill' },
  { re: /^#\/settlement\/(\d+)\/?$/, screen: settlement, nav: 'portfolio', title: 'Settlement · Quill' },
  { re: /^#\/feed\/?$/, screen: feed, nav: 'feed', title: 'Price feed · Quill' },
];

const NAV = [
  { id: 'markets', label: 'Markets', href: '#/markets' },
  { id: 'portfolio', label: 'Portfolio', href: '#/portfolio' },
  { id: 'fixing', label: 'Fixing', href: '#/fixing' },
  { id: 'feed', label: 'Feed', href: '#/feed' },
];

let cfg = null;
let current = { token: 0, cleanups: [], nav: null, key: null };
const headerEl = document.getElementById('site-header');
const mainEl = document.getElementById('main');
const footerEl = document.getElementById('site-footer');

function match(hash) {
  for (const r of ROUTES) {
    const m = r.re.exec(hash);
    if (m) return { route: r, params: m.slice(1).map((p) => (p === undefined ? undefined : decodeURIComponent(p))) };
  }
  return null;
}

async function route() {
  const hash = location.hash || '#/';
  // Plain in-page anchors (#how, #oracle) belong to the landing page.
  if (!hash.startsWith('#/')) {
    if (current.nav !== 'landing') await show({ route: ROUTES[0], params: [] }, '#/');
    const target = document.getElementById(hash.slice(1));
    if (target) target.scrollIntoView({ behavior: 'smooth', block: 'start' });
    return;
  }
  const m = match(hash);
  if (!m) {
    renderHeader('none');
    mount(mainEl, html`<div class="main"><div class="card card-pad"><h1 class="h1">Page not found</h1><p class="lede">There is no screen at ${hash}. <a href="#/markets">Go to the markets</a>.</p></div></div>`);
    return;
  }
  await show(m, hash);
}

async function show(m, key) {
  for (const fn of current.cleanups) {
    try { fn(); } catch { /* ignore */ }
  }
  const token = current.token + 1;
  const sameScreen = current.key === key;
  current = { token, cleanups: [], nav: m.route.nav, key };
  document.title = m.route.title;
  renderHeader(m.route.nav);
  renderFooter();
  const ctx = {
    root: mainEl,
    cfg,
    params: m.params,
    isCurrent: () => current.token === token,
    onCleanup: (fn) => current.cleanups.push(fn),
    rerender: () => route(),
  };
  if (!sameScreen) window.scrollTo(0, 0);
  try {
    await m.route.screen(ctx);
  } catch (e) {
    console.error(e);
    if (ctx.isCurrent()) mount(mainEl, html`<div class="main"><div class="status status--err">Something went wrong drawing this screen: ${e.message || String(e)}</div></div>`);
  }
}

// ---------- header ----------

function walletControls() {
  const w = walletState();
  if (w.status === 'loading') return html`<span class="acct-chip" role="status"><span class="live-dot live-dot--sun"></span>Wallet loading…</span>`;
  if (w.status === 'error') return html`<span class="acct-chip" title="${w.error || ''}"><span class="live-dot live-dot--still" style="background:var(--ember)"></span>Wallet unavailable</span>`;
  if (!w.accountId) return html`<button type="button" class="btn btn--sm" data-act="signin">Sign in</button>`;
  return html`
    <span class="acct-chip"><span class="live-dot live-dot--still"></span><span class="acct-name" title="${w.accountId}">${w.accountId}</span><span data-slot="balance" class="nowrap"></span></span>
    <button type="button" class="btn btn--sm" data-act="faucet">Get test USDC</button>
    <button type="button" class="btn btn--sm" data-act="signout">Sign out</button>`;
}

function renderHeader(active) {
  if (active === 'landing') {
    mount(headerEl, html`
      <nav class="nav nav--landing" aria-label="Main">
        <a class="brand" href="#/">${logoMark(32)}<span class="brand-word">quill</span></a>
        <div class="nav-anchors">
          <a href="#how">How it works</a><a href="#markets">Markets</a><a href="#oracle">Oracle</a><a href="#dev">Developers</a>
        </div>
        <a class="btn btn--sun" href="#/markets" style="font-size:15px">Open app</a>
      </nav>
      <div class="scroll-progress" aria-hidden="true"></div>`);
    return;
  }
  mount(headerEl, html`
    <nav class="nav" aria-label="App">
      <a class="brand" href="#/">${logoMark(30)}<span class="brand-word">quill</span><span class="sr-only">home</span></a>
      <div class="nav-links">
        ${NAV.map((n) => html`<a class="nav-link" href="${n.href}" ${raw(n.id === active ? 'aria-current="page"' : '')}>${n.label}</a>`)}
      </div>
      <div class="nav-account">
        ${walletControls()}
        <a class="btn btn--sun btn--sm" href="#/collateral" ${raw(active === 'collateral' ? 'aria-current="page"' : '')}>Add collateral</a>
      </div>
    </nav>`);
  headerEl.querySelector('[data-act="signin"]')?.addEventListener('click', signIn);
  headerEl.querySelector('[data-act="signout"]')?.addEventListener('click', signOut);
  headerEl.querySelector('[data-act="faucet"]')?.addEventListener('click', (e) => faucet(e.currentTarget));
  fillBalance();
}

async function fillBalance() {
  const w = walletState();
  const slot = headerEl.querySelector('[data-slot="balance"]');
  if (!slot || !w.accountId || !cfg.usdc) return;
  try {
    const bal = await viewCached(cfg.rpc, cfg.usdc, 'ft_balance_of', { account_id: w.accountId });
    slot.textContent = `· ${microText(BigInt(bal || '0'))} ${cfg.usdc_symbol}`;
  } catch {
    slot.textContent = '';
  }
}

async function faucet(button) {
  if (!cfg.usdc) {
    toast('The test USDC contract is not configured on this server.');
    return;
  }
  button.disabled = true;
  const old = button.textContent;
  button.textContent = 'Confirm in wallet…';
  try {
    // The faucet registers the account on the token itself, so no storage_deposit is needed first.
    const { hashes, failure } = await call(cfg.usdc, 'faucet', { amount: '500000000' });
    const link = hashes[0] ? html` <a href="${explorerTx(hashes[0])}" target="_blank" rel="noopener">Transaction ↗</a>` : '';
    if (failure) toast(html`<strong>Faucet failed:</strong> ${failure}${link}`, 12000);
    else toast(html`500 ${cfg.usdc_symbol} sent to your account.${link}`, 12000);
    forget('view:');
    renderHeader(current.nav);
  } catch (e) {
    toast(html`<strong>Faucet failed:</strong> ${e.message || String(e)}`, 12000);
  } finally {
    button.disabled = false;
    button.textContent = old;
  }
}

function renderFooter() {
  mount(footerEl, html`
    <div class="inner">
      <div style="display:flex;align-items:center;gap:10px;flex-wrap:wrap">
        ${logoMark(24)}<span class="brand-word" style="font-size:19px">quill</span>
        <span class="mono muted" style="font-size:12px">Built for the NEAR Hackathon, November 2026</span>
      </div>
      <nav aria-label="Footer">
        <a href="#/markets">Markets</a><a href="#/fixing">Fixing</a><a href="#/feed">Feed</a><a href="#/collateral">Collateral</a>
      </nav>
    </div>`);
}

// ---------- boot ----------

/** Older wallet flows return by redirect with ?transactionHashes= or ?errorCode=; report and tidy the URL. */
function handleReturnParams() {
  const q = new URLSearchParams(location.search);
  if (![...q.keys()].length) return;
  const hashes = (q.get('transactionHashes') || '').split(',').filter(Boolean);
  if (hashes.length) toast(html`Transaction sent. ${hashes.map((h) => html`<a href="${explorerTx(h)}" target="_blank" rel="noopener">${h.slice(0, 8)}… ↗</a> `)}`, 15000);
  if (q.get('errorCode')) toast(html`<strong>Wallet:</strong> ${q.get('errorMessage') || q.get('errorCode')}`, 15000);
  history.replaceState(null, '', location.pathname + location.hash);
}

async function boot() {
  document.getElementById('skip')?.addEventListener('click', (e) => {
    e.preventDefault();
    mainEl.focus();
  });
  cfg = await loadConfig();
  window.addEventListener('hashchange', route);
  window.addEventListener('quill:balances', () => renderHeader(current.nav));
  mainEl.addEventListener('click', (e) => {
    if (!e.target.closest('[data-retry]')) return;
    forget('');
    route();
  });
  let lastAccount = walletState().accountId;
  onWalletChange((w) => {
    renderHeader(current.nav);
    if (w.accountId !== lastAccount) {
      lastAccount = w.accountId;
      forget('portfolio:');
      route();
    }
  });
  await route();
  initWallet(cfg).then(handleReturnParams);
}

boot().catch((e) => {
  console.error(e);
  mainEl.innerHTML = `<div class="main"><div class="status status--err">Quill could not start: ${esc(e.message || String(e))}</div></div>`;
});
