import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { auditDragon, engineSettings, taxedSwaps } from "./audit-dragon.mjs";

const DRAGON = "dragon.thewyrm.near";
const DAY0_MS = 20_000 * 86_400_000;
const MIN = 60_000;
const ns = (ms) => String(BigInt(ms) * 1_000_000n);
const yocto = (near) => String(BigInt(Math.round(near * 1e6)) * 10n ** 18n);
const wyrm = (n) => String(BigInt(n) * 10n ** 18n);
const BREATH_MIN = 100;

// The cases were sized for this ladder and strategy, so they hold whatever the shipped presets say.
const config = (cadence = { spread_minutes: 0, gap_seconds: 60 }) => ({
  preset: "test",
  preset_value: {
    strategy: { budget_bps: 3000, cap_bps: 250, hunt: null, auto_burn_bps: 0 },
    ladder: { steps_bps: [1000, 2000, 3000], extra_bps: [1000, 1500, 2000] },
    cadence,
  },
  engine: { spike_bps: 1500, tax: { buy_bps: 400, sell_bps: 400 } },
  peers: { dcl: "dcl.near", wrap: "wrap.near" },
  coin: { token: "wyrm.near" },
});

const event = (name, data) => ({ standard: "wyrm", version: "1.0.0", event: name, data: [data] });
const receipt = (tx, events) => ({ tx, id: tx, predecessor: "keeper.near", receiver: DRAGON, actions: [], ok: true, value: null, logs: [], events });

// One observation per minute at `tpnAt(minute)` tokens per NEAR; `buys` maps a minute to the slice the Dragon logged.
function day({ minutes = 130, tpnAt = () => 1000, buys = {}, failedAt = [] }) {
  const out = [];
  for (let m = 0; m < minutes; m++) {
    const at = DAY0_MS + m * MIN;
    const tpn = tpnAt(m);
    const events = [event("observe", { at: ns(at), tokens_per_near: wyrm(tpn), reserve: yocto(100), hoard_wyrm: "0", breath_at: ns(DAY0_MS + BREATH_MIN * MIN) })];
    const b = buys[m];
    if (b) events.push(event(b.kind, { at: ns(at), amount_in: yocto(b.near), quoted_out: wyrm(Math.floor(b.near * tpn * 0.99)) }));
    if (failedAt.includes(m)) events.push(event("swap_failed", { at: ns(at), slice: "Breath", amount_in: yocto(30), reason: "refunded" }));
    out.push(receipt(`O${m}`, events));
  }
  return out;
}

const summary = (r) => r.decisions.map((x) => [x.tx, x.pass, x.why]);
const audit = (receipts, cfg = config()) => auditDragon(receipts, DRAGON, cfg);

test("the engine runs on the preset's strategy, ladder and cadence", () => {
  const s = engineSettings(config({ spread_minutes: 90, gap_seconds: 30 }));
  assert.deepEqual([s.budget_bps, s.cap_bps, s.hunt, s.steps_bps, s.gap_ms, s.spread_ms, s.spike_bps], [3000, 250, false, [1000, 2000, 3000], 30_000, 90 * MIN, 1500]);
});

test("a breath buy of the whole budget at the breath minute agrees with the engine", () => {
  assert.deepEqual(summary(audit(day({ buys: { [BREATH_MIN]: { kind: "breath_slice", near: 30 } } }))), [[`O${BREATH_MIN}`, true, null]]);
});

test("a slice the engine reaches by halving agrees, one it cannot reach does not", () => {
  assert.deepEqual(summary(audit(day({ buys: { [BREATH_MIN]: { kind: "breath_slice", near: 7.5 } } }))), [[`O${BREATH_MIN}`, true, null]]);
  assert.deepEqual(summary(audit(day({ buys: { [BREATH_MIN]: { kind: "breath_slice", near: 10 } } }))), [[`O${BREATH_MIN}`, false, "the engine sized 7.5 NEAR, the dragon traded 10 NEAR"]]);
});

test("a breath buy before the breath minute is flagged", () => {
  assert.deepEqual(summary(audit(day({ buys: { 50: { kind: "breath_slice", near: 30 } } }))), [["O50", false, "the engine decided none (before breath), the dragon traded by breath"]]);
});

test("a spread breath opens one minute's share at the breath minute, so the whole budget at once is flagged", () => {
  const spread = config({ spread_minutes: 60, gap_seconds: 60 });
  assert.deepEqual(summary(audit(day({ buys: { [BREATH_MIN]: { kind: "breath_slice", near: 0.5 } } }), spread)), [[`O${BREATH_MIN}`, true, null]]);
  assert.equal(audit(day({ buys: { [BREATH_MIN]: { kind: "breath_slice", near: 30 } } }), spread).decisions[0].pass, false);
  assert.equal(audit(day({ buys: { [BREATH_MIN]: { kind: "breath_slice", near: 30 } } })).decisions[0].pass, true, "the same buy passes without a spread");
});

test("the preset's gap holds a second slice back", () => {
  const buys = { [BREATH_MIN]: { kind: "breath_slice", near: 15 }, [BREATH_MIN + 1]: { kind: "breath_slice", near: 15 } };
  assert.deepEqual(audit(day({ buys })).decisions.map((x) => x.pass), [true, true]);
  assert.deepEqual(audit(day({ buys }), config({ spread_minutes: 0, gap_seconds: 120 })).decisions.map((x) => x.pass), [true, false]);
});

test("a dip buy after three samples over ten minutes past step one agrees; one minute earlier it is flagged", () => {
  const tpnAt = (m) => (m >= 60 ? 1200 : 1000);
  assert.deepEqual(summary(audit(day({ tpnAt, buys: { 70: { kind: "dip_slice", near: 10 } } }))), [["O70", true, null]]);
  assert.equal(audit(day({ tpnAt, buys: { 69: { kind: "dip_slice", near: 10 } } })).decisions[0].pass, false);
});

test("a second buy in the same day only gets what the first left, and a refunded swap gives its spend back", () => {
  const twice = audit(day({ buys: { [BREATH_MIN]: { kind: "breath_slice", near: 30 }, [BREATH_MIN + 2]: { kind: "breath_slice", near: 30 } } }));
  assert.deepEqual(twice.decisions.map((x) => x.pass), [true, false]);
  const retried = audit(day({ buys: { [BREATH_MIN + 2]: { kind: "breath_slice", near: 30 } }, failedAt: [BREATH_MIN] }));
  assert.deepEqual([summary(retried), retried.refunded], [[[`O${BREATH_MIN + 2}`, true, null]], 1]);
});

test("observations where a trade was due but none was made are counted, not failed", () => {
  const r = audit(day({}));
  assert.deepEqual([r.decisions.length, r.unmade, r.idle], [0, 130 - BREATH_MIN, BREATH_MIN]);
});

test("an observe event without its fields stops the replay with the missing field named", () => {
  assert.throws(() => audit([receipt("X", [event("observe", { at: "1" })])]), /observe in tx X has no "tokens_per_near" field/);
});

const nep141 = (...moves) => ({ standard: "nep141", version: "1.0.0", event: "ft_transfer", data: moves });
const leg = (tx, { coin = "wyrm.near", sender, payee, gross, net, tax, method = "ft_transfer" }) => ({
  tx, id: `${tx}:${sender}`, predecessor: sender, receiver: coin, ok: true, value: null, logs: [],
  actions: [{ kind: "FunctionCall", method, args: { receiver_id: payee, amount: String(gross) }, deposit: 1n }],
  events: [nep141({ old_owner_id: sender, new_owner_id: payee, amount: String(net) }, ...(tax === null ? [] : [{ old_owner_id: sender, new_owner_id: coin, amount: String(tax), memo: "tax" }]))],
});
const sliceTx = (tx, name, d) => receipt(tx, [event(name, { at: "1", amount_in: String(d.amount_in), quoted_out: "0", min_out: String(d.min_out) })]);

test("a taxed buy pairs DCL's net transfer to the dragon with a `tax` transfer of 4% of amount_out", () => {
  const buy = (gross, net, tax, min_out = 9_000) => [sliceTx("B", "breath_slice", { amount_in: 1, min_out }), leg("B", { sender: "dcl.near", payee: DRAGON, gross, net, tax })];
  assert.deepEqual(taxedSwaps(buy(10_000, 9_600, 400), config(), DRAGON).map((s) => [s.pass, s.leg.net, s.leg.tax]), [[true, 9_600n, 400n]]);
  assert.match(taxedSwaps(buy(10_000, 9_700, 300), config(), DRAGON)[0].why, /tax 300 in 1 transfer\(s\), expected one of 400/);
  assert.match(taxedSwaps(buy(10_000, 10_000, null), config(), DRAGON)[0].why, /in 0 transfer/);
  assert.match(taxedSwaps(buy(10_000, 9_500, 400), config(), DRAGON)[0].why, /net 9500 is not gross 10000 less tax 400/);
  assert.match(taxedSwaps(buy(10_000, 9_600, 400, 10_001), config(), DRAGON)[0].why, /amount_out 10000 is below min_out 10001/);
  assert.match(taxedSwaps([sliceTx("B", "breath_slice", { amount_in: 1, min_out: 1 })], config(), DRAGON)[0].why, /no transfer of wyrm.near from dcl.near/);
});

test("a taxed hunt sell sends its gross, DCL gets it less 4%, and the wNEAR back meets min_out", () => {
  const sell = (back, logged = 10_000) => [
    sliceTx("S", "hunt_slice", { amount_in: logged, min_out: 50 }),
    leg("S", { sender: DRAGON, payee: "dcl.near", gross: 10_000, net: 9_600, tax: 400, method: "ft_transfer_call" }),
    leg("S", { coin: "wrap.near", sender: "dcl.near", payee: DRAGON, gross: back, net: back, tax: null }),
  ];
  assert.deepEqual(taxedSwaps(sell(50), config(), DRAGON).map((s) => [s.kind, s.pass, s.back]), [["hunt", true, 50n]]);
  assert.match(taxedSwaps(sell(49), config(), DRAGON)[0].why, /wNEAR back 49 is below min_out 50/);
  assert.deepEqual(taxedSwaps(sell(50, 9_000), config(), DRAGON).map((s) => [s.pass, s.why]), [[false, "sent 10000, the slice logged 9000"]]);
});

const holdWorld = () => JSON.parse(fs.readFileSync(new URL("./fixtures/hold-world-receipts.json", import.meta.url)))
  .map((r) => ({ ...r, at: BigInt(r.at), actions: r.actions.map((a) => (a.deposit === undefined ? a : { ...a, deposit: BigInt(a.deposit) })) }));
const HOLD = { preset: "hoarder", preset_value: { strategy: { budget_bps: 1000, cap_bps: 200, hunt: null, auto_burn_bps: 0 }, ladder: { steps_bps: [2000, 3500, 5000], extra_bps: [0, 0, 0] }, cadence: { spread_minutes: 0, gap_seconds: 60 } },
  engine: { spike_bps: 1500, tax: { buy_bps: 400, sell_bps: 400 } }, peers: { dcl: "dcl.test.near", wrap: "wrap.test.near" }, coin: { token: "wyrm-2.nearly.test.near" } };

test("on the recorded hold world both breath buys re-derive on the hoarder cadence and each delivered 96% with a `tax` transfer", () => {
  const receipts = holdWorld();
  const r = auditDragon(receipts, "dragon.thewyrm.test.near", HOLD);
  assert.deepEqual([r.decisions.map((x) => [x.made.kind, x.pass]), r.refunded], [[["breath", true], ["breath", true]], 1]);
  const swaps = taxedSwaps(receipts, HOLD, "dragon.thewyrm.test.near");
  assert.deepEqual(swaps.map((s) => [s.pass, s.leg.net * 10_000n / s.leg.gross, s.leg.tax + s.leg.net === s.leg.gross]), [[true, 9_600n, true], [true, 9_600n, true]]);
});
