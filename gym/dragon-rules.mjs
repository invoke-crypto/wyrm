// The Dragon's buying rules from docs/SPEC.md "dragon", as one step function shaped like the
// contract's on_observe: `observe(dragon, obs)` changes `dragon` in place (the contract's &mut self)
// and returns the slice to trade, and `settle` records a fill like on_swap. No clock, no randomness,
// no I/O: everything comes in through `obs`. Field names follow the contract's state in snake_case.
// Times are milliseconds (the contract uses nanoseconds), amounts are NEAR and whole WYRM as floats.
// Quotes come in as functions on obs, standing in for the DCL quote calls of the Quoting phase.

export const MINUTE_MS = 60_000;
export const DAY_MS = 86_400_000;

export const FIXED = Object.freeze({
  slice_gap_ms: 60_000,
  confirm_samples: 3,
  confirm_span_ms: 10 * MINUTE_MS,
  sample_every_ms: MINUTE_MS,
  ring: 1440,
  slippage_bps: 150,
  max_halvings: 6,
  // SPEC decisions (Nov 3): the smallest slice is 0.01 NEAR of value.
  min_slice_near: 0.01,
});

export const DEFAULT_SETTINGS = Object.freeze({
  budget_bps: 2000,
  steps_bps: [2000, 3500, 5000],
  extra_bps: [500, 1000, 1500],
  cap_bps: 250,
  hunt: false,
  sell_multiple_bps: 25000,
  max_sell_bps: 1000,
  paused_until: 0,
  // Fixed at launch in the contract. A setting here only so the Gym can sweep it.
  window_ms: DAY_MS,
  // Fixed at 1,500 in the contract; null switches the spike guard off for the Gym's comparison.
  spike_bps: 1500,
  // Fixed at 60 s in the contract. A setting here only so the Gym can sweep it.
  gap_ms: FIXED.slice_gap_ms,
  // 0 in the contract: the whole budget opens at the breath minute. A positive value opens it linearly over
  // that many ms, and the breath minute is drawn so the spread ends inside the UTC day.
  spread_ms: 0,
});

export function newDragon(settings = {}) {
  return {
    settings: { ...DEFAULT_SETTINGS, ...settings },
    day: null, // { index, breath_at, base_reserve, budget, spent, sold_wyrm, base_hoard }
    hoard: { wyrm: 0, near_spent: 0, wyrm_bought: 0, donated: 0, realized_near: 0 },
    samples: [], // oldest first, { at, tokens_per_near }
    // Running sum of tokens_per_near * gap-to-next-sample over `samples`, so the reference is O(1).
    ref_area: 0,
    last_slice_at: -Infinity,
  };
}

// The breath minute is drawn once per UTC day from the first observe's random seed (u32 of env::random_seed()).
export const breathMinute = (random_u32, spreadMinutes = 0) => random_u32 % Math.max(1, 1440 - spreadMinutes);

function recordSample(d, at, tpn) {
  const s = d.samples, last = s.at(-1);
  if (last && at - last.at < FIXED.sample_every_ms) return;
  if (last) d.ref_area += last.tokens_per_near * (at - last.at);
  s.push({ at, tokens_per_near: tpn });
  while (s.length > FIXED.ring || (s.length > 1 && s[0].at < at - d.settings.window_ms)) {
    d.ref_area -= s[0].tokens_per_near * (s[1].at - s[0].at);
    s.shift();
  }
}

// Time-weighted mean over the window. Each sample weighs the gap until the next sample, the newest
// one weighs the time since it was taken, so a sample taken after a long silence starts at weight 0.
export function reference(d, now) {
  const s = d.samples;
  if (!s.length) return null;
  const span = now - s[0].at;
  if (span <= 0) return s[0].tokens_per_near;
  return (d.ref_area + s.at(-1).tokens_per_near * (now - s.at(-1).at)) / span;
}

// A dip of step bps: tokens_per_near >= reference * 10_000 / (10_000 - step).
const beyond = (tpn, ref, step) => tpn * (10_000 - step) >= ref * 10_000;

// Deepest step confirmed now: the current quote and at least 3 consecutive newest samples spanning 10 min are beyond it.
export function confirmedStep(d, now, tpn) {
  const ref = reference(d, now);
  if (ref == null) return -1;
  let best = -1;
  d.settings.steps_bps.forEach((step, k) => {
    if (!beyond(tpn, ref, step)) return;
    let n = 0, first = null;
    for (let i = d.samples.length - 1; i >= 0 && beyond(d.samples[i].tokens_per_near, ref, step); i--) { n++; first = d.samples[i].at; }
    if (n >= FIXED.confirm_samples && now - first >= FIXED.confirm_span_ms) best = k;
  });
  return best;
}

function rollDay(d, obs) {
  const index = Math.floor(obs.now / DAY_MS);
  if (d.day?.index === index) return;
  d.day = {
    index,
    breath_at: index * DAY_MS + breathMinute(obs.random_u32, Math.round(d.settings.spread_ms / MINUTE_MS)) * MINUTE_MS,
    base_reserve: obs.reserve,
    budget: (obs.reserve * d.settings.budget_bps) / 10_000,
    spent: 0,
    sold_wyrm: 0,
    base_hoard: d.hoard.wyrm,
  };
}

// Spending allowed so far today: the breath budget once the breath minute has passed, plus the extras
// of the confirmed step and every step above it (cumulative), as bps of the day's opening reserve.
export function allowance(d, now, step) {
  const { day, settings } = d;
  const since = now - day.breath_at;
  let a = since < 0 ? 0 : settings.spread_ms > 0 ? day.budget * Math.min(1, (since + MINUTE_MS) / settings.spread_ms) : day.budget;
  for (let k = 0; k <= step; k++) a += (day.base_reserve * settings.extra_bps[k]) / 10_000;
  return a;
}

// Halve the candidate until its average price is at most cap_bps worse than the 1-unit price; null if still too big.
export function fitSlice(amount, unitOut, quote, capBps) {
  for (let i = 0; i <= FIXED.max_halvings && amount >= FIXED.min_slice_near; i++, amount /= 2) {
    if (quote(amount) * 10_000 >= amount * unitOut * (10_000 - capBps)) return amount;
  }
  return null;
}

// Spot more than spike_bps above the reference price: tokens_per_near * (10_000 + spike) < reference * 10_000.
export const spiking = (tpn, ref, spikeBps) => spikeBps != null && ref != null && tpn * (10_000 + spikeBps) < ref * 10_000;

/**
 * obs: { now, reserve (NEAR), hoard_wyrm, tokens_per_near (quote for 1 NEAR), random_u32,
 *        quote_buy(near) -> WYRM out, quote_sell(wyrm) -> NEAR out }
 * returns { kind: "none" | "breath" | "dip" | "hunt", amount_in, reference, step }
 *   amount_in is NEAR for a buy and WYRM for a hunt sell.
 */
export function observe(d, obs) {
  const { now, tokens_per_near: tpn } = obs;
  d.hoard.wyrm = obs.hoard_wyrm;
  recordSample(d, now, tpn);
  rollDay(d, obs);
  const s = d.settings, ref = reference(d, now);
  const none = (why) => ({ kind: "none", amount_in: 0, reference: ref, step: -1, why });
  if (now - d.last_slice_at < s.gap_ms) return none("gap");

  if (s.hunt && d.hoard.wyrm_bought > 0) {
    const avgCostNear = d.hoard.near_spent / d.hoard.wyrm_bought;
    if (10_000 / tpn >= (avgCostNear * s.sell_multiple_bps)) {
      const left = (d.day.base_hoard * s.max_sell_bps) / 10_000 - d.day.sold_wyrm;
      const want = Math.min(left, d.hoard.wyrm);
      // SPEC defines the impact check for buys only; a sell is held to the same cap against the sell quote for 1 NEAR worth.
      const amount = want > 0 ? fitSlice(want, obs.quote_sell(tpn) / tpn, obs.quote_sell, s.cap_bps) : null;
      if (amount) return take(d, now, { kind: "hunt", amount_in: amount, reference: ref, step: -1 });
    }
  }
  if (now < s.paused_until) return none("paused");
  if (spiking(tpn, ref, s.spike_bps)) return none("spike");
  const step = confirmedStep(d, now, tpn);
  const want = Math.min(allowance(d, now, step) - d.day.spent, obs.reserve);
  if (!(want > 0)) return none(now < d.day.breath_at && step < 0 ? "before breath" : "budget spent");
  const amount = fitSlice(want, tpn, obs.quote_buy, s.cap_bps);
  if (amount == null) return none("impact");
  return take(d, now, { kind: step >= 0 ? "dip" : "breath", amount_in: amount, reference: ref, step });
}

// State changes before the swap goes out, as the contract does before any cross-contract call.
function take(d, now, action) {
  d.last_slice_at = now;
  if (action.kind === "hunt") d.day.sold_wyrm += action.amount_in;
  else d.day.spent += action.amount_in;
  return action;
}

export const minOut = (quote) => (quote * (10_000 - FIXED.slippage_bps)) / 10_000;

// On a filled swap: a buy received `amount_out` WYRM, a hunt sale received `amount_out` NEAR.
export function settle(d, action, amount_out) {
  const h = d.hoard;
  if (action.kind === "hunt") {
    const cost = (h.near_spent / h.wyrm_bought) * action.amount_in;
    h.realized_near += amount_out - cost;
    h.near_spent -= cost; h.wyrm_bought -= action.amount_in; h.wyrm -= action.amount_in;
  } else {
    h.near_spent += action.amount_in; h.wyrm_bought += amount_out; h.wyrm += amount_out;
  }
}
