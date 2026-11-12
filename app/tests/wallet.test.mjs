// Messages from the wallet popup: only MyNearWallet may report transaction hashes.
import test from 'node:test';
import assert from 'node:assert/strict';

import { isWalletOrigin, hashesFromMessage } from '../js/wallet.js';

const HASH = '7UQK6j9MwNG3r4Bfz48LfuU7gYYNzYPcSBdEJafpkrMa';

test('only MyNearWallet and its subdomains over https are trusted', () => {
  for (const good of ['https://testnet.mynearwallet.com', 'https://app.mynearwallet.com', 'https://mynearwallet.com']) {
    assert.ok(isWalletOrigin(good), good);
  }
  for (const bad of [
    'https://evilmynearwallet.com',
    'https://mynearwallet.com.evil.io',
    'http://testnet.mynearwallet.com',
    'null',
    '',
    'not a url',
    'https://localhost:8080',
  ]) {
    assert.ok(!isWalletOrigin(bad), bad);
  }
});

test('hashes are taken from a success message from the wallet', () => {
  const data = { status: 'success', transactionHashes: `${HASH},${HASH.replace('7', '8')}` };
  assert.deepEqual(hashesFromMessage('https://testnet.mynearwallet.com', data), [HASH, HASH.replace('7', '8')]);
});

test('anything else is ignored', () => {
  const ok = { status: 'success', transactionHashes: HASH };
  assert.equal(hashesFromMessage('https://evilmynearwallet.com', ok), null, 'look-alike origin');
  assert.equal(hashesFromMessage('https://testnet.mynearwallet.com', { status: 'failure', transactionHashes: HASH }), null);
  assert.equal(hashesFromMessage('https://testnet.mynearwallet.com', { status: 'success' }), null);
  assert.equal(hashesFromMessage('https://testnet.mynearwallet.com', 'success'), null);
  assert.equal(hashesFromMessage('https://testnet.mynearwallet.com', null), null);
  assert.equal(
    hashesFromMessage('https://testnet.mynearwallet.com', { status: 'success', transactionHashes: '<img src=x onerror=alert(1)>' }),
    null,
    'only base58 hashes are accepted',
  );
});
