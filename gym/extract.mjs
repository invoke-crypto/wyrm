// Reads the local trade database (WYRM_GYM_DB) and writes gym/data/nearly-coins.json.gz: per-coin minute bars
// (price in NEAR, NEAR volume, trade count) and per-day pool NEAR depth estimates.
// Nothing about traders or counterparties leaves this file.
import { DatabaseSync } from "node:sqlite";
import { mkdirSync, writeFileSync } from "node:fs";
import { gzipSync } from "node:zlib";

import { DB_PATH as DB } from "./coins.mjs";
const OUT = new URL("./data/nearly-coins.json.gz", import.meta.url);
const MIN = 60_000, DAY = 86_400_000;
// A trade smaller than this is too noisy to set the price.
const MIN_PRICE_NEAR = 0.01;
const SUFFIX = ".nearlytrade.near";

const db = new DatabaseSync(DB, { readOnly: true });
const nearUsd = db.prepare(`select time, close from near_usd order by time`).all();
const nearUsdLast = nearUsd.at(-1).close;
const nearUsdAt = (ts) => nearUsd[Math.max(0, upperBound(nearUsd, ts, (r) => r.time) - 1)].close;
const tokenInfo = new Map(db.prepare(`select account_id, decimals, liquidity_usd, main_pool from tokens where account_id like '%${SUFFIX}'`).all().map((t) => [t.account_id, t]));
const [dataStart, dataEnd] = Object.values(db.prepare(`select min(ts) a, max(ts) b from trades`).get()).map(Number);
const rows = db.prepare(`select ts, token, side, amount, counterparty cp, counterparty_amount ca, usd, pool_id from trades where token like '%${SUFFIX}' order by token, ts, id`).all();

const median = (xs) => { const s = [...xs].sort((a, b) => a - b); return s[Math.floor(s.length / 2)]; };
function upperBound(arr, x, key) {
  let lo = 0, hi = arr.length;
  while (lo < hi) { const m = (lo + hi) >> 1; if (key(arr[m]) <= x) lo = m + 1; else hi = m; }
  return lo;
}
// Nearly launches a coin in a 1%-fee DCL pool, against wNEAR or another token (RHEA, QTC, another Nearly coin).
// Only that pool prices the coin: side pools showed prices 35x off. The token table names it as main_pool.
const HOUR = 3_600_000;
const launchPool = (tok) => `REFDCL-${[tok, "wrap.near"].sort().join("|")}|10000`;
const pricePool = (tok) => { const mp = tokenInfo.get(tok)?.main_pool; return mp?.startsWith("REFDCL-") && mp.endsWith("|10000") ? mp : launchPool(tok); };

// A non-Nearly counterparty's NEAR value per raw unit, per hour: the median of usd / raw amount over that hour's
// trades, over NEAR/USD. The median drops the usd column's outliers. Decimals cancel, so none are needed.
const usdPerRaw = new Map();
for (const r of rows) {
  if (r.cp === "wrap.near" || r.cp.endsWith(SUFFIX) || !(r.usd > 0) || !(Number(r.ca) > 0)) continue;
  const k = `${r.cp}|${Math.floor(Number(r.ts) / HOUR)}`;
  (usdPerRaw.get(k) ?? usdPerRaw.set(k, []).get(k)).push(r.usd / Number(r.ca));
}
for (const [k, xs] of usdPerRaw) usdPerRaw.set(k, median(xs));
function usdTrackNear(cp, ts, raw) {
  const h = Math.floor(ts / HOUR);
  for (let d = 0; d <= 6; d++) for (const hh of [h - d, h + d]) { const u = usdPerRaw.get(`${cp}|${hh}`); if (u) return (u * raw) / nearUsdAt(ts); }
  return null;
}

const coins = new Map();
for (const r of rows) {
  const tok = Number(r.amount) / 10 ** (tokenInfo.get(r.token)?.decimals ?? 18);
  const wnear = r.cp === "wrap.near";
  const near = wnear ? Number(r.ca) / 1e24 : null;
  const inPricePool = r.pool_id === pricePool(r.token);
  const price = inPricePool && near >= MIN_PRICE_NEAR && tok > 0 ? near / tok : null;
  let ev = coins.get(r.token);
  if (!ev) coins.set(r.token, (ev = []));
  ev.push({ ts: Number(r.ts), side: r.side, tok, near, wnear, price, inPricePool, cp: r.cp, caRaw: Number(r.ca), cpTok: Number(r.ca) / 1e18, usdNear: r.usd > 0 ? r.usd / nearUsdAt(Number(r.ts)) : null });
}

const priceTracks = new Map();
const buildTracks = () => { for (const [t, ev] of coins) {
  const priced = ev.filter((e) => e.price != null);
  if (priced.length) priceTracks.set(t, (ts) => { const i = upperBound(priced, ts, (e) => e.ts); return i ? priced[i - 1].price : null; });
} };
buildTracks();
// Price-pool trades against a token: value the counterparty with its own NEAR track when it is a Nearly coin,
// else with the hourly usd track. Three rounds resolve chains such as a coin paired with a RHEA-paired coin.
for (let round = 0; round < 3; round++) {
  let added = 0;
  for (const [t, ev] of coins) for (const e of ev) {
    if (!e.inPricePool || e.wnear || e.price != null || !(e.tok > 0)) continue;
    const own = priceTracks.get(e.cp)?.(e.ts);
    const near = own != null ? e.cpTok * own : e.cp.endsWith(SUFFIX) ? null : usdTrackNear(e.cp, e.ts, e.caRaw);
    if (near >= MIN_PRICE_NEAR) { e.near = near; e.price = near / e.tok; added++; }
  }
  buildTracks();
  if (!added) break;
}
// Check the usd track against a direct track: NEARLY is priced in wNEAR, so its usd-track value should agree.
const nearlyCheck = [];
for (const r of rows) {
  if (r.cp !== "nearly-993927.nearlytrade.near" || !(r.usd > 0)) continue;
  const k = `${r.cp}|${Math.floor(Number(r.ts) / HOUR)}`, direct = priceTracks.get(r.cp)?.(Number(r.ts));
  if (direct) nearlyCheck.push({ k, ts: Number(r.ts), direct, usd: r.usd / Number(r.ca) / nearUsdAt(Number(r.ts)) * 1e18 });
}
const byHour = new Map();
for (const c of nearlyCheck) (byHour.get(c.k) ?? byHour.set(c.k, []).get(c.k)).push(c);
const usdTrackError = median([...byHour.values()].map((cs) => Math.abs(median(cs.map((c) => c.usd)) / median(cs.map((c) => c.direct)) - 1)));
// A cross-pair trade has no exact NEAR amount. The usd column is unreliable (one 49.8 NEAR trade reads $49.4M), so take the lowest estimate.
let unvalued = 0;
for (const [t, ev] of coins) for (const e of ev) {
  if (e.wnear || (e.inPricePool && e.price != null)) continue;
  const own = priceTracks.get(t)?.(e.ts), cpP = priceTracks.get(e.cp)?.(e.ts);
  const ests = [own != null ? e.tok * own : null, cpP != null ? e.cpTok * cpP : null, e.usdNear].filter((x) => x > 0);
  if (ests.length) e.near = Math.min(...ests); else unvalued++;
}

const sig = (x, n) => (x === 0 ? 0 : Number(x.toPrecision(n)));
// CPMM-equivalent NEAR depth from a trade between two same-side neighbours within 30 min, so the fee spread cancels.
function depthSamples(priced) {
  const out = [];
  for (let i = 1; i < priced.length - 1; i++) {
    const j = priced[i - 1], e = priced[i], k = priced[i + 1];
    if (e.near < 0.5 || j.side !== k.side || k.ts - j.ts > 30 * MIN) continue;
    const r = Math.sqrt(k.price / j.price);
    const y = e.side === "buy" ? (r > 1 ? e.near / (r - 1) : null) : (r < 1 ? e.near / (1 - r) : null);
    if (y) out.push({ day: Math.floor(e.ts / DAY), y, near: e.near, move: k.price / j.price - 1 });
  }
  return out;
}

const out = [];
for (const [token, ev] of coins) {
  const bars = new Map();
  for (const e of ev) {
    if (e.near == null) continue;
    const m = Math.floor(e.ts / MIN);
    const b = bars.get(m) ?? { p: 0, v: 0, b: 0, n: 0 };
    if (e.price != null) b.p = e.price;
    b.v += e.near; b.n++;
    if (e.side === "buy") b.b += e.near;
    bars.set(m, b);
  }
  if (!bars.size) continue;
  const mins = [...bars.keys()];
  const firstDay = Math.floor((mins[0] * MIN) / DAY), lastDay = Math.floor((mins.at(-1) * MIN) / DAY);
  const samples = depthSamples(ev.filter((e) => e.price != null));
  const liq = tokenInfo.get(token)?.liquidity_usd;
  const snap = liq > 0 ? liq / nearUsdLast / 2 : null;
  const coinDepth = samples.length >= 10 ? median(samples.map((s) => s.y)) : snap;
  const depth = [];
  for (let d = firstDay; d <= lastDay; d++) {
    const ys = samples.filter((s) => s.day === d).map((s) => s.y);
    depth.push(ys.length >= 5 ? sig(median(ys), 4) : coinDepth != null ? sig(coinDepth, 4) : null);
  }
  out.push({
    id: token.slice(0, -SUFFIX.length),
    pair: (() => { const [a, b] = pricePool(token).slice(7).split("|"); const cp = a === token ? b : a; return cp.endsWith(SUFFIX) ? cp.slice(0, -SUFFIX.length) : cp; })(),
    start_min: mins[0],
    depth_source: samples.length >= 10 ? "implied" : snap != null ? "snapshot" : "none",
    snapshot_depth_near: snap != null ? sig(snap, 4) : null,
    depth_day0: firstDay,
    depth,
    // One row per depth sample: [day offset, trade size in NEAR, price move from the trade before to the trade after].
    impact: samples.map((s) => [s.day - firstDay, sig(s.near, 3), sig(s.move, 3)]),
    bars: {
      dm: mins.map((m, i) => (i ? m - mins[i - 1] : 0)),
      p: mins.map((m) => sig(bars.get(m).p, 6)),
      v: mins.map((m) => sig(bars.get(m).v, 5)),
      // NEAR value of the buys alone: Nearly pays the creator from the NEAR side of the pool fee.
      b: mins.map((m) => sig(bars.get(m).b, 5)),
      n: mins.map((m) => bars.get(m).n),
    },
  });
}
out.sort((a, b) => a.start_min - b.start_min || (a.id < b.id ? -1 : 1));

const dataset = {
  about: "Derived from Nearly.trade swaps recorded by a private indexer. Price is NEAR per whole token from the coin's launch pool; volume is NEAR notional of every valued swap, buy volume of its buys; depth is an estimated pool NEAR side. No account ids.",
  data_start_ms: dataStart, data_end_ms: dataEnd, near_usd_last: nearUsdLast,
  swaps: rows.length, unvalued_swaps: unvalued,
  usd_track_error: sig(usdTrackError, 3),
  coins: out,
};
mkdirSync(new URL("./data/", import.meta.url), { recursive: true });
const gz = gzipSync(JSON.stringify(dataset), { level: 9 });
writeFileSync(OUT, gz);
console.log(`wrote ${OUT.pathname} ${(gz.length / 1e6).toFixed(2)} MB, ${out.length} coins, ${out.reduce((a, c) => a + c.bars.dm.length, 0)} bars, ${rows.length} swaps, ${unvalued} unvalued, usd track vs direct NEARLY price ${(usdTrackError * 100).toFixed(2)}% median error`);
