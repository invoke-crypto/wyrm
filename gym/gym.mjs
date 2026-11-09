// Runs everything: rebuilds gym/data/nearly-coins.json.gz from the private DB when it exists (skip with
// --no-extract), then the top-coin study, presets, the Scale income estimator, sandwich math, 60-day scenarios,
// SVG charts in gym/out/ and docs/PARAMETERS.md. Deterministic: rerun and diff.
import { existsSync, writeFileSync, mkdirSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { DAY_MS, MINUTE_MS, DEFAULT_SETTINGS, newDragon, observe, settle } from "./dragon-rules.mjs";
import { run, strategies, dragonStrategy, rng, breakEven, bestProfit, quoteBuy, quoteSell, incomeOf, costOf, segmentValue, ZERO_SNAP, COST, SUPPLY, WYRM_TAX, FEE_BATCH_MIN, FEE_BATCH_AT } from "./sim.mjs";
import { DB_PATH, loadDataset, coinView, densePath } from "./coins.mjs";
import { topCoinStudy, describe } from "./study.mjs";
import { lineChart, barChart, fmt } from "./svg.mjs";
import { renderParameters } from "./parameters.mjs";

if (!process.argv.includes("--no-extract") && existsSync(DB_PATH)) {
  const r = spawnSync(process.execPath, [fileURLToPath(new URL("./extract.mjs", import.meta.url))], { stdio: "inherit" });
  if (r.status) process.exit(r.status);
}

const OUT = new URL("./out/", import.meta.url);
const ds = loadDataset();
const ACTIVE = { swaps: 300, days: 2, pricedMinutes: 100 };
const SEED_RESERVE = 80; // the Scale sale: 20 at 0.5, 30 at 1, 20 at 2 NEAR
const INCOME = incomeOf(WYRM_TAX);

const sorted = (xs) => xs.filter(Number.isFinite).sort((a, b) => a - b);
const q = (xs, f) => { const s = sorted(xs); return s.length ? s[Math.min(s.length - 1, Math.floor(f * s.length))] : null; };
const median = (xs) => q(xs, 0.5);
const dist = (xs) => ({ n: sorted(xs).length, p10: q(xs, 0.1), p25: q(xs, 0.25), p50: q(xs, 0.5), p75: q(xs, 0.75), p90: q(xs, 0.9), p99: q(xs, 0.99), max: sorted(xs).at(-1) ?? null });
const sum = (xs) => xs.reduce((a, b) => a + b, 0);

const coins = ds.coins.map(coinView);
const study = topCoinStudy(ds);
const preset = Object.fromEntries(study.presets.map((p) => [p.name, p]));
const wyrm = preset.hoarder.settings;

const coinDays = coins.flatMap((c) => [...c.dayVol].map(([d, v]) => ({ v, b: c.dayBuy.get(d) })));
const lifetimes = coins.map((c) => c.life);
const income = {
  coins: coins.length, coinDays: coinDays.length,
  dailyVolume: dist(coinDays.map((d) => d.v)),
  dragonPerDay: dist(coinDays.map((d) => d.b * INCOME.perBuy + d.v * INCOME.perVolume)),
  perScalePerDay: dist(coinDays.map((d) => (d.b * INCOME.perBuy + d.v * INCOME.perVolume) / 100)),
  lifetimeVolume: dist(lifetimes),
  coinsOverLifetime: Object.fromEntries([10, 100, 1000, 10000].map((t) => [t, lifetimes.filter((v) => v >= t).length])),
};

// The broad set: active coins that are not top coins, never used for tuning.
const topIds = new Set(study.definition.tiers[0].ids);
const active = coins.filter((c) => c.swaps >= ACTIVE.swaps && c.dayVol.size >= ACTIVE.days && c.bars.p.filter((p) => p > 0).length >= ACTIVE.pricedMinutes).map(densePath);
const others = active.filter((p) => !topIds.has(p.id));
const wholeOf = (paths, strat) => new Map(paths.map((p) => [p.id, segmentValue(ZERO_SNAP, run(p, strat).end)]));
const instantOthers = wholeOf(others, strategies.instant);
const vsInstant = (M) => ({ ...dist([...M].map(([id, v]) => v / instantOthers.get(id))), beats: [...M].filter(([id, v]) => v > instantOthers.get(id)).length / M.size });
const broad = {
  coins: others.length, activeCoins: active.length,
  rows: [["plain DCA", strategies.dca], ["daily lump 30%", strategies.dailyLump], ["contract default", dragonStrategy()], ...study.presets.slice(0, 4).map((p) => [`${p.name} preset`, dragonStrategy(p.settings)])]
    .map(([name, strat]) => ({ name, ...vsInstant(wholeOf(others, strat)) })),
};

const BE = { wyrm: breakEven(), noTax: breakEven(1e-9, costOf(0)), ...Object.fromEntries([10, 20, 30].map((s) => [`selfTrigger${s}`, breakEven(100 * (1 - Math.sqrt(1 - s / 100)))])) };
const sandwichCurve = [0.005, 0.01, 0.02, 0.03, 0.05, 0.06, 0.08, 0.1, 0.15].map((f) => ({ fillOverDepth: f, wyrm: bestProfit(f), noTax: bestProfit(f, costOf(0)) }));

// 60-day scenarios stitched from real days, run with the WYRM default.
const topCoin = [...active].sort((a, b) => sum(b.vol) - sum(a.vol))[0].id;
const dayLib = [];
for (const p of active) {
  if (p.id === topCoin) continue;
  const firstFull = Math.ceil(p.t0 / DAY_MS) * DAY_MS;
  for (let start = (firstFull - p.t0) / MINUTE_MS; start + 1440 <= p.price.length; start += 1440) {
    let open = 0;
    for (let i = start; i >= 0 && !open; i--) open = p.price[i];
    if (!open) continue;
    const rel = new Float64Array(1440), vol = p.vol.slice(start, start + 1440), buy = p.buy.slice(start, start + 1440);
    let last = open, hi = open, lo = open, trades = 0;
    for (let i = 0; i < 1440; i++) {
      if (p.price[start + i] > 0) last = p.price[start + i];
      rel[i] = last / open; hi = Math.max(hi, last); lo = Math.min(lo, last); trades += p.trades[start + i];
    }
    dayLib.push({ id: p.id, open, rel, vol, buy, depth: p.depth(start), r: last / open, range: hi / lo, trades });
  }
}
const swarmCut = q(dayLib.map((d) => d.trades), 0.9);
const CLASSES = {
  bleed: (d) => d.r >= 0.96 && d.r <= 0.995,
  flat: (d) => d.r >= 0.97 && d.r <= 1.03,
  chop: (d) => d.r >= 0.95 && d.r <= 1.05 && d.range >= 1.15,
  pump: (d) => d.r >= 1.4,
  dump: (d) => d.r <= 0.7,
  swarm: (d) => d.r >= 0.9 && d.r <= 1.1 && d.trades >= swarmCut,
};
const pools = Object.fromEntries(Object.entries(CLASSES).map(([k, f]) => [k, dayLib.filter(f)]));
const days = (k, n) => Array(n).fill(k);
const RECIPES = {
  "slow bleed": days("bleed", 60),
  "pump and dump": [...days("flat", 10), ...days("pump", 3), ...days("dump", 4), ...days("bleed", 43)],
  "sideways chop": days("chop", 60),
  "bot swarm": days("swarm", 60),
  "dip-step attack": days("chop", 60),
};
// Volumes come from real days of coins already trading, so the price level must come from the same days.
const startPrice = median(dayLib.map((d) => d.open));
const SCEN_T0 = (Math.floor(ds.data_end_ms / DAY_MS) + 1) * DAY_MS;
const ATTACK_DROP = 0.12, ATTACK_MINUTES = 20;
function scenarioPath(name, recipe) {
  const rand = rng(`scenario|${name}`);
  const n = recipe.length * 1440, price = new Float64Array(n), vol = new Float64Array(n), buy = new Float64Array(n), dayDepth = [];
  let level = startPrice;
  recipe.forEach((cls, d) => {
    const day = pools[cls][Math.floor(rand() * pools[cls].length)];
    const attackAt = name === "dip-step attack" ? 60 + Math.floor(rand() * 1300) : -1;
    for (let i = 0; i < 1440; i++) {
      const dropped = i >= attackAt && i < attackAt + ATTACK_MINUTES ? 1 - ATTACK_DROP : 1;
      price[d * 1440 + i] = level * day.rel[i] * dropped;
      // Keep the real day's turnover: its NEAR volume scaled from its own coin's price level to this path's.
      vol[d * 1440 + i] = (day.vol[i] * level) / day.open;
      buy[d * 1440 + i] = (day.buy[i] * level) / day.open;
    }
    dayDepth.push(day.depth);
    level *= day.r;
  });
  return { id: name, t0: SCEN_T0, price, vol, buy, depth: (i) => dayDepth[Math.floor(i / 1440)] };
}
const scenarios = Object.entries(RECIPES).map(([name, recipe]) => {
  const path = scenarioPath(name, recipe);
  const opts = { seed_reserve: SEED_RESERVE, daily: true };
  const dr = run(path, dragonStrategy(wyrm), opts), inst = run(path, strategies.instant, opts), old = run(path, dragonStrategy(), opts);
  const fb = run(path, dragonStrategy(wyrm), { ...opts, feedback: true });
  const dayIncome = Array.from({ length: recipe.length }, (_, d) => sum(path.buy.subarray(d * 1440, (d + 1) * 1440)) * INCOME.perBuy + sum(path.vol.subarray(d * 1440, (d + 1) * 1440)) * INCOME.perVolume);
  return {
    name,
    days: dr.days.map((d, i) => ({ ...d, heldPct: (d.hoard / SUPPLY) * 100, heldPctFeedback: (fb.days[i].hoard / SUPPLY) * 100, scaleIncome: dayIncome[i] / 100 })),
    priceEndOverStart: dr.lastPrice / startPrice,
    medianDayVolume: median(dr.days.map((d) => d.volume)),
    dragon: { tpn: dr.tpn, spent: dr.spent, reserve: dr.reserve, hoard: dr.tokens, inflow: dr.inflow, slices: dr.slices.length },
    dragonVsInstant: segmentValue(ZERO_SNAP, dr.end) / segmentValue(ZERO_SNAP, inst.end),
    contractVsInstant: segmentValue(ZERO_SNAP, old.end) / segmentValue(ZERO_SNAP, inst.end),
  };
});

// The attacker's side of a dip-step attack on a pool with feedback: dump to push the price 12% down,
// hold it there while the Dragon confirms and buys, then buy the same tokens back. Each side pays the pool fee.
function attackEconomics(settings, depth, reserve) {
  let Y = depth, X = depth / startPrice;
  const k0 = X * Y;
  const price = () => Y / X;
  const sellTokens = (t) => { const tin = t * (1 - COST); const out = Y - k0 / (X + tin); X += tin; Y -= out; return out; };
  const buyWithNear = (n) => { const nin = n * (1 - COST); const out = X - k0 / (Y + nin); Y += nin; X -= out; return out; };
  const d = newDragon(settings);
  const t0 = SCEN_T0 + 600 * MINUTE_MS;
  let res = reserve, hoard = 0;
  const obsAt = (now) => ({
    now, reserve: res, hoard_wyrm: hoard, random_u32: 0,
    tokens_per_near: quoteBuy(1, price(), Y), quote_buy: (a) => quoteBuy(a, price(), Y), quote_sell: (w) => quoteSell(w, price(), Y),
  });
  for (let i = -1440; i < 0; i++) observe(d, obsAt(t0 + i * MINUTE_MS));
  d.day.spent = d.day.budget; // the day's breath is already spent, so every buy in the window is the dip's extra
  const p0 = price();
  const dumped = (Math.sqrt(k0 / (p0 * (1 - ATTACK_DROP))) - X) / (1 - COST);
  const nearIn = sellTokens(dumped);
  let dragonNear = 0, dragonTokens = 0;
  for (let i = 0; i < ATTACK_MINUTES; i++) {
    const a = observe(d, obsAt(t0 + i * MINUTE_MS));
    if (a.kind === "breath" || a.kind === "dip") {
      const got = buyWithNear(a.amount_in);
      settle(d, a, got); res -= a.amount_in; hoard += got; dragonNear += a.amount_in; dragonTokens += got;
    }
  }
  const [x1, y1] = [X, Y];
  let lo = 0, hi = 10 * nearIn;
  for (let it = 0; it < 60; it++) { const m = (lo + hi) / 2; X = x1; Y = y1; if (buyWithNear(m) >= dumped) hi = m; else lo = m; }
  return { depth, reserve, dumpNear: dumped * p0, attackerPnL: nearIn - hi, dragonNear, dragonAvgPriceVsBefore: dragonNear > 0 ? dragonNear / dragonTokens / p0 : null };
}
const medianDepth = study.depth.p50;
const attack = [["contract default", DEFAULT_SETTINGS], ["dip-hunter preset", preset["dip-hunter"].settings]].flatMap(([who, s]) =>
  [[medianDepth, 200], [medianDepth, 2000], [medianDepth / 4, 2000]].map(([y, r]) => ({ who, ...attackEconomics(s, y, r) })));

const units = (s) => ({ budget_bps: s.budget_bps, steps_bps: s.steps_bps, extra_bps: s.extra_bps, cap_bps: s.cap_bps, hunt: s.hunt, sell_multiple_bps: s.sell_multiple_bps, max_sell_bps: s.max_sell_bps });
const presetsJson = {
  generated_by: "node gym/gym.mjs",
  data: { from: new Date(ds.data_start_ms).toISOString().slice(0, 10), to: new Date(ds.data_end_ms).toISOString().slice(0, 10), coins: study.coins, tier: study.definition.tiers[0] },
  metric: "Tokens per NEAR of income, unspent NEAR marked at the last price after the pool fee, per coin over instant buying of the same income; median over coins.",
  held_out: "cv: 5-fold by coin, each fold's setting chosen on the other coins. time: chosen on activity before the split, scored after it.",
  income_model: { token_tax_per_side: WYRM_TAX, dragon_share_of_buy_volume: INCOME.perBuy, dragon_share_of_all_volume: INCOME.perVolume, fee_batches: `every ${FEE_BATCH_MIN} min at :${FEE_BATCH_AT}`, cost_per_buy: COST },
  default: "hoarder",
  presets: study.presets.map((p) => ({
    name: p.name, suits: p.suits, describe: describe(p.settings),
    settings: units(p.settings),
    needs_contract_change: p.settings.spread_ms > 0 ? { spread_hours: p.settings.spread_ms / 3_600_000, slice_gap_seconds: p.settings.gap_ms / 1000 } : null,
    chosen_on: p.objective,
    held_out: { cv_vs_instant: p.cv.heldOut, cv_vs_dca: p.cv.vsDcaHeldOut, time_split: { chosen: p.time.chosen, in_sample: p.time.inSample, held_out: p.time.heldOut }, by_tier: Object.fromEntries(study.definition.tiers.map((t, k) => [t.name, p.cv.byTier[k]])) },
    in_sample: p.medians,
    unspent_share: p.unspent,
    sandwich: { share_of_near_over_break_even: p.exposure.overBreakEven, if_depth_half: p.exposure.overIfDepthHalf, slices_per_active_day: p.exposure.slicesPerDay },
    seed_spread_vs_instant: study.seedSpread[p.name].map((x) => x.whole),
    hunt: p.hunt ?? null,
    without_tax: study.noTax.presets.find((t) => t.name === p.name),
  })),
};
const estimatorJson = { generated_by: "node gym/gym.mjs", ...study.estimator };

const results = {
  dataset: { coins: ds.coins.length, swaps: ds.swaps, unvaluedSwaps: ds.unvalued_swaps, from: new Date(ds.data_start_ms).toISOString(), to: new Date(ds.data_end_ms).toISOString(), usdTrackError: ds.usd_track_error },
  assumptions: { income: INCOME, cost: COST, tax: WYRM_TAX, feeBatchMin: FEE_BATCH_MIN, feeBatchAt: FEE_BATCH_AT, seedReserve: SEED_RESERVE, active: ACTIVE, startPrice, attackDrop: ATTACK_DROP, attackMinutes: ATTACK_MINUTES, contractDefaults: DEFAULT_SETTINGS },
  income, broad, study,
  sandwich: { breakEven: BE, curve: sandwichCurve },
  scenarios: scenarios.map(({ days: d, ...s }) => ({ ...s, end: d.at(-1), scaleIncomeMedian: median(d.map((x) => x.scaleIncome)) })),
  scenarioDayPools: Object.fromEntries(Object.entries(pools).map(([k, v]) => [k, v.length])),
  attack,
};

mkdirSync(OUT, { recursive: true });
const r5 = (k, v) => (typeof v === "number" ? +v.toPrecision(5) : v instanceof Set ? [...v] : v);
writeFileSync(new URL("results.json", OUT), JSON.stringify(results, r5, 1) + "\n");
writeFileSync(new URL("presets.json", OUT), JSON.stringify(presetsJson, r5, 1) + "\n");
writeFileSync(new URL("estimator.json", OUT), JSON.stringify(estimatorJson, r5, 1) + "\n");

const scen = (key) => scenarios.map((s) => ({ name: s.name, values: s.days.map((d) => d[key]) }));
const sub60 = `60 days stitched from real Nearly days, 80 NEAR seed reserve, WYRM default (${describe(wyrm)})`;
const budgetRows = [...study.stages["budget outside the Council range"].rows, ...study.stages.cadence.rows.filter((r) => !r.settings.spread_ms)].sort((a, b) => a.settings.budget_bps - b.settings.budget_bps);
const charts = {
  "reserve.svg": lineChart({ title: "Dragon NEAR reserve", subtitle: sub60, xLabel: "day", yLabel: "NEAR", series: scen("reserve"), log: true }),
  "hoard.svg": lineChart({ title: "Dragon hoard", subtitle: `${sub60}, no market feedback`, xLabel: "day", yLabel: "WYRM", series: scen("hoard") }),
  "held-off-market.svg": lineChart({ title: "Share of supply held off the market", subtitle: "Hoard / 1B supply. Each buy lifts later prices by its full CPMM impact (upper bound on feedback)", xLabel: "day", yLabel: "% of supply", series: scen("heldPctFeedback") }),
  "scale-income.svg": lineChart({ title: "Income per Scale per day", subtitle: "Half of the tithe (2.4% of volume from the 400 bps tax, plus 0.48% of buy volume) over 100 Scales", xLabel: "day", yLabel: "NEAR per Scale", series: scen("scaleIncome"), log: true }),
  "strategies.svg": barChart({
    title: "Tokens per NEAR of income against instant buying", subtitle: `${study.coins} top coins, held out by coin (5 folds); the contract default and baselines are fixed settings`, xLabel: "median ratio vs instant buy",
    base: 1, baseName: "instant = 1",
    bars: [
      { name: "plain DCA", value: study.baselines.dca.vsInstant.p50, series: 0 },
      { name: "daily lump 30%", value: study.baselines.dailyLump.vsInstant.p50, series: 0 },
      { name: "contract default", value: study.baselines.contract.vsInstant.p50, series: 0 },
      ...study.presets.slice(0, 4).map((p) => ({ name: `${p.name}${p.name === "hoarder" ? " (WYRM)" : ""}`, value: p.cv.heldOut.p50, series: p.name === "hoarder" ? 1 : 2, note: `${fmt(p.unspent * 100)}% unspent` })),
    ],
  }),
  "budget.svg": barChart({
    title: "Daily breath budget on top coins", subtitle: "One breath, no dips. Median tokens per NEAR of income vs instant; note is the share of income never spent", xLabel: "median ratio vs instant", base: 1, baseName: "instant = 1",
    bars: budgetRows.map((r) => ({ name: `${r.settings.budget_bps / 100}%${r.settings.budget_bps < 1000 ? " (below range)" : ""}`, value: r.medians.whole, series: r.settings.budget_bps === wyrm.budget_bps ? 1 : 0, note: `${fmt(r.unspent * 100)}% unspent` })),
  }),
  "sandwich.svg": barChart({
    title: "Best sandwich profit around one buy", subtitle: `CPMM pool of 1,000 NEAR, 1% pool fee and 4% tax per side. Break-even ${fmt(BE.wyrm * 100)}% of depth`, xLabel: "attacker profit, NEAR",
    bars: sandwichCurve.map((c) => ({ name: `buy ${fmt(c.fillOverDepth * 100)}% of depth`, value: c.wyrm, series: c.fillOverDepth === 0.02 ? 1 : 0 })),
  }),
};
rmSync(new URL("ladder-heldout.svg", OUT), { force: true });
for (const [file, svg] of Object.entries(charts)) writeFileSync(new URL(file, OUT), svg);
writeFileSync(new URL("../docs/PARAMETERS.md", import.meta.url), renderParameters(results, presetsJson, estimatorJson));
console.log(`gym: ${study.coins} top coins tuned, ${others.length} other coins checked, ${Object.keys(charts).length} charts, results.json, presets.json and estimator.json in gym/out/, docs/PARAMETERS.md written`);
