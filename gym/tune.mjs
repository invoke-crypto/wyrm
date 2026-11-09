// Held-out selection over precomputed scores. A table maps a config key to a Map of coin id -> score (higher is
// better, null when the coin has no score). Nothing here runs a simulation.

const finite = (xs) => xs.filter((x) => x != null && Number.isFinite(x)).sort((a, b) => a - b);
export const q = (xs, f) => { const s = finite(xs); return s.length ? s[Math.min(s.length - 1, Math.floor(f * s.length))] : null; };
export const median = (xs) => q(xs, 0.5);
export const spread = (xs) => ({ n: finite(xs).length, p25: q(xs, 0.25), p50: q(xs, 0.5), p75: q(xs, 0.75), beats: finite(xs).filter((x) => x > 1 + 1e-9).length / (finite(xs).length || 1) });

const scoresOf = (table, key, ids) => ids.map((id) => table.get(key).get(id));

// A challenger must beat an earlier key by more than this share of the best median to be chosen. Keys come
// incumbent first, so a gain inside the noise keeps the incumbent. Compare it with the spread of a preset across
// breath-minute seeds in docs/PARAMETERS.md: a gap smaller than that spread is noise.
export const TOLERANCE = 0.02;

// The earliest key whose median score over `ids` is within `tol` of the best median.
export function choose(table, keys, ids, tol = TOLERANCE) {
  const meds = keys.map((k) => median(scoresOf(table, k, ids)));
  const best = Math.max(...meds.filter((m) => m != null));
  if (!Number.isFinite(best)) return null;
  return keys[meds.findIndex((m) => m != null && m >= best - tol * Math.abs(best))];
}

// Coins in launch order go to folds round-robin, so every fold mixes early and late launches.
export const foldsOf = (ids, k) => Array.from({ length: k }, (_, f) => ids.filter((_, i) => i % k === f));

/**
 * k-fold by coin. Each fold's config is chosen on the other folds' coins and scored on its own coins, so every
 * held-out score comes from a choice that never saw that coin. `inSample` is the best config chosen and scored on
 * all coins, the number that must not be reported as the result.
 */
export function crossValidate(table, keys, ids, k = 5) {
  const folds = foldsOf(ids, k).filter((f) => f.length).map((test) => {
    const train = ids.filter((id) => !test.includes(id));
    return { test, chosen: choose(table, keys, train) };
  });
  const heldOut = new Map(folds.flatMap((f) => f.test.map((id) => [id, table.get(f.chosen).get(id)])));
  const best = choose(table, keys, ids);
  const counts = {};
  for (const f of folds) counts[f.chosen] = (counts[f.chosen] ?? 0) + 1;
  return { folds, heldOut, heldOutSpread: spread([...heldOut.values()]), inSample: { key: best, ...spread(scoresOf(table, best, ids)) }, chosenCounts: counts };
}

// Score the fold choices of a cross-validation on another table, such as the same configs against plain DCA.
export const applyFolds = (cv, table) => spread(cv.folds.flatMap((f) => f.test.map((id) => table.get(f.chosen).get(id))));

/**
 * Split by time: choose on the scores before the split, report the same config after it. `best after` is the
 * config that would have won after the split with hindsight, the ceiling the held-out number is measured against.
 */
export function timeSplit(before, after, keys, ids) {
  const trainIds = ids.filter((id) => keys.some((k) => before.get(k).get(id) != null));
  const testIds = ids.filter((id) => keys.some((k) => after.get(k).get(id) != null));
  const chosen = choose(before, keys, trainIds);
  const hindsight = choose(after, keys, testIds, 0);
  return {
    chosen, trainCoins: trainIds.length, testCoins: testIds.length,
    inSample: spread(scoresOf(before, chosen, trainIds)),
    heldOut: spread(scoresOf(after, chosen, testIds)),
    bestAfter: { key: hindsight, ...spread(scoresOf(after, hindsight, testIds)) },
  };
}
