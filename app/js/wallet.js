// NEAR Wallet Selector (MyNearWallet, plus Meteor when it loads), from esm.sh with pinned versions.
// MyNearWallet 10.x signs in a popup and posts the transaction hashes back to this window;
// we listen for that message so every action can show its hash and the contract's own result.

import { txStatus, outcomeFailure, forget } from './api.js';

export const SELECTOR_VERSION = '10.1.4';
const esm = (pkg) => `https://esm.sh/@near-wallet-selector/${pkg}@${SELECTOR_VERSION}`;

export const TGAS_300 = '300000000000000';
export const ONE_YOCTO = '1';
export const STORAGE_DEPOSIT = '1250000000000000000000'; // 0.00125 NEAR

const state = { status: 'loading', error: null, accountId: null, walletId: null };
const listeners = new Set();
let selector = null;
let modal = null;
let actionCreators = null;
let config = null;

export function walletState() {
  return { ...state };
}

export function onWalletChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function emit() {
  for (const fn of listeners) fn(walletState());
}

function setAccount(accountId, walletId) {
  if (state.accountId === accountId && state.status === 'ready') return;
  state.accountId = accountId || null;
  state.walletId = walletId || null;
  state.status = 'ready';
  emit();
}

export async function initWallet(cfg) {
  config = cfg;
  if (!cfg.claim_market) {
    state.status = 'error';
    state.error = 'The claim-market contract is not configured on this server.';
    emit();
    return;
  }
  try {
    const [core, mnw, ui] = await Promise.all([import(esm('core')), import(esm('my-near-wallet')), import(esm('modal-ui'))]);
    actionCreators = core.actionCreators;
    const modules = [mnw.setupMyNearWallet()];
    try {
      const meteor = await import(esm('meteor-wallet'));
      modules.push(meteor.setupMeteorWallet());
    } catch {
      // Meteor is optional.
    }
    selector = await core.setupWalletSelector({
      network: cfg.network === 'mainnet' ? 'mainnet' : 'testnet',
      modules,
      // Use the server's RPC (FastNEAR) for the selector too; the public RPC rate-limits.
      fallbackRpcUrls: cfg.rpc ? [cfg.rpc] : undefined,
      createAccessKeyFor: cfg.claim_market,
    });
    modal = ui.setupModal(selector, { contractId: cfg.claim_market, description: 'Sign in to mint, send and claim on Quill (NEAR testnet).' });
    const pick = (s) => {
      const acc = (s.accounts || []).find((a) => a.active) || (s.accounts || [])[0];
      setAccount(acc ? acc.accountId : null, s.selectedWalletId);
    };
    pick(selector.store.getState());
    selector.store.observable.subscribe(pick);
  } catch (e) {
    console.warn('Wallet selector failed to load', e);
    state.status = 'error';
    state.error = 'The wallet could not load. Check your connection and reload.';
    emit();
  }
}

export function signIn() {
  if (modal) modal.show();
}

export async function signOut() {
  if (!selector) return;
  try {
    const w = await selector.wallet();
    await w.signOut();
  } catch (e) {
    console.warn(e);
  }
  setAccount(null, null);
}

// ---------- signing ----------

let capturedHashes = null;

/** True only for MyNearWallet itself or its subdomains, over https: never "evilmynearwallet.com". */
export function isWalletOrigin(origin) {
  let url;
  try {
    url = new URL(origin);
  } catch {
    return false; // "null" from sandboxed frames and file pages
  }
  return url.protocol === 'https:' && /(^|\.)mynearwallet\.com$/.test(url.hostname);
}

/** Transaction hashes from a wallet's postMessage, or null if the message is not one. */
export function hashesFromMessage(origin, data) {
  if (!isWalletOrigin(origin)) return null;
  if (!data || typeof data !== 'object' || data.status !== 'success' || !data.transactionHashes) return null;
  const hashes = String(data.transactionHashes).split(',').map((h) => h.trim()).filter((h) => /^[1-9A-HJ-NP-Za-km-z]{43,44}$/.test(h));
  return hashes.length ? hashes : null;
}

if (typeof window !== 'undefined') {
  window.addEventListener('message', (ev) => {
    const hashes = hashesFromMessage(ev.origin, ev.data);
    if (hashes) capturedHashes = hashes;
  });
}

function hashesFrom(result) {
  const list = Array.isArray(result) ? result : [result];
  const out = [];
  for (const r of list) {
    if (!r || typeof r !== 'object') continue;
    const h = (r.transaction && r.transaction.hash) || (r.transaction_outcome && r.transaction_outcome.id);
    if (h) out.push(h);
  }
  return out;
}

function fnCall(c) {
  return actionCreators.functionCall(c.method, c.args, BigInt(c.gas || TGAS_300), BigInt(c.deposit || '0'));
}

/**
 * Sign and send one or more transactions, each `{ receiverId, calls: [{ method, args, deposit, gas }] }`.
 * Resolves `{ hashes, failure }` where failure is the contract's own message (or null);
 * throws if the wallet refused or the user closed it.
 */
export async function send(txs) {
  if (!selector) throw new Error(state.error || 'Wallet not ready yet.');
  if (!state.accountId) throw new Error('Sign in first.');
  const wallet = await selector.wallet();
  capturedHashes = null;
  let result;
  if (txs.length === 1) {
    result = await wallet.signAndSendTransaction({ receiverId: txs[0].receiverId, actions: txs[0].calls.map(fnCall) });
  } else {
    result = await wallet.signAndSendTransactions({
      transactions: txs.map((t) => ({ receiverId: t.receiverId, actions: t.calls.map(fnCall) })),
    });
  }
  let hashes = hashesFrom(result);
  if (!hashes.length && capturedHashes) hashes = capturedHashes;
  let failure = null;
  for (const h of hashes) {
    try {
      const o = await txStatus(config.rpc, h, state.accountId);
      failure = failure || outcomeFailure(o);
    } catch (e) {
      console.warn('Could not read the outcome of', h, e);
    }
  }
  if (!hashes.length && result) failure = outcomeFailure(Array.isArray(result) ? result[0] : result);
  forget('portfolio:');
  forget('markets');
  forget('market:');
  forget('view:');
  return { hashes, failure };
}

/** Convenience: a single function call. */
export function call(receiverId, method, args, deposit = '0', gas = TGAS_300) {
  return send([{ receiverId, calls: [{ method, args, deposit, gas }] }]);
}

export function explorerTx(hash) {
  const base = (config && config.explorer) || 'https://testnet.nearblocks.io';
  const path = /nearblocks/.test(base) ? '/txns/' : '/transactions/';
  return base.replace(/\/$/, '') + path + hash;
}
