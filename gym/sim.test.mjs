import { test } from "node:test";
import assert from "node:assert/strict";
import { breakEven, sandwichProfit, buyTokens, run, strategies, dragonStrategy, segmentValue, ZERO_SNAP, costOf, WYRM_TAX } from "./sim.mjs";

const DAY0 = 20_000 * 86_400_000;
const flat = (n, over = {}) => ({ id: "t", t0: DAY0, price: new Float64Array(n).fill(0.01), vol: new Float64Array(n).fill(2), buy: new Float64Array(n).fill(1), depth: () => 500, ...over });

test("sandwich: with WYRM's 4% tax a known buy is safe below 5.5% of depth, without it only below 1%", () => {
  const best = (f, fee) => Math.max(...[0.001, 0.005, 0.01, 0.05, 0.1, 0.3, 1].map((a) => sandwichProfit(1000, f * 1000, a * 1000, fee)));
  assert.ok(best(0.025, costOf(WYRM_TAX)) <= 0);
  assert.ok(best(0.1, costOf(WYRM_TAX)) > 0);
  const taxed = breakEven(), untaxed = breakEven(1e-9, costOf(0));
  assert.ok(taxed > 0.05 && taxed < 0.06, `break-even ${taxed}`);
  assert.ok(untaxed > 0.009 && untaxed < 0.012, `break-even without tax ${untaxed}`);
});

test("fill: a tiny buy pays the pool fee and the tax, a large one also pays impact", () => {
  assert.ok(Math.abs(buyTokens(1, 0.01, 1e12) - 95) < 1e-6);
  assert.ok(buyTokens(100, 0.01, 100) < 9500 * 0.6);
});

test("run: income is the tithe's share of volume, and the Dragon's own buys pay some of it back", () => {
  const n = 3 * 1440, path = flat(n);
  const none = run(path, strategies.none, { seed_reserve: 10 });
  assert.ok(Math.abs(none.inflow - (10 + n * (1 * 0.0024 + 2 * 0.012))) < 1e-9, "0.24% of buys plus 1.2% of all volume");
  assert.ok(Math.abs(run(path, strategies.none, { seed_reserve: 10, tax: 0 }).inflow - (10 + n * 0.0024)) < 1e-9, "without a tax only the pool-fee share");
  for (const s of [strategies.instant, strategies.dca, strategies.dailyLump, dragonStrategy()]) {
    const r = run(path, s, { seed_reserve: 10 });
    assert.ok(Math.abs(r.reserve + r.spent - r.inflow) < 1e-9, `${s.name} conserves NEAR`);
    assert.ok(r.reserve >= -1e-12, s.name);
    assert.ok(r.inflow > none.inflow, `${s.name} gets its own tax back through the tithe`);
  }
});

test("run: income reaches the reserve once an hour, at minute 12", () => {
  const path = flat(120);
  const r = run(path, strategies.none, { marks: [DAY0 + 12 * 60_000, DAY0 + 13 * 60_000, DAY0 + 72 * 60_000, DAY0 + 73 * 60_000] });
  const [before, after, nextBefore, nextAfter] = r.snaps.map((s) => s.inflow);
  assert.equal(before, 0);
  assert.ok(Math.abs(after - 13 * 0.0264) < 1e-9, "minutes 0 to 12 arrive together");
  assert.equal(nextBefore, after);
  assert.ok(Math.abs(nextAfter - after - 60 * 0.0264) < 1e-9, "then one hour at a time");
});

test("score: NEAR left unspent counts as bought at the end price, so holding back is not free", () => {
  const r = run(flat(1440), strategies.none);
  assert.ok(Math.abs(segmentValue(ZERO_SNAP, r.end) - (1 - costOf(WYRM_TAX)) / 0.01) < 1e-6);
  const inst = run(flat(1440), strategies.instant);
  assert.ok(segmentValue(ZERO_SNAP, inst.end) < segmentValue(ZERO_SNAP, r.end), "on a flat price, buying into a 500 NEAR pool pays impact the mark does not");
});

test("plain DCA: buys every hour from the last day's income, at the same price as instant on a flat path", () => {
  const path = flat(3 * 1440), dca = run(path, strategies.dca), inst = run(path, strategies.instant);
  assert.ok(dca.buys >= 70, `buys ${dca.buys}`);
  assert.ok(Math.abs(dca.tpn / inst.tpn - 1) < 0.002);
});
