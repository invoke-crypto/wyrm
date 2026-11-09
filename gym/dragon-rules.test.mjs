import { test } from "node:test";
import assert from "node:assert/strict";
import { newDragon, observe, settle, reference, breathMinute, MINUTE_MS, DAY_MS } from "./dragon-rules.mjs";

const DAY0 = 20_000 * DAY_MS;
// The budget and dip ladder these tests size their expected amounts from, so they do not move with the defaults.
const LADDER = { budget_bps: 3000, steps_bps: [1000, 2000, 3000], extra_bps: [1000, 1500, 2000] };
// A constant-product pool with NEAR depth `depth_near` and mid price `tokens_per_near` WYRM per NEAR; obs carries its quotes.
function obs(now, { depth_near = 1e9, tokens_per_near = 100, ...over } = {}) {
  const quote_buy = (a) => (tokens_per_near * a * depth_near) / (depth_near + a);
  const quote_sell = (w) => ((w / tokens_per_near) * depth_near) / (depth_near + w / tokens_per_near);
  return { now, reserve: 1000, hoard_wyrm: 0, random_u32: 600, ...over, tokens_per_near: quote_buy(1), quote_buy, quote_sell };
}

// Drives the Dragon one observe per minute from `from` for `minutes`, filling buys at the quoted price.
function drive(d, from, minutes, over = () => ({})) {
  const actions = [];
  let reserve = over(from).reserve ?? 1000;
  for (let i = 0; i < minutes; i++) {
    const now = from + i * MINUTE_MS;
    const o = obs(now, { ...over(now), reserve, hoard_wyrm: d.hoard.wyrm });
    const a = observe(d, o);
    if (a.kind !== "none") { settle(d, a, a.amount_in * o.tokens_per_near); reserve -= a.amount_in; actions.push({ ...a, now }); }
  }
  return actions;
}

// Kept samples: at most 1440, none older than 24h before the newest.
function bruteReference(all, now, windowMs = DAY_MS) {
  const s = all.filter((x) => x.at >= all.at(-1).at - windowMs).slice(-1440);
  let area = 0;
  for (let i = 0; i < s.length; i++) area += s[i].tokens_per_near * ((i + 1 < s.length ? s[i + 1].at : now) - s[i].at);
  return now > s[0].at ? area / (now - s[0].at) : s[0].tokens_per_near;
}

test("breath: nothing is spent before the day's breath minute, spending starts at it", () => {
  const d = newDragon();
  const acts = drive(d, DAY0, 1440);
  assert.equal(breathMinute(600), 600);
  assert.equal(acts[0].now, DAY0 + 600 * MINUTE_MS);
  assert.ok(acts.every((a) => a.kind === "breath"));
});

test("breath: the minute is drawn once per day, a later seed does not move it", () => {
  const d = newDragon();
  observe(d, obs(DAY0, { random_u32: 600 }));
  observe(d, obs(DAY0 + MINUTE_MS, { random_u32: 5 }));
  assert.equal(d.day.breath_at, DAY0 + 600 * MINUTE_MS);
  observe(d, obs(DAY0 + DAY_MS, { random_u32: 1445 }));
  assert.equal(d.day.breath_at, DAY0 + DAY_MS + 5 * MINUTE_MS);
});

test("daily budget: a flat day spends exactly 30% of the opening reserve", () => {
  const d = newDragon(LADDER);
  const acts = drive(d, DAY0, 1440, () => ({ depth_near: 1000 }));
  const spent = acts.reduce((a, x) => a + x.amount_in, 0);
  assert.ok(Math.abs(spent - 300) < 1e-9, `spent ${spent}`);
  const next = drive(d, DAY0 + DAY_MS, 1440, () => ({ reserve: 700, depth_near: 1000 }));
  assert.ok(Math.abs(next.reduce((a, x) => a + x.amount_in, 0) - 210) < 1e-9, "next day budgets on its own opening reserve");
});

test("slice cap and gap: every slice moves its average price at most 2.5%, slices at least 60 s apart", () => {
  const d = newDragon(LADDER);
  const acts = drive(d, DAY0, 1440, () => ({ depth_near: 400 }));
  assert.ok(acts.length > 1);
  const o = obs(DAY0, { depth_near: 400 });
  assert.ok(acts.every((a) => o.quote_buy(a.amount_in) * 10_000 >= a.amount_in * o.tokens_per_near * 9_750), "impact within 250 bps");
  assert.equal(acts[0].amount_in, 300 / 32, "300 halved five times is the first amount that fits a 400 NEAR pool");
  for (let i = 1; i < acts.length; i++) assert.ok(acts[i].now - acts[i - 1].now >= 60_000);
  const rapid = newDragon(LADDER);
  assert.equal(observe(rapid, obs(DAY0 + 700 * MINUTE_MS)).kind, "breath");
  assert.equal(observe(rapid, obs(DAY0 + 700 * MINUTE_MS + 59_000)).why, "gap");
});

test("slice cap: skipped when six halvings still move the price more than 2.5%", () => {
  const d = newDragon(LADDER);
  assert.equal(observe(d, obs(DAY0 + 700 * MINUTE_MS, { depth_near: 100 })).why, "impact");
  assert.equal(observe(newDragon(LADDER), obs(DAY0 + 700 * MINUTE_MS, { depth_near: 200 })).kind, "breath");
});

test("spike guard: no buy while spot is more than 15% above the reference", () => {
  const warm = () => { const d = newDragon(); for (let i = 0; i < 1440; i++) observe(d, obs(DAY0 - DAY_MS + i * MINUTE_MS)); return d; };
  assert.equal(observe(warm(), obs(DAY0 + 700 * MINUTE_MS, { tokens_per_near: 80 })).why, "spike", "price 25% above");
  assert.equal(observe(warm(), obs(DAY0 + 700 * MINUTE_MS, { tokens_per_near: 90 })).kind, "breath", "price 11% above");
  assert.equal(observe(newDragon({ spike_bps: null }), obs(DAY0 + 700 * MINUTE_MS, { tokens_per_near: 80 })).kind, "breath");
});

test("reference: one sample after a long silence does not drag it", () => {
  const d = newDragon();
  for (let i = 0; i < 720; i++) observe(d, obs(DAY0 + i * MINUTE_MS, { tokens_per_near: 100 }));
  const late = DAY0 + 18 * 60 * MINUTE_MS;
  observe(d, obs(late, { tokens_per_near: 1000 }));
  assert.ok(Math.abs(reference(d, late) - 100) < 1e-5, `reference ${reference(d, late)}`);
  const expected = 100 + (900 * 10) / (18 * 60 + 10);
  assert.ok(Math.abs(reference(d, late + 10 * MINUTE_MS) - expected) < 1e-5, "ten minutes later it has moved by its time share only");
});

test("reference: the running sum matches a brute-force time-weighted mean", () => {
  const d = newDragon();
  const all = [];
  let seed = 7, now = DAY0;
  const rand = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
  for (let i = 0; i < 4000; i++) {
    now += MINUTE_MS * (1 + Math.floor(rand() * (rand() < 0.02 ? 600 : 3)));
    const tpn = 50 + rand() * 100;
    if (!all.length || now - all.at(-1).at >= MINUTE_MS) all.push({ at: now, tokens_per_near: tpn });
    observe(d, obs(now, { tokens_per_near: tpn }));
    const q = now + Math.floor(rand() * 5 * MINUTE_MS);
    assert.ok(Math.abs(reference(d, q) - bruteReference(all, q)) < 1e-6, `at sample ${i}`);
  }
  assert.ok(d.samples.length <= 1440);
});

test("dip: confirmed only after 3 samples spanning 10 minutes beyond the step", () => {
  const d = newDragon({ ...LADDER, budget_bps: 0 });
  for (let i = 0; i < 1440; i++) observe(d, obs(DAY0 - DAY_MS + i * MINUTE_MS, { tokens_per_near: 100 }));
  const dipStart = DAY0 + 10 * MINUTE_MS;
  const kinds = [];
  for (let i = 0; i <= 12; i++) kinds.push(observe(d, obs(dipStart + i * MINUTE_MS, { tokens_per_near: 115, depth_near: 1e6 })).kind);
  assert.deepEqual(kinds.slice(0, 10), Array(10).fill("none"), "nothing for the first 10 minutes");
  assert.equal(kinds[10], "dip");
});

test("dip: a one-minute spike is never confirmed, and a recovered price stops dip buying", () => {
  const d = newDragon({ budget_bps: 0 });
  for (let i = 0; i < 1440; i++) observe(d, obs(DAY0 - DAY_MS + i * MINUTE_MS, { tokens_per_near: 100 }));
  const kinds = [];
  for (let i = 0; i < 30; i++) kinds.push(observe(d, obs(DAY0 + i * MINUTE_MS, { tokens_per_near: i % 5 === 0 ? 200 : 100 })).kind);
  assert.ok(kinds.every((k) => k === "none"));
  for (let i = 30; i < 45; i++) observe(d, obs(DAY0 + i * MINUTE_MS, { tokens_per_near: 115, depth_near: 100 }));
  assert.equal(observe(d, obs(DAY0 + 45 * MINUTE_MS, { tokens_per_near: 100, depth_near: 100 })).kind, "none");
});

test("dip: extras are cumulative, a confirmed step 3 adds all three", () => {
  const run = (dipTpn) => {
    const d = newDragon(LADDER);
    for (let i = 0; i < 1440; i++) observe(d, obs(DAY0 - DAY_MS + i * MINUTE_MS));
    return drive(d, DAY0, 1440, (now) => ({ random_u32: 0, tokens_per_near: now >= DAY0 + 60 * MINUTE_MS ? dipTpn : 100, depth_near: 1e6 })).reduce((a, x) => a + x.amount_in, 0);
  };
  assert.ok(Math.abs(run(160) - 750) < 1e-9, "30% breath + 10% + 15% + 20%");
  assert.ok(Math.abs(run(115) - 400) < 1e-9, "step 1 only: 30% + 10%");
});

test("hunt: sells only above the multiple of average cost, capped per day", () => {
  const d = newDragon({ hunt: true, budget_bps: 0 });
  d.hoard = { wyrm: 1000, near_spent: 10, wyrm_bought: 1000, realized_near: 0 };
  assert.equal(observe(d, obs(DAY0, { hoard_wyrm: 1000, tokens_per_near: 50 })).kind, "none", "price 0.02 is 2x cost, below 2.5x");
  const a = observe(d, obs(DAY0 + MINUTE_MS, { hoard_wyrm: 1000, tokens_per_near: 30 }));
  assert.equal(a.kind, "hunt");
  assert.equal(a.amount_in, 100, "10% of the opening hoard");
  settle(d, a, 3);
  assert.ok(Math.abs(d.hoard.realized_near - 2) < 1e-12);
  assert.equal(observe(d, obs(DAY0 + 2 * MINUTE_MS, { hoard_wyrm: 900, tokens_per_near: 30 })).kind, "none");
});

test("pause: no buys while paused", () => {
  const d = newDragon({ paused_until: DAY0 + DAY_MS });
  assert.equal(drive(d, DAY0, 1440).length, 0);
});

test("spread: the budget opens linearly over the spread, and all of it is spent by its end", () => {
  const d = newDragon({ ...LADDER, spread_ms: 6 * 60 * MINUTE_MS, gap_ms: 15 * MINUTE_MS });
  const acts = drive(d, DAY0, 1440, () => ({ depth_near: 1e9 }));
  const start = d.day.breath_at;
  assert.ok(start + 6 * 60 * MINUTE_MS <= DAY0 + DAY_MS, "the spread ends inside the UTC day");
  const spentBy = (t) => acts.filter((a) => a.now <= t).reduce((s, a) => s + a.amount_in, 0);
  assert.ok(spentBy(start + 3 * 60 * MINUTE_MS) <= (300 * 181) / 360 + 1e-9, "about half of 300 after half the spread, counting the breath minute");
  assert.ok(spentBy(start + 3 * 60 * MINUTE_MS) >= 140, "and not much less");
  assert.ok(Math.abs(spentBy(DAY0 + DAY_MS) - 300) < 1e-9, "the whole budget by the end of the day");
  for (let i = 1; i < acts.length; i++) assert.ok(acts[i].now - acts[i - 1].now >= 15 * MINUTE_MS, "slices keep the configured gap");
});

test("slice floor: an amount under 0.01 NEAR is never sent", () => {
  const d = newDragon({ budget_bps: 1000 });
  const acts = drive(d, DAY0, 1440, () => ({ reserve: 0.05 }));
  assert.equal(acts.length, 0);
});
