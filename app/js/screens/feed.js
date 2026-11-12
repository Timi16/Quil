// Paid price feed (design/Feed.dc.html): 402 with a price, pay with an ft_transfer, ask again with X-Payment.

import { api, settle } from '../api.js';
import { html, mount, raw, loading, errText, statusBox } from '../ui.js';
import { walletState, signIn, call, explorerTx, ONE_YOCTO } from '../wallet.js';
import { amt, toMicro, indexLabel, INDEXES } from '../format.js';

export default async function feed(ctx) {
  const { root, cfg } = ctx;
  const sym = cfg.usdc_symbol || 'tUSDC';
  let tab = 'curl';
  let index = 'H100';
  let offer = null;

  mount(root, html`
    <div class="main">
      <div class="page-head"><div>
        <h1 class="h1">Buy the price, one <span class="mark">read</span> at a time.</h1>
        <p class="lede">The latest fixing as a paid API. No account and no key. A request without payment gets a 402 with the price; pay in ${sym} on NEAR and ask again with the transaction hash and your account.</p>
      </div></div>

      <section aria-label="How a paid read works" class="grid-auto" style="grid-template-columns:repeat(auto-fit,minmax(min(100%,230px),1fr));gap:18px" data-slot="steps"></section>

      <div class="row" style="gap:24px">
        <section class="card" aria-label="Endpoints and prices" style="flex:1 1 460px;min-width:0;overflow:hidden">
          <h2 class="h2" style="padding:16px 20px">Endpoints</h2>
          <div data-slot="endpoints">${loading('Asking the feed for its price…')}</div>
        </section>
        <section class="card card--night" aria-label="Code sample" style="flex:1 1 520px;min-width:0;overflow:hidden">
          <div class="seg" role="group" aria-label="Choose a language" style="border:0;border-bottom:1px solid var(--night-rule);border-radius:0;background:transparent" data-slot="tabs"></div>
          <div data-slot="code"></div>
        </section>
      </div>

      <section class="card" aria-label="Try a paid read" style="overflow:hidden">
        <div class="card-head">
          <h2 class="h2 h2--sm">Try it with your wallet</h2>
          <div class="seg" role="group" aria-label="Choose index" data-slot="idx"></div>
        </div>
        <div style="padding:0 20px 20px;display:flex;flex-direction:column;gap:14px">
          <p class="hint">This does the real flow: ask, get the 402, sign an <span class="mono">ft_transfer</span> of the price to the feed's account, then ask again with <span class="mono">X-Payment: &lt;tx hash&gt;</span> and <span class="mono">X-Payer: &lt;your account&gt;</span>. Each hash buys one read.</p>
          <div style="display:flex;flex-wrap:wrap;gap:12px;align-items:center">
            <button type="button" class="btn btn--sun btn--lg" data-act="try">Pay and read</button>
            <span class="hint" data-slot="tryhint"></span>
          </div>
          <div class="status" data-slot="status" aria-live="polite"></div>
          <pre class="code card--night" style="border-radius:10px;display:none" data-slot="out" tabindex="0" aria-label="Feed responses"></pre>
        </div>
      </section>
    </div>`);
  const $ = (s) => root.querySelector(`[data-slot="${s}"]`);

  const drawTabs = () => {
    mount($('tabs'), [['curl', 'curl'], ['js', 'JavaScript']].map(([id, label]) => html`<button type="button" data-tab="${id}" aria-pressed="${id === tab ? 'true' : 'false'}" style="${id === tab ? 'background:var(--sun);color:var(--ink)' : 'background:transparent;color:var(--night-text)'};border-left:0;border-right:1px solid var(--night-rule)">${label}</button>`));
    mount($('code'), codeSample(tab, offer, sym));
  };
  const drawIdx = () => mount($('idx'), INDEXES.map((i) => html`<button type="button" data-index="${i}" aria-pressed="${i === index ? 'true' : 'false'}">${indexLabel(i)}</button>`));
  $('tabs').addEventListener('click', (e) => { const b = e.target.closest('[data-tab]'); if (b) { tab = b.dataset.tab; drawTabs(); } });
  $('idx').addEventListener('click', (e) => { const b = e.target.closest('[data-index]'); if (b) { index = b.dataset.index; drawIdx(); drawTry(); } });
  drawTabs();
  drawIdx();
  drawSteps(null);

  const first = await settle(api.feed('H100'));
  if (!ctx.isCurrent()) return;
  if (first.ok && first.value.status === 402 && first.value.body) offer = first.value.body;
  drawSteps(offer);
  drawTabs();
  mount($('endpoints'), html`
    <div class="table-wrap">
      <table class="table" style="min-width:440px">
        <thead><tr><th scope="col">GET</th><th scope="col" class="r">PER READ</th></tr></thead>
        <tbody>
          ${INDEXES.map((i) => html`<tr class="qrow"><th scope="row">/v1/feed/${i}<br><span class="muted" style="font-family:var(--font-body);font-weight:400;font-size:13px">Latest ${indexLabel(i)} fixing plus the pinned code hash</span></th><td class="r nowrap" style="font-weight:600;font-size:15px">${offer ? amt(offer.price) : '—'} <span class="muted" style="font-weight:400;font-size:11px">${sym}</span></td></tr>`)}
          <tr class="qrow"><th scope="row">/v1/fixings/{index}/latest<br><span class="muted" style="font-family:var(--font-body);font-weight:400;font-size:13px">The same fixing, unpaid, from Quill's own read API</span></th><td class="r" style="font-weight:600">free</td></tr>
        </tbody>
      </table>
    </div>
    <p class="card-note card-note--body">${offer ? html`Pay to <span class="mono">${offer.pay_to}</span> in <span class="mono">${offer.asset}</span>. ${offer.how || ''}` : first.ok ? 'The feed answered without asking for payment.' : `The paid feed is not available: ${errText(first.error, 'The paid feed')}`} Reading the latest fixing from the oracle contract itself is always free.</p>`);
  drawTry();

  function drawTry() {
    const w = walletState();
    const btn = root.querySelector('[data-act="try"]');
    const hint = $('tryhint');
    if (!offer) {
      btn.disabled = true;
      btn.textContent = 'Pay and read';
      hint.textContent = 'The feed has not offered a price yet.';
      return;
    }
    if (!w.accountId) {
      btn.disabled = w.status !== 'ready';
      btn.textContent = 'Sign in to try';
      hint.textContent = 'You need a NEAR testnet wallet with some test USDC.';
      return;
    }
    btn.disabled = false;
    btn.textContent = `Pay ${amt(offer.price)} ${sym} and read ${indexLabel(index)}`;
    hint.textContent = `From ${w.accountId}.`;
  }

  root.querySelector('[data-act="try"]').addEventListener('click', async (e) => {
    const btn = e.currentTarget;
    const w = walletState();
    if (!w.accountId) { signIn(); return; }
    const box = statusBox($('status'));
    const out = $('out');
    const log = [];
    const write = (line) => { log.push(line); out.style.display = 'block'; mount(out, raw(log.join('\n'))); };
    btn.disabled = true;
    try {
      write(`<span class="c-dim">$</span> GET /v1/feed/${index}`);
      const ask = await api.feed(index);
      if (ask.status === 200) {
        write(`<span class="c-ok">200 OK</span> (no payment asked)\n${escJson(ask.body)}`);
        box.ok('The feed answered without payment.');
        return;
      }
      const o = ask.body || {};
      write(`<span class="c-sun">402 Payment Required</span>\n${escJson(o)}`);
      const price = o.price_micro && /^\d+$/.test(o.price_micro) ? BigInt(o.price_micro) : toMicro(o.price);
      if (price === null || !o.pay_to || !o.asset) throw new Error('The 402 did not include a price, a token and a payee.');
      box.pending(`Pay ${amt(o.price)} ${sym} to ${o.pay_to}: confirm in your wallet…`);
      const { hashes, failure } = await call(o.asset, 'ft_transfer', { receiver_id: o.pay_to, amount: price.toString(), memo: `quill feed ${index}` }, ONE_YOCTO);
      if (failure) { box.err(failure, hashes); return; }
      const hash = hashes[0];
      if (!hash) throw new Error('The wallet did not return a transaction hash.');
      write(`\n<span class="c-dim"># paid: <a href="${explorerTx(hash)}" target="_blank" rel="noopener" style="color:var(--sun)">${hash}</a></span>\n<span class="c-dim">$</span> GET /v1/feed/${index}  X-Payment: ${hash.slice(0, 10)}…  X-Payer: ${w.accountId}`);
      box.pending('Paid. Asking again with the transaction hash…');
      let res = null;
      for (let i = 0; i < 6; i++) {
        res = await api.feed(index, hash, w.accountId);
        if (res.status === 200) break;
        // Only "not found yet" is worth retrying; "already used" or "pays less" is final.
        if (res.body && res.body.error && !/not found/i.test(res.body.error)) break;
        await new Promise((r) => setTimeout(r, 2000));
      }
      if (res.status === 200) {
        write(`<span class="c-ok">200 OK</span>\n${escJson(res.body)}`);
        const f = res.body && res.body.fixing;
        box.ok(f ? `Paid read: ${indexLabel(index)} = ${amt(f.value)} (fixing #${f.slot}) for ${amt(o.price)} ${sym}.` : `Read ${indexLabel(index)} for ${amt(o.price)} ${sym}.`, [hash]);
      } else {
        write(`<span class="c-sun">402</span>\n${escJson(res.body)}`);
        box.err((res.body && (res.body.error || res.body.how)) || 'The feed did not accept the payment yet.', [hash]);
      }
    } catch (err) {
      box.err(err.message || String(err));
      write(`<span class="c-sun">error</span> ${esc(err.message || String(err))}`);
    } finally {
      btn.disabled = false;
      window.dispatchEvent(new Event('quill:balances'));
    }
  });

  function drawSteps(o) {
    const price = o ? `${amt(o.price)} ${sym}` : 'the price';
    mount($('steps'), html`
      <div class="card card--flat" style="padding:16px;display:flex;flex-direction:column;gap:8px"><span class="step-num" style="background:var(--cobalt-soft)">01 · ASK</span><span class="mono" style="font-size:13px">GET /v1/feed/H100</span><span class="hint">Your agent asks for the latest fixing.</span></div>
      <div class="card card--sun" style="padding:16px;display:flex;flex-direction:column;gap:8px"><span class="step-num" style="background:var(--sun)">02 · 402</span><span class="mono" style="font-size:13px">Payment Required · ${price}</span><span class="hint" style="color:var(--ink-mid)">The reply names the price, the token and who to pay.</span></div>
      <div class="card card--flat" style="padding:16px;display:flex;flex-direction:column;gap:8px"><span class="step-num" style="background:var(--ember-soft)">03 · PAY</span><span class="mono" style="font-size:13px">ft_transfer on NEAR</span><span class="hint">Send at least the price to the payee. The agent pays its own gas.</span></div>
      <div class="card card--leaf" style="padding:16px;display:flex;flex-direction:column;gap:8px"><span class="step-num" style="background:var(--leaf-bright)">04 · 200</span><span class="mono" style="font-size:13px">X-Payment + X-Payer</span><span class="hint" style="color:var(--leaf-ink)">Ask again with the tx hash and the account that signed it. You get the fixing and the pinned code hash. A hash buys one read.</span></div>`);
  }
}

function esc(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
}

function escJson(o) {
  return esc(JSON.stringify(o, null, 2));
}

function codeSample(tab, offer, sym) {
  const origin = location.origin;
  const price = offer ? (offer.price_micro || toMicro(offer.price).toString()) : '<price in micro-USDC>';
  const payTo = offer ? offer.pay_to : '<pay_to>';
  const asset = offer ? offer.asset : '<usdc contract>';
  if (tab === 'curl') {
    return html`<pre class="code"><span class="c-dim"># 1. ask</span>
$ curl -i ${origin}/v1/feed/H100
<span class="c-sun">HTTP/1.1 402 Payment Required</span>
${offer ? JSON.stringify(offer, null, 2) : '{ "price": "…", "asset": "…", "pay_to": "…", "how": "…" }'}

<span class="c-dim"># 2. pay ${offer ? `${amt(offer.price)} ${sym}` : 'the price'} on NEAR testnet</span>
$ near contract call-function as-transaction ${asset} \\
    ft_transfer json-args '{"receiver_id":"${payTo}","amount":"${price}"}' \\
    prepaid-gas '30 Tgas' attached-deposit '1 yoctoNEAR' \\
    sign-as you.testnet network-config testnet sign-with-keychain send

<span class="c-dim"># 3. ask again with the transaction hash</span>
$ curl -H "X-Payment: $TX_HASH" -H "X-Payer: you.testnet" \\
    ${origin}/v1/feed/H100
<span class="c-ok">HTTP/1.1 200 OK</span>
{ "fixing": { "index": "H100", "slot": …, "value": "…", "sources": …, … },
  "oracle": "…", "code_hash": "…", "wasm_url": "…",
  "paid": { "tx": "…", "payer": "you.testnet", "amount": "…" } }</pre>`;
  }
  return html`<pre class="code"><span class="c-dim">// any NEAR signer works; this uses a wallet-selector wallet</span>
const url = "${origin}/v1/feed/H100";
let res = await fetch(url);
if (res.status === 402) {
  const offer = await res.json();   // { price, asset, pay_to, how }
  const out = await wallet.signAndSendTransaction({
    receiverId: offer.asset,
    actions: [actionCreators.functionCall("ft_transfer",
      { receiver_id: offer.pay_to, amount: offer.price_micro },
      30_000_000_000_000n, 1n)],
  });
  res = await fetch(url, { headers: {
    "X-Payment": out.transaction.hash,
    "X-Payer": accountId,
  } });
}
const { fixing, code_hash } = await res.json();
<span class="c-dim">// fixing.value is a decimal string; code_hash is the pinned program</span>
console.log(fixing.value, code_hash);</pre>`;
}
