import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { CHECKS, impactOk, orderViolation, runCheck, sliceRuleBreak, wyrmAccounts } from "./checks.mjs";
import { replaying, view } from "../keeper/chain.mjs";

const NEAR = 10n ** 24n;
const DAY = 86_400n * 10n ** 9n;
const H = wyrmAccounts("thewyrm.test.near");
const A = wyrmAccounts("thewyrm.near");
const read = (url) => JSON.parse(fs.readFileSync(url));
const fixture = (name) => read(new URL(`./fixtures/${name}`, import.meta.url));
// Nearly's real JSON on mainnet (launch 3657, not ours), recorded by V2 for the mocks.
const mainnet = (name) => read(new URL(`../contracts/mock-nearly/fixtures/${name}.json`, import.meta.url)).result;
const check = (id) => CHECKS.find((c) => c.id === id);
const judge = (id, data, accounts = H) => {
  const { items, summary } = check(id).judge(data, accounts);
  return { pass: items.every((i) => i.pass), fails: items.filter((i) => !i.pass).map((i) => i.evidence), oks: items.filter((i) => i.pass).map((i) => i.evidence), summary };
};

// A verify context over the recorded hold world: RPC answers replayed, history from the recorded receipts.
const VIEWS = fixture("hold-world-views.json");
const RECEIPTS = fixture("hold-world-receipts.json")
  .map((r) => ({ ...r, at: BigInt(r.at), actions: r.actions.map((a) => (a.deposit === undefined ? a : { ...a, deposit: BigInt(a.deposit) })) }));
const SOURCE = read(new URL("./releases.json", import.meta.url)).source;
function holdWorld({ network = "sandbox", releases } = {}) {
  const rpc = replaying(VIEWS);
  const ctx = {
    rpc, accounts: H, opts: { network }, cache: new Map(),
    config: () => view(rpc, H.dragon, "get_config"),
    history: async () => RECEIPTS,
    releases: () => releases ?? [],
  };
  return ctx;
}
const codeHashes = async () => {
  const ctx = holdWorld();
  const code = async (a) => { const v = await ctx.rpc("query", { request_type: "view_account", finality: "final", account_id: a }); return v.global_contract_hash ?? v.code_hash; };
  return { tithe: await code(H.tithe), dragon: await code(H.dragon), scales: await code(H.scales), scale_vault: await code(H.vaults[0]) };
};
const localBuild = async () => [{ name: "local build", ...SOURCE, code: await codeHashes() }];
const run = async (id, ctx) => runCheck(check(id), ctx);
const config = await holdWorld().config();

test("on the recorded hold world checks 1, 2, 3, 4 and 12 pass against its local build", async () => {
  const ctx = holdWorld({ releases: await localBuild() });
  for (const id of [1, 2, 3, 4, 12]) {
    const r = await run(id, ctx);
    assert.ok(r.pass, `check ${id}: ${r.summary} ${r.items.filter((i) => !i.pass).map((i) => i.evidence).join("; ")}`);
  }
});

test("check 2 fails code no release holds, and names drill on a mainnet system", async () => {
  const none = await run(2, holdWorld());
  assert.equal(none.pass, false);
  assert.match(none.items.find((i) => !i.pass).evidence, /no release in verify\/releases.json has that dragon/);
  const data = await check(2).read(holdWorld({ network: "mainnet", releases: await localBuild() }));
  assert.equal(judge(2, data).pass, true, "hoarder on mainnet passes");
  const drill = { ...data, config: { ...data.config, preset: "drill", preset_value: { ...data.config.preset_value, network: "TestnetOnly" } } };
  assert.match(judge(2, drill).fails.join(), /mainnet system runs the drill preset/);
  assert.equal(judge(2, { ...drill, network: "testnet" }).fails.some((f) => /drill preset/.test(f)), false, "drill is fine on testnet");
});

test("check 2 fails a preset hash or engine version the release does not hold, and a mismatch between dragon and scales", async () => {
  const data = await check(2).read(holdWorld({ releases: await localBuild() }));
  assert.match(judge(2, { ...data, scalesConfig: { ...data.scalesConfig, preset_hash: "X" } }).fails.join(), /preset_hash: dragon .*, scales X/);
  const other = [{ ...data.releases[0], presets: { ...data.releases[0].presets, hoarder: "Y" }, engine_version: 201 }];
  assert.deepEqual(judge(2, { ...data, releases: other }).fails, [`hoarder hash ${config.preset_hash}, local build holds Y`, "engine_version 200, local build is 201"]);
});

test("check 1 fails a key, a missing account and ash with code", () => {
  const rows = [
    { account: H.tithe, view: { code_hash: "T" }, keys: [{ public_key: "ed25519:K", access_key: { permission: "FullAccess" } }] },
    { account: H.dragon, view: null, keys: null },
    { account: "ash.thewyrm.test.near", view: { code_hash: "C" }, keys: [] },
    { account: H.scales, view: { code_hash: "S" }, keys: [] },
  ];
  assert.deepEqual(judge(1, { ash: "ash.thewyrm.test.near", rows }).fails, [`${H.tithe} has 1 key(s): ed25519:K FullAccess`, `${H.dragon} does not exist`, "ash.thewyrm.test.near runs code C; ash has none"]);
});

test("check 3 names the coin from get_coin and lists the orphan launch as not bound", async () => {
  const data = await check(3).read(holdWorld());
  const r = judge(3, data);
  assert.ok(r.pass);
  assert.equal(r.summary, "the coin is launch 1 (WYRM, token wyrm-2.nearly.test.near), from get_coin");
  assert.ok(r.oks.includes("launch 0 (WYRM, token wyrm.nearly.test.near) is an orphan launch by the dragon, not bound"));
});

test("check 3 fails an unbound dragon, a launch by someone else, and a fee_to elsewhere", () => {
  assert.deepEqual(judge(3, { coin: null }).fails, ["the dragon is not bound: get_coin is null"]);
  const record = mainnet("get_launch");
  const coin = { launch_id: "3657", token: record.token, pool_id: record.pool_id };
  assert.deepEqual(judge(3, { coin, record, feeTo: mainnet("get_fee_to"), bySymbol: record }).fails, [
    `launch 3657 (BIGREEN, token bigreen.nearlytrade.near) was launched by blueisgreen.near, not ${H.dragon}`, `fee_to null, not ${H.tithe}`,
  ]);
});

test("check 3 fails when get_coin's token or pool is not the launch record's", async () => {
  const data = await check(3).read(holdWorld());
  const { token, pool_id } = data.coin;
  assert.deepEqual(judge(3, { ...data, coin: { ...data.coin, token: "other.test.near" } }).fails, [`get_coin names other.test.near / ${pool_id}, the record ${token} / ${pool_id}`]);
  assert.deepEqual(judge(3, { ...data, coin: { ...data.coin, pool_id: "P" } }).fails, [`get_coin names ${token} / P, the record ${token} / ${pool_id}`]);
});

test("check 4 passes the hold world and fails Nearly's real tax record of another launch", async () => {
  const data = await check(4).read(holdWorld());
  assert.ok(judge(4, data).pass);
  assert.deepEqual(judge(4, { ...data, tax: mainnet("get_tax") }).fails, ["buy_bps 100, expected 400", "sell_bps 100, expected 400", "creator_bps 3300, expected 10000", "burn_bps 3300, expected 0", "holders_bps 3400, expected 0"]);
  assert.match(judge(4, { ...data, pool: mainnet("get_pool") }).fails[0], /Running with bigreen.nearlytrade.near wrap.near, expected Running with wrap.test.near wyrm-2/);
  assert.match(judge(4, { ...data, feeOpts: { mode: "burn" } }).fails[0], /fee mode burn/);
});

const ev = (event, ...data) => ({ standard: "wyrm", version: "1.0.0", event, data });
const rc = (tx, predecessor, receiver, { actions = [], events = [], ok = true, value = null } = {}) => ({ tx, id: `${tx}:${receiver}`, predecessor, receiver, actions, events, ok, value, logs: [] });
const transfer = (deposit) => ({ kind: "Transfer", deposit: BigInt(deposit) });
const call = (method, args, deposit = 0n) => ({ kind: "FunctionCall", method, args, deposit });

// A tithe round of `free` yocto, paying no vault from `retiredFrom` to 90.
function round({ free = 100n * NEAR + 37n, retiredFrom = null, skipVault = null, extra = [], tamper = 0n } = {}) {
  const half = free / 2n;
  const perScale = (free - half) / 100n;
  const dust = free - half - perScale * 100n;
  const retired = retiredFrom === null ? 0n : BigInt(91 - retiredFrom);
  const toDragon = half + perScale * retired + tamper;
  const paid = A.vaults.filter((_, i) => retiredFrom === null || i + 1 < retiredFrom || i + 1 >= 91);
  return [
    rc("R1", "keeper.near", A.tithe, { actions: [call("distribute", {})], events: [ev("tithe_round", { round: 1, to_dragon: String(toDragon), per_scale: String(perScale), dust: String(dust) })] }),
    rc("R1", A.tithe, A.dragon, { actions: [transfer(toDragon)] }),
    ...paid.filter((v) => v !== skipVault).map((v) => rc("R1", A.tithe, v, { actions: [transfer(perScale)] })),
    ...extra,
  ];
}
const stats = (retired_from) => ({ retired_from });

test("check 5 passes a round before retirement and one after it, reading retired_from from get_stats", () => {
  assert.equal(judge(5, { receipts: round(), stats: stats(null) }, A).pass, true);
  assert.equal(judge(5, { receipts: round(), stats: stats(46) }, A).pass, true, "a round from before the retirement");
  const after = judge(5, { receipts: round({ retiredFrom: 46 }), stats: stats(46) }, A);
  assert.ok(after.pass, after.fails.join());
  assert.match(after.oks[0], /to each of 55 vaults/);
});

test("check 5 fails a skipped vault, a paid retired vault, a dragon share off by one, and a missing field", () => {
  assert.match(judge(5, { receipts: round({ skipVault: A.vaults[41] }), stats: stats(null) }, A).fails[0], /042\.scales\.thewyrm\.near got nothing/);
  const paidRetired = round({ retiredFrom: 46, extra: [rc("R1", A.tithe, A.vaults[59], { actions: [transfer(1n)] })] });
  assert.match(judge(5, { receipts: paidRetired, stats: stats(46) }, A).fails[0], /retired 060\.scales/);
  assert.match(judge(5, { receipts: round({ tamper: 2n }), stats: stats(null) }, A).fails[0], /neither half nor half plus/);
  assert.match(judge(5, { receipts: round({ retiredFrom: 46 }), stats: stats(null) }, A).fails[0], /neither half/, "retired shares with no retirement on record");
  assert.match(judge(5, { receipts: [rc("R2", "k", A.tithe, { events: [ev("tithe_round", { round: "2" })] })], stats: stats(null) }, A).fails[0], /no "to_dragon" field/);
});

test("checks 6, 7, 8, 13 and 14 pass the recorded hold world's trades", async () => {
  const ctx = holdWorld();
  for (const id of [6, 7, 8, 13, 14, 15]) {
    const r = await run(id, ctx);
    assert.ok(r.pass, `check ${id}: ${r.summary} ${r.items.filter((i) => !i.pass).map((i) => i.evidence).join("; ")}`);
  }
  assert.match((await run(6, ctx)).summary, /2 buys \(2 breath, 0 dip\), 2\/2 re-derived by gym\/dragon-rules.mjs on the hoarder cadence/);
  assert.match((await run(14, ctx)).items[0].evidence, /net 13778324116489439840505 \(96\.00%\), tax 574096838187059993354/);
});

const T0 = 20_000n * DAY;
const engine = config.engine;
const breath = (o = {}) => ({ at: String(T0 + 3600n * 10n ** 9n), breath_at: String(T0 + 600n * 10n ** 9n), amount_in: String(NEAR), quoted_out: "990", min_out: "975",
  tokens_per_near: "1000", reference: "1000", cap_bps: 250, budget: String(10n * NEAR), ...o });
const dip = (o = {}) => breath({ step_bps: 2000, reference: "800", tokens_per_near: "1000", confirmations: 3, confirmed_since: String(T0), ...o });
const slice = (name, d) => ({ name, data: d, receipt: { tx: "S" } });

test("a buy slice breaks its rule when it buys a spike, before the breath, on a shallow dip or an unconfirmed one", () => {
  assert.equal(sliceRuleBreak(slice("breath_slice", breath()), engine), null);
  assert.equal(sliceRuleBreak(slice("breath_slice", breath({ reference: null })), engine), null, "no reference, no spike guard");
  assert.equal(sliceRuleBreak(slice("dip_slice", dip()), engine), null);
  const cases = [
    [slice("breath_slice", breath({ at: String(T0 + 60n * 10n ** 9n) })), /before the breath minute/],
    [slice("breath_slice", breath({ breath_at: String(T0 - DAY + 1n) })), /not on the slice's UTC day/],
    [slice("breath_slice", breath({ reference: "1200", tokens_per_near: "1000" })), /spike/],
    [slice("dip_slice", dip({ reference: "820" })), /dip step 2000 bps not reached/],
    [slice("dip_slice", dip({ confirmations: 2 })), /fewer than 3/],
    [slice("dip_slice", dip({ confirmed_since: String(T0 + 3600n * 10n ** 9n - 599n * 10n ** 9n) })), /under 600 s/],
    [slice("dip_slice", dip({ reference: null })), /without a reference/],
  ];
  for (const [ev, why] of cases) assert.match(sliceRuleBreak(ev, engine), why);
});

test("the price-impact cap admits exactly cap_bps of impact and refuses one unit more", () => {
  const d = { amount_in: String(2n * NEAR), tokens_per_near: "10000", cap_bps: 250 };
  assert.equal(impactOk({ ...d, quoted_out: "19500" }), true);
  assert.equal(impactOk({ ...d, quoted_out: "19499" }), false);
});

const peers = config.peers;
function buy(tx, d, sent = BigInt(d.amount_in)) {
  return [
    rc(tx, H.dragon, H.dragon, { events: [ev("breath_slice", d)] }),
    rc(tx, H.dragon, peers.wrap, { actions: [call("ft_transfer_call", { receiver_id: peers.dcl, amount: String(sent), msg: "{}" }, 1n)] }),
  ];
}

test("check 7 sums the wNEAR really sent to DCL per UTC day against the budget", () => {
  const day = [...buy("S1", breath()), ...buy("S2", breath({ at: String(T0 + 7200n * 10n ** 9n) }))];
  assert.equal(judge(7, { receipts: day, config }).pass, true);
  const over = [...day, ...buy("S3", breath({ amount_in: String(9n * NEAR), quoted_out: "8900" }))];
  assert.match(judge(7, { receipts: over, config }).fails.join(), /spent 11000000000000000000000000, budget 10000000000000000000000000/);
  assert.match(judge(7, { receipts: buy("S4", breath({ quoted_out: "974" })), config }).fails.join(), /exceeds 250 bps impact/);
  assert.match(judge(7, { receipts: buy("S5", breath(), 2n * NEAR), config }).fails.join(), /logged 1000000000000000000000000, sent 2000000000000000000000000/);
});

test("check 8 fails a receipt to a stranger, a call to the tithe beyond get_config, and a call to the creator", () => {
  const fine = RECEIPTS;
  assert.equal(judge(8, { receipts: fine, config }).pass, true);
  const bad = [rc("Y", H.dragon, "alice.near", { actions: [transfer(1n)] }), rc("Z", H.dragon, H.tithe, { actions: [call("distribute", {})] }), rc("W", H.dragon, peers.creator, { actions: [call("x", {})] })];
  assert.deepEqual(judge(8, { receipts: [...fine, ...bad], config }).fails, [
    "receipt Y:alice.near tx Y to alice.near: Transfer", `receipt Z:${H.tithe} tx Z to ${H.tithe}: distribute`, `receipt W:${peers.creator} tx W to ${peers.creator}: x`,
  ]);
});

test("orderViolation is wyrm-types Order::check against the preset", () => {
  const p = config.preset_value;
  assert.equal(orderViolation({ SetSlices: { cap_bps: 200 } }, p), null);
  assert.equal(orderViolation({ SetSlices: { cap_bps: 201 } }, p), "cap_bps must be 50 to 200, got 201");
  assert.equal(orderViolation({ SetBreath: { budget_bps: 999 } }, p), "budget_bps must be 1000 to 4000, got 999");
  assert.equal(orderViolation({ SetPosture: { hunt: null } }, p), null);
  assert.equal(orderViolation({ SetPosture: { hunt: { sell_multiple_bps: 19_999, max_sell_bps: 100 } } }, p), "sell_multiple_bps must be at least 20000, got 19999");
  assert.equal(orderViolation({ SetPosture: { hunt: { sell_multiple_bps: 25_000, max_sell_bps: 1501 } } }, p), "max_sell_bps must be 1 to 1500, got 1501");
  assert.equal(orderViolation({ Grant: { to: "g.near", amount: "0" } }, p), "amount must be above 0");
  assert.equal(orderViolation({ SetAutoBurn: { bps: 5001 } }, p), "auto_burn_bps must be 0 to 5000, got 5001");
  assert.equal(orderViolation({ Burn: { amount: "5" } }, { ...p, features: { ...p.features, burn: false } }), "Burn is disabled in the hoarder preset");
});

// The hold world's proposals as get_proposals returned them.
const PROPOSALS = [
  { id: 0, kind: { Order: { SetSlices: { cap_bps: 100 } } }, status: "Executed" },
  { id: 1, kind: { Order: { SetPosture: { hunt: { sell_multiple_bps: 25000, max_sell_bps: 1000 } } } }, status: "Rejected" },
  { id: 2, kind: "Signal", status: "Carried" },
  { id: 3, kind: { Order: { Grant: { to: "nobody.test.near", amount: "1000000000000000000" } } }, status: { LeashHeld: { error: "transfer failed" } } },
  { id: 4, kind: { Order: { Burn: { amount: "100000000000000000000" } } }, status: "Open" },
];
const council = (o = {}) => ({ config, scalesConfig: { dragon: H.dragon }, proposals: PROPOSALS, receipts: RECEIPTS, ...o });

test("check 9 passes the hold world's Council: the executed order fits the preset and the leash hold has its outflow_failed", () => {
  const r = judge(9, council());
  assert.ok(r.pass, r.fails.join());
  assert.ok(r.oks.some((e) => /proposal 0 went to Executing in finalize tx/.test(e)));
  assert.ok(r.oks.some((e) => /proposal 3 Grant held by the leash \(transfer failed\), outflow_failed in tx/.test(e)));
});

test("check 9 fails an executed order outside the preset, an execution without its finalize, and a hold without a refusal", () => {
  const swapped = PROPOSALS.map((p) => (p.id === 0 ? { ...p, kind: { Order: { SetSlices: { cap_bps: 300 } } } } : p));
  assert.match(judge(9, council({ proposals: swapped })).fails[0], /proposal 0 SetSlices executed although cap_bps must be 50 to 200, got 300/);
  const unfinalized = [...PROPOSALS, { id: 9, kind: { Order: { SetSlices: { cap_bps: 100 } } }, status: "Executed" }];
  assert.deepEqual(judge(9, council({ proposals: unfinalized })).fails, ["proposal 9 has no finalize to Executing"]);
  const noRefusal = RECEIPTS.map((r) => ({ ...r, ok: true, events: r.events.filter((e) => e.event !== "outflow_failed") }));
  assert.match(judge(9, council({ receipts: noRefusal })).fails[0], /proposal 3 Grant held by the leash \(transfer failed\) with no order_refused/);
  assert.match(judge(9, council({ scalesConfig: { dragon: "other.near" } })).fails[0], /scales' dragon is other.near/);
});

function collection({ sold = 45, retired_from = 46, mutate = () => {} } = {}) {
  const ids = [...Array.from({ length: sold }, (_, i) => i + 1), ...Array.from({ length: 10 }, (_, i) => 91 + i)];
  const scales = ids.map((i) => ({ token_id: String(i), owner: i > 90 ? "creator.near" : "x.near", voting: i <= 90, vault: A.vaults[i - 1] }));
  const data = { supply: String(scales.length), scales, sale: { sold, retired_from, state: "Closed" }, creator: "creator.near", parents: A.vaults.map(() => A.scales) };
  mutate(data);
  return data;
}

test("check 10 passes a closed sale and fails a voting creator Scale, a minted retired id, a foreign vault and a missing vault", () => {
  assert.ok(judge(10, collection(), A).pass);
  assert.deepEqual(judge(10, collection({ mutate: (d) => { d.scales.at(-1).voting = true; } }), A).fails, ["creator Scale 100 has voting true"]);
  assert.deepEqual(judge(10, collection({ mutate: (d) => { d.scales.push({ token_id: "50", voting: true, vault: A.vaults[49] }); d.supply = "56"; } }), A).fails, ["Scale 50 is minted, but the sale retired ids from 46"]);
  assert.deepEqual(judge(10, collection({ mutate: (d) => { d.scales[0].vault = "001.scales.evil.near"; } }), A).fails, ["Scale 1 vault 001.scales.evil.near, expected 001.scales.thewyrm.near"]);
  assert.deepEqual(judge(10, collection({ mutate: (d) => { d.parents[99] = null; } }), A).fails, ["100.scales.thewyrm.near does not exist"]);
  assert.deepEqual(judge(10, collection({ mutate: (d) => { d.scales.splice(50, 1); d.supply = "54"; } }), A).fails, ["creator Scale 96 is not minted"]);
});

test("check 11 passes the hold world's sales and fails a missing creator cut or vault sweep", () => {
  const sale = { creator_bps: 1000 };
  const r = judge(11, { receipts: RECEIPTS, sale, config });
  assert.ok(r.pass, r.fails.join());
  assert.match(r.summary, /^3 sales, 6000000000000000000000000 yocto/);
  const noCut = RECEIPTS.filter((x) => !(x.predecessor === H.scales && x.receiver === peers.creator));
  assert.match(judge(11, { receipts: noCut, sale, config }).fails[0], /200000000000000000000000 not sent to creator\.thewyrm\.test\.near/);
  const noSweep = RECEIPTS.map((x) => ({ ...x, events: x.events.filter((e) => e.event !== "scale_swept") }));
  assert.match(judge(11, { receipts: noSweep, sale, config }).fails[0], /no scale_swept/);
});

test("check 12 holds when the bound launch's creator, who receives the dev buy, is the dragon", () => {
  const record = { id: 1, symbol: "WYRM", token: "w.near", creator: H.dragon, dev_buy_tokens: "5" };
  assert.ok(judge(12, { coin: { launch_id: "1" }, record }).pass);
  assert.match(judge(12, { coin: { launch_id: "1" }, record: { ...record, creator: "dev.near" } }).fails[0], /creator dev.near, so its dev buy goes there/);
});

test("check 13 holds grants and burns each to 50 bps a time and 100 bps per rolling 30 days, in separate windows", () => {
  const paid = (name, tx, at, amount, hoard = 10_000n) => rc(tx, H.dragon, H.dragon, { events: [ev(name, { at: String(at), amount: String(amount), hoard: String(hoard) })] });
  const g = (tx, at, amount) => paid("grant_paid", tx, at, amount);
  const b = (tx, at, amount) => paid("burn_paid", tx, at, amount);
  assert.ok(judge(13, { receipts: [g("G1", 0n, 50n), g("G2", 10n * DAY, 50n), b("B1", 11n * DAY, 50n), b("B2", 12n * DAY, 50n)], config }).pass, "each kind has its own window");
  assert.match(judge(13, { receipts: [b("B1", 0n, 51n)], config }).fails[0], /burn tx B1: 51 is above 50 bps/);
  assert.match(judge(13, { receipts: [g("G1", 0n, 50n), g("G2", 10n * DAY, 50n), g("G3", 29n * DAY, 1n)], config }).fails[0], /G3: 30-day total 101 above 100 bps/);
  assert.ok(judge(13, { receipts: [g("G1", 0n, 50n), g("G2", 10n * DAY, 50n), g("G3", 30n * DAY, 50n)], config }).pass, "a grant 30 days old leaves the window");
});

test("checks 14 and 15 split the taxed swaps into buys and sells", () => {
  assert.match(judge(14, { receipts: RECEIPTS, config, kind: "buy" }).summary, /^2 buys against a 400 bps buy tax/);
  assert.match(judge(15, { receipts: RECEIPTS, config, kind: "sell" }).summary, /^0 hunt sells against a 400 bps sell tax/);
  const untaxed = RECEIPTS.map((r) => ({ ...r, events: r.events.map((e) => (e.standard === "nep141" ? { ...e, data: e.data.filter((m) => m.memo !== "tax") } : e)) }));
  assert.match(judge(14, { receipts: untaxed, config, kind: "buy" }).fails[0], /tax 0 in 0 transfer\(s\), expected one of 574096838187059993354/);
});

test("a check whose read throws fails with the reason instead of crashing the run", async () => {
  const r = await runCheck({ id: 99, title: "t", read: async () => { throw new Error("HTTP 503"); }, judge: () => assert.fail() }, { accounts: A });
  assert.deepEqual([r.pass, r.summary], [false, "could not read: HTTP 503"]);
  const shape = await runCheck({ id: 98, title: "t", read: async () => ({}), judge: () => { throw new TypeError("x is undefined"); } }, { accounts: A });
  assert.deepEqual([shape.pass, shape.summary], [false, "unexpected data shape: x is undefined"]);
});
