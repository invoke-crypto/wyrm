// Writes the mock-mode fixtures in the raw JSON shapes the v2 contracts' views return, one world per sale state,
// and copies the Gym's estimator. lair/chain.test.mjs checks every world against the responses recorded from the
// real contracts in the sandbox (sandbox.json).
// Run: node lair/fixtures/generate.mjs
import { copyFileSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const dir = new URL('./', import.meta.url);
const NS_DAY = 86_400_000_000_000n;
const NS_HOUR = 3_600_000_000_000n;
const NS_MIN = 60_000_000_000n;
const NEAR = 10n ** 24n;
const WYRM = 10n ** 18n;
const utc = (d, h, m = 0) => BigInt(Date.UTC(2026, 10, d, h, m)) * 1_000_000n;
const root = 'thewyrm.near';
const vault = (i) => `${String(i).padStart(3, '0')}.scales.${root}`;
const near = (x) => ((BigInt(Math.round(x * 1000)) * NEAR) / 1000n).toString();
const s = (n) => n.toString();

const YOU = 'ember.near';
const CREATOR = 'creator.thewyrm.near';
const DRAGON = `dragon.${root}`;
const TITHE = `tithe.${root}`;
const PRICE = 2n * NEAR;
const CREATOR_BPS = 1000;
const BOUND_AT = utc(9, 12);
const SALE_ENDS = BOUND_AT + 14n * NS_DAY;
const SEASON = 7n * NS_DAY;
const VOTING = 2n * NS_DAY;
const COIN = { launch_id: '3651', pool_id: 'wrap.near|wyrm.nearlytrade.near|10000', token: 'wyrm.nearlytrade.near', wrap_is_x: true };

const names = ['kestrel.near', 'molly-m.near', 'dune.near', 'velvet.near', 'gorm.near', 'pinecone.near', 'hoardwatch.near', 'ruby-scale.near', 'saltmarsh.near', 'tavi.near', 'a7f3c2e0b9d14f6e8a2b5c7d9e1f3a5b7c9d1e3f5a7b9c1d3e5f7a9b1c3d5e7f'];
const tierOf = (heldNs) => (heldNs >= 14n * NS_DAY ? 'Obsidian' : heldNs >= 7n * NS_DAY ? 'Molten' : heldNs >= 3n * NS_DAY ? 'Ember' : 'Ash');

function rng(seed) {
  return () => ((seed = (seed * 16807) % 2147483647) - 1) / 2147483646;
}

// scales mints 91 to 100 to the creator at init and sells 1 to 90 from bind on, sweeping each new vault.
function collection(now, sold, rnd) {
  const tokens = [];
  const history = {};
  // A launch-day rush buys the first 30 in two hours, then the rest trickle in over the 14 days.
  const mintAt = (i) => BOUND_AT + (i <= 30 ? BigInt(i) * 3n * NS_MIN : 2n * NS_HOUR + (BigInt(i - 30) * (SALE_ENDS - BOUND_AT - 4n * NS_HOUR)) / 20n);
  for (let i = 1; i <= 100; i++) {
    const creator = i > 90;
    if (!creator && i > sold) continue;
    const at = creator ? BOUND_AT - 18n * NS_HOUR : mintAt(i);
    let owner = creator ? CREATOR : names[Math.floor(rnd() * names.length)];
    if ([3, 17, 42].includes(i)) owner = YOU;
    let held = at;
    const h = [{ Minted: { to: owner, price: creator ? '0' : s(PRICE), at: s(at) } }];
    if (!creator) h.push({ Swept: { amount: near(0.002 + rnd() * 0.01), at: s(at + 15_000_000_000n) } });
    if (!creator && ![3, 17, 42].includes(i) && rnd() < 0.35 && now - at > 4n * NS_DAY) {
      const next = names[Math.floor(rnd() * names.length)];
      held = at + BigInt(Math.round(rnd() * Number(now - at - 2n * NS_DAY)));
      h.push({ Claimed: { by: owner, amount: near(0.05 + rnd() * 0.3), at: s(held - NS_HOUR) } });
      h.push({ Transferred: { from: owner, to: next, price: rnd() < 0.8 ? near(5.5 + rnd() * 4) : null, at: s(held) } });
      owner = next;
    }
    const claimed = h.filter((e) => e.Claimed).reduce((n, e) => n + BigInt(e.Claimed.amount), 0n);
    tokens.push({
      token_id: String(i), owner, voting: !creator, hold_start: s(held), tier: tierOf(now - held),
      claimed_total: s(claimed), claiming_since: null, listed_price: null, vault: vault(i),
    });
    history[i] = h;
  }
  return { tokens, history };
}

function market(now, tokens, rnd) {
  const ids = new Set(tokens.map((t) => t.token_id));
  const listed = [8, 12, 19, 25, 31, 42, 47, 95, 98].filter((t) => ids.has(String(t)));
  const listings = listed.map((t) => ({ token_id: String(t), price: near(t === 42 ? 9.5 : 5.2 + rnd() * 6) }));
  for (const l of listings) tokens.find((t) => t.token_id === l.token_id).listed_price = l.price;
  const offers = [
    { buyer: 'kestrel.near', token_id: '42', amount: near(7.1), at: s(now - 10n * NS_HOUR) },
    { buyer: 'velvet.near', token_id: null, amount: near(5.35), at: s(now - 29n * NS_HOUR) },
    { buyer: YOU, token_id: '8', amount: near(6.2), at: s(now - 22n * NS_HOUR) },
    { buyer: 'gorm.near', token_id: '17', amount: near(6.4), at: s(now - 2n * NS_HOUR) },
  ]
    .filter((o) => o.token_id === null || ids.has(o.token_id))
    .map((o, id) => ({ id, ...o }));
  return { listings, offers };
}

// Proposals in creation order, so ids are 0-based like the contract's Vector index. A finalized bond is paid out,
// so its view shows 0. Strict kinds open only after the sale closed.
function council(now, tokens, history) {
  const draft = [
    { kind: { Order: { SetSlices: { cap_bps: 150 } } }, cls: 'Light', status: 'Executed', at: utc(16, 14), yes: 19, no: 3, description: 'Smaller slices while the pool is thin.', proposer: 'molly-m.near' },
    { kind: 'Signal', cls: 'Light', status: 'Carried', at: utc(17, 9), yes: 15, no: 2, description: 'Should the Lair add a leaderboard of the longest-held Scales?', proposer: 'tavi.near' },
    { kind: { Order: { SetPosture: { hunt: { sell_multiple_bps: 25000, max_sell_bps: 1000 } } } }, cls: 'Strict', status: 'Executed', at: utc(23, 13), yes: 30, no: 5, description: 'Turn Hunt on. The hoard is up 3x on cost.', proposer: 'velvet.near' },
    { kind: { Order: { Grant: { to: 'artist.near', amount: s(40_000n * WYRM) } } }, cls: 'Strict', status: 'Executed', at: utc(24, 10), yes: 30, no: 4, description: 'Pay the artist who drew the four Scale tiers.', proposer: 'ruby-scale.near' },
    { kind: { Order: { Grant: { to: 'lair-dev.near', amount: s(39_800n * WYRM) } } }, cls: 'Strict', status: { LeashHeld: { error: `grants in any 30 days are at most 100 bps of the hoard; ${40_000n * WYRM} already sent` } }, at: utc(25, 8), yes: 33, no: 2, description: 'Fund the Lair work for the next month.', proposer: 'gorm.near' },
    { kind: { Order: { Burn: { amount: s(20_000n * WYRM) } } }, cls: 'Light', status: 'Executed', at: utc(26, 18), yes: 21, no: 6, description: 'Burn a little of the hoard as a show of faith.', proposer: 'dune.near' },
    { kind: { Order: { SetBreath: { budget_bps: 1500 } } }, cls: 'Light', status: 'Open', at: utc(29, 14), yes: 14, no: 3, description: 'Breathe a little faster while volume is up.', proposer: 'molly-m.near' },
    { kind: { Order: { SetAutoBurn: { bps: 1000 } } }, cls: 'Light', status: 'Open', at: now - 20n * NS_HOUR, yes: 9, no: 4, description: 'Burn 10% of every buy.', proposer: 'saltmarsh.near' },
  ].filter((p) => p.at < now);
  const proposals = [];
  const votes = {};
  const mintedAt = (t) => BigInt(history[t.token_id][0].Minted.at);
  // A Scale votes when its holder at the proposal's creation had held it a season (scales council.rs, seasoned_at).
  const heldSince = (t, at) => history[t.token_id].map((e) => e.Minted ?? e.Transferred).filter((e) => e && BigInt(e.at) <= at).at(-1);
  for (const [id, p] of draft.entries()) {
    const voters = tokens.filter((t) => t.voting && BigInt(heldSince(t, p.at)?.at ?? p.at) + SEASON <= p.at && !(p.status === 'Open' && t.owner === YOU)).map((t) => t.token_id);
    if (voters.length < p.yes + p.no) throw new Error(`proposal ${id} wants ${p.yes + p.no} votes from ${voters.length} seasoned Scales`);
    const list = voters.slice(0, p.yes + p.no).map((token_id, k) => ({ token_id, approve: k < p.yes }));
    if (id === 6) list.push({ token_id: '42', approve: true });
    votes[id] = list.sort((a, b) => Number(a.token_id) - Number(b.token_id));
    proposals.push({
      id, kind: p.kind, class: p.cls, proposer: p.proposer, description: p.description, bond: p.status === 'Open' ? near(1) : '0',
      created_at: s(p.at), ends_at: s(p.at + VOTING), electorate: tokens.filter((t) => t.voting && mintedAt(t) <= p.at).length, yes: list.filter((v) => v.approve).length, no: list.filter((v) => !v.approve).length, status: p.status,
    });
  }
  return { proposals, votes };
}

function dragon(now, { bound, rnd, closed, opening, reserve }) {
  const REF = 300_000n * WYRM;
  const ratios = (shape) => {
    if (!bound) return [];
    const out = [];
    let r = 1;
    for (let k = 287; k >= 0; k--) {
      r = Math.min(1.06, Math.max(0.95, r + (rnd() - 0.5) * 0.012));
      let ratio = r;
      if (shape === 'dip' && k < 9) ratio = 0.77 + rnd() * 0.01;
      if (shape === 'breath' && k < 30) ratio = k < 10 ? 0.86 + rnd() * 0.01 : 0.79 + rnd() * 0.02;
      if (shape === 'hunt') ratio = 1.05 + ((287 - k) / 287) * 0.6 + (rnd() - 0.5) * 0.02;
      out.push(Math.round(ratio * 10_000) / 10_000);
    }
    return out;
  };
  // Every shape still takes its draws, so the seeded Scales and market after them stay.
  const R = { flat: ratios('flat'), dip: ratios('dip'), breath: ratios('breath'), hunt: ratios('hunt') };
  // The dip holds for the three samples the log says confirmed it, and the buy lands on the third.
  const [held, dip] = [R.breath.slice(-10, -5), R.dip.slice(-3)];
  const slide = [0.85, 0.83, 0.81];
  const lift = Math.round((dip[2] + 0.012) * 10_000) / 10_000;
  const recent = [...held, ...slide, ...dip, lift];
  const ease = (r) => r.map((v, i) => {
    const j = i - (r.length - recent.length);
    return j >= 0 ? recent[j] : v + (recent[0] - v) * Math.max(0, 1 + j / 72);
  });
  const tpn = (ratio) => s((REF * 10_000n) / BigInt(Math.round(ratio * 10_000)));
  const series = (r) => ease(r).map((ratio, i) => ({ at: s(now - BigInt((r.length - 1 - i) * 5) * NS_MIN), tokens_per_near: tpn(ratio) }));
  const samples = series(R.flat);
  // The price minutes ago, on the straight line between the two samples around it, as the Lair draws it.
  const tpnAgo = (minutes) => {
    const k = minutes / 5;
    const [older, newer] = [recent.at(-1 - Math.ceil(k)), recent.at(-1 - Math.floor(k))];
    return tpn(older + (newer - older) * (Math.ceil(k) - k));
  };
  // Only the closed world's Council turned Hunt on; Hunt on is Strict, and Strict needs a closed sale.
  const hunt = closed ? { max_sell_bps: 1000, sell_multiple_bps: 25000 } : null;
  const strategy = { auto_burn_bps: 0, budget_bps: 1000, cap_bps: closed ? 150 : 200, hunt };
  const hoard = {
    ash_due: '0', auto_burned: '0', burned: s(closed ? 20_000n * WYRM : 0n), donated: s(bound ? 12_400n * WYRM : 0n), granted: s(closed ? 40_000n * WYRM : 0n),
    near_spent: near(closed ? 26.13 : bound ? 11.8 : 0), realized_near: '0',
    wyrm: s(closed ? 7_900_000n * WYRM : bound ? 3_988_400n * WYRM : 0n), wyrm_bought: s(closed ? 7_947_600n * WYRM : bound ? 3_976_000n * WYRM : 0n),
  };
  const midnight = (now / NS_DAY) * NS_DAY;
  const breathAt = bound ? midnight + 13n * 60n * NS_MIN : 18_446_744_073_709_551_615n;
  const budget = (opening * BigInt(strategy.budget_bps)) / 10_000n;
  // The day's three slices fit its budget even before the failed one is refunded.
  const third = (budget / 3n / 10n ** 21n) * 10n ** 21n;
  const amount = s(third < BigInt(near(2.1)) ? third : BigInt(near(2.1)));
  const last = (x) => x.at(-1)?.tokens_per_near ?? null;
  const slice = (at, amountIn, tpn, extra = {}) => ({
    at: s(at), breath_at: s(breathAt), amount_in: amountIn, quoted_out: s((BigInt(amountIn) * BigInt(tpn)) / NEAR),
    min_out: s((((BigInt(amountIn) * BigInt(tpn)) / NEAR) * 9850n) / 10_000n), tokens_per_near: tpn, reference: s(REF), cap_bps: strategy.cap_bps, budget: s(budget), ...extra,
  });
  const ago = (m) => now - BigInt(m) * NS_MIN;
  const events = [[BOUND_AT - 4n * NS_HOUR, 'launch_sent', { ok: true }]];
  if (bound) events.push([BOUND_AT, 'bound', { launch_id: COIN.launch_id, pool_id: COIN.pool_id, token: COIN.token }]);
  if (closed) {
    const settled = (d, h) => utc(d, h) + 2n * NS_MIN;
    events.push(
      [settled(18, 14), 'settings_changed', { ...strategy, hunt: null }],
      [settled(25, 13), 'settings_changed', { ...strategy }],
      [settled(26, 10), 'grant_paid', { at: s(settled(26, 10)), to: 'artist.near', amount: s(40_000n * WYRM), hoard: s(8_000_000n * WYRM) }],
      [settled(27, 8), 'order_refused', { reason: `grants in any 30 days are at most 100 bps of the hoard; ${40_000n * WYRM} already sent` }],
      [settled(28, 18), 'burn_paid', { at: s(settled(28, 18)), amount: s(20_000n * WYRM), hoard: s(7_940_000n * WYRM) }],
    );
  }
  if (bound)
    events.push(
      [ago(43), 'breath_slice', slice(ago(43), amount, tpnAgo(43))],
      [ago(40), 'swap_failed', { at: s(ago(40)), slice: 'Breath', amount_in: amount, reason: 'refunded' }],
      [ago(37), 'breath_slice', slice(ago(37), amount, tpnAgo(37))],
      [ago(5), 'dip_slice', slice(ago(5), amount, tpnAgo(5), { step_bps: 2000, confirmations: 3, confirmed_since: s(ago(15)) })],
    );
  const log = events.map(([at, event, data], seq) => ({ at: s(at), data, event, seq: String(seq) }));
  const day = (extra = {}) => ({ breath_at: s(breathAt), budget: s(budget), index: s(now / NS_DAY), opening_reserve: s(opening), sold_wyrm: '0', spent: '0', ...extra });
  const base = (mood, extra = {}) => ({
    coin: bound ? COIN : null, day: day(), dip: null, hoard, last_launch_at: s(BOUND_AT - 4n * NS_HOUR), last_slice_at: bound ? s(ago(5)) : '0',
    log_next: String(log.length), mood, pending: [], phase: 'Idle', reference: bound ? s(REF) : null, reserve, reserve_near: reserve, reserve_wnear: '0',
    samples: samples.length, spot: last(samples), strategy, ...extra,
  });
  // One state per world, keyed by the mood the Lair reads from the log at `now`: a buy 5 minutes ago is Breathing,
  // and no sell is ever logged, so no world is Hunting. That buy's swap is done, and the dip that took it is still
  // confirmed.
  const states = bound
    ? {
        Breathing: {
          state: base('Breathing', { day: day({ spent: s(budget / 3n) }), dip: { confirmed: true, first_at: s(ago(15)), samples: 3, step: 1, step_bps: 2000 } }),
          samples,
        },
      }
    : { Sleeping: { state: base('Sleeping'), samples: [] } };
  return { states, log, coin: bound ? COIN : null };
}

const PRESETS = {
  hoarder: { sale: { creator_bps: CREATOR_BPS, length: Number(14n * NS_DAY), price: s(PRICE) }, times: { season: Number(SEASON), voting: Number(VOTING) }, network: 'Any' },
  drill: { sale: { creator_bps: 1000, length: Number(2n * NS_HOUR), price: near(0.1) }, times: { season: Number(10n * NS_MIN), voting: Number(30n * NS_MIN) }, network: 'TestnetOnly' },
};
const bounds = { auto_burn_bps: { hi: 5000, lo: 0 }, budget_bps: { hi: 4000, lo: 1000 }, cap_bps: { hi: 200, lo: 50 }, max_sell_bps: { hi: 1500, lo: 1 }, sell_multiple_min_bps: 20000 };

// The views that do not change with the sale state; mock.js swaps in the preset's values for ?preset=drill.
const fixed = {
  presets: PRESETS,
  council: {
    bond: s(NEAR), bounds,
    classes: [
      { class: 'Light', min_electorate: 10, needs_closed_sale: false, rule: '(yes + no) * 10 >= electorate * 3 and yes > no' },
      { class: 'Strict', min_electorate: 45, needs_closed_sale: true, rule: '(yes + no) * 3 >= electorate * 2 and yes * 5 >= (yes + no) * 4' },
    ],
    finalize_tgas: 150,
    kinds: [['SetBreath', 'Light'], ['SetSlices', 'Light'], ['SetPosture on', 'Strict'], ['SetPosture off', 'Light'], ['Grant', 'Strict'], ['Burn', 'Light'], ['SetAutoBurn', 'Light'], ['Signal', 'Light']].map(([kind, cls]) => ({ class: cls, kind })),
    preset: 'hoarder', season_ns: s(SEASON), storage_fee: s(10n ** 22n), voting_ns: s(VOTING),
  },
  scalesConfig: {
    claim_timeout_ns: s(10n * NS_MIN), creator: CREATOR, dragon: DRAGON, engine_version: 200, escrow: near(25.1), preset: 'hoarder',
    preset_hash: '6kx9i6kbBhmUsJBvxJdJVG1gRtdd2sg51c5JsxxHqpPE', royalty_bps: 500, royalty_receiver: CREATOR, season_ns: s(SEASON),
    storage_fee: s(10n ** 22n), tithe: TITHE, vault_code_hash: 'CfrWztMxwXp1nqykVyaDZk9oYvZJ27JHqH6BCQ3dSxus', vaults_created: 100,
  },
  dragonConfig: {
    engine: {
      dip_samples: 3, dip_span_ns: s(10n * NS_MIN), launch_margin: s(86n * 10n ** 21n), outflow_bps: 50, outflow_window_bps: 100, outflow_window_ns: s(30n * NS_DAY),
      slippage_bps: 150, spike_bps: 1500, stale_ns: s(5n * NS_MIN), tax: { burn_bps: 0, buy_bps: 400, creator_bps: 10000, holders_bps: 0, sell_bps: 400 },
    },
    engine_version: 200,
    peers: { ash: `ash.${root}`, creator: CREATOR, dcl: 'dclv2.ref-labs.near', nearly: 'nearlytrade.near', scales: `scales.${root}`, tithe: TITHE, wrap: 'wrap.near' },
    preset: 'hoarder',
    preset_hash: '6kx9i6kbBhmUsJBvxJdJVG1gRtdd2sg51c5JsxxHqpPE',
    preset_value: {
      bounds, cadence: { gap_seconds: 60, spread_minutes: 0 }, features: { auto_burn: true, burn: true, grant: true, hunt: true },
      ladder: { extra_bps: [0, 0, 0], steps_bps: [2000, 3500, 5000] }, name: 'hoarder', royalty_to: 'Creator',
      strategy: { auto_burn_bps: 0, budget_bps: 1000, cap_bps: 200, hunt: null }, ...PRESETS.hoarder,
    },
  },
};

// waiting: the launch is sent and not bound. open: a day after bind, before any Scale is seasoned.
// closed: a week after the 14-day sale closed with 50 sold.
// The Dragon's reserve at the day's opening is its share of every Scale sold plus `held`, the NEAR it holds from
// everything else; `out` has left it since the opening.
export const SCENARIOS = {
  waiting: { now: utc(9, 8), sold: 0, held: 1.2, out: 0, sale: { state: 'Waiting' } },
  open: { now: utc(10, 15, 4), sold: 23, held: 14.5, out: 1.8, sale: { state: 'Open', opened_at: s(BOUND_AT), ends_at: s(SALE_ENDS) } },
  closed: { now: utc(30, 15, 4) + NS_DAY, sold: 50, held: 195, out: 18.4, sale: { state: 'Closed', retired_from: 51, tithe_told: true } },
};
const openingOf = (sc) => (BigInt(sc.sold) * PRICE * BigInt(10_000 - CREATOR_BPS)) / 10_000n + BigInt(near(sc.held));
const reserveOf = (sc) => openingOf(sc) - BigInt(near(sc.out));

function world(name, sc, seed) {
  const rnd = rng(seed);
  const bound = name !== 'waiting';
  const { tokens, history } = collection(sc.now, sc.sold, rnd);
  // An unbound Dragon has not observed a day, so its opening reserve is 0.
  const d = dragon(sc.now, { bound, rnd, closed: name === 'closed', opening: bound ? openingOf(sc) : 0n, reserve: s(reserveOf(sc)) });
  const sale = { creator_bps: CREATOR_BPS, ends_at: null, opened_at: null, price: s(PRICE), retired_from: null, sold: sc.sold, tithe_told: false, ...sc.sale };
  return {
    now: s(sc.now), you: YOU,
    scales: { tokens, history },
    market: bound ? market(sc.now, tokens, rnd) : { listings: [], offers: [] },
    council: name === 'closed' ? council(sc.now, tokens, history) : { proposals: [], votes: {} },
    dragon: d,
    sale,
  };
}

const accounts = {};
const rnd = rng(7);
const codeHash = { tithe: 'E3x8RkV3gqW1pZ9bLmT2sYcN4dF6hJ7kQ5vA8uB2wC1', dragon: '9fT2hQ8rLm4nB6vX1cZ3kP7wJ5sD8gY2aE4uR6tN9oV', scales: '4KpL9wN2xR7tB3vM8cQ1zF6hJ5sG4dY2aE9uT7nP3oW', vault: 'Hq3T8vN1mK6pR2xB9cW4zL7fJ5sD3gY8aE1uQ6tM2oV' };
accounts[TITHE] = { amount: near(3.71), code_hash: codeHash.tithe, keys: [] };
accounts[DRAGON] = { amount: s(reserveOf(SCENARIOS.closed)), code_hash: codeHash.dragon, keys: [] };
accounts[`scales.${root}`] = { amount: near(48.2), code_hash: codeHash.scales, keys: [] };
accounts[`ash.${root}`] = { amount: near(0.0021), code_hash: '11111111111111111111111111111111', keys: [] };
for (let i = 1; i <= 100; i++) {
  accounts[vault(i)] = { amount: s(BigInt(near(0.01)) + BigInt(near(0.08 + rnd() * 1.1))), code_hash: '11111111111111111111111111111111', global_contract_hash: codeHash.vault, keys: [] };
}

const files = {
  'accounts.json': accounts,
  'fixed.json': fixed,
  ...Object.fromEntries(Object.entries(SCENARIOS).map(([name, sc], i) => [`world-${name}.json`, world(name, sc, 42 + i)])),
};
if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  for (const [name, data] of Object.entries(files)) writeFileSync(new URL(name, dir), JSON.stringify(data) + '\n');
  copyFileSync(new URL('../../gym/out/estimator.json', import.meta.url), new URL('estimator.json', dir));
  console.log('wrote', Object.keys(files).join(', '), 'and copied estimator.json');
}
