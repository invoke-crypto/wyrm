import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// The RPC answers from a mock world, as in pages.test.mjs, behind a rate limiter the test controls: `limit(body)`
// answers a request with a refusal or lets it through.
let world = null;
let limit = () => null;
let inFlight = 0;
let maxInFlight = 0;
const calls = [];
const kinds = [];
const urls = [];
globalThis.location = { search: '' };
const stored = new Map();
globalThis.sessionStorage = { getItem: (k) => stored.get(k) ?? null, setItem: (k, v) => stored.set(k, String(v)) };
globalThis.fetch = async (url, init) => {
  if (String(url).startsWith('file:')) return { json: async () => JSON.parse(readFileSync(url, 'utf8')) };
  const body = JSON.parse(init.body);
  const { params } = body;
  calls.push(params.account_id);
  kinds.push(params.request_type);
  urls.push(String(url));
  const account = ['view_account', 'view_access_key_list'].includes(params.request_type);
  if (account) inFlight++;
  maxInFlight = Math.max(maxInFlight, inFlight);
  try {
    await new Promise((r) => setTimeout(r, 1));
    const refusal = limit(body, String(url));
    if (refusal === 'throw') throw new TypeError('fetch failed');
    if (refusal === 'http') return { ok: false, status: 429, json: async () => ({}) };
    if (refusal === 'error') return { ok: true, status: 200, json: async () => ({ jsonrpc: '2.0', id: 'lair', error: { code: -32000, message: 'Server error', data: 'wasm execution failed' } }) };
    if (refusal === 'json') return { ok: true, status: 200, json: async () => ({ jsonrpc: '2.0', id: 'lair', error: { code: -429, message: 'Too many requests' } }) };
    const result = {
      call_function: () => ({ result: [...new TextEncoder().encode(JSON.stringify(world.view(params.account_id, params.method_name, JSON.parse(atob(params.args_base64)))))] }),
      view_account: () => world.account(params.account_id) ?? { error: `account ${params.account_id} does not exist` },
      view_access_key_list: () => ({ keys: world.keys(params.account_id) }),
    }[params.request_type]();
    return { ok: true, status: 200, json: async () => ({ result }) };
  } finally {
    if (account) inFlight--;
  }
};
const chain = await import('./chain.js');
const { config } = await import('./config.js');
const { createMock } = await import('./mock.js');
const verify = await import('./pages/verify.js');

config.rpcRetryMs = [0, 0, 0, 0, 0];
world = await createMock();
const reset = (fn) => {
  limit = fn;
  calls.length = 0;
  kinds.length = 0;
  urls.length = 0;
  maxInFlight = 0;
};
const count = (kind) => kinds.filter((k) => k === kind).length;
const vaultsRead = () => calls.filter((id, i) => kinds[i] === 'view_access_key_list' && /^\d{3}\.scales\./.test(id)).sort();
const esc = (id) => id.replaceAll('.', '\\.');

// The Verify cache is keyed by network and root, so a root of its own gives a test a cold cache.
let visits = 0;
const coldCache = () => (config.root = `cold-${++visits}`);

const KEY = [{ public_key: 'ed25519:K', access_key: { permission: 'FullAccess' } }];
async function inWorld({ created = 100, keyed = [] }, fn) {
  const { view: worldView, keys: worldKeys } = world;
  world.view = (contract, method, args) => {
    const r = worldView(contract, method, args);
    return contract === config.contracts.scales && method === 'get_config' ? { ...r, vaults_created: created } : r;
  };
  world.keys = (id) => (keyed.includes(id) ? KEY : worldKeys(id));
  try {
    return await fn();
  } finally {
    Object.assign(world, { view: worldView, keys: worldKeys });
  }
}

test('a read refused with HTTP 429, a JSON-RPC -429 or a failed fetch is retried until it is answered', async () => {
  const want = await chain.getSale();
  for (const kind of ['http', 'json', 'throw']) {
    let refused = 0;
    reset(() => (refused++ < 3 ? kind : null));
    assert.deepEqual(await chain.getSale(), want, kind);
    assert.equal(calls.length, 4, `${kind}: three refusals, then the answer`);
  }
});

test('an RPC that rate-limits is left for the next one in the list, and later reads stay there', async () => {
  const want = await chain.getSale();
  config.rpcRetryMs = [60_000];
  try {
    let refusing = null;
    reset((_, url) => ((refusing ??= url) === url ? 'http' : null));
    assert.deepEqual(await chain.getSale(), want);
    await chain.getSale();
    const next = config.rpcs[(config.rpcs.indexOf(refusing) + 1) % config.rpcs.length];
    assert.equal(urls.filter((u) => u === refusing).length, 1, 'the refusing RPC is asked once');
    assert.ok(urls.length > 1 && urls.slice(1).every((u) => u === next), 'every later read goes to the next RPC');
  } finally {
    config.rpcRetryMs = [0, 0, 0, 0, 0];
  }
});

test('a read still refused after every retry fails, and other RPC errors are not retried', async () => {
  reset(() => 'http');
  await assert.rejects(chain.getSale(), /RPC 429/);
  assert.equal(calls.length, (config.rpcRetryMs.length + 1) * config.rpcs.length, 'each round asks every RPC once');
  reset(() => 'error');
  await assert.rejects(chain.getSale(), /wasm execution failed/);
  assert.equal(calls.length, 1);
});

test('the Verify page reads 4 accounts at a time and shows a row the RPC never answers as not read', async () => {
  coldCache();
  const unread = config.vaultId(7);
  reset((body) => (body.params.account_id === unread || body.params.account_id === config.contracts.tithe ? 'http' : null));
  const page = String(verify.view(await verify.load()));
  assert.ok(maxInFlight <= 4, `${maxInFlight} reads at once`);
  assert.match(page, /99 of 100 exist\. 99 have zero access keys\. 1 could not be read from the RPC; reload to try again\./);
  assert.match(page, new RegExp(`<li class="unread"><a [^>]*title="${unread.replaceAll('.', '\\.')}: not read"`));
  assert.doesNotMatch(page, new RegExp(`${unread.replaceAll('.', '\\.')}</a> does not exist`), 'an unread vault is not reported missing');
  assert.match(page, new RegExp(`${config.contracts.tithe.replaceAll('.', '\\.')}</a></th><td><span class="v v-unk">not read</span></td><td><span class="v v-unk">not read</span></td>`));
  assert.match(page, /All 100 run the same global contract/);
  reset(() => null);
  const whole = String(verify.view(await verify.load()));
  assert.match(whole, /100 of 100 exist\. 100 have zero access keys\.\s+All 100 run the same global contract/);
  assert.doesNotMatch(whole, /not read|could not be read/);
});

test('a page load reads each vault once, its key list, and the vault code from vault 001', async () => {
  coldCache();
  reset(() => null);
  const page = String(verify.view(await verify.load()));
  assert.equal(count('view_account'), 5, 'the four core accounts and vault 001');
  assert.equal(count('view_access_key_list'), 104, 'the four core accounts and the 100 vaults');
  const vault1 = world.account(config.vaultId(1));
  assert.ok(page.includes(`Vault 001 runs <code class="hash">${vault1.global_contract_hash ?? vault1.code_hash}</code>`));
  assert.match(page, /<p class="hint">This page reads the code of vault 001\. <code>npm run verify<\/code> checks all 100\.<\/p>/);
});

test('not read never hides a failing code hash, an account or vault holding keys, or a missing vault', async () => {
  coldCache();
  const unread = config.vaultId(7);
  const keyed = config.vaultId(12);
  const missing = [config.vaultId(99), config.vaultId(100)];
  const pinned = { ...config.expectedCodeHashes };
  Object.assign(config.expectedCodeHashes, { tithe: 'not-the-tithe-code', vault: 'not-the-vault-code' });
  reset((body) => (body.params.account_id === unread ? 'http' : null));
  try {
    const page = String(verify.view(await inWorld({ created: 98, keyed: [keyed, config.contracts.dragon] }, verify.load)));
    assert.match(page, new RegExp(`${esc(config.contracts.tithe)}</a></th>\\s*<td>0 <span class="v v-ok">pass</span></td>\\s*<td><code class="hash">[^<]+</code> <span class="v v-bad">fail</span>`));
    assert.match(page, new RegExp(`${esc(config.contracts.dragon)}</a></th>\\s*<td>1 <span class="v v-bad">fail</span></td>`));
    assert.match(page, /Vault 001 runs <code class="hash">[^<]+<\/code> <span class="v v-bad">fail<\/span>/);
    assert.match(page, new RegExp(`${esc(keyed)}</a> has 1 access key\\.`));
    for (const id of missing) {
      assert.match(page, new RegExp(`${esc(id)}</a> does not exist yet`));
      assert.match(page, new RegExp(`<li class="missing"><a [^>]*title="${esc(id)}: missing"`));
    }
    const problems = page.match(/<ul class="problems">([\s\S]*?)<\/ul>/)[1];
    assert.deepEqual([...problems.matchAll(/<li>[\s\S]*?>([^<]+)<\/a>/g)].map((m) => m[1]), [keyed, ...missing], 'the unread vault is no problem line');
    assert.deepEqual([...page.matchAll(/title="([^"]+): not read"/g)].map((m) => m[1]), [unread]);
    assert.equal(page.match(/not read/g).length, 2, "the unread vault's title and its screen-reader label, nothing else");
    assert.match(page, /97 of 100 exist\. 96 have zero access keys\. 1 could not be read from the RPC/);
  } finally {
    Object.assign(config.expectedCodeHashes, pinned);
  }
});

test('a second load reads only the core accounts and the vaults the first found unread, missing or keyed', async () => {
  coldCache();
  const [first, unread, keyed, missing] = [1, 7, 12, 99].map((n) => config.vaultId(n));
  const world98 = { created: 98, keyed: [first, keyed] };
  reset((body) => (body.params.account_id === unread ? 'http' : null));
  await inWorld(world98, verify.load);
  reset(() => null);
  const page = String(verify.view(await inWorld(world98, verify.load)));
  assert.equal(count('view_account'), 5, 'the four core accounts, and vault 001 code again since vault 001 holds a key');
  assert.deepEqual(vaultsRead(), [first, unread, keyed], 'missing vaults need no read, so only the unread and the keyed ones');
  for (const id of [first, keyed]) {
    assert.match(page, new RegExp(`${esc(id)}</a> has 1 access key\\.`));
    assert.match(page, new RegExp(`<li class="keyed"><a [^>]*title="${esc(id)}: 1 keys"`));
  }
  assert.match(page, new RegExp(`<li class="missing"><a [^>]*title="${esc(missing)}: missing"`));
  assert.match(page, /98 of 100 exist\. 96 have zero access keys\.\s+All 100/);

  reset(() => null);
  const grown = String(verify.view(await inWorld({ created: 100 }, verify.load)));
  assert.deepEqual(vaultsRead(), [first, keyed, missing, config.vaultId(100)], 'a vault created since is read, not assumed');
  assert.match(grown, /100 of 100 exist\. 100 have zero access keys\./);
});

test('a cached vault is never shown for another root or network', async () => {
  const root = coldCache();
  const keyed = config.vaultId(12);
  reset(() => null);
  await verify.load();
  assert.equal(vaultsRead().length, 100);
  for (const [network, other] of [['sandbox', root], [config.network, 'another-root']]) {
    const home = config.network;
    Object.assign(config, { network, root: other });
    try {
      reset(() => null);
      const page = String(verify.view(await inWorld({ keyed: [keyed] }, verify.load)));
      assert.equal(vaultsRead().length, 100, `${network} ${other}: every vault read again`);
      assert.match(page, new RegExp(`${esc(keyed)}</a> has 1 access key\\.`), `${network} ${other}: not the cached zero`);
    } finally {
      Object.assign(config, { network: home, root });
    }
  }
  reset(() => null);
  await verify.load();
  assert.deepEqual(vaultsRead(), [], 'back on the first root, the cache serves every vault');

  coldCache();
  config.mock = true;
  try {
    await verify.load();
  } finally {
    config.mock = false;
  }
  reset(() => null);
  const page = String(verify.view(await inWorld({ keyed: [keyed] }, verify.load)));
  assert.equal(vaultsRead().length, 100, 'the mock world caches nothing for the real network');
  assert.match(page, new RegExp(`${esc(keyed)}</a> has 1 access key\\.`));
});

test('a reload in the same tab takes the cache from sessionStorage, without the vaults found keyed', async () => {
  coldCache();
  const keyed = config.vaultId(12);
  reset(() => null);
  await inWorld({ keyed: [keyed] }, verify.load);
  assert.ok(stored.has(`lair.verify.${config.network}.${config.root}`));
  const reloaded = await import('./chain.js?reload');
  reset(() => null);
  const { vaults, vaultCode } = await inWorld({ keyed: [keyed] }, reloaded.getVerifyTargets);
  assert.deepEqual(vaultsRead(), [keyed]);
  assert.equal(count('view_account'), 4, 'the four core accounts; vault 001 code is cached');
  assert.equal(vaults.find((v) => v.id === keyed).keys, 1);
  assert.ok(vaults.every((v) => v.read && v.exists));
  assert.equal(vaultCode.globalHash ?? vaultCode.codeHash, world.account(config.vaultId(1)).global_contract_hash ?? world.account(config.vaultId(1)).code_hash);
});

test('a chain reset under the same root shows its vaults missing, not the cached ones', async () => {
  coldCache();
  reset(() => null);
  await verify.load();
  reset(() => null);
  const page = String(verify.view(await inWorld({ created: 0 }, verify.load)));
  assert.deepEqual(vaultsRead(), [], 'a vault the chain says is missing is not read');
  assert.match(page, /\b0 of 100 exist\./);
});

test('the Lair pins the mainnet code hashes of the release verify/releases.json names', () => {
  const { releases } = JSON.parse(readFileSync(new URL('../verify/releases.json', import.meta.url), 'utf8'));
  const { code } = releases.at(-1);
  assert.deepEqual(config.expectedCodeHashes, { tithe: code.tithe, dragon: code.dragon, scales: code.scales, vault: code.scale_vault });
});
