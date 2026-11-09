// Coins from gym/data/nearly-coins.json.gz as the Gym uses them: minute bars, dense replay paths, and the
// "top coin" definition the tuning study selects on.
import { readFileSync } from "node:fs";
import { gunzipSync } from "node:zlib";
import { DAY_MS, MINUTE_MS } from "./dragon-rules.mjs";
import { SUPPLY } from "./sim.mjs";

// The local SQLite trade database that gym/extract.mjs reads, named by WYRM_GYM_DB. Without it the Gym uses the
// published dataset.
export const DB_PATH = process.env.WYRM_GYM_DB;
export const loadDataset = () => JSON.parse(gunzipSync(readFileSync(new URL("./data/nearly-coins.json.gz", import.meta.url))));

const sum = (xs) => xs.reduce((a, b) => a + b, 0);
const median = (xs) => { const s = [...xs].sort((a, b) => a - b); return s[Math.floor(s.length / 2)]; };

export function coinView(c) {
  let m = c.start_min;
  const minutes = c.bars.dm.map((d) => (m += d));
  const dayVol = new Map(), dayBuy = new Map();
  minutes.forEach((mm, i) => {
    const d = Math.floor(mm / 1440);
    dayVol.set(d, (dayVol.get(d) ?? 0) + c.bars.v[i]);
    dayBuy.set(d, (dayBuy.get(d) ?? 0) + c.bars.b[i]);
  });
  return { ...c, minutes, dayVol, dayBuy, swaps: sum(c.bars.n), life: sum(c.bars.v) };
}

export function densePath(c) {
  const n = c.minutes.at(-1) - c.start_min + 1;
  const price = new Float64Array(n), vol = new Float64Array(n), buy = new Float64Array(n), trades = new Float64Array(n);
  c.minutes.forEach((mm, i) => { const k = mm - c.start_min; price[k] = c.bars.p[i]; vol[k] = c.bars.v[i]; buy[k] = c.bars.b[i]; trades[k] = c.bars.n[i]; });
  const t0 = c.start_min * MINUTE_MS;
  const depth = (i) => c.depth[Math.min(c.depth.length - 1, Math.floor((t0 + i * MINUTE_MS) / DAY_MS) - c.depth_day0)];
  return { id: c.id, t0, price, vol, buy, trades, depth, start_min: c.start_min };
}

/**
 * A top coin is judged by its peak market cap in NEAR and by trading that lasted. The peak is the highest hourly
 * median price, over hours with at least 3 priced minutes, times the 1B supply, so one stray trade cannot make a
 * coin top. Sustained means at least `SUSTAIN.days` UTC days that each traded at least `SUSTAIN.share` of the
 * tier's market cap in NEAR.
 */
export const TIERS = Object.freeze([
  { name: "rising", peak_mc_near: 10_000 },
  { name: "top", peak_mc_near: 30_000 },
  { name: "breakout", peak_mc_near: 100_000 },
]);
export const SUSTAIN = Object.freeze({ days: 5, share: 0.01, min_priced_per_hour: 3 });

export function coinProfile(c) {
  const hours = new Map();
  c.minutes.forEach((mm, i) => {
    if (!(c.bars.p[i] > 0)) return;
    const h = Math.floor(mm / 60);
    (hours.get(h) ?? hours.set(h, []).get(h)).push(c.bars.p[i]);
  });
  let peak = 0, peakHour = null, first = null;
  for (const [h, ps] of [...hours].sort((a, b) => a[0] - b[0])) {
    if (ps.length < SUSTAIN.min_priced_per_hour) continue;
    const p = median(ps);
    if (first == null) first = p;
    if (p > peak) { peak = p; peakHour = h; }
  }
  const dailyVolumes = [...c.dayVol.values()];
  return {
    id: c.id, pair: c.pair ?? "wrap.near",
    peak_mc_near: peak * SUPPLY, first_mc_near: (first ?? 0) * SUPPLY,
    peak_day: peakHour == null ? null : Math.floor(peakHour / 24) - Math.floor(c.start_min / 1440),
    days: c.dayVol.size, life: c.life, dailyVolumes,
  };
}

// Index into TIERS of the highest tier the coin meets, or -1.
export function tierOf(p) {
  let best = -1;
  TIERS.forEach((t, k) => {
    const sustained = p.dailyVolumes.filter((v) => v >= t.peak_mc_near * SUSTAIN.share).length >= SUSTAIN.days;
    if (p.peak_mc_near >= t.peak_mc_near && sustained) best = k;
  });
  return best;
}
