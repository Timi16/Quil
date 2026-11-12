// Same-origin read API (see app/INTERFACE.md) plus a few NEAR RPC views.

export class ApiError extends Error {
  constructor(status, message, body) {
    super(message);
    this.status = status;
    this.body = body;
  }
}

async function request(path, opts = {}) {
  let res;
  try {
    res = await fetch(path, opts);
  } catch (e) {
    throw new ApiError(0, 'Could not reach the Quill API. Is it running?');
  }
  let body = null;
  try {
    body = await res.json();
  } catch {
    body = null;
  }
  if (!res.ok) {
    const msg = (body && body.error) || (res.status === 404 ? 'Not found' : `HTTP ${res.status}`);
    throw new ApiError(res.status, msg, body);
  }
  return body;
}

// Small cache so moving between screens does not refetch everything (the live index takes ~10 s).
const cache = new Map();
function cached(key, ttlMs, fn) {
  const hit = cache.get(key);
  const now = Date.now();
  if (hit && now - hit.at < ttlMs) return hit.promise;
  const promise = fn();
  cache.set(key, { at: now, promise });
  promise.catch(() => cache.delete(key));
  return promise;
}

export function forget(prefix = '') {
  for (const k of cache.keys()) if (k.startsWith(prefix)) cache.delete(k);
}

const enc = encodeURIComponent;

export const api = {
  health: () => request('/health'),
  live: () => cached('live', 60_000, () => request('/v1/index/live')),
  markets: () => cached('markets', 15_000, () => request('/v1/markets')),
  market: (id) => cached(`market:${id}`, 15_000, () => request(`/v1/markets/${enc(id)}`)),
  latest: (index) => cached(`latest:${index}`, 30_000, () => request(`/v1/fixings/${enc(index)}/latest`)),
  history: (index, hours = 48) => cached(`history:${index}:${hours}`, 60_000, () => request(`/v1/fixings/${enc(index)}/history?hours=${hours}`)),
  portfolio: (account) => cached(`portfolio:${account}`, 10_000, () => request(`/v1/portfolio/${enc(account)}`)),
  tokens: () => cached('tokens', 300_000, () => request('/v1/collateral/tokens')),
  quote: (body) => request('/v1/collateral/quote', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) }),
  depositStatus: (address) => request(`/v1/collateral/status/${enc(address)}`),
  /** Paid feed. Pass the payment tx hash and its signer to pay. Returns { status, body } for 200 and 402; throws otherwise. */
  async feed(index, txHash, payer) {
    const headers = txHash ? { 'X-Payment': txHash, 'X-Payer': payer || '' } : {};
    let res;
    try {
      res = await fetch(`/v1/feed/${enc(index)}`, { headers });
    } catch {
      throw new ApiError(0, 'Could not reach the Quill API.');
    }
    let body = null;
    try { body = await res.json(); } catch { body = null; }
    if (res.status === 200 || res.status === 402) return { status: res.status, body };
    throw new ApiError(res.status, (body && body.error) || `HTTP ${res.status}`, body);
  },
};

/** Settle a promise into { ok, value } or { ok: false, error } so one failed call never blanks a screen. */
export async function settle(promise) {
  try {
    return { ok: true, value: await promise };
  } catch (error) {
    return { ok: false, error };
  }
}

// ---------- config ----------

let configPromise = null;

/**
 * /v1/config, or a fallback assembled from /health and the claim-market `config` view
 * while that route does not exist yet. Never hard-codes account names.
 */
export function loadConfig() {
  if (!configPromise) configPromise = buildConfig();
  return configPromise;
}

async function buildConfig() {
  const base = {
    network: 'testnet',
    rpc: 'https://test.rpc.fastnear.com', // only used if /v1/config is missing
    claim_market: null,
    fixing_oracle: null,
    usdc: null,
    usdc_symbol: 'tUSDC',
    feed_payee: null,
    explorer: 'https://testnet.nearblocks.io',
    fromFallback: false,
  };
  try {
    const c = await request('/v1/config');
    const out = { ...base };
    for (const [k, v] of Object.entries(c || {})) if (v !== null && v !== undefined) out[k] = v;
    return out;
  } catch {
    // fall through
  }
  const out = { ...base, fromFallback: true };
  try {
    const h = await request('/health');
    if (h && h.chain) {
      out.claim_market = h.chain.claim_market || null;
      out.fixing_oracle = h.chain.fixing_oracle || null;
    }
  } catch {
    return out;
  }
  if (out.claim_market) {
    try {
      const pair = await rpcView(out.rpc, out.claim_market, 'config', {});
      if (Array.isArray(pair)) {
        out.usdc = pair[0] || null;
        out.fixing_oracle = out.fixing_oracle || pair[1] || null;
      }
    } catch {
      // leave usdc null; screens explain what is missing
    }
  }
  return out;
}

// ---------- NEAR RPC ----------

function b64(json) {
  const bytes = new TextEncoder().encode(JSON.stringify(json));
  let s = '';
  bytes.forEach((b) => { s += String.fromCharCode(b); });
  return btoa(s);
}

async function rpc(url, method, params) {
  let res;
  try {
    res = await fetch(url, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 'quill', method, params }),
    });
  } catch {
    throw new ApiError(0, 'Could not reach the NEAR RPC.');
  }
  const body = await res.json().catch(() => null);
  if (!body) throw new ApiError(res.status, 'Bad answer from the NEAR RPC.');
  if (body.error) {
    const cause = body.error.cause && body.error.cause.name;
    throw new ApiError(res.status, (body.error.data && String(body.error.data)) || cause || body.error.message || 'RPC error', body.error);
  }
  return body.result;
}

/** Call a view method; the result bytes are JSON. */
export async function rpcView(url, contract, method, args = {}) {
  const r = await rpc(url, 'query', {
    request_type: 'call_function',
    finality: 'final',
    account_id: contract,
    method_name: method,
    args_base64: b64(args),
  });
  if (r && r.error) throw new ApiError(400, String(r.error));
  const text = new TextDecoder().decode(new Uint8Array(r.result));
  return text ? JSON.parse(text) : null;
}

export function viewCached(url, contract, method, args, ttlMs = 15_000) {
  return cached(`view:${contract}:${method}:${JSON.stringify(args)}`, ttlMs, () => rpcView(url, contract, method, args));
}

/** Transaction outcome by hash, waiting until it has executed. */
export function txStatus(url, hash, signer) {
  return rpc(url, 'tx', { tx_hash: hash, sender_account_id: signer, wait_until: 'EXECUTED_OPTIMISTIC' });
}

/** First failure message anywhere in an outcome (the contract's own panic text), or null. */
export function outcomeFailure(outcome) {
  if (!outcome) return null;
  const statuses = [];
  if (outcome.status) statuses.push(outcome.status);
  if (outcome.transaction_outcome) statuses.push(outcome.transaction_outcome.outcome.status);
  for (const r of outcome.receipts_outcome || []) statuses.push(r.outcome.status);
  for (const s of statuses) {
    if (s && typeof s === 'object' && s.Failure) return failureText(s.Failure);
  }
  return null;
}

function failureText(f) {
  const exec = f && f.ActionError && f.ActionError.kind && f.ActionError.kind.FunctionCallError;
  if (exec && exec.ExecutionError) return String(exec.ExecutionError).replace(/^Smart contract panicked: /, '');
  if (f && f.ActionError && f.ActionError.kind) return JSON.stringify(f.ActionError.kind);
  return JSON.stringify(f);
}
