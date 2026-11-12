// The app's API client against a scripted fetch: errors, caching, the paid feed, config, RPC.
import test from 'node:test';
import assert from 'node:assert/strict';

import { api, ApiError, forget, settle, rpcView, outcomeFailure } from '../js/api.js';

/** Replace fetch with a script: each call gets the next answer. Returns the requests seen. */
function script(...answers) {
  const seen = [];
  globalThis.fetch = async (url, opts = {}) => {
    seen.push({ url, opts });
    const next = answers.length > 1 ? answers.shift() : answers[0];
    if (next instanceof Error) throw next;
    const { status = 200, body } = next;
    return {
      ok: status >= 200 && status < 300,
      status,
      json: async () => {
        if (body === undefined) throw new SyntaxError('not JSON');
        return body;
      },
    };
  };
  return seen;
}

test.beforeEach(() => forget());

test('a JSON error from the API keeps its status and message', async () => {
  script({ status: 400, body: { error: '`x` is not a market id' } });
  await assert.rejects(api.market('x'), (e) => e instanceof ApiError && e.status === 400 && e.message === '`x` is not a market id');
});

test('a non-JSON error still becomes a readable ApiError', async () => {
  script({ status: 502 });
  await assert.rejects(api.markets(), (e) => e.status === 502 && e.message === 'HTTP 502');
  forget();
  script({ status: 404 });
  await assert.rejects(api.markets(), (e) => e.status === 404 && e.message === 'Not found');
});

test('an unreachable API says so', async () => {
  script(new TypeError('Failed to fetch'));
  await assert.rejects(api.health(), (e) => e.status === 0 && /Could not reach/.test(e.message));
});

test('identical reads within the cache time share one request', async () => {
  const seen = script({ body: [{ id: 0 }] });
  const [a, b] = await Promise.all([api.markets(), api.markets()]);
  assert.deepEqual(a, b);
  await api.markets();
  assert.equal(seen.length, 1);
});

test('a failed read is not cached', async () => {
  const seen = script({ status: 503, body: { error: 'down' } }, { body: [] });
  await assert.rejects(api.markets());
  await Promise.resolve(); // let the cache drop the failure
  assert.deepEqual(await api.markets(), []);
  assert.equal(seen.length, 2);
});

test('forget clears only what it names', async () => {
  const seen = script({ body: [] });
  await api.portfolio('alice.testnet');
  await api.markets();
  forget('portfolio:');
  await api.portfolio('alice.testnet');
  await api.markets();
  assert.equal(seen.length, 3);
});

test('path parts are URL-encoded', async () => {
  const seen = script({ body: {} });
  await api.latest('H1/../00');
  await api.depositStatus('a b');
  assert.equal(seen[0].url, '/v1/fixings/H1%2F..%2F00/latest');
  assert.equal(seen[1].url, '/v1/collateral/status/a%20b');
});

test('a quote is POSTed as JSON and never cached', async () => {
  const seen = script({ body: { dry: true } });
  const body = { origin_asset: 'nep141:x', amount_out: '5000000', recipient: 'a.near', refund_to: '0xabc', dry: true };
  await api.quote(body);
  await api.quote(body);
  assert.equal(seen.length, 2);
  assert.equal(seen[0].opts.method, 'POST');
  assert.deepEqual(JSON.parse(seen[0].opts.body), body);
});

test('the feed returns 402 terms without throwing', async () => {
  const seen = script({ status: 402, body: { error: 'payment required', price: '0.010000' } });
  const r = await api.feed('H100');
  assert.equal(r.status, 402);
  assert.equal(r.body.price, '0.010000');
  assert.deepEqual(seen[0].opts.headers, {}, 'no payment headers without a payment');
});

test('a paid feed read sends both payment headers', async () => {
  const seen = script({ status: 200, body: { fixing: { value: '2.643750' } } });
  const r = await api.feed('H100', 'TxHash111', 'agent.testnet');
  assert.equal(r.status, 200);
  assert.deepEqual(seen[0].opts.headers, { 'X-Payment': 'TxHash111', 'X-Payer': 'agent.testnet' });
});

test('feed errors other than 402 throw', async () => {
  script({ status: 503, body: { error: 'the paid feed is off' } });
  await assert.rejects(api.feed('H100'), (e) => e.status === 503 && /off/.test(e.message));
});

test('settle never throws', async () => {
  assert.deepEqual(await settle(Promise.resolve(1)), { ok: true, value: 1 });
  const failed = await settle(Promise.reject(new Error('x')));
  assert.equal(failed.ok, false);
  assert.equal(failed.error.message, 'x');
});

test('RPC views decode the byte result and surface contract errors', async () => {
  const bytes = [...new TextEncoder().encode('{"spec":"mt-1.0.0"}')];
  const seen = script({ body: { result: { result: bytes } } });
  assert.deepEqual(await rpcView('https://rpc', 'market.testnet', 'mt_metadata_contract', { a: 'ü' }), { spec: 'mt-1.0.0' });
  const sent = JSON.parse(seen[0].opts.body);
  const args = Buffer.from(sent.params.args_base64, 'base64').toString('utf8');
  assert.equal(args, '{"a":"ü"}', 'non-ASCII args survive base64');

  script({ body: { result: { error: 'wasm execution failed: MethodNotFound' } } });
  await assert.rejects(rpcView('https://rpc', 'm', 'nope'), /MethodNotFound/);

  script({ body: { error: { cause: { name: 'UNKNOWN_ACCOUNT' }, message: 'Server error' } } });
  await assert.rejects(rpcView('https://rpc', 'm', 'x'), /UNKNOWN_ACCOUNT/);

  script({ body: { result: { result: [] } } });
  assert.equal(await rpcView('https://rpc', 'm', 'x'), null, 'an empty result is null');
});

test('the contract panic text is pulled out of a failed outcome', () => {
  const panic = { Failure: { ActionError: { kind: { FunctionCallError: { ExecutionError: 'Smart contract panicked: A transfer to a new holder must be at least 0.01 unit' } } } } };
  assert.equal(outcomeFailure({ status: { SuccessValue: '' }, receipts_outcome: [{ outcome: { status: panic } }] }), 'A transfer to a new holder must be at least 0.01 unit');
  assert.equal(outcomeFailure({ status: { SuccessValue: '' }, receipts_outcome: [] }), null);
  assert.equal(outcomeFailure(null), null);
  const other = { Failure: { ActionError: { kind: { AccountDoesNotExist: { account_id: 'x' } } } } };
  assert.match(outcomeFailure({ status: other }), /AccountDoesNotExist/);
});
