import { test } from "node:test";
import assert from "node:assert/strict";
import { choose, crossValidate, timeSplit } from "./tune.mjs";
import { perScalePerDay, titheRate } from "./study.mjs";

const table = (rows) => new Map(Object.entries(rows).map(([k, xs]) => [k, new Map(xs.map((v, i) => [`c${i}`, v]))]));

test("choose: a challenger inside the tolerance keeps the incumbent, a clear winner replaces it", () => {
  const ids = ["c0", "c1", "c2"];
  assert.equal(choose(table({ incumbent: [1, 1, 1], challenger: [1.01, 1.01, 1.01] }), ["incumbent", "challenger"], ids), "incumbent");
  assert.equal(choose(table({ incumbent: [1, 1, 1], challenger: [1.1, 1.1, 1.1] }), ["incumbent", "challenger"], ids), "challenger");
});

test("cross-validation: a setting that won on one coin's luck scores lower held out than in sample", () => {
  const t = table({ lucky: [3, 1, 1], steady: [1, 2, 2] });
  const cv = crossValidate(t, ["steady", "lucky"], ["c0", "c1", "c2"], 3);
  assert.equal(cv.inSample.key, "steady");
  assert.ok(cv.heldOutSpread.p50 < cv.inSample.p50, `held out ${cv.heldOutSpread.p50}, in sample ${cv.inSample.p50}`);
});

test("time split: reports the setting chosen before the split, scored after it, not the hindsight winner", () => {
  const before = table({ fast: [2, 2], slow: [1, 1] }), after = table({ fast: [0.9, 0.9], slow: [1.3, 1.3] });
  const ts = timeSplit(before, after, ["fast", "slow"], ["c0", "c1"]);
  assert.equal(ts.chosen, "fast");
  assert.equal(ts.heldOut.p50, 0.9);
  assert.equal(ts.bestAfter.key, "slow");
});

test("estimator: 10,000 NEAR a day with WYRM's 4% tax pays each Scale about 1.3 NEAR, without a tax about 0.12", () => {
  assert.ok(Math.abs(perScalePerDay(10_000, titheRate({ buyShare: 0.5, tax: 0.04 })) - 1.32) < 1e-9);
  assert.ok(Math.abs(perScalePerDay(10_000, titheRate({ buyShare: 0.5 })) - 0.12) < 1e-9);
});
