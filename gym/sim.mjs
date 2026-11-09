// Market model and buying strategies over a dense per-minute path. Pure functions of their inputs.
import { newDragon, observe, settle, DAY_MS, MINUTE_MS } from "./dragon-rules.mjs";

// Every DCL swap pays a 1% pool fee. DCL keeps 20% of it and the locker's position earns 0.8% (docs/research/NEARLY.md).
export const POOL_FEE = 0.01;
// The creator side gets 60% of the locker's 0.8%. Nearly pays it in NEAR from the NEAR side, so it is 0.48% of buy
// volume: fees_near_total / volume_y_in is 0.008 measured, creator_share_bps 6000 on recent launches.
export const CREATOR_OF_BUYS = 0.008 * 0.6;
// A token tax is taken on both sides; the platform keeps 40% (platform_bps 4000) and the creator side gets 60%.
export const CREATOR_OF_TAX = 0.6;
// WYRM launches with 400 bps on buys and 400 bps on sells, and the whole creator side goes to the tithe (operator).
export const WYRM_TAX = 0.04;
export const DRAGON_SHARE = 0.5; // the tithe splits the creator side 50/50 between the Dragon and the 100 Scales
// The Dragon's income per NEAR of buy volume and per NEAR of all volume, at a token tax of `tax` a side.
export const incomeOf = (tax = 0) => ({ perBuy: CREATOR_OF_BUYS * DRAGON_SHARE, perVolume: tax * CREATOR_OF_TAX * DRAGON_SHARE });
// What one swap costs the buyer: the pool fee, plus a token tax when the coin has one.
export const costOf = (tax = 0) => POOL_FEE + tax;
export const COST = costOf(WYRM_TAX);
export const SUPPLY = 1e9;
// Nearly claims fees about :07 and pushes them to fee_to about :12 of every hour (docs/research/NEARLY.md).
export const FEE_BATCH_MIN = 60;
export const FEE_BATCH_AT = 12;

export function rng(seed) {
  let h = 2166136261;
  for (const c of String(seed)) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  return () => {
    h += 0x6d2b79f5;
    let t = h;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// CPMM on NEAR depth y: buying with B NEAR at mid price p (NEAR per token) gets B'/p * y/(y+B') after fee and tax.
export function buyTokens(near, price, depth, cost = COST) {
  const b = near * (1 - cost);
  return (b / price) * (depth / (depth + b));
}
export function sellNear(tokens, price, depth, cost = COST) {
  const gross = tokens * price;
  return gross * (depth / (depth + gross)) * (1 - cost);
}
// The DCL quote the Dragon sees charges the pool fee; Nearly's tax is taken on the actual swap.
export const quoteBuy = (near, price, depth) => buyTokens(near, price, depth, POOL_FEE);
export const quoteSell = (tokens, price, depth) => sellNear(tokens, price, depth, POOL_FEE);

/**
 * A path is dense per minute: { t0 (ms of minute 0), price: Float64Array (0 = not priced yet), vol and buy:
 * Float64Array (NEAR of all swaps and of buys), depth(i) }. Every strategy sees the same inflow, `incomeOf(tax)`
 * applied to each minute's volumes, plus `seed_reserve` at minute 0. Every buy pays `costOf(tax)`, and its own
 * creator-side fee and tax come back as income in the next batch, because the Dragon's buys are buy volume too.
 * Income is paid into the reserve at minute `batch_at` of each `batch_min` block of UTC minutes, and what is still pending
 * at the end of the path is added to the reserve then. `marks` (ms, ascending) record a snapshot of the books
 * as each mark is crossed, for scoring a time split with `segmentValue`.
 * `feedback` lifts every later price by the buy's CPMM impact ((y + b) / y)^2 and never decays: an upper bound
 * on how much the treasury's own buying moves the market, where the default (no feedback) is the lower bound.
 */
export function run(path, strategy, { tax = WYRM_TAX, seed_reserve = 0, seed = 0, daily = false, feedback = false, batch_min = FEE_BATCH_MIN, batch_at = FEE_BATCH_AT, marks = [] } = {}) {
  const rand = rng(`${path.id}|${seed}`);
  const { perBuy, perVolume } = incomeOf(tax), cost = costOf(tax);
  const st = { cost, reserve: seed_reserve, tokens: 0, spent: 0, got: 0, inflow: seed_reserve, buys: 0, realized: 0, slices: [] };
  const days = [];
  const act = strategy.init?.(rand);
  let lastPrice = 0, dayVol = 0, lift = 1, pending = 0, mark = 0;
  const snaps = [];
  const snap = () => ({ reserve: st.reserve, inflow: st.inflow, got: st.got, spent: st.spent, price: lastPrice, cost });
  const n = path.price.length;
  for (let i = 0; i < n; i++) {
    const now = path.t0 + i * MINUTE_MS;
    for (; mark < marks.length && marks[mark] <= now; mark++) snaps.push(snap());
    pending += path.buy[i] * perBuy + path.vol[i] * perVolume; dayVol += path.vol[i];
    if (Math.floor(now / MINUTE_MS) % batch_min === batch_at) { st.reserve += pending; st.inflow += pending; pending = 0; }
    if (path.price[i] > 0) lastPrice = path.price[i] * lift;
    if (lastPrice > 0) {
      const spent = st.spent, depth = path.depth(i);
      strategy.step(st, act, { now, i, price: lastPrice, depth, rand });
      if (st.spent > spent) pending += (st.spent - spent) * (perBuy + perVolume);
      if (feedback && st.spent > spent) {
        const k = ((depth + (st.spent - spent) * (1 - cost)) / depth) ** 2;
        lift *= k; lastPrice *= k;
      }
    }
    if (daily && ((i + 1) % 1440 === 0 || i === n - 1)) {
      days.push({ reserve: st.reserve, hoard: st.tokens, price: lastPrice, volume: dayVol });
      dayVol = 0;
    }
  }
  st.reserve += pending; st.inflow += pending;
  for (; mark < marks.length; mark++) snaps.push(snap());
  const V = st.reserve + st.tokens * lastPrice;
  return { ...st, V, lastPrice, tpn: st.spent > 0 ? st.got / st.spent : null, days, snaps, end: snap() };
}

export const ZERO_SNAP = Object.freeze({ reserve: 0, inflow: 0, got: 0, spent: 0, price: 0, cost: COST });
/**
 * Tokens per NEAR of money available in a segment between two snapshots: what was bought in it, plus the NEAR
 * still unspent at its end marked as bought at the end price after fee and tax (no impact), over the reserve at
 * its start plus the income inside it. Unspent NEAR is not free: holding back only pays if later prices are lower.
 */
export function segmentValue(a, b) {
  const avail = a.reserve + (b.inflow - a.inflow);
  if (!(avail > 0) || !(b.price > 0)) return null;
  return (b.got - a.got + (b.reserve * (1 - b.cost)) / b.price) / avail;
}

function buy(st, near, price, depth, kind = "buy", now = 0) {
  if (!(near > 0)) return 0;
  const got = buyTokens(near, price, depth, st.cost);
  st.reserve -= near; st.spent += near; st.tokens += got; st.got += got; st.buys++;
  if (kind !== "buy") st.slices.push({ kind, near, depth });
  return got;
}

export const strategies = {
  none: { name: "no treasury", step() {} },
  instant: { name: "instant buy", step(st, _a, m) { if (st.reserve >= 0.05) buy(st, st.reserve, m.price, m.depth); } },
  // Plain DCA of an income stream: on each hour, buy one 24th of the income of the last 24 hours.
  dca: {
    name: "plain DCA",
    init: () => ({ hour: -1, seen: 0, ring: [] }),
    step(st, a, m) {
      const hour = Math.floor(m.now / 3_600_000);
      if (hour === a.hour) return;
      a.hour = hour; a.ring.push(st.inflow - a.seen); a.seen = st.inflow;
      if (a.ring.length > 24) a.ring.shift();
      buy(st, Math.min(st.reserve, a.ring.reduce((x, y) => x + y, 0) / 24), m.price, m.depth);
    },
  },
  // The prototype's control: once per UTC day at a random minute, 30% of the reserve in one fill.
  dailyLump: {
    name: "daily lump",
    init: () => ({ day: -1, at: 0 }),
    step(st, a, m) {
      const day = Math.floor(m.now / DAY_MS);
      if (day !== a.day) { a.day = day; a.at = day * DAY_MS + Math.floor(m.rand() * 1440) * MINUTE_MS; a.done = false; }
      if (!a.done && m.now >= a.at) { a.done = true; buy(st, 0.3 * st.reserve, m.price, m.depth); }
    },
  },
};

// The Dragon's own rules, one observe per minute (a keeper poking every minute).
export function dragonStrategy(settings = {}, name = "Dragon") {
  return {
    name,
    init: () => newDragon(settings),
    step(st, d, m) {
      const action = observe(d, {
        now: m.now, reserve: st.reserve, hoard_wyrm: st.tokens, random_u32: Math.floor(m.rand() * 2 ** 32),
        tokens_per_near: quoteBuy(1, m.price, m.depth),
        quote_buy: (a) => quoteBuy(a, m.price, m.depth),
        quote_sell: (w) => quoteSell(w, m.price, m.depth),
      });
      if (action.kind === "none") return;
      if (action.kind === "hunt") {
        const near = sellNear(action.amount_in, m.price, m.depth, st.cost);
        const before = d.hoard.realized_near;
        settle(d, action, near);
        st.tokens -= action.amount_in; st.reserve += near; st.realized += d.hoard.realized_near - before;
        return;
      }
      settle(d, action, buy(st, action.amount_in, m.price, m.depth, action.kind, m.now));
    },
  };
}

// Best sandwich profit around a known buy of B NEAR into a CPMM with NEAR depth y, attacker size A, fee per side.
export function sandwichProfit(y, B, A, fee = COST) {
  let X = 1e9, Y = y;
  const buyN = (n) => { const k = X * Y; Y += n; const out = X - k / Y; X -= out; return out; };
  const sellT = (t) => { const k = X * Y; X += t; const out = Y - k / X; Y -= out; return out; };
  const got = buyN(A * (1 - fee));
  buyN(B * (1 - fee));
  return sellT(got) * (1 - fee) - A;
}
export const bestProfit = (f, fee = COST) => { let b = 0; for (let a = 0.001; a <= 3; a *= 1.05) b = Math.max(b, sandwichProfit(1000, f * 1000, a * 1000, fee)); return b; };
// Smallest buy (as a fraction of NEAR depth) a sandwich profits on, given an attacker overhead cost (NEAR per 1000
// depth) and the fee each side pays.
export function breakEven(cost = 1e-9, fee = COST) {
  let lo = 0.0001, hi = 1;
  for (let i = 0; i < 40; i++) { const m = (lo + hi) / 2; if (bestProfit(m, fee) > cost) hi = m; else lo = m; }
  return hi;
}
