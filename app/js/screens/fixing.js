// Fixing (design/Fixing.dc.html): latest fixing, live sources, pinned code, history, countdown.

import { api, settle, rpcView } from '../api.js';
import { html, mount, loading, emptyState, errText, shieldIcon, failState } from '../ui.js';
import { amt, microText, toMicro, indexLabel, indexMeta, utcFull, utcShort, secsToNextHour, mmss, shortHash, deltaText, INDEXES } from '../format.js';
import { parseEvidence, problemsFor, liveValue, SOURCE_NAMES } from '../data.js';

export default async function fixing(ctx) {
  const { root, cfg } = ctx;
  const index = INDEXES.includes(ctx.params[0]) ? ctx.params[0] : 'H100';
  const meta = indexMeta(index);
  const isInf = meta.kind === 'inf';

  mount(root, html`
    <div class="main">
      <div class="page-head">
        <div>
          <h1 class="h1">The price, and the <span class="mark">proof.</span></h1>
          <p class="lede">Every hour OutLayer runs our oracle program inside an Intel TDX enclave. It reads public prices, takes a median per provider and then across providers, and the contract stores the answer after checking it.</p>
        </div>
        <div class="seg" role="group" aria-label="Choose index">
          ${INDEXES.map((i) => html`<button type="button" data-index="${i}" aria-pressed="${i === index ? 'true' : 'false'}">${indexLabel(i)}</button>`)}
        </div>
      </div>

      <section aria-label="How the fixing is made" class="flow">
        <div class="flow-step"><span class="label">01 · READ</span><b>${isInf ? 'NEAR AI Cloud' : '4 public sources'}</b><span>${isInf ? 'The public model list: output price of one pinned model, per 1M tokens.' : 'Akash, RunPod, Vast.ai, Shadeform. Live listings, no API keys.'}</span></div>
        <div class="flow-arrow" aria-hidden="true"></div>
        <div class="flow-step flow-step--night"><span class="label">02 · COMPUTE IN A TEE</span><b>Median of medians</b><span>One median per provider, then the median across them. Each provider is one vote.</span></div>
        <div class="flow-arrow" aria-hidden="true"></div>
        <div class="flow-step"><span class="label">03 · PINNED CODE</span><b>Hash checked first</b><span>OutLayer fetches the program and runs it only if its SHA-256 matches the hash the contract pins.</span></div>
        <div class="flow-arrow" aria-hidden="true"></div>
        <div class="flow-step flow-step--leaf"><span class="label">04 · WRITE ON NEAR</span><b>Checked, then stored</b><span>The contract checks the hour, the source count and any sudden jump before a fixing can settle a market.</span></div>
      </section>

      <div class="row" style="gap:24px">
        <section class="card card--deep" aria-label="Latest fixing" style="flex:999 1 560px;min-width:0" data-slot="latest">${loading('Reading the latest fixing…')}</section>
        <aside class="card card--night" aria-label="Pinned oracle code" style="flex:1 1 380px;min-width:0;padding:20px;display:flex;flex-direction:column;gap:14px" data-slot="code">${loading('Reading the oracle contract…')}</aside>
      </div>

      <section class="card card--flat" aria-label="Fixing history" style="overflow:hidden">
        <div class="card-head">
          <h2 class="h2 h2--sm">Recent fixings · ${indexLabel(index)}</h2>
          <a class="link-strong" href="#/feed">Get these by API →</a>
        </div>
        <div data-slot="history">${loading('Loading history…')}</div>
      </section>
    </div>`);
  const $ = (s) => root.querySelector(`[data-slot="${s}"]`);
  root.querySelector('.seg').addEventListener('click', (e) => {
    const b = e.target.closest('[data-index]');
    if (b) location.hash = `#/fixing/${b.dataset.index}`;
  });

  const pLatest = settle(api.latest(index));
  const pHist = settle(api.history(index, 48));
  const pCode = cfg.fixing_oracle ? settle(rpcView(cfg.rpc, cfg.fixing_oracle, 'code_ref', {})) : Promise.resolve({ ok: false, error: new Error('The oracle contract is not configured on this server.') });
  const pLive = settle(api.live());

  const [latest, code] = await Promise.all([pLatest, pCode]);
  if (!ctx.isCurrent()) return;
  drawLatest(latest, null);
  drawCode(code, latest.ok ? latest.value : null);

  const hist = await pHist;
  if (!ctx.isCurrent()) return;
  drawHistory(hist);

  const live = await pLive;
  if (!ctx.isCurrent()) return;
  drawLatest(latest, live);

  // Countdown to the next hourly fixing.
  const tick = () => {
    const el = root.querySelector('[data-out="countdown"]');
    if (el) el.textContent = mmss(secsToNextHour());
  };
  tick();
  const timer = setInterval(tick, 1000);
  ctx.onCleanup(() => clearInterval(timer));

  function drawLatest(latestR, liveR) {
    const f = latestR.ok ? latestR.value : null;
    const head = html`
      <div class="slip-head">
        <span>${f ? `FIXING · #${f.slot} · ${indexLabel(index)}` : `FIXING · ${indexLabel(index)}`}</span>
        <span style="display:flex;align-items:center;gap:6px"><span class="live-dot"></span>NEXT IN <span data-out="countdown">${mmss(secsToNextHour())}</span></span>
      </div>`;
    const top = f
      ? html`
        <div style="padding:18px 20px 6px;display:flex;flex-wrap:wrap;justify-content:space-between;align-items:flex-end;gap:14px">
          <div style="display:flex;flex-direction:column;gap:4px;min-width:0">
            <span class="label">MEDIAN · ${utcFull(f.at)} · ${f.sources} SOURCE${f.sources === 1 ? '' : 'S'}</span>
            <span class="big big--xl">$${amt(f.value)}<span class="big-unit" style="font-size:14px"> / ${meta.unit.replace('USD / ', '')}</span></span>
          </div>
          ${f.accepted ? html`<span class="stamp stamp--in" style="margin-bottom:8px;font-size:14px;padding:7px 14px">RUN IN TEE · STORED</span>` : html`<span class="stamp stamp--warn stamp--in" style="margin-bottom:8px">FLAGGED · NOT USED</span>`}
        </div>`
      : emptyState(latestR.error && latestR.error.status === 404 ? 'No fixing yet' : 'Fixing unavailable right now', errText(latestR.error, `A ${indexLabel(index)} fixing`));

    let table;
    if (!liveR) table = html`<p class="card-note card-note--body">Reading the sources this minute (takes a few seconds)…</p>`;
    else if (!liveR.ok) table = html`<p class="card-note card-note--body">Live sources unavailable: ${errText(liveR.error, 'The live index')}</p>`;
    else {
      const rows = parseEvidence(liveR.value.evidence).filter((e) => e.index === index);
      const probs = problemsFor(liveR.value.problems, index);
      const lv = liveValue(liveR.value, index);
      const total = rows.reduce((s, r) => s + (/^\d+$/.test(r.listings) ? Number(r.listings) : 0), 0);
      table = html`
        <div class="table-wrap">
          <table class="table table--notop" style="min-width:560px">
            <caption class="sr-only">Sources read this minute for ${indexLabel(index)}</caption>
            <thead><tr><th scope="col">SOURCE · THIS MINUTE</th><th scope="col" class="r">LISTINGS READ</th><th scope="col" class="r">PROVIDER MEDIAN</th><th scope="col" class="r">STATUS</th></tr></thead>
            <tbody>
              ${rows.map((r) => html`<tr class="qrow"><th scope="row">${r.name} <span class="muted" style="font-weight:400;font-size:11px">${r.api}</span></th><td class="r">${r.listings || '—'}</td><td class="r" style="font-weight:600">${microText(r.micro)}</td><td class="r"><span class="badge badge--leaf">IN</span></td></tr>`)}
              ${probs.map((p) => { const [src, ...rest] = p.split(':'); return html`<tr><th scope="row">${SOURCE_NAMES[src.trim()] || src}</th><td class="r muted" colspan="2" style="font-family:var(--font-body)">${rest.join(':').trim() || p}</td><td class="r"><span class="badge badge--warn">OUT</span></td></tr>`; })}
              <tr class="total-row"><th scope="row">Median across providers</th><td class="r">${total || '—'}</td><td class="r" style="font-weight:600;font-size:15px">${lv ? amt(lv.value) : '—'}</td><td class="r" style="font-size:11px">${meta.short}</td></tr>
            </tbody>
          </table>
        </div>
        <p class="card-note card-note--body">These rows are read live by the API right now, not taken from the enclave run, so they can differ from the stored fixing. The program can show it read these prices and did this maths; it cannot show a list price is what you would really pay.</p>`;
    }
    mount($('latest'), html`${head}${top}${table}`);
  }

  function drawCode(codeR, f) {
    const c = codeR.ok ? codeR.value : null;
    mount($('code'), html`
      <div style="display:flex;align-items:center;gap:12px">
        <span class="shield">${shieldIcon}</span>
        <div style="display:flex;flex-direction:column;gap:2px">
          <h2 class="h2">${c ? 'Code pinned on-chain' : 'Pinned code unavailable'}</h2>
          <span style="font-size:13px;color:var(--night-text)">${c ? 'Read from the oracle contract just now.' : errText(codeR.error, 'The oracle code reference')}</span>
        </div>
      </div>
      <dl class="night-dl">
        <div><dt>ENCLAVE</dt><dd>OutLayer · Intel TDX</dd></div>
        <div><dt>CODE HASH</dt><dd title="${c ? c.wasm_hash : ''}">${c ? `sha256:${shortHash(c.wasm_hash)}` : '—'}</dd></div>
        <div><dt>PROGRAM</dt><dd>${c ? html`<a href="${c.wasm_url}" target="_blank" rel="noopener">${c.wasm_url.split('/').pop()} ↗</a>` : '—'}</dd></div>
        <div><dt>BUILD TARGET</dt><dd>${c ? c.build_target : '—'}</dd></div>
        <div><dt>INPUTS HASH</dt><dd title="${f ? f.inputs : ''}">${f ? shortHash(f.inputs) : '—'}</dd></div>
        <div><dt>CONTRACT</dt><dd>${cfg.fixing_oracle || '—'}</dd></div>
      </dl>
      <p style="margin:0;font-size:13px;line-height:1.5;color:var(--night-text)">OutLayer refuses to run a program whose hash does not match. The contract trusts OutLayer's callback; it does not verify a TDX quote itself.</p>
      <div style="display:flex;flex-direction:column;gap:6px">
        <span style="font-size:13px;font-weight:700">Don't trust us. Rebuild it.</span>
        <pre class="pre-night">$ make oracle-wasm
${c ? c.wasm_hash : 'sha256 of quill-oracle.wasm'}  <span class="c-ok">must match the hash above</span></pre>
      </div>`);
  }

  function drawHistory(histR) {
    if (!histR.ok) {
      mount($('history'), failState('No history to show', histR.error, 'Fixing history'));
      return;
    }
    const list = histR.value;
    if (!list.length) {
      mount($('history'), emptyState('No fixings in the last 48 hours', 'The keeper requests one every hour.'));
      return;
    }
    mount($('history'), html`
      <div class="table-wrap">
        <table class="table" style="min-width:640px">
          <thead><tr><th scope="col">FIXING</th><th scope="col">TIME · UTC</th><th scope="col" class="r">${indexLabel(index)}</th><th scope="col" class="r">CHANGE</th><th scope="col">SOURCES</th><th scope="col">STATUS</th></tr></thead>
          <tbody>
            ${list.map((f, i) => {
              const prev = list[i + 1];
              const d = prev ? toMicro(f.value) - toMicro(prev.value) : null;
              const col = d === null || d === 0n ? 'var(--ink-soft)' : d > 0n ? 'var(--leaf)' : 'var(--ember-text)';
              return html`<tr class="qrow"><td style="font-weight:600">#${f.slot}</td><td class="muted">${utcShort(f.at)}</td><td class="r" style="font-weight:600">${amt(f.value)}</td><td class="r" style="color:${col}">${d === null ? '—' : deltaText(d)}</td><td>${f.sources}</td><td>${f.accepted ? html`<span class="badge badge--leaf">STORED</span>` : html`<span class="badge badge--warn">FLAGGED</span>`}</td></tr>`;
            })}
          </tbody>
        </table>
      </div>
      <p class="card-note">Flagged fixings are stored but left out of settlement unless the next two hours agree with them.</p>`);
  }
}
