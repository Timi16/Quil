// Landing page (design/Main.dc.html) with live data.

import { api, settle, rpcView } from '../api.js';
import { html, mount, mascot, arrowIcon } from '../ui.js';
import { microText, amt, toMicro, longPerUnit, indexLabel, utcFull, utcDay, utcHour, nextHour, shortHash, deltaText, timeLeft } from '../format.js';
import { parseEvidence, sparkline, chronological, trendWords } from '../data.js';

export default async function landing(ctx) {
  const { root, cfg } = ctx;
  mount(root, page());
  const $ = (s) => root.querySelector(`[data-slot="${s}"]`);

  // Kick everything off at once; fill each slot when its data lands.
  const pMarkets = settle(api.markets());
  const pH100 = settle(api.latest('H100'));
  const pB200 = settle(api.latest('B200'));
  const pInf = settle(api.latest('INF_OUT'));
  const pHistH = settle(api.history('H100', 48));
  const pHistI = settle(api.history('INF_OUT', 48));
  const pCode = cfg.fixing_oracle ? settle(rpcView(cfg.rpc, cfg.fixing_oracle, 'code_ref', {})) : Promise.resolve({ ok: false });
  const pLive = settle(api.live());
  const pFeed = settle(api.feed('H100'));

  const [markets, h100, b200, inf, histH, histI, code] = await Promise.all([pMarkets, pH100, pB200, pInf, pHistH, pHistI, pCode]);
  if (!ctx.isCurrent()) return;

  const mkts = markets.ok ? markets.value : [];
  const gpuMarket = mkts.find((m) => m.index === 'H100' && m.status === 'open') || mkts.find((m) => m.index === 'H100') || null;
  const infMarket = mkts.find((m) => m.index === 'INF_OUT') || null;
  const fix = h100.ok ? h100.value : null;

  mount($('pill'), fix
    ? html`<span class="live-dot"></span>Latest fixing · H100 $${amt(fix.value)} / GPU-hr`
    : html`<span class="live-dot live-dot--still"></span>${h100.error && h100.error.status === 404 ? 'No H100 fixing on-chain yet' : 'Latest fixing unavailable right now'}`);
  mount($('slip'), slip(fix, null, code.ok ? code.value : null, 'loading'));
  mount($('tape'), tape({ h100: fix, b200: b200.ok ? b200.value : null, inf: inf.ok ? inf.value : null, hist: histH.ok ? histH.value : [], gpuMarket }));
  mount($('how'), howCards(gpuMarket, fix));
  mount($('markets'), marketCards(gpuMarket, infMarket, fix, inf.ok ? inf.value : null, histH.ok ? histH.value : null, histI.ok ? histI.value : null));
  mountSlider($('slider'), gpuMarket, fix);

  const live = await pLive;
  if (!ctx.isCurrent()) return;
  mount($('slip'), slip(fix, live.ok ? live.value : null, code.ok ? code.value : null, live.ok ? 'ok' : 'failed'));

  const feed = await pFeed;
  if (!ctx.isCurrent()) return;
  mount($('dev'), devSample(feed.ok && feed.value.status === 402 ? feed.value.body : null, fix));
}

function page() {
  return html`
  <div class="landing">
    <section class="hero">
      <div class="hero-copy">
        <span class="pill rise" data-slot="pill"><span class="live-dot live-dot--sun"></span>Reading the latest fixing…</span>
        <h1 class="hero-title rise rise-1">Hedge the price of compute before it <span class="hero-spike">spikes.</span></h1>
        <p class="hero-lede rise rise-2">Quill is a market for the price of AI compute on NEAR. Lock in a GPU-hour price until expiry, fully collateralized, settled on hourly fixings computed in OutLayer's TEE.</p>
        <div class="rise rise-3" style="display:flex;flex-wrap:wrap;gap:14px">
          <a class="btn btn--sun btn--lg" href="#/markets">Open the market ${arrowIcon}</a>
          <a class="btn btn--lg" href="#how">See how it works</a>
        </div>
        <span class="mono muted rise rise-4" style="font-size:12px">No margin · No liquidations · Collateral from any chain</span>
      </div>
      <div class="hero-slip-col rise" style="animation-delay:.3s">
        <div class="card hero-slip">
          <div class="hero-hog"><div class="float hog-tap">${mascot('stand', 150)}</div></div>
          <div data-slot="slip"></div>
        </div>
        <div class="mono muted" style="display:flex;align-items:center;gap:10px;font-size:11px;flex-wrap:wrap">
          <span class="chip chip--sun" style="border-radius:4px">Live</span>
          Sources are read this minute. The big figure is the latest fixing stored on-chain.
        </div>
      </div>
    </section>

    <div class="tape" aria-hidden="true" data-slot="tape"></div>

    <section id="how" style="display:flex;flex-direction:column;gap:14px;padding-top:96px">
      <span class="eyebrow">HOW IT WORKS</span>
      <h2 class="sec-title">Mint. Trade. Settle.</h2>
      <p class="sec-lede">One contract, two tokens, no margin. The whole payout is locked in before anyone trades.</p>
      <div data-slot="how"></div>
    </section>

    <section id="markets" style="display:flex;flex-direction:column;gap:14px">
      <span class="eyebrow">MARKETS</span>
      <h2 class="sec-title" style="margin-bottom:22px">Two indexes. One contract.</h2>
      <div data-slot="markets"></div>
    </section>

    <section style="display:flex;flex-wrap:wrap;align-items:center;gap:40px 56px">
      <div style="flex:1 1 420px;min-width:0;display:flex;flex-direction:column;gap:18px;align-items:flex-start">
        <div class="float float--slow hog-tap">${mascot('ball', 104, 'Quill the hedgehog curled into a ball')}</div>
        <h2 class="sec-title">Fully collateralized. Nothing to <span class="mark">liquidate.</span></h2>
        <p class="hero-lede" style="font-size:18px">Every pair is backed 100% the moment it's minted. There is no margin, so there is no margin call, no liquidation bot, and no margin bug to drain. LONG and SHORT always add up to the collateral.</p>
      </div>
      <div class="card reveal" style="flex:1 1 420px;min-width:0;padding:22px;display:flex;flex-direction:column;gap:16px" data-slot="slider"></div>
    </section>

    <section id="oracle" style="display:flex;flex-direction:column;gap:14px">
      <span class="eyebrow">WHY NEAR</span>
      <h2 class="sec-title">A price you can <span class="mark">check.</span></h2>
      <p class="sec-lede">Compute markets exist. Quill adds three things they were missing, each built on NEAR.</p>
      <div class="card reveal table-wrap">
        <table class="edge-table">
          <thead><tr><th scope="col" style="width:18%"><span class="sr-only">Aspect</span></th><th scope="col" style="width:34%">OTHER COMPUTE MARKETS</th><th scope="col" class="us">QUILL</th></tr></thead>
          <tbody>
            <tr><th scope="row">Oracle</th><td>A workflow run in simulation. You take the price on trust.</td><td class="us"><strong>Run in a TEE, code pinned on-chain.</strong> OutLayer runs our oracle program only if its SHA-256 matches the hash the contract pins.</td></tr>
            <tr><th scope="row">Underlying</th><td>One GPU rental index scraped from web pages.</td><td class="us"><strong>GPU-hours, plus inference.</strong> A median across four marketplaces, and NEAR AI Cloud's own per-token price list.</td></tr>
            <tr><th scope="row">Collateral</th><td>One chain. Bridge first, then trade.</td><td class="us"><strong>Any chain.</strong> Quote USDC from Ethereum, Base or Solana through NEAR Intents.</td></tr>
          </tbody>
        </table>
      </div>
    </section>

    <section id="dev" style="display:flex;flex-wrap:wrap;align-items:center;gap:40px 56px">
      <div style="flex:1 1 380px;min-width:0;display:flex;flex-direction:column;gap:18px;align-items:flex-start">
        <span class="eyebrow">FOR DEVELOPERS AND AGENTS</span>
        <h2 class="sec-title" style="font-size:clamp(32px,4.6vw,54px)">Buy the price, one read at a time.</h2>
        <p class="hero-lede" style="font-size:18px">The same fixing is a paid feed. No account, no API key. Ask, get a 402 with the price, pay in test USDC on NEAR, ask again with the transaction hash and your account.</p>
        <div style="display:flex;flex-wrap:wrap;gap:14px">
          <a class="btn btn--sun" href="#/feed" style="min-height:48px;font-size:15px">See the feed</a>
          <a class="btn" href="#/fixing" style="min-height:48px;font-size:15px">How the fixing is made</a>
        </div>
      </div>
      <div class="card card--night term reveal" style="flex:1 1 460px" data-slot="dev">${devSample(null, null)}</div>
    </section>

    <section style="display:flex;flex-direction:column;gap:14px">
      <span class="eyebrow">HONEST LIMITS</span>
      <h2 class="sec-title" style="font-size:clamp(32px,4.6vw,54px);margin-bottom:22px">What this is, and isn't.</h2>
      <div class="grid-auto" style="grid-template-columns:repeat(auto-fit,minmax(min(100%,250px),1fr));gap:18px">
        <div class="limit reveal"><b>Testnet only</b><span>Contracts run on NEAR testnet with test USDC. Only the cross-chain collateral quote touches mainnet.</span></div>
        <div class="limit reveal"><b>No order book yet</b><span>LONG and SHORT live inside the contract. You mint pairs and send a side to someone; there is no exchange listing.</span></div>
        <div class="limit reveal"><b>List prices are sticky</b><span>The index is a median of public list prices. What you actually pay for a GPU can differ.</span></div>
        <div class="limit reveal"><b>No quote checked on-chain</b><span>The contract trusts OutLayer to run the pinned program. It does not verify a TDX quote itself.</span></div>
      </div>
    </section>

    <section style="padding-top:72px;padding-bottom:56px">
      <div class="cta-band reveal">
        <div style="display:flex;flex-direction:column;gap:12px;flex:1 1 420px;min-width:0">
          <h2 class="sec-title">Prices spike. <span style="color:var(--sun)">Curl up.</span></h2>
          <p style="margin:0;font-size:18px;color:var(--night-text);max-width:480px">Mint your first pair on testnet in about two minutes. No real money.</p>
        </div>
        <div style="display:flex;align-items:center;gap:22px;flex-wrap:wrap">
          <div class="float hog-tap">${mascot('stand', 130)}</div>
          <a class="btn btn--lg" href="#/markets" style="font-size:17px">Open the market ${arrowIcon}</a>
        </div>
      </div>
    </section>
  </div>`;
}

function slip(fix, live, code, liveState) {
  const rows = live ? parseEvidence(live.evidence).filter((e) => e.index === 'H100') : [];
  let srcRows;
  if (liveState === 'loading') srcRows = html`<div class="src-row"><span class="muted">Reading Akash, RunPod, Vast.ai and Shadeform…</span></div>`;
  else if (!rows.length) srcRows = html`<div class="src-row"><span class="muted">Sources unavailable this minute.</span></div>`;
  else srcRows = rows.map((r, i) => html`<div class="src-row src-row--in" style="animation-delay:${(0.4 + i * 0.35).toFixed(2)}s"><span>${r.name}</span><span style="display:flex;gap:12px;align-items:center"><span class="badge badge--leaf" style="font-size:10px">IN</span><span class="num" style="font-weight:500">${microText(r.micro)}</span></span></div>`);
  const stamp = !fix ? html`<span class="stamp stamp--ink" style="margin-bottom:6px">WAITING</span>`
    : fix.accepted ? html`<span class="stamp" style="margin-bottom:6px;animation:qStampLoop 9s ease-out .4s infinite both">RUN IN TEE · STORED</span>`
      : html`<span class="stamp stamp--warn" style="margin-bottom:6px">FLAGGED</span>`;
  return html`
    <div class="slip-head"><span>${fix ? `FIXING · #${fix.slot}` : 'FIXING · NONE YET'}</span><span class="slip-tag">H100 · GPU-HOUR</span></div>
    <div class="src-rows">
      <div style="display:flex;justify-content:space-between;padding-bottom:8px;font-size:11px;letter-spacing:.08em;color:var(--ink-soft);gap:8px"><span>SOURCE · PROVIDER MEDIAN NOW</span><span>USD / GPU-HR</span></div>
      ${srcRows}
    </div>
    <div class="slip-total">
      <div style="display:flex;flex-direction:column;gap:4px;min-width:0">
        <span class="label">LATEST HOURLY FIXING${fix ? ` · ${fix.sources} SOURCES` : ''}</span>
        <span class="big" style="animation:qMedLoop 9s ease-out .4s infinite both">${fix ? `$${amt(fix.value)}` : '—'}</span>
      </div>
      ${stamp}
    </div>
    <div class="slip-foot">
      <span>code ${code ? shortHash(code.wasm_hash, 4, 4) : '—'}</span>
      <span>inputs ${fix ? shortHash(fix.inputs, 4, 4) : '—'}</span>
      <span>${fix ? utcFull(fix.at) : 'no fixing yet'}</span>
    </div>`;
}

function tape({ h100, b200, inf, hist, gpuMarket }) {
  const items = [];
  const prevH = chronological(hist).slice(-2);
  const dH = prevH.length === 2 ? toMicro(prevH[1].value) - toMicro(prevH[0].value) : null;
  const cls = (d) => (d === null ? 'tape-flat' : d > 0n ? 'tape-up' : d < 0n ? 'tape-down' : 'tape-flat');
  if (h100) items.push({ sym: 'H100', val: amt(h100.value), delta: dH === null ? 'last hour' : deltaText(dH), cls: cls(dH) });
  if (b200) items.push({ sym: 'B200', val: amt(b200.value), delta: `${b200.sources} sources`, cls: 'tape-flat' });
  if (inf) items.push({ sym: 'INF-OUT', val: `${amt(inf.value)} /1M`, delta: 'NEAR AI Cloud', cls: 'tape-flat' });
  if (gpuMarket && gpuMarket.long_value) items.push({ sym: `${indexLabel(gpuMarket.index)} LONG`, val: amt(gpuMarket.long_value), delta: 'per unit', cls: 'tape-flat' });
  if (gpuMarket && gpuMarket.short_value) items.push({ sym: `${indexLabel(gpuMarket.index)} SHORT`, val: amt(gpuMarket.short_value), delta: 'per unit', cls: 'tape-flat' });
  items.push({ sym: 'NEXT FIXING', val: utcHour(nextHour()), delta: 'hourly', cls: 'tape-flat' });
  const one = items.map((t) => html`<span class="tape-item"><span class="tape-sym">${t.sym}</span><span>${t.val}</span><span class="${t.cls}">${t.delta}</span><span class="tape-sep">◆</span></span>`);
  return html`<div class="tape-track">${one}${one}${one}${one}</div>`;
}

function howCards(m, fix) {
  const n = 100n;
  let deposit = null;
  let settleRow = html`<div class="box-line" style="background:var(--leaf-soft)"><span>Index settles</span><span style="font-weight:600">24h mean</span></div>`;
  let split = html`<div class="box-line"><span><span class="long-text">LONG</span> + <span class="short-text">SHORT</span></span><span>= what was locked</span></div>`;
  if (m) {
    const floor = toMicro(m.floor);
    const cap = toMicro(m.cap);
    const width = cap - floor;
    deposit = n * width;
    if (fix) {
      const idx = toMicro(fix.value);
      const l = n * longPerUnit(idx, floor, cap);
      settleRow = html`<div class="box-line" style="background:var(--leaf-soft)"><span>If it settled at today's</span><span style="font-weight:600">$${amt(fix.value)}</span></div>`;
      split = html`<div class="box-line"><span><span class="long-text">LONG</span> ${microText(l)}</span><span><span class="short-text">SHORT</span> ${microText(deposit - l)}</span></div>`;
    }
  }
  const unitWord = 'GPU-hour';
  return html`
    <div class="grid-auto" style="grid-template-columns:repeat(auto-fit,minmax(min(100%,280px),1fr))">
      <article class="card how-card reveal">
        <div style="display:flex;align-items:center;gap:12px"><span class="step-num" style="background:var(--cobalt-soft)">01</span><h3>Mint</h3></div>
        <p>Deposit USDC equal to the most the market can ever pay. You get one LONG and one SHORT for every ${unitWord}.</p>
        <div style="margin-top:auto;display:flex;flex-direction:column;gap:8px">
          <div class="box-line" style="background:var(--paper-2)"><span>Deposit${m ? ` · band ${amt(m.floor)}–${amt(m.cap)}` : ''}</span><span style="font-weight:600">${deposit !== null ? `${microText(deposit)} USDC` : '(cap − floor) × size'}</span></div>
          <div style="display:flex;gap:8px">
            <span class="box-line side--long" style="flex:1;justify-content:center;font-weight:600">100 LONG</span>
            <span class="box-line side--short" style="flex:1;justify-content:center;font-weight:600">100 SHORT</span>
          </div>
        </div>
      </article>
      <article class="card how-card reveal">
        <div style="display:flex;align-items:center;gap:12px"><span class="step-num" style="background:var(--sun-soft)">02</span><h3>Trade</h3></div>
        <p>Keep the side you want and pass on the other. Renting GPUs and fear a rise? Keep LONG. Renting them out and fear a fall? Keep SHORT.</p>
        <div style="margin-top:auto;display:flex;flex-direction:column;gap:8px">
          <div class="box-line"><span class="long-text">LONG</span><span>pays if H100 rises</span></div>
          <div class="box-line"><span class="short-text">SHORT</span><span>pays if H100 falls</span></div>
        </div>
      </article>
      <article class="card how-card reveal">
        <div style="display:flex;align-items:center;gap:12px"><span class="step-num" style="background:var(--leaf-soft)">03</span><h3>Settle</h3></div>
        <p>At expiry the contract takes the mean of the last 24 hourly fixings and splits the collateral between the two sides. That's it.</p>
        <div style="margin-top:auto;display:flex;flex-direction:column;gap:8px">${settleRow}${split}</div>
      </article>
    </div>`;
}

function marketCards(gm, im, fix, inf, histH, histI) {
  const valsH = histH ? chronological(histH).map((f) => Number(f.value)) : [];
  const valsI = histI ? chronological(histI).map((f) => Number(f.value)) : [];
  const figs = (m) => html`
    <div class="figs">
      <div><span class="label">BAND</span><span>${m ? `${amt(m.floor)} – ${amt(m.cap)}` : '—'}</span></div>
      <div><span class="label">LONG</span><span class="long-text">${m && m.long_value ? amt(m.long_value) : '—'}</span></div>
      <div><span class="label">SHORT</span><span class="short-text">${m && m.short_value ? amt(m.short_value) : '—'}</span></div>
    </div>`;
  return html`
    <div class="grid-auto" style="grid-template-columns:repeat(auto-fit,minmax(min(100%,320px),1fr))">
      <article class="card mkt-card reveal">
        <div class="top">
          <div style="display:flex;justify-content:space-between;align-items:center;gap:10px;flex-wrap:wrap"><span class="mono" style="font-size:12px;font-weight:600;letter-spacing:.08em">${gm ? `H100 · ${utcDay(gm.expires_at).toUpperCase()}` : 'H100'}</span><span class="chip chip--sun">MAIN MARKET</span></div>
          <h3>GPU-hour price</h3>
          <p class="hint" style="font-size:15px">What an hour on an NVIDIA H100 rents for, as a median across public marketplaces.</p>
        </div>
        <div style="padding:14px 22px 0;display:flex;flex-wrap:wrap;align-items:flex-end;justify-content:space-between;gap:16px">
          <span class="big" style="font-size:44px">${fix ? `$${amt(fix.value)}` : '—'}<span class="big-unit"> / GPU-hr</span></span>
          ${sparkline(valsH, { w: 200, h: 44, stroke: 'var(--cobalt)', label: `H100 index: ${trendWords(valsH, 48)}` })}
        </div>
        ${figs(gm)}
        <a class="btn btn--ink cta" href="${gm ? `#/trade/${gm.id}` : '#/markets'}" style="min-height:48px;font-size:15px">${gm ? 'Trade H100' : 'See markets'}</a>
      </article>
      <article class="card mkt-card reveal">
        <div class="top">
          <div style="display:flex;justify-content:space-between;align-items:center;gap:10px;flex-wrap:wrap"><span class="mono" style="font-size:12px;font-weight:600;letter-spacing:.08em">${im ? `INF-OUT · ${utcDay(im.expires_at).toUpperCase()}` : 'INF-OUT · NO MARKET YET'}</span><span class="chip chip--leaf">ONLY ON NEAR</span></div>
          <h3>Inference price</h3>
          <p class="hint" style="font-size:15px">What NEAR AI Cloud charges per million output tokens, read from its public price list. It moves in steps, so use it to protect a budget.</p>
        </div>
        <div style="padding:14px 22px 0;display:flex;flex-wrap:wrap;align-items:flex-end;justify-content:space-between;gap:16px">
          <span class="big" style="font-size:44px">${inf ? `$${amt(inf.value)}` : '—'}<span class="big-unit"> / 1M tokens</span></span>
          ${sparkline(valsI, { w: 200, h: 44, stroke: 'var(--leaf)', label: `Inference price: ${trendWords(valsI, 48)}` })}
        </div>
        ${figs(im)}
        <a class="btn cta" href="${im ? `#/trade/${im.id}` : '#/fixing/INF_OUT'}" style="min-height:48px;font-size:15px">${im ? 'Trade INF-OUT' : 'See the inference fixing'}</a>
      </article>
    </div>
    <span class="mono muted" style="font-size:11px">Live testnet values. ${gm ? `H100 market ${timeLeft(gm.expires_at) ? `expires in ${timeLeft(gm.expires_at)}` : 'has expired'}.` : ''}</span>`;
}

function mountSlider(el, m, fix) {
  if (!m) {
    mount(el, html`<span class="label">PAYOUT</span><p class="hint">No market is open yet, so there is no band to draw. Once one is created you can drag its price here.</p>`);
    return;
  }
  const floor = toMicro(m.floor);
  const cap = toMicro(m.cap);
  const width = cap - floor;
  const size = 100n;
  const total = size * width;
  const cent = 10000n;
  const pad = width / 7n;
  const lo = ((floor - pad > 0n ? floor - pad : 0n) / cent);
  const hi = ((cap + pad) / cent);
  const start = fix ? toMicro(fix.value) / cent : (floor + width / 2n) / cent;
  mount(el, html`
    <div style="display:flex;justify-content:space-between;align-items:baseline;gap:12px;flex-wrap:wrap">
      <label for="idx" class="label" style="font-size:12px">IF ${indexLabel(m.index)} SETTLES AT</label>
      <span class="big" style="font-size:40px" data-out="price"></span>
    </div>
    <input id="idx" class="range" type="range" min="${lo.toString()}" max="${hi.toString()}" step="1" value="${start.toString()}" aria-describedby="idx-help">
    <div class="mono muted" style="display:flex;justify-content:space-between;font-size:11px"><span>floor ${amt(m.floor)}</span><span>cap ${amt(m.cap)}</span></div>
    <div class="payout-bar" role="img" data-out="bar"><div class="l" data-out="lbar"></div><div class="s" data-out="sbar"></div></div>
    <div class="figs3">
      <div><span class="label">LONG GETS</span><span class="long-text" data-out="long"></span></div>
      <div><span class="label">SHORT GETS</span><span class="short-text" data-out="short"></span></div>
      <div><span class="label">TOTAL, ALWAYS</span><span style="font-weight:600">${microText(total)} USDC</span></div>
    </div>
    <span class="hint" id="idx-help">Drag the price. ${size.toString()} ${m.index === 'INF_OUT' ? 'units' : 'GPU-hours'} on ${indexLabel(m.index)} · ${utcDay(m.expires_at)}, band ${amt(m.floor)} to ${amt(m.cap)}.</span>`);
  const input = el.querySelector('#idx');
  const out = (k) => el.querySelector(`[data-out="${k}"]`);
  const update = () => {
    const price = BigInt(input.value) * cent;
    const l = size * longPerUnit(price, floor, cap);
    const s = total - l;
    const pct = Number((l * 100n) / (total || 1n));
    out('price').textContent = `$${microText(price)}`;
    out('long').textContent = `${microText(l)} USDC`;
    out('short').textContent = `${microText(s)} USDC`;
    out('lbar').style.width = `${pct}%`;
    out('lbar').textContent = pct >= 14 ? `LONG ${pct}%` : '';
    out('sbar').textContent = pct <= 86 ? `SHORT ${100 - pct}%` : '';
    out('sbar').classList.toggle('edge', pct > 0 && pct < 100);
    out('bar').setAttribute('aria-label', `LONG gets ${microText(l)} USDC and SHORT gets ${microText(s)} USDC`);
    input.setAttribute('aria-valuetext', `$${microText(price)}`);
  };
  input.addEventListener('input', update);
  update();
}

function devSample(pay, fix) {
  const origin = location.origin;
  const price = pay && pay.price ? `${amt(pay.price)} ${'tUSDC'}` : 'the price';
  return html`
    <div class="term-bar"><span class="term-dot" style="background:var(--ember)"></span><span class="term-dot" style="background:var(--sun)"></span><span class="term-dot" style="background:var(--leaf-bright)"></span><span style="margin-left:8px">agent.sh</span></div>
    <pre class="code"><span class="c-dim">$</span> curl ${origin}/v1/feed/H100
<span class="c-sun">402 Payment Required</span> · ${price} to ${pay && pay.pay_to ? pay.pay_to : 'the feed payee'}

<span class="c-dim"># pay with an ft_transfer on NEAR, then ask again</span>
<span class="c-dim">$</span> curl -H "X-Payment: $TX_HASH" -H "X-Payer: you.testnet" \\
    ${origin}/v1/feed/H100
<span class="c-ok">200 OK</span>
${fix ? `{
  "fixing": {
    "index": "H100",
    "slot": ${fix.slot},
    "value": "${fix.value}",
    "sources": ${fix.sources}
  },
  "code_hash": "…"
}` : '{ "fixing": { "index": "H100", "value": "…" }, "code_hash": "…" }'}</pre>`;
}

