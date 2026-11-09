// The top-coin study: tune every Dragon setting on coins that became top coins on Nearly, validate it held out
// by coin and by time, derive five presets and the Scale income estimator. Pure function of the dataset.
import { DEFAULT_SETTINGS } from "./dragon-rules.mjs";
import { readFileSync } from "node:fs";
import { run, strategies, dragonStrategy, segmentValue, ZERO_SNAP, breakEven, incomeOf, costOf, CREATOR_OF_BUYS, CREATOR_OF_TAX, WYRM_TAX } from "./sim.mjs";
import { coinView, densePath, coinProfile, tierOf, TIERS, SUSTAIN } from "./coins.mjs";
import { crossValidate, applyFolds, timeSplit, choose, median, q, spread, TOLERANCE } from "./tune.mjs";

const H = 3_600_000, MIN = 60_000;
const BREAK_EVEN = breakEven();
// The middle of the data window (Sep 22 to Oct 18). Choices are made on the weeks before and scored after.
export const SPLIT_MS = Date.UTC(2026, 9, 5);
// A segment counts for a coin only when at least this much income reached the Dragon inside it.
const MIN_SEGMENT_INCOME = 1;
export const SEEDS = [0, 1, 2];

// The tithe gives half of what arrives to the 100 Scales in equal parts.
export const TITHE = Object.freeze({ scales_share: 0.5, scales: 100 });
// Creator-side NEAR per NEAR of total volume. The pool-fee share is paid on the NEAR side (buys); `sellSide` adds the
// coin side of sells, which Nearly sells for NEAR (from its source, not measured). A token tax counts on both sides.
export const titheRate = ({ buyShare, tax = 0, sellSide = false }) => CREATOR_OF_BUYS * (sellSide ? 1 : buyShare) + tax * CREATOR_OF_TAX;
export const perScalePerDay = (dailyVolumeNear, rate) => (dailyVolumeNear * rate * TITHE.scales_share) / TITHE.scales;
const LIVE = new URL("../docs/research/dcl-impact.json", import.meta.url);
const LIVE_IDS = { NEARLY: "nearly-993927", BATMAN: "batman-4", SINGULARTY: "singularty", NINU: "ninu-4", DRAGON: "dragon-3" };

const NO_DIPS = [0, 0, 0];
const SPREADS = [["one breath", 0], ["over 1h", H], ["over 6h", 6 * H], ["over 24h", 24 * H]];
const GAPS = [["1 min", MIN], ["5 min", 5 * MIN], ["15 min", 15 * MIN]];
const BUDGETS = [1000, 1500, 2000, 2500, 3000, 4000];
// Below the Council floor, to see whether the best budget sits on the floor only because the range ends there.
const BUDGETS_OUTSIDE = [500, 700];
const STEP_SETS = [[500, 1000, 1500], [1000, 2000, 3000], [1500, 3000, 4500], [2000, 3500, 5000]];
const EXTRA_SETS = [[500, 1000, 1500], [1000, 1500, 2000], [2000, 2500, 2500]];
// The Council range is 0.5% to 2%. 1% is the incumbent, a factor 5 under the sandwich break-even.
const CAPS = [100, 50, 150, 200];
const CAPS_OUTSIDE = [250, 10_000];
const SPIKES = [1500, 1000, 2500, null];
const HUNTS = [20000, 25000, 30000, 50000].flatMap((sell) => [500, 1000, 1500].map((max) => [sell, max]));

const pctList = (bps) => bps.map((b) => b / 100).join("/");
export function describe(s) {
  const cadence = s.spread_ms ? `spread ${s.spread_ms / H}h, gap ${s.gap_ms / MIN} min` : "one breath";
  const dips = s.extra_bps.every((e) => e === 0) ? "no dips" : `steps ${pctList(s.steps_bps)}, extras ${pctList(s.extra_bps)}`;
  const hunt = s.hunt ? `, hunt ${s.sell_multiple_bps / 10_000}x ${s.max_sell_bps / 100}%/day` : "";
  return `budget ${s.budget_bps / 100}%, ${cadence}, ${dips}, cap ${s.cap_bps / 100}%, spike ${s.spike_bps == null ? "off" : `${s.spike_bps / 100}%`}${hunt}`;
}
const settingsOf = (over) => ({ ...DEFAULT_SETTINGS, ...over });

function evaluate(paths, strategy, seed = 0, tax = WYRM_TAX) {
  const be = tax === WYRM_TAX ? BREAK_EVEN : breakEven(1e-9, costOf(tax));
  return new Map(paths.map((p) => {
    const r = run(p, strategy, { marks: [SPLIT_MS], seed, tax });
    const [mid] = r.snaps;
    const fills = r.slices.map((s) => s.near / s.depth);
    const spentOver = (f) => r.slices.reduce((a, s, i) => a + (f(fills[i]) ? s.near : 0), 0);
    const sliced = r.slices.reduce((a, s) => a + s.near, 0);
    const activeDays = new Set();
    for (let i = 0; i < p.vol.length; i++) if (p.vol[i] > 0) activeDays.add(Math.floor((p.t0 + i * MIN) / 86_400_000));
    return [p.id, {
      whole: segmentValue(ZERO_SNAP, r.end),
      before: mid.inflow >= MIN_SEGMENT_INCOME ? segmentValue(ZERO_SNAP, mid) : null,
      after: r.end.inflow - mid.inflow >= MIN_SEGMENT_INCOME ? segmentValue(mid, r.end) : null,
      tpn: r.tpn, unspent: r.reserve / r.inflow, inflow: r.inflow, VoverI: r.V / r.inflow, tokens: r.tokens, realized: r.realized,
      slices: { n: fills.length, spent: sliced, overBE: spentOver((f) => f > be), overHalf: spentOver((f) => 2 * f > be), p50: q(fills, 0.5), p90: q(fills, 0.9), max: fills.reduce((a, f) => Math.max(a, f), 0) },
      slicesPerDay: fills.length / (activeDays.size || 1),
    }];
  }));
}

const ratioTable = (runs, base, seg) => new Map([...runs].map(([k, M]) => [k, new Map([...M].map(([id, m]) => {
  const b = base.get(id);
  return [id, m[seg] != null && b[seg] != null ? m[seg] / b[seg] : null];
}))]));

// Sandwich exposure: the share of NEAR spent in slices above the break-even fill, pooled over coins, and per-coin
// medians of the slice-to-depth fill.
function exposure(M) {
  const sl = [...M.values()].map((m) => m.slices);
  const spent = sl.reduce((a, s) => a + s.spent, 0) || 1;
  return { slices: sl.reduce((a, s) => a + s.n, 0), fill: { p50: median(sl.map((s) => s.p50)), p90: median(sl.map((s) => s.p90)), max: Math.max(0, ...sl.map((s) => s.max)) },
    overBreakEven: sl.reduce((a, s) => a + s.overBE, 0) / spent, overIfDepthHalf: sl.reduce((a, s) => a + s.overHalf, 0) / spent, slicesPerDay: median([...M.values()].map((m) => m.slicesPerDay)) };
}

// Impact against size, from real trades. Each sample is a trade between two same-side neighbours; the CPMM depth that explains its move is
// size / (sqrt(1 + move) - 1). If impact were concave in size, big trades would imply deeper pools than small ones.
function impactBySize(coins) {
  const buckets = [[0.5, 2], [2, 5], [5, 20], [20, 50], [50, 200], [200, Infinity]];
  const rows = buckets.map(([lo, hi]) => ({ lo, hi, ratios: [] }));
  for (const c of coins) for (const [day, near, move] of c.impact ?? []) {
    const depth = c.depth[day];
    const r = Math.sqrt(1 + move);
    const implied = move > 0 ? near / (r - 1) : near / (1 - r);
    if (!(depth > 0) || !Number.isFinite(implied)) continue;
    rows.find((b) => near >= b.lo && near < b.hi)?.ratios.push(implied / depth);
  }
  return rows.map((b) => ({ lo: b.lo, hi: b.hi, n: b.ratios.length, impliedOverDayDepth: median(b.ratios), p25: q(b.ratios, 0.25), p75: q(b.ratios, 0.75) }));
}

export function topCoinStudy(ds) {
  const views = ds.coins.map(coinView);
  const profiles = views.map((c) => ({ c, p: coinProfile(c) })).map((x) => ({ ...x, tier: tierOf(x.p) }));
  const tierCounts = TIERS.map((t, k) => ({ ...t, coins: profiles.filter((x) => x.tier >= k).length, ids: profiles.filter((x) => x.tier >= k).map((x) => x.c.id) }));
  const peakOnly = TIERS.map((t) => profiles.filter((x) => x.p.peak_mc_near >= t.peak_mc_near).length);
  const top = profiles.filter((x) => x.tier >= 0);
  const paths = top.map((x) => densePath(x.c));
  const ids = paths.map((p) => p.id);
  const tierIds = (k) => new Set(top.filter((x) => x.tier >= k).map((x) => x.c.id));
  const BE = BREAK_EVEN;

  const instant = evaluate(paths, strategies.instant), dca = evaluate(paths, strategies.dca), lump = evaluate(paths, strategies.dailyLump);
  const contract = evaluate(paths, dragonStrategy());

  // Every config the study visits, keyed by its description, in visiting order. Each stage lists its incumbent
  // first and then the rest by fewer slices, so a challenger inside the tolerance does not replace it.
  const runs = new Map(), configs = new Map();
  const visit = (over, stage) => {
    const s = settingsOf(over), key = describe(s);
    if (!runs.has(key)) { runs.set(key, evaluate(paths, dragonStrategy(s))); configs.set(key, { key, settings: s, stages: new Set() }); }
    configs.get(key).stages.add(stage);
    return key;
  };
  const stageKeys = (stage) => [...configs.values()].filter((c) => c.stages.has(stage)).map((c) => c.key);
  // Scores, per coin, against instant buying: the whole window (the objective), each side of SPLIT_MS, and robust,
  // the worse of the two sides (the side that exists when a coin traded on one side only), reported but not chosen on.
  const tables = () => {
    const whole = ratioTable(runs, instant, "whole"), before = ratioTable(runs, instant, "before"), after = ratioTable(runs, instant, "after");
    const robust = new Map([...whole.keys()].map((k) => [k, new Map(ids.map((id) => {
      const b = before.get(k).get(id), a = after.get(k).get(id);
      return [id, b == null ? a : a == null ? b : Math.min(a, b)];
    }))]));
    return { whole, before, after, robust, dca: ratioTable(runs, dca, "whole") };
  };
  const bestOf = (stage) => configs.get(choose(tables().whole, stageKeys(stage), ids)).settings;

  visit({ extra_bps: NO_DIPS }, "cadence");
  for (const [, spread_ms] of SPREADS) for (const [, gap_ms] of spread_ms ? [...GAPS].reverse() : [GAPS[0]]) for (const budget_bps of BUDGETS) visit({ budget_bps, extra_bps: NO_DIPS, spread_ms, gap_ms }, "cadence");
  for (const budget_bps of BUDGETS_OUTSIDE) visit({ budget_bps, extra_bps: NO_DIPS }, "budget outside the Council range");
  const cadence = bestOf("cadence");
  for (const budget_bps of [...new Set([cadence.budget_bps, 1000, 1500, 2000])]) {
    visit({ ...cadence, budget_bps }, "dips");
    for (const steps_bps of STEP_SETS) for (const extra_bps of EXTRA_SETS) {
      if (budget_bps + extra_bps.reduce((a, b) => a + b, 0) > 8000) continue;
      visit({ ...cadence, budget_bps, steps_bps, extra_bps }, "dips");
    }
  }
  const dips = bestOf("dips");
  for (const cap_bps of CAPS) visit({ ...dips, cap_bps }, "cap");
  for (const cap_bps of CAPS_OUTSIDE) visit({ ...dips, cap_bps }, "cap outside the Council range");
  // Caps tie on tokens, so among the caps inside the tolerance take the least NEAR spent above break-even if the
  // true depth were half the estimate, then the larger cap (fewer slices).
  const capped = (() => {
    const keys = stageKeys("cap"), w = tables().whole, meds = keys.map((k) => median(scoresFor(w, k, ids))), best = Math.max(...meds);
    const tied = keys.filter((_, i) => meds[i] >= best * (1 - TOLERANCE)).map((k) => ({ k, e: exposure(runs.get(k)), cap: configs.get(k).settings.cap_bps }));
    tied.sort((a, b) => a.e.overIfDepthHalf - b.e.overIfDepthHalf || a.e.overBreakEven - b.e.overBreakEven || b.cap - a.cap);
    return configs.get(tied[0].k).settings;
  })();
  for (const spike_bps of SPIKES) visit({ ...capped, spike_bps }, "spike");
  const buying = bestOf("spike");
  const defaultKey = describe(buying);

  // Preset families on the tuned cap and spike guard, each with the objective that matches its bet.
  const fam = (over, stage) => visit({ ...over, cap_bps: buying.cap_bps, spike_bps: buying.spike_bps }, stage);
  for (const [, spread_ms] of SPREADS.slice(2)) for (const [, gap_ms] of [...GAPS].reverse()) for (const budget_bps of BUDGETS) fam({ budget_bps, extra_bps: NO_DIPS, spread_ms, gap_ms }, "steady");
  for (const budget_bps of [1000, 1500, 2000]) for (const steps_bps of STEP_SETS.slice(0, 2)) for (const extra_bps of EXTRA_SETS.slice(1)) fam({ ...cadence, budget_bps, steps_bps, extra_bps }, "dip-hunter");
  for (const budget_bps of [2500, 3000, 4000]) fam({ budget_bps, extra_bps: NO_DIPS }, "swift");

  const T = tables();
  const buyingStages = ["cadence", "dips", "cap", "spike"];
  const pooled = [...configs.values()].filter((c) => buyingStages.some((s) => c.stages.has(s))).map((c) => c.key);
  const medians = (k) => Object.fromEntries(["whole", "before", "after", "robust", "dca"].map((o) => [o, median(scoresFor(T[o], k, ids))]));

  // Held out two ways: k-fold by coin on the objective, and chosen before SPLIT_MS then scored after it.
  const report = (keys, objective) => {
    const cv = crossValidate(T[objective], keys, ids);
    const tierHeld = TIERS.map((_, k) => spread([...cv.heldOut].filter(([id]) => tierIds(k).has(id)).map(([, v]) => v)));
    return { objective, candidates: keys.length, cv: { inSample: cv.inSample, heldOut: cv.heldOutSpread, vsDcaHeldOut: applyFolds(cv, T.dca), chosenCounts: cv.chosenCounts, folds: cv.folds.length, byTier: tierHeld }, time: timeSplit(T.before, T.after, keys, ids) };
  };
  const detail = (key) => {
    const M = runs.get(key);
    return { key, settings: configs.get(key).settings, medians: medians(key), unspent: median([...M.values()].map((m) => m.unspent)), tpnVsInstant: median([...M].map(([id, m]) => m.tpn / instant.get(id).tpn)), exposure: exposure(M) };
  };
  const stages = Object.fromEntries([...buyingStages, "cap outside the Council range", "budget outside the Council range"].map((s) => [s, { ...(buyingStages.includes(s) ? report(stageKeys(s), "whole") : {}), rows: stageKeys(s).map(detail) }]));

  // Hunt is scored on end value (NEAR plus the hoard at the last price) against the same config without Hunt.
  const huntRuns = new Map(HUNTS.map(([sell, max]) => {
    const s = settingsOf({ ...buying, hunt: true, sell_multiple_bps: sell, max_sell_bps: max });
    return [describe(s), { settings: s, M: evaluate(paths, dragonStrategy(s)) }];
  }));
  const base = runs.get(defaultKey);
  const huntTable = new Map([...huntRuns].map(([k, h]) => [k, new Map([...h.M].map(([id, m]) => [id, m.VoverI / base.get(id).VoverI]))]));
  const hoardTable = new Map([...huntRuns].map(([k, h]) => [k, new Map([...h.M].map(([id, m]) => [id, base.get(id).tokens > 0 ? m.tokens / base.get(id).tokens : null]))]));
  const huntCv = crossValidate(huntTable, [...huntRuns.keys()], ids);
  const huntKey = choose(huntTable, [...huntRuns.keys()], ids);
  const hunt = {
    rows: [...huntRuns].map(([k, h]) => ({ key: k, sell: h.settings.sell_multiple_bps, max: h.settings.max_sell_bps, endValue: spread(scoresFor(huntTable, k, ids)), hoardKept: median(scoresFor(hoardTable, k, ids)), coinsSold: [...h.M.values()].filter((m) => m.realized > 0).length })),
    cv: { inSample: huntCv.inSample, heldOut: huntCv.heldOutSpread, hoardKeptHeldOut: applyFolds(huntCv, hoardTable), chosenCounts: huntCv.chosenCounts },
    chosen: huntKey, settings: huntRuns.get(huntKey).settings,
  };

  // Five presets for the factory. The hoarder is the stage-wise winner and the WYRM default; the others are the
  // held-out best of their family, each scored on the objective that matches what it bets on.
  const FAMILIES = [
    ["steady", "whole", "Spreads each day's budget over hours in small slices, no dip extras. For a creator who wants buys visible through the day. Needs a spread setting the contract does not have yet."],
    ["dip-hunter", "whole", "Shallow dip steps with large extras, so most income reaches the market. For a creator who wants more buy pressure on dips and accepts fewer tokens per NEAR."],
    ["swift", "before", "Large budget in one breath. For a coin still in its launch run, where each day of waiting costs tokens."],
  ];
  const hoarder = { name: "hoarder", objective: "whole", suits: "Smallest daily budget in one breath, no dip extras. Buys the most tokens per NEAR on top coins after their peak, and holds the most NEAR back.", ...detail(defaultKey), ...report(pooled, "whole") };
  const presets = [hoarder, ...FAMILIES.map(([name, objective, suits]) => {
    const keys = stageKeys(name);
    return { name, objective, suits, ...detail(choose(T[objective], keys, ids)), ...report(keys, objective) };
  })];
  presets.push({ name: "harvester", objective: "end value", suits: "The hoarder with Hunt on: sells a small slice of the hoard while the price is far above its cost. For a creator who wants profit taken back into NEAR.", ...detail(defaultKey), ...report(pooled, "whole"), settings: hunt.settings, key: huntKey, hunt: hunt.cv });
  const wyrmDefault = { ...hoarder, preset: "hoarder" };

  // Seed spread: the breath minute is random, so rerun each preset and the contract default on other seeds.
  const seedRuns = (s, first) => SEEDS.map((seed) => {
    const M = seed === 0 && first ? first : evaluate(paths, dragonStrategy(s), seed);
    return { whole: median([...M].map(([id, m]) => m.whole / instant.get(id).whole)), before: median([...M].map(([id, m]) => (m.before != null && instant.get(id).before != null ? m.before / instant.get(id).before : null))) };
  });
  const seedSpread = Object.fromEntries([["contract default", seedRuns(DEFAULT_SETTINGS, contract)], ...presets.map((p) => [p.name, seedRuns(p.settings, runs.get(p.key) ?? null)])]);

  // Comparison without a token tax: the same presets on the same coins. Not tuned again: the question is what the
  // tax does to a chosen setting. Income loses the tax share and buys stop paying the tax.
  const instantNoTax = evaluate(paths, strategies.instant, 0, 0);
  const noTax = {
    tax: 0, income: incomeOf(0), breakEven: breakEven(1e-9, costOf(0)),
    presets: presets.map((p) => {
      const M = evaluate(paths, dragonStrategy(p.settings), 0, 0), base = runs.get(p.key) ?? evaluate(paths, dragonStrategy(p.settings));
      return { name: p.name, vsInstant: spread([...M].map(([id, m]) => m.whole / instantNoTax.get(id).whole)),
        hoardVsTax: spread([...M].map(([id, m]) => (base.get(id).tokens > 0 ? m.tokens / base.get(id).tokens : null))),
        incomeVsTax: median([...M].map(([id, m]) => m.inflow / base.get(id).inflow)), unspent: median([...M.values()].map((m) => m.unspent)), exposure: exposure(M) };
    }),
  };

  // Income: the daily volume top coins actually reached, and per-Scale income brackets.
  const days = top.flatMap((x) => x.p.dailyVolumes.filter((v) => v > 0));
  const buyShareDays = top.flatMap((x) => [...x.c.dayVol].filter(([, v]) => v > 0).map(([d, v]) => x.c.dayBuy.get(d) / v));
  const buyShare = top.reduce((acc, x) => acc + [...x.c.dayBuy.values()].reduce((u, v) => u + v, 0), 0) / top.reduce((acc, x) => acc + x.c.life, 0);
  const dist = (xs) => ({ n: xs.length, p10: q(xs, 0.1), p25: q(xs, 0.25), p50: q(xs, 0.5), p75: q(xs, 0.75), p90: q(xs, 0.9), p99: q(xs, 0.99), max: Math.max(...xs) });
  const BRACKETS = [100, 300, 1_000, 3_000, 10_000, 30_000, 100_000, 300_000];
  const SCENARIOS = {
    tax_400: { label: "WYRM: 400 bps token tax on buys and on sells, 60% of it to the tithe, plus the measured pool-fee share", rate: titheRate({ buyShare, tax: WYRM_TAX }) },
    tax_400_with_sell_side: { label: "WYRM, plus the coin side of the pool fee on sells, sold for NEAR (from Nearly's source, not measured)", rate: titheRate({ buyShare, tax: WYRM_TAX, sellSide: true }) },
    no_tax: { label: "comparison: no token tax, NEAR side of the pool fee only", rate: titheRate({ buyShare }) },
  };
  const estimator = {
    formula: "per_scale_near_per_day = daily_volume_near * tithe_rate * 0.5 / 100",
    tithe_rate_formula: "tithe_rate = 0.0048 * buy_share (+ 0.0048 * (1 - buy_share) with sell side) + tax_per_side * 0.6, tax_per_side = 0.04 for WYRM",
    buy_share: { pooled: buyShare, perCoinDay: dist(buyShareDays) },
    scenarios: Object.fromEntries(Object.entries(SCENARIOS).map(([k, v]) => [k, { ...v }])),
    assumptions: [
      "DCL takes a 1% pool fee on every swap and keeps 20% of it; the locker earns 0.8% (docs/research/NEARLY.md).",
      "The creator side gets 60% of that 0.8% (creator_share_bps 6000 on launches 3643 to 3647), paid in NEAR from the NEAR side: 0.48% of buy volume. fees_near_total / volume_y_in measured 0.008 on three launches.",
      "buy_share is the NEAR value of buys over all volume, pooled over the top coins in the Gym data.",
      "The coin side of sells is sold for NEAR by Nearly's seller. Its schedule and slippage were not measured, so it is a separate, upper scenario.",
      "WYRM's tax is 400 bps on buys and 400 bps on sells (operator decision). Nearly's platform keeps 40% (platform_bps 4000 on observed launches) and the tithe gets 60%: 2.4% of all volume. The tax also charges every Dragon buy, and Nearly sells the coin-side tax for NEAR, which is sell pressure.",
      "Nearly pays fee_to about once an hour. The tithe sends half of each round to the 100 Scales in equal parts; the rest goes to the Dragon.",
      "The tithe rate is the main uncertainty. The Nov 8 rehearsal measures it on WYRM's own pool.",
    ],
    near_usd: ds.near_usd_last,
    topCoinDailyVolume: { tier: TIERS[0].name, coins: top.length, ...dist(days) },
    topCoinMedianDay: dist(top.map((x) => median(x.p.dailyVolumes))),
    brackets: BRACKETS.map((v) => ({
      daily_volume_near: v,
      share_of_top_coin_days_at_or_above: days.filter((d) => d >= v).length / days.length,
      per_scale_near_per_day: Object.fromEntries(Object.entries(SCENARIOS).map(([k, sc]) => [k, perScalePerDay(v, sc.rate)])),
      dragon_near_per_day: Object.fromEntries(Object.entries(SCENARIOS).map(([k, sc]) => [k, v * sc.rate * (1 - TITHE.scales_share)])),
    })),
  };

  // Live quotes at one mainnet block: the CPMM depth each quote implies, a/impact - a, against the Gym's last-day
  // estimate for the same coin. Constant implied depth across sizes means impact is linear in size, as the Gym assumes.
  const live = JSON.parse(readFileSync(LIVE, "utf8"));
  const liveImpact = { block: live.block_height, coins: live.coins.map((c) => {
    const rows = c.rows.filter((r) => r.impact_bps_vs_0_01_near > 0).map((r) => ({ near: r.in_near, bps: r.impact_bps_vs_0_01_near, implied: (r.in_near * 10_000) / r.impact_bps_vs_0_01_near - r.in_near }));
    const gym = top.find((x) => x.c.id === LIVE_IDS[c.symbol])?.c;
    return { symbol: c.symbol, rows, impliedAt10: rows.find((r) => r.near === 10)?.implied ?? null, impliedAt100: rows.find((r) => r.near === 100)?.implied ?? null, gymLastDepth: gym ? gym.depth.at(-1) : null,
      sliceAt: Object.fromEntries([50, 100, 150, 200].map((bps) => [bps, (() => { const y = rows.find((r) => r.near === 100)?.implied; return y ? (y * bps) / (10_000 - bps) : null; })()])) };
  }) };

  const depths = paths.map((p) => median(Array.from({ length: Math.ceil(p.price.length / 1440) }, (_, d) => p.depth(d * 1440))));
  const baselines = Object.fromEntries([["dca", dca], ["dailyLump", lump], ["contract", contract]].map(([k, M]) => [k, {
    vsInstant: spread([...M].map(([id, m]) => m.whole / instant.get(id).whole)),
    before: spread([...M].map(([id, m]) => (m.before != null && instant.get(id).before != null ? m.before / instant.get(id).before : null))),
    after: spread([...M].map(([id, m]) => (m.after != null && instant.get(id).after != null ? m.after / instant.get(id).after : null))),
    robust: spread([...M].map(([id, m]) => { const i = instant.get(id); const b = m.before != null && i.before != null ? m.before / i.before : null, a = m.after != null && i.after != null ? m.after / i.after : null; return b == null ? a : a == null ? b : Math.min(a, b); })),
    unspent: median([...M.values()].map((m) => m.unspent)),
  }]));

  return {
    definition: { tiers: tierCounts.map(({ ids: tierCoinIds, ...t }) => ({ ...t, ids: tierCoinIds })), peakOnly, sustain: SUSTAIN, totalCoins: views.length,
      firstMc: dist(profiles.map((x) => x.p.first_mc_near).filter((x) => x > 0)), peakMc: dist(profiles.map((x) => x.p.peak_mc_near).filter((x) => x > 0)),
      pairs: Object.fromEntries([...new Set(top.map((x) => x.p.pair))].map((pr) => [pr, top.filter((x) => x.p.pair === pr).length])),
      peakDay: dist(top.map((x) => x.p.peak_day).filter((x) => x != null)),
      usdTrackError: ds.usd_track_error },
    split: new Date(SPLIT_MS).toISOString(), coins: ids.length, breakEven: BE, tax: WYRM_TAX, income: incomeOf(WYRM_TAX), noTax, liveImpact,
    depth: dist(depths), impactBySize: impactBySize(top.map((x) => x.c)),
    baselines, stages, cadence: describe(cadence), defaultKey, wyrmDefault, presets, hunt, seedSpread, estimator,
  };
}
const scoresFor = (table, key, ids) => ids.map((id) => table.get(key).get(id));
