// Every contract name, method name and JSON shape the Lair uses lives here.
// Raw RPC results are parsed into domain objects at this boundary (BigInt amounts,
// BigInt nanosecond times, tagged unions), and nothing past it re-checks them.
import { config } from './config.js';
import { NS_PER_MS } from './format.js';
import { MAX_VOTE_TOKENS, STORAGE_FEE, chunk } from './domain.js';
import { wallet } from './wallet.js';
import { html } from './ui.js';

const C = config.contracts;
const ONE_YOCTO = '1';
let mock = null;

async function mockBackend() {
  if (!mock) mock = await (await import('./mock.js')).createMock();
  return mock;
}

// Chain time minus this browser's clock. A fast-forwarded sandbox or a visitor's wrong clock would
// otherwise skew seasoning, Ash countdowns and every "ago".
let clockOffsetNs = 0n;

export function now() {
  return mock ? mock.now() : BigInt(Date.now()) * NS_PER_MS + clockOffsetNs;
}

export async function ready() {
  if (config.mock) return void (await mockBackend());
  try {
    const { header } = await rpc({}, 'block');
    clockOffsetNs = BigInt(header.timestamp_nanosec) - BigInt(Date.now()) * NS_PER_MS;
  } catch (e) {
    console.warn('chain clock', e);
  }
}

class Busy extends Error {}

async function rpcOnce(url, params, method) {
  let res;
  try {
    res = await fetch(url, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 'lair', method, params: { finality: 'final', ...params } }),
    });
  } catch (e) {
    throw new Busy(`RPC unreachable: ${e.message}`);
  }
  if (res.status === 429) throw new Busy('RPC 429');
  if (!res.ok) throw new Error(`RPC ${res.status}`);
  const body = await res.json();
  if (body.error?.code === -429) throw new Busy('RPC -429');
  if (body.error) throw new Error(body.error.data ?? body.error.message ?? 'RPC error');
  if (body.result?.error) throw new Error(body.result.error);
  return body.result;
}

// Public RPCs answer a burst of reads with HTTP 429 or a JSON-RPC -429, and a dropped connection fails the fetch.
// A refused read moves every later read to the next RPC in the list; the Lair waits only once all of them refused.
let current = Math.floor(Math.random() * config.rpcs.length);

async function rpc(params, method = 'query') {
  for (let waits = 0, refused = 0; ; ) {
    const at = current;
    try {
      return await rpcOnce(config.rpcs[at], params, method);
    } catch (e) {
      if (!(e instanceof Busy)) throw e;
      if (current === at) current = (at + 1) % config.rpcs.length;
      if (++refused < config.rpcs.length) continue;
      if (waits >= config.rpcRetryMs.length) throw e;
      refused = 0;
      await new Promise((r) => setTimeout(r, config.rpcRetryMs[waits++] * (1 + Math.random())));
    }
  }
}

async function view(contract, method, args = {}) {
  if (config.mock) return (await mockBackend()).view(contract, method, args);
  const result = await rpc({
    request_type: 'call_function',
    account_id: contract,
    method_name: method,
    args_base64: btoa(JSON.stringify(args)),
  });
  return JSON.parse(new TextDecoder().decode(new Uint8Array(result.result)));
}

async function viewAccount(accountId) {
  if (config.mock) return (await mockBackend()).account(accountId);
  try {
    return await rpc({ request_type: 'view_account', account_id: accountId });
  } catch (e) {
    if (/does not exist|UNKNOWN_ACCOUNT/i.test(String(e.message))) return null;
    throw e;
  }
}

async function viewAccessKeys(accountId) {
  if (config.mock) return (await mockBackend()).keys(accountId);
  return (await rpc({ request_type: 'view_access_key_list', account_id: accountId })).keys;
}

async function inPool(items, limit, fn) {
  const out = new Array(items.length);
  let next = 0;
  await Promise.all(
    Array.from({ length: Math.min(limit, items.length) }, async () => {
      while (next < items.length) {
        const i = next++;
        out[i] = await fn(items[i]);
      }
    }),
  );
  return out;
}

const big = (v) => BigInt(v);
const bigOrNull = (v) => (v == null ? null : BigInt(v));

// Rust enums arrive externally tagged: "Open" or {"LeashHeld": {...}}.
function tagged(raw) {
  if (typeof raw === 'string') return { type: raw };
  const [type] = Object.keys(raw);
  return { type, ...raw[type] };
}

function parseScale(r) {
  return {
    id: r.token_id,
    owner: r.owner,
    voting: r.voting,
    holdStart: big(r.hold_start),
    claimedTotal: big(r.claimed_total),
    claimingSince: bigOrNull(r.claiming_since),
    listedPrice: bigOrNull(r.listed_price),
    vault: r.vault,
  };
}

// scales ScaleEvent: Minted { to, price, at }, Transferred { from, to, price: null when not a sale, at },
// Claimed { by, amount, at }, Swept { amount, at } (the mint sweep to the Dragon).
function parseHistory(r) {
  const e = tagged(r);
  return {
    type: e.type,
    at: big(e.at),
    from: e.from ?? null,
    to: e.to ?? e.by ?? null,
    price: bigOrNull(e.price),
    amount: bigOrNull(e.amount),
  };
}

// Offer and proposal ids are JSON numbers, kept as strings in the Lair.
function parseOffer(r) {
  return { id: String(r.id), buyer: r.buyer, tokenId: r.token_id, amount: big(r.amount), at: big(r.at) };
}

// kind is "Signal" or {"Order": {<Order>}}; SetPosture's hunt is null or a HuntLine.
function parseKind(raw) {
  if (raw === 'Signal') return { type: 'Signal' };
  const kind = tagged(raw.Order);
  if (kind.amount !== undefined) kind.amount = big(kind.amount);
  return kind;
}

function parseProposal(r) {
  return {
    id: String(r.id),
    kind: parseKind(r.kind),
    class: r.class,
    proposer: r.proposer,
    description: r.description,
    bond: big(r.bond),
    createdAt: big(r.created_at),
    endsAt: big(r.ends_at),
    electorate: r.electorate,
    yes: r.yes,
    no: r.no,
    status: tagged(r.status),
  };
}

function parseStrategy(s) {
  return {
    budgetBps: s.budget_bps,
    capBps: s.cap_bps,
    hunt: s.hunt && { sellMultipleBps: s.hunt.sell_multiple_bps, maxSellBps: s.hunt.max_sell_bps },
    autoBurnBps: s.auto_burn_bps,
  };
}

function parseCoin(c) {
  return c && { launchId: c.launch_id, token: c.token, poolId: c.pool_id };
}

// Only what the Lair shows. The live mood, the phase, the day with its breath time, the dip, the reference and the
// ladder are public views, but the Lair never reads them, so no page can show when the Dragon will buy next.
function parseDragonState(r) {
  const h = r.hoard;
  return {
    coin: parseCoin(r.coin),
    reserve: big(r.reserve),
    hoard: {
      wyrm: big(h.wyrm),
      nearSpent: big(h.near_spent),
      wyrmBought: big(h.wyrm_bought),
      donated: big(h.donated),
      granted: big(h.granted),
      burned: big(h.burned),
      autoBurned: big(h.auto_burned),
      ashDue: big(h.ash_due),
    },
    strategy: parseStrategy(r.strategy),
  };
}

function parseDragonConfig(r) {
  return {
    preset: r.preset,
    outflowBps: r.engine.outflow_bps,
    windowBps: r.engine.outflow_window_bps,
  };
}

// dragon LogView: { seq, at, event, data }, the event's fields flattened into data.
// Slice amounts: a buy spends amount_in NEAR for quoted_out WYRM; a hunt sells amount_in WYRM for quoted_out NEAR.
function parseLogEntry(r) {
  const d = r.data;
  const base = { seq: Number(r.seq), at: big(r.at), event: r.event };
  switch (r.event) {
    case 'breath_slice':
    case 'dip_slice':
    case 'hunt_slice':
      return { ...base, amountIn: big(d.amount_in), quotedOut: big(d.quoted_out) };
    case 'swap_failed':
      return { ...base, slice: d.slice, amountIn: big(d.amount_in) };
    case 'grant_paid':
      return { ...base, to: d.to, amount: big(d.amount) };
    case 'burn_paid':
    case 'auto_burn':
      return { ...base, amount: big(d.amount) };
    case 'outflow_failed':
      return { ...base, kind: d.kind, amount: big(d.amount) };
    case 'settings_changed':
      return { ...base, strategy: parseStrategy(d) };
    case 'order_refused':
    case 'launch_refused':
      return { ...base, reason: d.reason };
    case 'bind_refused':
      return { ...base, launchId: d.launch_id, reason: d.reason };
    case 'launch_sent':
      return { ...base, ok: d.ok };
    case 'bound':
      return { ...base, launchId: d.launch_id, token: d.token };
    default:
      return base;
  }
}

function parseSale(r) {
  return {
    state: r.state,
    price: big(r.price),
    creatorBps: r.creator_bps,
    sold: r.sold,
    openedAt: bigOrNull(r.opened_at),
    endsAt: bigOrNull(r.ends_at),
    retiredFrom: r.retired_from,
    titheTold: r.tithe_told,
  };
}

function parseCouncil(r) {
  return {
    preset: r.preset,
    kinds: r.kinds.map((k) => ({ kind: k.kind, class: k.class })),
    classes: Object.fromEntries(r.classes.map((c) => [c.class, { minElectorate: c.min_electorate, needsClosedSale: c.needs_closed_sale, rule: c.rule }])),
    bounds: r.bounds,
    seasonNs: big(r.season_ns),
    votingNs: big(r.voting_ns),
    bond: big(r.bond),
    storageFee: big(r.storage_fee),
  };
}

// The log ring keeps the newest 256 entries; get_log pages from the oldest, so read the whole ring.
const LOG_CAP = 256;

export async function getCoin() {
  return parseCoin(await view(C.dragon, 'get_coin'));
}

// The preset and the accounts scales names never change after init, so one read serves the whole visit.
let system = null;
export function getSystem() {
  system ??= view(C.scales, 'get_config').then((r) => ({
    preset: r.preset,
    creator: r.creator,
    tithe: r.tithe,
    royaltyReceiver: r.royalty_receiver,
  }));
  return system;
}

export async function getDragon() {
  const [state, samples, log] = await Promise.all([
    view(C.dragon, 'get_state'),
    view(C.dragon, 'get_samples', { limit: 1440 }),
    view(C.dragon, 'get_log', { limit: LOG_CAP }),
  ]);
  return {
    state: parseDragonState(state),
    samples: samples.map((x) => ({ at: big(x.at), tokensPerNear: big(x.tokens_per_near) })),
    log: log.map(parseLogEntry).reverse(),
  };
}

export async function getDragonState() {
  return parseDragonState(await view(C.dragon, 'get_state'));
}

export async function getDragonConfig() {
  return parseDragonConfig(await view(C.dragon, 'get_config'));
}

export async function getCouncil() {
  return parseCouncil(await view(C.scales, 'get_council'));
}

export async function getElectorateNow() {
  return view(C.scales, 'eligible_now');
}

export async function getScales() {
  const pages = await Promise.all([0, 50].map((from) => view(C.scales, 'get_scales', { from, limit: 50 })));
  return pages.flat().map(parseScale);
}

export async function getScale(tokenId) {
  const [scale, history, offers] = await Promise.all([
    view(C.scales, 'get_scale', { token_id: tokenId }),
    view(C.scales, 'get_history', { token_id: tokenId }),
    view(C.scales, 'get_offers_for', { token_id: tokenId }),
  ]);
  return scale && { scale: parseScale(scale), history: history.map(parseHistory).reverse(), offers: offers.map(parseOffer) };
}

export async function getUnclaimed(scales) {
  const balances = await inPool(scales, 10, async (s) => {
    const acc = await viewAccount(s.vault);
    if (!acc) return null;
    const free = big(acc.amount) - config.vaultReserveYocto;
    return free > 0n ? free : 0n;
  });
  return new Map(scales.map((s, i) => [s.id, balances[i]]));
}

export async function getSale() {
  return parseSale(await view(C.scales, 'get_sale'));
}

export async function getOffers() {
  const raw = await view(C.scales, 'get_offers', { from: 0, limit: 200 });
  return raw.map(parseOffer);
}

export async function getProposals() {
  const raw = await view(C.scales, 'get_proposals', { from: 0, limit: 100 });
  return raw.map(parseProposal).sort((a, b) => Number(b.id) - Number(a.id));
}

export async function getVotedTokens(proposalId) {
  const raw = await view(C.scales, 'get_votes', { id: Number(proposalId) });
  return new Set(raw.map((v) => v.token_id));
}

const CORE_ROLES = ['tithe', 'dragon', 'scales', 'ash'];

// A vault that exists with no keys stays that way: vaults are never deleted, and their code, published by hash, has
// no path to add a key or deploy other code. So a browser session reads each such vault, and vault 001's code, once.
// The memory copy still serves when sessionStorage is missing or refuses, as in some private windows.
const verifyCaches = new Map();

function verifyCache() {
  const key = `lair.verify.${config.mock ? 'mock' : config.network}.${config.root}`;
  if (!verifyCaches.has(key)) {
    let saved = null;
    try {
      saved = JSON.parse(sessionStorage.getItem(key));
    } catch {}
    verifyCaches.set(key, { key, fine: new Set(saved?.fine ?? []), vaultCode: saved?.vaultCode ?? null });
  }
  return verifyCaches.get(key);
}

function saveVerifyCache({ key, fine, vaultCode }) {
  try {
    sessionStorage.setItem(key, JSON.stringify({ fine: [...fine], vaultCode }));
  } catch {}
}

// One read per vault keeps a page load inside a public RPC's per-minute allowance. A missing account's key list
// reads as empty, so existence comes from vaults_created, which the Scales raise only after probing the new vaults.
export async function getVerifyTargets() {
  const cache = verifyCache();
  const created = (await view(C.scales, 'get_config')).vaults_created;
  const rows = [
    ...CORE_ROLES.map((role) => ({ role, id: C[role] })),
    ...(created > 0 ? [{ role: 'vault code', id: config.vaultId(1) }] : []),
    ...Array.from({ length: 100 }, (_, i) => ({ role: 'vault', id: config.vaultId(i + 1), exists: i < created })),
  ];
  const read = await inPool(rows, 4, async (t) => {
    if (t.role === 'vault' && t.exists && cache.fine.has(t.id)) return { ...t, read: true, exists: true, keys: 0 };
    if (t.role === 'vault code' && cache.vaultCode) return { ...t, read: true, exists: true, ...cache.vaultCode };
    try {
      if (t.role === 'vault') {
        const keys = t.exists ? (await viewAccessKeys(t.id)).length : null;
        return { ...t, read: true, keys };
      }
      const acc = await viewAccount(t.id);
      const code = { exists: Boolean(acc), codeHash: acc?.code_hash ?? null, globalHash: acc?.global_contract_hash ?? null };
      if (t.role === 'vault code') return { ...t, read: true, ...code };
      const keys = acc ? (await viewAccessKeys(t.id)).length : null;
      return { ...t, read: true, ...code, keys };
    } catch (e) {
      console.warn('verify read', t.id, e);
      return { ...t, read: false };
    }
  });
  const vaults = read.filter((t) => t.role === 'vault');
  const vaultCode = read.find((t) => t.role === 'vault code') ?? null;
  for (const v of vaults) if (v.read && v.exists && v.keys === 0) cache.fine.add(v.id);
  if (vaultCode?.read && vaultCode.exists && cache.fine.has(vaultCode.id)) {
    cache.vaultCode = { codeHash: vaultCode.codeHash, globalHash: vaultCode.globalHash };
  }
  saveVerifyCache(cache);
  return { core: read.filter((t) => CORE_ROLES.includes(t.role)), vaultCode, vaults };
}

function fn(method, args, deposit = '0', gas = config.gas.call) {
  return { type: 'FunctionCall', params: { methodName: method, args, gas, deposit } };
}

const scalesTx = (...actions) => ({ receiverId: C.scales, actions });

export const tx = {
  buyScale: (price) => [scalesTx(fn('buy_scale', {}, price.toString(), config.gas.heavy))],
  claim: (tokenId) => [scalesTx(fn('claim', { token_id: tokenId }, ONE_YOCTO, config.gas.heavy))],
  claimAll: (tokenIds) => tokenIds.map((id) => scalesTx(fn('claim', { token_id: id }, ONE_YOCTO, config.gas.heavy))),
  list: (tokenId, price) => [scalesTx(fn('list', { token_id: tokenId, price: price.toString() }, ONE_YOCTO))],
  delist: (tokenId) => [scalesTx(fn('delist', { token_id: tokenId }, ONE_YOCTO))],
  buy: (tokenId, price) => [scalesTx(fn('buy', { token_id: tokenId }, price.toString(), config.gas.heavy))],
  makeOffer: (tokenId, amount) => [scalesTx(fn('make_offer', { token_id: tokenId }, (amount + STORAGE_FEE).toString()))],
  cancelOffer: (offerId) => [scalesTx(fn('cancel_offer', { offer_id: Number(offerId) }, ONE_YOCTO))],
  acceptOffer: (offerId, tokenId) => [scalesTx(fn('accept_offer', { offer_id: Number(offerId), token_id: tokenId }, ONE_YOCTO, config.gas.heavy))],
  propose: (kind, description, tokenId, deposit) => [scalesTx(fn('propose', { kind, description, token_id: tokenId }, deposit.toString()))],
  vote: (proposalId, tokenIds, approve) =>
    chunk(tokenIds, MAX_VOTE_TOKENS).map((ids) => scalesTx(fn('vote', { proposal_id: Number(proposalId), token_ids: ids, approve }, ONE_YOCTO))),
  // finalize refuses under 150 Tgas, and a passed order reaches the Dragon in the same transaction.
  finalize: (proposalId) => [scalesTx(fn('finalize', { proposal_id: Number(proposalId) }, '0', config.gas.heavy))],
};

export async function send(transactions) {
  if (config.mock) {
    const m = await mockBackend();
    for (const t of transactions) for (const a of t.actions) m.call(wallet.accountId, t.receiverId, a.params.methodName, a.params.args, BigInt(a.params.deposit));
    return;
  }
  await wallet.signAndSend(transactions);
}

export function explorerAccount(id) {
  return config.explorer ? `${config.explorer}/address/${id}` : null;
}

export function accountLink(id, text = id) {
  const href = explorerAccount(id);
  return href ? html`<a href="${href}" target="_blank" rel="noopener">${text}</a>` : html`<span>${text}</span>`;
}
