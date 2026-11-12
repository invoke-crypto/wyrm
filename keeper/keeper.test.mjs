import { test } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import { GAS, MAX_TRIES, STEPS, TICK_REQUESTS, VAULT_READS, accountsFor, advancePass, checkCredentials, classify, dueSteps, newState, plan, readSnapshot, readVaults, signingKey, tick } from "./keeper.mjs";
import { b58decode, b58encode, httpRpc, parseSecretKey, replaying, signedFunctionCall } from "./chain.mjs";

const TGAS = 10n ** 12n;
const NEAR = 10n ** 24n;
const accounts = accountsFor("thewyrm.near");
const peers = { nearly: "nearlytrade.near", wrap: "wrap.near", dcl: "dclv2.ref-labs.near", ash: "ash.thewyrm.near", creator: "thewyrm.near" };
const coin = { launch_id: "3712", token: "wyrm.nearlytrade.near", pool_id: "wrap.near|wyrm.nearlytrade.near|10000", wrap_is_x: true };
const quiet = {
  coin, peers, launch: null, tithe: { wnear: 0n, wyrm: 0n }, due: [], vaultsCreated: 100,
  sale: { state: "Closed", tithe_told: true }, sweepable: [],
};
const calls = (s, steps = STEPS) => plan({ ...quiet, ...s }, steps, accounts).map((a) => `${a.method}${a.args.proposal_id !== undefined ? `(${a.args.proposal_id})` : ""}`);
const fixture = (name) => JSON.parse(fs.readFileSync(new URL(`../verify/fixtures/${name}`, import.meta.url)));

test("a bound Dragon is poked every minute at 1,000 Tgas, above the 760 Tgas floor", () => {
  const [poke] = plan(quiet, ["minute"], accounts);
  assert.deepEqual([poke.receiver, poke.method, poke.gas], [accounts.dragon, "poke", 1_000n * TGAS]);
  assert.ok(poke.gas >= 760n * TGAS);
});

test("before bind there is no poke, and a parked launch by the Dragon is resumed with 300 Tgas", () => {
  const parked = { coin: null, launch: { id: "7", creator: accounts.dragon, step: "CreatePool", inflight: false } };
  const actions = plan({ ...quiet, ...parked }, ["minute"], accounts);
  assert.deepEqual(actions.map((a) => [a.receiver, a.method, a.args, a.gas]), [[accounts.dragon, "nearly_resume", { launch_id: "7" }, 300n * TGAS]]);
  for (const step of ["CreateToken", "Deposit", "AddLiquidity", "DevBuy", "ReadDevTokens", "DeliverDevTokens"]) {
    assert.deepEqual(calls({ ...parked, launch: { ...parked.launch, step } }, ["minute"]), ["nearly_resume"], step);
  }
});

test("a launch that is Done, Failed, in flight or not the Dragon's is never resumed", () => {
  const launch = { id: "7", creator: accounts.dragon, step: "DevBuy", inflight: false };
  for (const change of [{ step: "Done" }, { step: "Failed" }, { inflight: true }, { creator: "stranger.near" }]) {
    assert.deepEqual(calls({ coin: null, launch: { ...launch, ...change } }, ["minute"]), [], JSON.stringify(change));
  }
  assert.deepEqual(calls({ coin: null, launch: null }, ["minute"]), []);
});

test("the hourly step pushes creator fees to the tithe before distribute, and finalizes each due proposal at 150 Tgas", () => {
  const actions = plan({ ...quiet, due: [4, 9] }, ["fees", "hourly"], accounts);
  assert.deepEqual(actions.map((a) => [a.receiver, a.method, a.args]), [
    [peers.nearly, "push_creator_fees", { account_id: accounts.tithe }],
    [accounts.tithe, "distribute", {}],
    [accounts.scales, "finalize", { proposal_id: 4 }],
    [accounts.scales, "finalize", { proposal_id: 9 }],
  ]);
  assert.deepEqual(actions.filter((a) => a.method === "finalize").map((a) => a.gas), [150n * TGAS, 150n * TGAS]);
});

test("unwrap and forward_ft are due only while the tithe holds wNEAR or the coin", () => {
  assert.deepEqual(calls({ tithe: { wnear: 1n, wyrm: 1n } }, ["hourly"]), ["unwrap", "forward_ft", "distribute"]);
  assert.deepEqual(plan({ ...quiet, tithe: { wnear: 0n, wyrm: 1n } }, ["hourly"], accounts)[0].args, { token: coin.token });
});

test("there is no execute and no open_sale in any step", () => {
  const all = plan({ ...quiet, due: [1], vaultsCreated: 50, sale: { state: "Closed", tithe_told: false }, sweepable: [{ id: "80", balance: NEAR }] }, STEPS, accounts);
  assert.ok(all.every((a) => !["execute", "open_sale"].includes(a.method)));
});

test("close_sale is called daily while the sale is open or closed untold, and stops once the tithe is told", () => {
  const sale = (state, tithe_told) => calls({ sale: { state, tithe_told } }, ["daily"]);
  assert.deepEqual(sale("Waiting", false), []);
  assert.deepEqual(sale("Open", false), ["close_sale"]);
  assert.deepEqual(sale("Closed", false), ["close_sale"]);
  assert.deepEqual(sale("Closed", true), []);
  assert.equal(plan({ ...quiet, sale: { state: "Open", tithe_told: false } }, ["daily"], accounts)[0].gas, 50n * TGAS);
});

test("sweep goes in chunks of 20 over vaults holding at least 0.05 NEAR, skipping the rest", () => {
  const sweepable = [
    ...Array.from({ length: 25 }, (_, i) => ({ id: String(46 + i), balance: NEAR / 10n })),
    ...Array.from({ length: 5 }, (_, i) => ({ id: String(71 + i), balance: NEAR / 100n })),
    { id: "80", balance: NEAR / 20n }, { id: "81", balance: NEAR / 20n - 1n },
  ];
  const sweeps = plan({ ...quiet, sweepable }, ["daily"], accounts);
  assert.deepEqual(sweeps.map((a) => a.args.token_ids.length), [20, 6]);
  assert.ok(sweeps[1].args.token_ids.includes("80") && !sweeps.flatMap((a) => a.args.token_ids).some((id) => ["71", "75", "81"].includes(id)));
  assert.ok(sweeps.every((a) => a.gas === 300n * TGAS && a.receiver === accounts.scales));
});

test("create_vaults is due daily until 100 vaults exist", () => {
  assert.deepEqual(calls({ vaultsCreated: 75 }, ["daily"]), ["create_vaults"]);
  assert.deepEqual(calls({ vaultsCreated: 100 }, ["daily"]), []);
});

test("the gas table is the one docs/GAS.md measures against", () => {
  const tgas = Object.fromEntries(Object.entries(GAS).map(([k, v]) => [k, Number(v / TGAS)]));
  assert.deepEqual(tgas, { poke: 1000, nearly_resume: 300, push_creator_fees: 50, unwrap: 100, forward_ft: 100, distribute: 300, finalize: 150, create_vaults: 300, close_sale: 50, sweep: 300 });
});

test("a fresh keeper owes every step; then a minute, 15 past the hour and UTC midnight each open their step", () => {
  const T = Date.UTC(2026, 10, 9, 10, 14, 30);
  assert.deepEqual(dueSteps(T, {}), ["minute", "fees", "hourly", "daily"]);
  const ran = { minute: T, fees: T, hourly: T, daily: T };
  assert.deepEqual(dueSteps(T + 20_000, ran), []);
  assert.deepEqual(dueSteps(T + 30_000, ran), ["minute", "fees", "hourly"], "10:15:00 opens the fees and hourly steps");
  const after = { minute: T + 30_000, fees: T + 30_000, hourly: T + 30_000, daily: T };
  assert.deepEqual(dueSteps(T + 30_000 + 59 * 60_000, after), ["minute"]);
  assert.deepEqual(dueSteps(T + 30_000 + 60 * 60_000, after), ["minute", "fees", "hourly"]);
  assert.deepEqual(dueSteps(Date.UTC(2026, 10, 10, 0, 0, 0), after), ["minute", "fees", "hourly", "daily"]);
});

const panicked = (msg) => ({ status: { Failure: { ActionError: { index: 0, kind: { FunctionCallError: { ExecutionError: `Smart contract panicked: ${msg}` } } } } } });

test("each known refusal is a no-op for its own method only; a nonce collision is a lost race", () => {
  const cases = [["poke", "not bound"], ["poke", "in flight"], ["nearly_resume", "already bound"], ["nearly_resume", "nothing to resume"],
    ["push_creator_fees", "nothing to push"], ["distribute", "below min_round"], ["finalize", "voting still open"], ["finalize", "not open"],
    ["create_vaults", "all 100 vaults exist"], ["sweep", "Scale 46 is minted"], ["unwrap", "nothing to unwrap"], ["forward_ft", "nothing to forward"]];
  for (const [method, msg] of cases) assert.equal(classify({ method }, { outcome: panicked(msg) }), "noop", `${method}: ${msg}`);
  assert.equal(classify({ method: "distribute" }, { outcome: panicked("in flight") }), "failed");
  assert.equal(classify({ method: "close_sale" }, { outcome: { status: { SuccessValue: Buffer.from('"not due"').toString("base64") } } }), "sent");
  assert.equal(classify({ method: "poke" }, { error: 'send_tx: {"cause":{"name":"INVALID_TRANSACTION","info":{"InvalidNonce":{}}}}' }), "race");
  assert.equal(classify({ method: "poke" }, { error: "HTTP 500" }), "failed");
});

const HOLD = accountsFor("thewyrm.test.near");

test("on the recorded bound hold world the keeper pokes, pushes fees, distributes, finalizes the due proposal and sweeps the first 20 vaults it reads", async () => {
  const rpc = replaying(fixture("hold-world-keeper-bound.json"));
  const snap = await readSnapshot(rpc, HOLD, STEPS);
  assert.equal(snap.coin.token, "wyrm-2.nearly.test.near");
  snap.sweepable = await readVaults(rpc, HOLD, snap, VAULT_READS);
  const actions = plan(snap, STEPS, HOLD);
  assert.deepEqual(actions.map((a) => [a.method, Number(a.gas / TGAS), a.args.token_ids?.length ?? a.args.proposal_id ?? a.args.account_id ?? null]), [
    ["poke", 1000, null], ["push_creator_fees", 50, HOLD.tithe], ["distribute", 300, null], ["finalize", 150, 4], ["sweep", 300, 20],
  ]);
});

function capture(fn) {
  const lines = [];
  const orig = console.log;
  console.log = (l) => lines.push(JSON.parse(l));
  return fn().finally(() => { console.log = orig; }).then(() => lines);
}

const { privateKey, publicKey } = crypto.generateKeyPairSync("ed25519");
const seed = privateKey.export({ format: "der", type: "pkcs8" }).subarray(-32);
const pub = publicKey.export({ format: "der", type: "spki" }).subarray(-32);
const secret = "ed25519:" + b58encode(Buffer.concat([seed, pub]));
const pk = parseSecretKey(secret).publicKeyText;
const keysOf = (permission) => ({
  [JSON.stringify(["query", { request_type: "view_account", finality: "final", account_id: "keeper.near" }])]: { result: { amount: "1" } },
  [JSON.stringify(["query", { request_type: "view_access_key_list", finality: "final", account_id: "keeper.near" }])]: { result: { keys: [{ public_key: pk, access_key: { nonce: 1, permission } }] } },
});
const nonceRead = { [JSON.stringify(["query", { request_type: "view_access_key_list", finality: "optimistic", account_id: "keeper.near" }])]: { result: { keys: [{ public_key: pk, access_key: { nonce: 7 } }] } } };
const RECEIVERS = [HOLD.dragon, HOLD.tithe, HOLD.scales, "nearly.test.near"];
const signer = { account: "keeper.near", keys: Object.fromEntries(RECEIVERS.map((r) => [r, { secret, publicKeyText: pk, methods: [] }])) };
const succeeded = { status: { SuccessValue: "" }, transaction: { hash: "TX" }, receipts_outcome: [] };

// A replayed world that also answers the nonce read and every send_tx, so `requests` counts the sends.
function live(name, outcome = () => succeeded) {
  const log = { ...fixture(name), ...nonceRead };
  const replay = replaying(log);
  const rpc = async (method, params) => {
    if (method !== "send_tx") return replay(method, params);
    replay.requests++;
    rpc.sent.push(params.signed_tx_base64);
    return outcome(rpc.sent.length, Buffer.from(params.signed_tx_base64, "base64"));
  };
  rpc.sent = [];
  Object.defineProperty(rpc, "requests", { get: () => replay.requests });
  return rpc;
}

// Ticks a minute apart from a fresh keeper, as one process runs them; dry-run unless `sends` is set.
async function ticks(rpc, n, { state = newState(), start = Date.UTC(2026, 10, 9, 0, 15), sends = false } = {}) {
  const out = [];
  for (let i = 0; i < n; i++) {
    const lines = await capture(() => tick({ rpc, accounts: HOLD, signer, dryRun: !sends, state, watch: { symbol: "WYRM" }, now: start + i * 60_000 }));
    const calls = lines.filter((l) => ["would_send", "sent", "noop", "failed", "race"].includes(l.event));
    out.push({ swept: calls.filter((l) => l.call.endsWith(".sweep")).flatMap((l) => l.args.token_ids), calls, lines, tick: lines.at(-1) });
  }
  return { out, state };
}

test("over consecutive ticks the bound world sweeps exactly the 25 vaults funded with 0.1 NEAR, never the five with 0.01", async () => {
  const views = fixture("hold-world-keeper-bound.json");
  const { out, state } = await ticks(replaying(views), 4);
  const swept = out.flatMap((t) => t.swept);
  const sweepable = Object.entries(views).find(([k]) => k.includes("get_sweepable"))[1].result.result;
  const ids = JSON.parse(Buffer.from(sweepable).toString());
  assert.equal(ids.length, 45);
  assert.deepEqual(swept, ids.slice(0, 25), "fund-vaults.mjs gave the first 25 0.1 NEAR and the next five 0.01");
  assert.deepEqual(out.map((t) => t.tick.sweep_scan), ["17 of 45 vaults read", "37 of 45 vaults read", undefined, undefined], "the first tick's four calls leave room for 17 vault reads");
  assert.equal(state.sweep, null, "the tithe is told but the pass moved NEAR, so tomorrow's daily step scans again");
});

test("no sending tick makes more than 30 RPC requests, sends included, on the bound, unbound and 90-vault closed-sale worlds", async () => {
  for (const name of ["hold-world-keeper-bound.json", "hold-world-keeper-unsold.json", "hold-world-keeper-unbound.json"]) {
    const rpc = live(name);
    const { out } = await ticks(rpc, 6, { sends: true });
    for (const { tick, calls } of out) {
      assert.ok(tick.requests <= TICK_REQUESTS, `${name} ${tick.steps} made ${tick.requests}`);
      assert.ok(calls.every((c) => c.event === "sent"), `${name} every call was sent`);
    }
    assert.equal(rpc.sent.length, out.reduce((n, t) => n + t.calls.length, 0), `${name} each call is one send_tx`);
  }
  const { out } = await ticks(live("hold-world-keeper-bound.json"), 1, { sends: true });
  assert.deepEqual([out[0].calls.length, out[0].tick.requests], [5, 30], "7 views, 17 vaults, one nonce read and 5 sends");
});

test("a tick whose calls would pass 30 requests defers the rest, and their step stays due", async () => {
  const views = fixture("hold-world-keeper-bound.json");
  const due = Object.keys(views).find((k) => k.includes('"get_due"'));
  const many = { ...views, [due]: { result: { ...views[due].result, result: [...Buffer.from(JSON.stringify(Array.from({ length: 30 }, (_, i) => i)))] } } };
  const replay = replaying({ ...many, ...nonceRead });
  const rpc = async (m, p) => (m === "send_tx" ? (replay.requests++, succeeded) : replay(m, p));
  Object.defineProperty(rpc, "requests", { get: () => replay.requests });
  const state = newState();
  const lines = await capture(() => tick({ rpc, accounts: HOLD, signer, dryRun: false, state, watch: { symbol: "WYRM" }, now: Date.UTC(2026, 10, 9, 0, 15) }));
  const last = lines.at(-1);
  assert.deepEqual([last.requests, last.actions, last.deferred], [30, 22, 11]);
  assert.equal(state.lastRun.hourly, undefined, "hourly still owes 11 finalize calls");
  assert.ok(state.lastRun.minute && state.lastRun.daily);
});

test("a failed call keeps its step due, so the next tick plans it again; the steps that succeeded wait for their next slot", async () => {
  const T = Date.UTC(2026, 10, 9, 0, 15);
  const rpc = live("hold-world-keeper-bound.json", (n) => (n === 3 ? { ...panicked("out of gas"), transaction: { hash: "TX" } } : succeeded));
  const { out, state } = await ticks(rpc, 1, { sends: true, start: T });
  assert.deepEqual(out[0].calls.map((c) => [c.call.split(".").at(-1), c.event]), [["poke", "sent"], ["push_creator_fees", "sent"], ["distribute", "failed"], ["finalize", "sent"], ["sweep", "sent"]]);
  assert.deepEqual([state.lastRun.minute, state.lastRun.hourly, state.lastRun.daily], [T, undefined, T]);
  assert.equal(state.sweep.next, 17, "the sweep succeeded, so the pass moved on");
  const retry = await ticks(rpc, 1, { sends: true, state, start: T + 20_000 });
  assert.deepEqual(retry.out[0].tick.steps, ["hourly"]);
  assert.deepEqual(retry.out[0].calls.map((c) => c.call.split(".").at(-1)), ["distribute", "finalize", "sweep"], "the push was sent, so its slot is closed");
  assert.equal(state.lastRun.hourly, T + 20_000);
});

const sends = (rpc, method) => rpc.sent.filter((tx) => Buffer.from(tx, "base64").includes(method)).length;
const refuse = (method, answer) => (_, tx) => (tx.includes(method) ? answer() : succeeded);
const unseen = () => ({ ...panicked("a text nobody has seen"), transaction: { hash: "TX" } });
const down = () => { throw new Error("send_tx: fetch failed"); };

test("a push refused with an unknown text, or lost in transport, closes its hourly slot at once", async () => {
  const T = Date.UTC(2026, 10, 9, 0, 15);
  for (const answer of [unseen, down]) {
    const rpc = live("hold-world-keeper-bound.json", refuse("push_creator_fees", answer));
    const { out, state } = await ticks(rpc, 3, { sends: true, start: T });
    assert.equal(sends(rpc, "push_creator_fees"), 1, answer.name);
    assert.equal(state.lastRun.fees, T, answer.name);
    const gaveUp = out[0].lines.filter((l) => l.event === "gave_up");
    assert.deepEqual(gaveUp.map((l) => [l.step, l.tries]), [["fees", 1]], answer.name);
    assert.match(gaveUp[0].failure, answer === unseen ? /a text nobody has seen/ : /fetch failed/);
  }
});

test("a poke lost in transport or refused is sent once in its minute, and the next minute pokes again", async () => {
  const T = Date.UTC(2026, 10, 9, 0, 15);
  for (const answer of [unseen, down]) {
    const rpc = live("hold-world-keeper-bound.json", refuse("poke", answer));
    const state = newState();
    const out = [];
    // The keeper ticks every 20 seconds, so three ticks fall in one minute.
    for (const s of [0, 20, 40]) out.push(...(await ticks(rpc, 1, { sends: true, state, start: T + s * 1000 })).out);
    assert.equal(sends(rpc, "poke"), 1, answer.name);
    assert.equal(state.lastRun.minute, T, answer.name);
    const gaveUp = out.flatMap((t) => t.lines).filter((l) => l.event === "gave_up" && l.step === "minute");
    assert.deepEqual(gaveUp.map((l) => l.tries), [1], answer.name);
    await ticks(rpc, 1, { sends: true, state, start: T + 60_000 });
    assert.equal(sends(rpc, "poke"), 2, `${answer.name}: the next minute pokes again`);
  }
});

test("a call that fails with an unknown refusal or in transport is sent at most 3 times in its slot, logged once, and is due again in the next slot", async () => {
  const T = Date.UTC(2026, 10, 9, 0, 15);
  for (const answer of [unseen, down]) {
    const rpc = live("hold-world-keeper-bound.json", refuse("distribute", answer));
    const { out, state } = await ticks(rpc, 6, { sends: true, start: T });
    assert.equal(sends(rpc, "distribute"), 3, answer.name);
    const gaveUp = out.flatMap((t) => t.lines).filter((l) => l.event === "gave_up");
    assert.deepEqual(gaveUp.map((l) => [l.step, l.tries]), [["hourly", 3]], answer.name);
    assert.equal(state.lastRun.hourly, T + 2 * 60_000, answer.name);
    await ticks(rpc, 1, { sends: true, state, start: T + 60 * 60_000 });
    assert.equal(sends(rpc, "distribute"), 4, `${answer.name}: the next slot sends it again`);
  }
});

test("a call that loses its nonce to another copy keeps its step due, and the next tick sends it again", async () => {
  const T = Date.UTC(2026, 10, 9, 0, 15);
  const race = () => { throw new Error("send_tx: InvalidTxError InvalidNonce { tx_nonce: 10, ak_nonce: 10 }"); };
  const rpc = live("hold-world-keeper-bound.json", (n, tx) => (n === 3 && tx.includes("distribute") ? race() : succeeded));
  const { out, state } = await ticks(rpc, 1, { sends: true, start: T });
  assert.ok(out[0].calls.some((c) => c.call.endsWith(".distribute") && c.event === "race"));
  assert.equal(state.lastRun.hourly, undefined, "the hourly step is still due");
  const retry = await ticks(rpc, 1, { sends: true, state, start: T + 20_000 });
  assert.ok(retry.out[0].calls.some((c) => c.call.endsWith(".distribute") && c.event === "sent"));
  assert.equal(state.lastRun.hourly, T + 20_000);
});

test("a sweep that keeps failing is sent 3 times, then the pass moves past its vaults", async () => {
  const rpc = live("hold-world-keeper-bound.json", refuse("sweep", unseen));
  const { out } = await ticks(rpc, 4, { sends: true });
  assert.deepEqual(out.map((t) => t.swept[0]), [out[0].swept[0], out[0].swept[0], out[0].swept[0], out[3].swept[0]]);
  assert.notEqual(out[3].swept[0], out[0].swept[0]);
  assert.deepEqual(out.flatMap((t) => t.lines).filter((l) => l.event === "gave_up").map((l) => l.step), ["sweep"]);
});

test("an idle tick sends nothing and reads no nonce: the final block and the Dragon's config, 2 requests", async () => {
  const T = Date.UTC(2026, 10, 9, 0, 15);
  const rpc = live("hold-world-keeper-bound.json");
  const state = { ...newState(), lastRun: Object.fromEntries(STEPS.map((s) => [s, T])) };
  const { out } = await ticks(rpc, 1, { sends: true, state, start: T + 20_000 });
  assert.deepEqual([out[0].tick.steps, out[0].tick.actions, out[0].tick.requests, rpc.sent.length], [[], 0, 2, 0]);
});

test("a failed sweep reads the same vaults again on the next tick", async () => {
  const rpc = live("hold-world-keeper-bound.json", (n) => (n === 5 ? { ...panicked("boom"), transaction: { hash: "TX" } } : succeeded));
  const { out, state } = await ticks(rpc, 2, { sends: true });
  assert.deepEqual([out[0].calls.at(-1).event, out[0].swept.length], ["failed", 17]);
  assert.deepEqual(out[1].swept.slice(0, 17), out[0].swept, "the next tick starts from the same vault");
  assert.equal(state.sweep.next, 20, "and reads its 20 from there");
});

test("a daily step retried after a failed close_sale goes on with the sweep pass it started, not a new one", async () => {
  const views = fixture("hold-world-keeper-bound.json");
  const key = Object.keys(views).find((k) => k.includes('"get_config"') && k.includes(HOLD.scales));
  const config = JSON.parse(Buffer.from(views[key].result.result));
  const untold = { ...config, sale: { ...config.sale, tithe_told: false } };
  const replay = replaying({ ...views, ...nonceRead, [key]: { result: { ...views[key].result, result: [...Buffer.from(JSON.stringify(untold))] } } });
  const rpc = async (m, p) => {
    if (m !== "send_tx") return replay(m, p);
    replay.requests++;
    const closeSale = Buffer.from(p.signed_tx_base64, "base64").includes("close_sale");
    return closeSale ? { ...panicked("boom"), transaction: { hash: "TX" } } : succeeded;
  };
  Object.defineProperty(rpc, "requests", { get: () => replay.requests });
  const T = Date.UTC(2026, 10, 9, 0, 15);
  const { out, state } = await ticks(rpc, 1, { sends: true, start: T });
  assert.ok(out[0].calls.some((c) => c.call.endsWith(".close_sale") && c.event === "failed"));
  const first = state.sweep.next;
  const retry = await ticks(rpc, 1, { sends: true, state, start: T + 20_000 });
  assert.deepEqual(retry.out[0].tick.steps, ["daily"]);
  assert.deepEqual([first, state.sweep.next], [16, 36], "the retry read the next 20 vaults of the same pass");
});

test("on a closed sale with 90 unsold vaults and the tithe told, a pass that moves nothing ends the sweep scan for good", async () => {
  const rpc = replaying(fixture("hold-world-keeper-unsold.json"));
  const { out, state } = await ticks(rpc, 6);
  assert.deepEqual(out.map((t) => t.tick.requests), [25, 22, 22, 22, 14, 2]);
  assert.deepEqual(out.flatMap((t) => t.swept), []);
  assert.equal(state.sweep, "done");
  const nextDay = await ticks(rpc, 1, { state, start: Date.UTC(2026, 10, 10, 0, 15) });
  assert.deepEqual([nextDay.out[0].tick.steps, nextDay.out[0].tick.requests], [STEPS, 6], "only the hourly and daily views, no get_sweepable and no vault");
});

test("a sweep pass ends after its last slice, and ends the scan only when the tithe was told and nothing was found", () => {
  const pass = { ids: Array.from({ length: 45 }, (_, i) => String(i)), next: 0, found: 0, titheTold: true };
  assert.deepEqual(advancePass(pass, 20, 3), { ...pass, next: 20, found: 3 });
  assert.equal(advancePass({ ...pass, next: 40 }, 5, 0), "done");
  assert.equal(advancePass({ ...pass, next: 40, found: 1 }, 5, 0), null);
  assert.equal(advancePass({ ...pass, next: 40, titheTold: false }, 5, 0), null);
});

test("on the recorded unbound hold world the keeper plans no poke, resumes the parked launch and reads no vault before the sale closes", async () => {
  const rpc = replaying(fixture("hold-world-keeper-unbound.json"));
  const snap = await readSnapshot(rpc, HOLD, STEPS, { symbol: "WYRM" });
  assert.deepEqual([snap.coin, snap.sale.state, snap.pass, rpc.requests], [null, "Waiting", null, 6]);
  const actions = plan(snap, STEPS, HOLD);
  assert.ok(!actions.some((a) => a.method === "poke"));
  const resume = actions.find((a) => a.method === "nearly_resume");
  assert.deepEqual([resume.receiver, resume.args, resume.gas], [HOLD.dragon, { launch_id: snap.launch.id }, 300n * TGAS]);
});

test("a dry-run tick logs each due call with its gas, sends nothing, and counts its reads", async () => {
  const rpc = replaying(fixture("hold-world-keeper-bound.json"));
  const lines = await capture(() => tick({ rpc, accounts: HOLD, dryRun: true, state: newState(), watch: { symbol: "WYRM" }, send: () => assert.fail("dry run sent a call") }));
  const planned = lines.filter((l) => l.event === "would_send");
  assert.deepEqual(planned.map((l) => [l.call, l.tgas]).slice(0, 3), [[`${HOLD.dragon}.poke`, 1000], ["nearly.test.near.push_creator_fees", 50], [`${HOLD.tithe}.distribute`, 300]]);
  assert.deepEqual(lines.at(-1).event === "tick" && [lines.at(-1).steps, lines.at(-1).requests], [STEPS, 24]);
});

test("the keeper refuses to start on a full access key and names it", async () => {
  await assert.rejects(checkCredentials(replaying(keysOf("FullAccess")), { account: "keeper.near", keys: {}, fallback: secret }), new RegExp(`${pk} is a full access key of keeper.near`));
  const ok = await checkCredentials(replaying(keysOf({ FunctionCall: { receiver_id: HOLD.dragon, method_names: [] } })), { account: "keeper.near", keys: { [HOLD.dragon]: secret } });
  assert.deepEqual(ok, { account: "keeper.near", keys: { [HOLD.dragon]: { secret, publicKeyText: pk, receiver: HOLD.dragon, methods: [] } } });
});

test("a key listed under another receiver stops the start; a send needs a key for its receiver that may call its method", async () => {
  const fc = (receiver_id, method_names) => replaying(keysOf({ FunctionCall: { receiver_id, method_names } }));
  const action = { receiver: "dragon.thewyrm.near", method: "poke" };
  await assert.rejects(checkCredentials(fc("tithe.thewyrm.near", []), { account: "keeper.near", keys: { [action.receiver]: secret } }), /may call tithe.thewyrm.near, not dragon.thewyrm.near/);
  const fallback = await checkCredentials(fc("tithe.thewyrm.near", []), { account: "keeper.near", keys: {}, fallback: secret });
  assert.throws(() => signingKey(fallback, action), /no key for dragon.thewyrm.near/);
  const viewOnly = await checkCredentials(fc(action.receiver, ["get_state"]), { account: "keeper.near", keys: {}, fallback: secret });
  assert.throws(() => signingKey(viewOnly, action), /may not call poke/);
  const poker = await checkCredentials(fc(action.receiver, ["poke", "nearly_resume"]), { account: "keeper.near", keys: {}, fallback: secret });
  assert.equal(signingKey(poker, action).secret, secret);
});

test("a sent tick signs with the receiver's key, numbers its sends from one nonce read, and logs a known refusal as a no-op", async () => {
  const rpc = replaying({ ...fixture("hold-world-keeper-bound.json"), ...nonceRead });
  const sent = [];
  const T = Date.UTC(2026, 10, 9, 10, 20);
  const state = { ...newState(), lastRun: { minute: T - 60_000, fees: T - 3_600_000, hourly: T - 3_600_000, daily: T } };
  const lines = await capture(() => tick({
    rpc, accounts: HOLD, dryRun: false, state, watch: { symbol: "WYRM" }, now: T, signer,
    send: async (_, call) => { sent.push(call); return { ...panicked("in flight"), transaction: { hash: "TX1" } }; },
  }));
  const hash = Object.entries(fixture("hold-world-keeper-bound.json")).find(([k]) => k.startsWith('["block"'))[1].result.header.hash;
  assert.deepEqual(sent.map((c) => [c.signer, c.receiver, c.method, c.secret === secret, c.nonce, c.blockHash]).slice(0, 3), [
    ["keeper.near", HOLD.dragon, "poke", true, 8n, hash], ["keeper.near", "nearly.test.near", "push_creator_fees", true, 9n, hash], ["keeper.near", HOLD.tithe, "distribute", true, 10n, hash],
  ]);
  assert.deepEqual([lines[0].event, lines[0].tx], ["noop", "TX1"]);
});

test("an overloaded RPC is retried whether it answers HTTP 429 or JSON-RPC error -429 inside an HTTP 200", async () => {
  let n = 0;
  let overload;
  const server = http.createServer((req, res) => {
    req.resume();
    req.on("end", () => {
      n++;
      if (n <= 2 && overload === "http") {
        res.writeHead(429);
        return res.end("Too Many Requests");
      }
      res.writeHead(200, { "content-type": "application/json" });
      const error = (n <= 2 && overload === "json") || (n <= 20 && overload === "forever") ? { code: -429, message: "Too many requests" } : overload === "other" ? { code: -32000, message: "Server error" } : undefined;
      res.end(JSON.stringify({ jsonrpc: "2.0", id: "wyrm", ...(error ? { error } : { result: { n } }) }));
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const url = `http://127.0.0.1:${server.address().port}`;
  try {
    for (const form of ["http", "json"]) {
      [n, overload] = [0, form];
      const rpc = httpRpc(url, { wait: () => 1 });
      assert.deepEqual([await rpc("block", {}), rpc.requests], [{ n: 3 }, 3], form);
    }
    [n, overload] = [0, "other"];
    const rpc = httpRpc(url, { wait: () => 1 });
    await assert.rejects(rpc("block", {}), /Server error/);
    assert.equal(rpc.requests, 1, "any other error is not retried");
    [n, overload] = [0, "forever"];
    const overloaded = httpRpc(url, { wait: () => 1 });
    await assert.rejects(overloaded("block", {}), /Too many requests/);
    assert.equal(overloaded.requests, 9, "a lasting -429 throws after 8 retries");
  } finally {
    server.close();
  }
});

test("an overloaded RPC hands the request to the next URL, and later requests stay there", async () => {
  const hits = { busy: 0, free: 0 };
  const serve = (name) => http.createServer((req, res) => {
    req.resume();
    req.on("end", () => {
      hits[name]++;
      res.writeHead(200, { "content-type": "application/json" });
      const answer = name === "busy" ? { error: { code: -429, message: "Rate limits exceeded" } } : { result: { from: name } };
      res.end(JSON.stringify({ jsonrpc: "2.0", id: "wyrm", ...answer }));
    });
  });
  const servers = [serve("busy"), serve("free")];
  await Promise.all(servers.map((s) => new Promise((r) => s.listen(0, "127.0.0.1", r))));
  const [busy, free] = servers.map((s) => `http://127.0.0.1:${s.address().port}`);
  let waits = 0;
  try {
    const rpc = httpRpc([busy, free], { wait: () => { waits++; return 1; } });
    assert.deepEqual(await rpc("block", {}), { from: "free" });
    assert.deepEqual(await rpc("block", {}), { from: "free" });
    assert.deepEqual([hits.busy, hits.free, waits], [1, 2, 0], "one 429, then the free URL for both requests, with no wait");

    const allBusy = httpRpc([busy, busy], { wait: () => 1 });
    await assert.rejects(allBusy("block", {}), /Rate limits exceeded/);
    assert.equal(allBusy.requests, 17, "with every URL overloaded it gives up after 8 rounds of the list");
  } finally {
    servers.forEach((s) => s.close());
  }
});

test("a signed call carries a valid ed25519 signature over the sha256 of its borsh bytes", () => {
  const blockHash = b58decode("11111111111111111111111111111111");
  const signed = signedFunctionCall({ signer: "keeper.near", seed, publicKey: pub, nonce: 5n, receiver: "dragon.near", blockHash, method: "poke", args: {}, gas: 300n * TGAS });
  const bytes = Buffer.from(signed.base64, "base64");
  const tx = bytes.subarray(0, bytes.length - 65);
  assert.equal(bytes[bytes.length - 65], 0, "signature key type is ed25519");
  const hash = crypto.createHash("sha256").update(tx).digest();
  assert.equal(b58encode(hash), signed.hash);
  assert.ok(crypto.verify(null, hash, publicKey, bytes.subarray(-64)));
  assert.equal(tx.readUInt32LE(0), "keeper.near".length);
  assert.equal(tx.subarray(4, 15).toString(), "keeper.near");
});

test("calls deferred by the cap over many ticks are never counted as attempts", async () => {
  const views = fixture("hold-world-keeper-bound.json");
  const due = Object.keys(views).find((k) => k.includes('"get_due"'));
  const many = { ...views, [due]: { result: { ...views[due].result, result: [...Buffer.from(JSON.stringify(Array.from({ length: 30 }, (_, i) => i)))] } } };
  const replay = replaying({ ...many, ...nonceRead });
  const rpc = async (m, p) => (m === "send_tx" ? (replay.requests++, succeeded) : replay(m, p));
  Object.defineProperty(rpc, "requests", { get: () => replay.requests });
  const state = newState();
  const T = Date.UTC(2026, 10, 9, 0, 15);
  const lines = [];
  for (let i = 0; i < 4; i++) lines.push(...(await capture(() => tick({ rpc, accounts: HOLD, signer, dryRun: false, state, watch: { symbol: "WYRM" }, now: T + i * 20_000 }))));
  assert.deepEqual(lines.filter((l) => l.event === "gave_up"), []);
  assert.equal(state.lastRun.hourly, undefined, "hourly still owes its deferred finalize calls");
});
