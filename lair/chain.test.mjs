import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// chain.js runs in the browser. Here the RPC answers with responses recorded from the real v2
// contracts in the sandbox (fixtures/sandbox.json), and fixture files load from disk. A view the
// dump recorded more than once answers with the entry `pick` names, or else the last one.
const recorded = JSON.parse(readFileSync(new URL('./fixtures/sandbox.json', import.meta.url), 'utf8'));
const last = (key) => (recorded._multi.includes(key) ? recorded[key].at(-1) : recorded[key]);
const requests = [];
const pick = {};
const block = { header: null };
globalThis.location = { search: '' };
globalThis.fetch = async (url, init) => {
  if (String(url).startsWith('file:')) return { json: async () => JSON.parse(readFileSync(url, 'utf8')) };
  const { method, params } = JSON.parse(init.body);
  if (method === 'block') return { ok: true, json: async () => ({ result: { header: block.header } }) };
  const args = JSON.parse(atob(params.args_base64));
  requests.push({ method: params.method_name, args });
  const key = `${params.account_id.split('.')[0]}.${params.method_name}`;
  let answer = key in pick ? recorded[key][pick[key]] : last(key);
  if (key === 'scales.get_scales') answer = answer.slice(args.from ?? 0, (args.from ?? 0) + (args.limit ?? 100));
  if (key === 'dragon.get_log' && args.from !== undefined && typeof args.from !== 'string') {
    return { ok: true, json: async () => ({ result: { error: 'Failed to deserialize input from JSON. Error: `invalid type: integer`, expected a string' } }) };
  }
  return { ok: true, json: async () => ({ result: { result: [...new TextEncoder().encode(JSON.stringify(answer))] } }) };
};
const chain = await import('./chain.js');
const { config } = await import('./config.js');
const { createMock } = await import('./mock.js');
const { classOf } = await import('./domain.js');

function keyPaths(v, path = '') {
  if (v === null || typeof v !== 'object') return [];
  return Object.keys(v).flatMap((k) => [path + k, ...keyPaths(v[k], `${path}${k}.`)]).sort();
}

test("the parsed Dragon state is the same whatever the contract's live mood, phase and day say", async () => {
  const states = recorded['dragon.get_state'];
  const count = states.length;
  const raw = states[2];
  const live = [
    { mood: 'Breathing', phase: { Swapping: { side: 'Buy' } } },
    { mood: 'Hunting', phase: { Swapping: { side: 'Sell' } } },
    { mood: 'Stirring' },
    { mood: 'Gorged', day: { ...raw.day, breath_at: String(BigInt(raw.day.breath_at) + 3_600_000_000_000n), spent: raw.day.budget } },
  ];
  try {
    pick['dragon.get_state'] = 2;
    const base = await chain.getDragonState();
    for (const change of live) {
      states.push({ ...raw, ...change });
      pick['dragon.get_state'] = states.length - 1;
      assert.deepEqual(await chain.getDragonState(), base, JSON.stringify(change));
      assert.deepEqual((await chain.getDragon()).state, base, JSON.stringify(change));
    }
  } finally {
    states.length = count;
    delete pick['dragon.get_state'];
  }
});

test('the Dragon page reads the real get_state, get_log and get_samples, and none of the strategy fields', async () => {
  pick['dragon.get_state'] = 2;
  const { state, samples, log } = await chain.getDragon();
  delete pick['dragon.get_state'];
  const raw = recorded['dragon.get_state'][2];
  assert.ok(raw.dip && raw.day.breath_at && raw.reference && raw.mood && raw.phase, 'the recorded view carries the dip, the breath time, the reference, the mood and the phase');
  assert.deepEqual(keyPaths(state), [
    'coin', 'coin.launchId', 'coin.poolId', 'coin.token',
    'hoard', 'hoard.ashDue', 'hoard.autoBurned', 'hoard.burned', 'hoard.donated', 'hoard.granted', 'hoard.nearSpent', 'hoard.wyrm', 'hoard.wyrmBought',
    'reserve',
    'strategy', 'strategy.autoBurnBps', 'strategy.budgetBps', 'strategy.capBps', 'strategy.hunt',
  ], 'the parsed state carries no live mood, phase, day, breath time, dip, reference or spot');
  assert.deepEqual(state.strategy, { budgetBps: 1000, capBps: 200, hunt: null, autoBurnBps: 0 });
  assert.equal(state.hoard.ashDue, 0n);
  assert.equal(state.hoard.nearSpent, BigInt(raw.hoard.near_spent));
  assert.equal(state.coin.token, raw.coin.token);
  assert.deepEqual(await chain.getDragonConfig(), { preset: 'hoarder', outflowBps: 50, windowBps: 100 });
  assert.equal(samples.length, last('dragon.get_samples').length);
  assert.ok(log.every((e, i) => i === 0 || e.seq < log[i - 1].seq), 'newest first');
  const rawBreath = last('dragon.get_log').find((e) => e.event === 'breath_slice');
  const breath = log.find((e) => e.seq === Number(rawBreath.seq));
  assert.equal(breath.amountIn, BigInt(rawBreath.data.amount_in));
  assert.equal(log.find((e) => e.event === 'swap_failed').slice, 'Breath');
  assert.deepEqual(log.find((e) => e.event === 'settings_changed').strategy, { budgetBps: 1000, capBps: 100, hunt: null, autoBurnBps: 0 }, 'the Council lowered the cap');
  const bound = last('dragon.get_log').find((e) => e.event === 'bound');
  assert.deepEqual(log.find((e) => e.event === 'bound'), { seq: Number(bound.seq), at: BigInt(bound.at), event: 'bound', launchId: raw.coin.launch_id, token: raw.coin.token });
  assert.equal(log.find((e) => e.event === 'outflow_failed').kind, 'grant');
  const logCall = requests.findLast((r) => r.method === 'get_log');
  assert.ok(logCall.args.from === undefined || typeof logCall.args.from === 'string', 'get_log takes from as a U64 string');
});

test("the page clock is the final block's time, so a fast-forwarded chain seasons on chain time", async () => {
  const ahead = 16n * 86_400_000_000_000n + 20n * 3_600_000_000_000n;
  const chainNs = BigInt(Date.now()) * 1_000_000n + ahead;
  block.header = { height: 1, timestamp: Number(chainNs), timestamp_nanosec: String(chainNs) };
  await chain.ready();
  const skew = chain.now() - chainNs;
  assert.ok(skew >= 0n && skew < 5_000_000_000n, `now() is chain time, off by ${skew} ns`);
});

test('the coin is null before bind, and its token account and launch id after', async () => {
  pick['dragon.get_coin'] = 0;
  assert.equal(await chain.getCoin(), null);
  pick['dragon.get_coin'] = 1;
  const raw = recorded['dragon.get_coin'][1];
  assert.deepEqual(await chain.getCoin(), { launchId: raw.launch_id, token: raw.token, poolId: raw.pool_id });
  delete pick['dragon.get_coin'];
});

test('the sale reads Waiting, Open with its end, and Closed with the retired ids', async () => {
  const states = [];
  for (const i of [0, 1, 2]) {
    pick['scales.get_sale'] = i;
    states.push(await chain.getSale());
  }
  delete pick['scales.get_sale'];
  const [waiting, open, closed] = states;
  assert.deepEqual([waiting.state, waiting.endsAt, waiting.price], ['Waiting', null, 2n * 10n ** 24n]);
  assert.equal(open.state, 'Open');
  assert.equal(open.endsAt - open.openedAt, 14n * 86_400_000_000_000n, 'the hoarder sale runs 14 days from bind');
  assert.equal(open.creatorBps, 1000);
  assert.deepEqual([closed.state, closed.sold, closed.retiredFrom, closed.endsAt, closed.titheTold], ['Closed', 45, 46, null, true]);
});

test('get_council lists the enabled kinds with the class each order takes', async () => {
  const c = await chain.getCouncil();
  assert.equal(c.preset, 'hoarder');
  assert.deepEqual(c.classes.Strict, { minElectorate: 45, needsClosedSale: true, rule: '(yes + no) * 3 >= electorate * 2 and yes * 5 >= (yes + no) * 4' });
  assert.equal(c.classes.Light.minElectorate, 10);
  assert.deepEqual(c.bounds.auto_burn_bps, { lo: 0, hi: 5000 });
  assert.equal(c.seasonNs, 7n * 86_400_000_000_000n);
  assert.equal(c.votingNs, 2n * 86_400_000_000_000n);
  const sample = {
    SetBreath: {}, SetSlices: {}, 'SetPosture on': { type: 'SetPosture', hunt: {} }, 'SetPosture off': { type: 'SetPosture', hunt: null },
    Grant: {}, Burn: {}, SetAutoBurn: {}, Signal: {},
  };
  assert.deepEqual(c.kinds.map((k) => k.kind), Object.keys(sample));
  for (const k of c.kinds) assert.equal(classOf({ type: k.kind, ...sample[k.kind] }), k.class, k.kind);
  const cfg = last('scales.get_config');
  assert.deepEqual(await chain.getSystem(), { preset: 'hoarder', creator: cfg.creator, tithe: cfg.tithe, royaltyReceiver: cfg.creator });
});

test('Council views parse the real proposals, classes, votes and LeashHeld reason', async () => {
  const proposals = await chain.getProposals();
  assert.deepEqual(proposals.map((p) => p.id), ['4', '3', '2', '1', '0'], 'ids are the 0-based numbers, newest first');
  assert.deepEqual(proposals.map((p) => p.status.type), ['Open', 'LeashHeld', 'Carried', 'Rejected', 'Executed']);
  assert.deepEqual(proposals.find((p) => p.id === '3').status, { type: 'LeashHeld', error: 'transfer failed' });
  const grant = proposals.find((p) => p.kind.type === 'Grant');
  assert.equal(typeof grant.kind.amount, 'bigint');
  assert.equal(grant.class, 'Strict');
  assert.deepEqual(proposals.find((p) => p.id === '2').kind, { type: 'Signal' });
  assert.deepEqual(proposals.find((p) => p.id === '1').kind, { type: 'SetPosture', hunt: { max_sell_bps: 1000, sell_multiple_bps: 25000 } });
  for (const p of proposals) assert.equal(classOf(p.kind), p.class, `#${p.id}`);
  assert.equal(proposals.find((p) => p.id === '0').bond, 0n, 'a finalized bond is paid out');
  pick['scales.get_votes'] = 0;
  const voted = await chain.getVotedTokens('0');
  delete pick['scales.get_votes'];
  assert.ok(voted.has('1') && voted.has('2'));
  assert.equal(requests.findLast((r) => r.method === 'get_votes').args.id, 0);
  assert.equal(await chain.getElectorateNow(), 45);
});

test('market and Scale views parse the real offers, listed prices, history and mint sweep', async () => {
  const offers = await chain.getOffers();
  assert.deepEqual(offers.map((o) => [o.id, o.tokenId]), [['0', '9']]);
  const scales = await chain.getScales();
  assert.deepEqual(scales.filter((s) => s.listedPrice !== null).map((s) => [s.id, s.listedPrice]), [['7', 8n * 10n ** 24n]], 'the listing comes with its Scale');
  assert.equal(scales.length, 55, "45 sold and the creator's 10");
  assert.equal(scales.find((s) => s.id === '91').voting, false);
  assert.equal(scales[0].vault, '001.scales.thewyrm.test.near');
  const { history } = await chain.getScale('1');
  assert.deepEqual(history.map((h) => h.type), ['Swept', 'Minted']);
  assert.equal(history[0].amount, BigInt(recorded['scales.get_history'][1].Swept.amount));
});

// The JSON type of every path in a response: arrays merge their elements, and a one-key object
// whose key is capitalized is a Rust enum variant with data, named in the path. A unit variant
// ("Open") is a plain string.
function shape(v, path = '$', out = new Map()) {
  const add = (t) => out.set(path, (out.get(path) ?? new Set()).add(t));
  if (v === null) add('null');
  else if (Array.isArray(v)) {
    add('array');
    for (const x of v) shape(x, `${path}[]`, out);
  } else if (typeof v === 'object') {
    const keys = Object.keys(v);
    if (keys.length === 1 && /^[A-Z]/.test(keys[0])) shape(v[keys[0]], `${path}<${keys[0]}>`, out);
    else {
      add('object');
      for (const k of keys) shape(v[k], `${path}.${k}`, out);
    }
  } else add(typeof v);
  return out;
}

// Mismatches between a fixture response and the recorded one. Paths under an enum variant or a
// log event only one side shows are skipped: the sandbox run did not reach every variant.
function mismatches(fixture, real) {
  const f = shape(fixture);
  const r = shape(real);
  const variants = (m) => new Set([...m.keys()].flatMap((p) => [...p.matchAll(/<\w+>/g)].map((x) => x[0])));
  const shared = (p, other) => [...p.matchAll(/<\w+>/g)].every((x) => variants(other).has(x[0]));
  const out = [];
  for (const [p, types] of f) {
    if (!shared(p, r)) continue;
    if (!r.has(p)) out.push(`fixture has ${p} the contract does not`);
    else {
      const want = new Set([...r.get(p)].filter((t) => t !== 'null'));
      const got = [...types].filter((t) => t !== 'null');
      if (want.size && got.some((t) => !want.has(t))) out.push(`${p} is ${got} in the fixture, ${[...want]} from the contract`);
    }
  }
  for (const p of r.keys()) if (shared(p, f) && !f.has(p)) out.push(`fixture lacks ${p}`);
  return out;
}

// Log entries differ by event, so they are compared per event name.
function byEvent(log) {
  return Object.fromEntries([...new Set(log.map((e) => e.event))].map((n) => [n, log.filter((e) => e.event === n)]));
}

const all = (key) => (recorded._multi.includes(key) ? recorded[key] : [recorded[key]]);

for (const sale of ['waiting', 'open', 'closed']) {
  test(`every mock view of the ${sale} world answers in the shape the real contract returns`, async () => {
    Object.assign(config, { mock: true, mockSale: sale });
    try {
      const m = await createMock();
      const D = config.contracts.dragon;
      const S = config.contracts.scales;
      const states = [m.view(D, 'get_state', {})];
      assert.equal(Boolean(states[0].strategy.hunt), sale === 'closed', 'Hunt is on only where the Council turned it on');
      const log = m.view(D, 'get_log', { limit: 256 });
      const realLog = byEvent(all('dragon.get_log').flat());
      // The sandbox never turned Hunt on, so a HuntLine is checked against the proposal view below.
      const fakeLog = byEvent(log.map((e) => (e.event === 'settings_changed' ? { ...e, data: { ...e.data, hunt: null } } : e)));
      const some = (v) => (Array.isArray(v) ? v.filter((x) => x !== null) : v);
      const pairs = {
        get_state: [states.map((st) => ({ ...st, strategy: { ...st.strategy, hunt: null } })), all('dragon.get_state').filter((st) => Boolean(st.coin) === (sale !== 'waiting'))],
        get_config: [[m.view(D, 'get_config', {})].map((c) => ({ ...c, strategy: { ...c.strategy, hunt: null } })), all('dragon.get_config').filter((c) => Boolean(c.coin) === (sale !== 'waiting'))],
        get_coin: [some([m.view(D, 'get_coin', {})]), some(all('dragon.get_coin'))],
        get_samples: [m.view(D, 'get_samples', { limit: 1440 }), all('dragon.get_samples').flat()],
        log_entry: [log.map(({ data, ...rest }) => rest), all('dragon.get_log').flat().map(({ data, ...rest }) => rest)],
        ...Object.fromEntries(Object.keys(realLog).filter((n) => fakeLog[n]).map((n) => [`log ${n}`, [fakeLog[n], realLog[n]]])),
        scales_get_config: [m.view(S, 'get_config', {}), recorded['scales.get_config']],
        get_council: [m.view(S, 'get_council', {}), recorded['scales.get_council']],
        get_scales: [m.view(S, 'get_scales', { from: 0, limit: 100 }), recorded['scales.get_scales']],
        get_scale: [m.view(S, 'get_scale', { token_id: '95' }), recorded['scales.get_scale']],
        get_history: [Object.values(JSON.parse(readFileSync(new URL(`./fixtures/world-${sale}.json`, import.meta.url))).scales.history).flat(), recorded['scales.get_history']],
        get_listings: [m.view(S, 'get_listings', {}), recorded['scales.get_listings']],
        get_offers: [m.view(S, 'get_offers', { from: 0, limit: 200 }), recorded['scales.get_offers']],
        get_proposals: [m.view(S, 'get_proposals', { from: 0, limit: 100 }), recorded['scales.get_proposals']],
        get_votes: [m.view(S, 'get_votes', { id: 6 }), all('scales.get_votes').flat()],
        get_sale: [[m.view(S, 'get_sale', {})], all('scales.get_sale')],
        eligible_now: [m.view(S, 'eligible_now', {}), recorded['scales.eligible_now']],
      };
      const hunting = states.find((st) => st.strategy.hunt);
      if (hunting) assert.deepEqual(Object.keys(hunting.strategy.hunt).sort(), Object.keys(recorded['scales.get_proposals'][1].kind.Order.SetPosture.hunt).sort(), 'a HuntLine as the proposal view shows it');
      const unrecorded = ['auto_burn', 'burn_paid', 'dip_slice', 'grant_paid', 'order_refused'];
      assert.deepEqual(Object.keys(fakeLog).filter((n) => !realLog[n] && !unrecorded.includes(n)), [], `${unrecorded} are checked by hand against events.rs`);
      for (const [view, [fixture, real]] of Object.entries(pairs)) if (!Array.isArray(fixture) || fixture.length) assert.deepEqual(mismatches(fixture, real), [], view);
    } finally {
      Object.assign(config, { mock: false, mockSale: 'closed' });
    }
  });
}

test('the mock rewinds a closed sale with ?sold= and applies the drill preset', async () => {
  Object.assign(config, { mock: true, mockSale: 'closed', mockSold: 40, mockPreset: 'drill' });
  try {
    const m = await createMock();
    const S = config.contracts.scales;
    const sale = m.view(S, 'get_sale', {});
    assert.deepEqual([sale.sold, sale.retired_from, sale.price], [40, 41, '100000000000000000000000']);
    const ids = m.view(S, 'get_scales', { from: 0, limit: 100 }).map((t) => Number(t.token_id));
    assert.deepEqual(ids, [...Array(40).keys()].map((i) => i + 1).concat([...Array(10).keys()].map((i) => i + 91)));
    assert.equal(m.view(S, 'get_council', {}).preset, 'drill');
    assert.equal(m.view(S, 'get_council', {}).season_ns, '600000000000');
  } finally {
    Object.assign(config, { mock: false, mockSold: null, mockPreset: null });
  }
});

test("the closed world's Council history is one the contracts could have written", async () => {
  const { tally, outflowWarning } = await import('./domain.js');
  const w = JSON.parse(readFileSync(new URL('./fixtures/world-closed.json', import.meta.url), 'utf8'));
  const { council } = JSON.parse(readFileSync(new URL('./fixtures/fixed.json', import.meta.url), 'utf8'));
  const season = BigInt(council.season_ns);
  const min = Object.fromEntries(council.classes.map((c) => [c.class, c.min_electorate]));
  const heldSince = (id, at) => w.scales.history[id].map((e) => e.Minted ?? e.Transferred).filter((e) => e && BigInt(e.at) <= at).at(-1);
  for (const p of w.council.proposals) {
    const votes = w.council.votes[p.id];
    const created = BigInt(p.created_at);
    const unseasoned = votes.filter((v) => !(BigInt(heldSince(v.token_id, created)?.at ?? created) + season <= created)).map((v) => v.token_id);
    assert.deepEqual(unseasoned, [], `proposal ${p.id}: every voter held its Scale a season before the proposal opened`);
    assert.equal(new Set(votes.map((v) => v.token_id)).size, votes.length, `proposal ${p.id}: a Scale votes once`);
    assert.deepEqual([p.yes, p.no], [votes.filter((v) => v.approve).length, votes.filter((v) => !v.approve).length], `proposal ${p.id}: the counts are its votes`);
    const status = typeof p.status === 'string' ? p.status : Object.keys(p.status)[0];
    if (status !== 'Open') assert.equal(tally(p, min[p.class]).passing, status !== 'Rejected', `proposal ${p.id} is ${status} on its valid votes`);
  }
  const caps = { outflowBps: 50, windowBps: 100 };
  const [grant, refused] = w.dragon.log.filter((e) => e.event === 'grant_paid' || e.event === 'order_refused');
  const amount = BigInt(grant.data.amount);
  const hoard = BigInt(grant.data.hoard);
  assert.equal(outflowWarning('Grant', { amount }, { hoardWyrm: hoard, ...caps }), null, 'the paid grant fits 50 bps of the hoard before it');
  const second = BigInt(w.council.proposals.find((p) => p.status.LeashHeld).kind.Order.Grant.amount);
  assert.equal(outflowWarning('Grant', { amount: second }, { hoardWyrm: hoard - amount, ...caps }), null, 'the second grant fits the single cap');
  assert.ok((amount + second) * 10_000n > (hoard - amount) * 100n, 'and is over the 30-day cap, as its refusal says');
  assert.match(refused.data.reason, /^grants in any 30 days/);
});

test('the shape check sees a field renamed, a number sent as a string, and a field missing', () => {
  const real = [{ id: 0, dip: { step: 1, first_at: '5' }, status: { LeashHeld: { error: 'x' } } }, { id: 1, dip: null, status: 'Executed' }];
  assert.deepEqual(mismatches([{ id: 0, dip: { step: 1, first_at: '5' }, status: 'Open' }], real), []);
  assert.deepEqual(mismatches([{ id: '0', dip: { step: 1, firstAt: '5' }, status: 'Open' }], real), [
    '$[].id is string in the fixture, number from the contract',
    'fixture has $[].dip.firstAt the contract does not',
    'fixture lacks $[].dip.first_at',
  ]);
});
