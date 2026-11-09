import { test } from "node:test";
import assert from "node:assert/strict";
import { coinView, coinProfile, tierOf, TIERS } from "./coins.mjs";

// A coin of `days` days, one bar a minute for the first `perDay` minutes of each day, at `price` with `dayVol` NEAR a day.
function coin({ days = 6, perDay = 120, price = 5e-5, dayVol = 400, spike = null }) {
  const p = [], v = [], b = [], n = [], dm = [];
  let last = 0;
  for (let d = 0; d < days; d++) for (let m = 0; m < perDay; m++) {
    const at = d * 1440 + m;
    dm.push(p.length ? at - last : 0); last = at;
    p.push(spike && spike.day === d && m === 0 ? spike.price : price);
    v.push(dayVol / perDay); b.push(dayVol / perDay / 2); n.push(1);
  }
  return coinView({ id: "c", start_min: 20_000 * 1440, bars: { dm, p, v, b, n }, depth: [], depth_day0: 20_000 });
}
const tier = (c) => TIERS[tierOf(coinProfile(c))]?.name ?? "none";

test("top coin: a 50,000 NEAR peak with five days of 300 NEAR volume is a top coin", () => {
  assert.equal(tier(coin({})), "top");
});

test("top coin: one stray trade at a huge price does not make a coin top", () => {
  assert.equal(tier(coin({ price: 2e-6, spike: { day: 2, price: 1e-3 } })), "none");
});

test("top coin: a high peak without sustained volume is not enough", () => {
  assert.equal(tier(coin({ days: 4 })), "none");
  assert.equal(tier(coin({ dayVol: 150 })), "rising", "150 NEAR a day sustains the rising tier but not the top tier");
});
