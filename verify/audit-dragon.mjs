// Re-derives every Dragon decision from its events with the Gym's rule engine (gym/dragon-rules.mjs), on the
// strategy, ladder and cadence of the Dragon's preset, and checks the coin's tax on every swap.
// Each `observe` event is fed to the engine in order. A trade the Dragon made passes when the engine, given the same
// observation, picks the same kind (breath, dip or hunt) and reaches the same amount by halving from its allowance.
// The engine sees the Dragon's logged quote for the amount it traded; any larger candidate is treated as over the
// cap, so a logged amount the halving could not reach shows as a mismatch.
//
//   npm run audit-dragon -- --network mainnet --root thewyrm.near [--verbose] [--rpc URL]
//
// Events read (fields in snake_case, amounts yocto, times ns):
//   observe          { at, tokens_per_near, reserve, hoard_wyrm, breath_at }
//   breath_slice     { at, amount_in, quoted_out, min_out, ... }   dip_slice { ..., step_bps }   hunt_slice { ... }
//   swap_failed      (in the same transaction as the observe: the spend is given back)
//   settings_changed { budget_bps, cap_bps, hunt, auto_burn_bps }
// A buy's coin arrives as one nep141 transfer of the net and one with memo `tax`; a hunt sell sends the same pair.
import { pathToFileURL } from "node:url";
import { DAY_MS, MINUTE_MS, newDragon, observe } from "../gym/dragon-rules.mjs";
import { wyrmEvents } from "./history.mjs";

const near = (yocto) => Number(BigInt(yocto) / 10n ** 12n) / 1e12;
const tokens = (raw) => Number(BigInt(raw) / 10n ** 6n) / 1e12;
const ms = (ns) => Number(BigInt(ns) / 1_000_000n);
const close = (a, b) => Math.abs(a - b) <= 1e-6 * Math.max(Math.abs(a), Math.abs(b), 1e-12);
const KIND = { breath_slice: "breath", dip_slice: "dip", hunt_slice: "hunt" };

const strategySettings = (s) => ({ budget_bps: s.budget_bps, cap_bps: s.cap_bps, hunt: s.hunt != null, ...(s.hunt ?? {}) });

// The engine's settings for a Dragon's `get_config()`: the preset's strategy, ladder and cadence, and the engine's
// spike guard. Council changes arrive later as settings_changed events.
export function engineSettings(config) {
  const p = config.preset_value;
  return {
    ...strategySettings(p.strategy),
    steps_bps: p.ladder.steps_bps,
    extra_bps: p.ladder.extra_bps,
    gap_ms: p.cadence.gap_seconds * 1000,
    spread_ms: p.cadence.spread_minutes * MINUTE_MS,
    spike_bps: config.engine.spike_bps,
  };
}

function need(ev, names) {
  for (const n of names) if (ev.data[n] === undefined) throw new Error(`${ev.name} in tx ${ev.receipt.tx} has no "${n}" field`);
  return ev.data;
}

// Returns { decisions: [{ tx, at, made, engine, pass, why }], idle, unmade, refunded }. A refunded swap gives its
// spend back but keeps the gap, as on_swap does.
export function auditDragon(receipts, dragon, config) {
  const events = wyrmEvents(receipts, dragon);
  const d = newDragon(engineSettings(config));
  const decisions = [];
  let idle = 0;
  let unmade = 0;
  let refunded = 0;
  for (const ev of events) {
    if (ev.name === "settings_changed") {
      Object.assign(d.settings, strategySettings(ev.data));
      continue;
    }
    if (ev.name !== "observe") continue;
    const o = need(ev, ["at", "tokens_per_near", "reserve", "hoard_wyrm", "breath_at"]);
    if (o.tokens_per_near === null) {
      idle++;
      continue;
    }
    const sameTx = events.filter((e) => e.receipt.tx === ev.receipt.tx);
    const slice = sameTx.find((e) => KIND[e.name]);
    const failed = sameTx.some((e) => e.name === "swap_failed");
    const now = ms(o.at);
    const tpn = tokens(o.tokens_per_near);
    const made = slice && need(slice, ["amount_in", "quoted_out"]);
    const hunt = slice?.name === "hunt_slice";
    const quote = (amount, sold) => {
      if (made && hunt === sold && close(amount, sold ? tokens(made.amount_in) : near(made.amount_in))) {
        return sold ? near(made.quoted_out) : tokens(made.quoted_out);
      }
      if (made && hunt === sold && amount > (sold ? tokens(made.amount_in) : near(made.amount_in))) return 0;
      return sold ? amount / tpn : amount * tpn;
    };
    const lastSlice = d.last_slice_at;
    const decision = observe(d, {
      now, reserve: near(o.reserve), hoard_wyrm: tokens(o.hoard_wyrm), tokens_per_near: tpn,
      random_u32: Math.round((ms(o.breath_at) - Math.floor(now / DAY_MS) * DAY_MS) / MINUTE_MS),
      quote_buy: (a) => quote(a, false), quote_sell: (a) => quote(a, true),
    });
    if (decision.kind !== "none" && !slice) {
      if (decision.kind === "hunt") d.day.sold_wyrm -= decision.amount_in;
      else d.day.spent -= decision.amount_in;
    }
    if (failed) {
      refunded++;
      continue;
    }
    if (!slice) {
      if (decision.kind === "none") idle++;
      else {
        unmade++;
        d.last_slice_at = lastSlice;
      }
      continue;
    }
    const trade = { kind: KIND[slice.name], amount_in: hunt ? tokens(made.amount_in) : near(made.amount_in) };
    const unit = hunt ? "WYRM" : "NEAR";
    const why = decision.kind !== trade.kind ? `the engine decided ${decision.kind}${decision.why ? ` (${decision.why})` : ""}, the dragon traded by ${trade.kind}`
      : !close(decision.amount_in, trade.amount_in) ? `the engine sized ${decision.amount_in} ${unit}, the dragon traded ${trade.amount_in} ${unit}`
      : null;
    decisions.push({ tx: ev.receipt.tx, at: o.at, made: trade, engine: decision, pass: !why, why });
  }
  return { decisions, idle, unmade, refunded };
}

const sum = (moves) => moves.reduce((s, m) => s + BigInt(m.amount), 0n);

// One taxed transfer of the coin: the receipt where `sender` moved `gross` to `payee`, and the coin's two transfer
// events in it, the net to `payee` and the one with memo `tax`. null when the transaction has no such receipt.
export function taxedTransfer(receipts, tx, { coin, sender, payee }) {
  for (const r of receipts) {
    if (r.tx !== tx || r.receiver !== coin || r.predecessor !== sender || !r.ok) continue;
    const call = r.actions.find((a) => (a.method === "ft_transfer" || a.method === "ft_transfer_call") && a.args?.receiver_id === payee);
    if (!call) continue;
    const moves = r.events.filter((e) => e.standard === "nep141" && e.event === "ft_transfer").flatMap((e) => e.data ?? []);
    const tax = moves.filter((m) => m.memo === "tax");
    return { gross: BigInt(call.args.amount), net: sum(moves.filter((m) => m.memo !== "tax" && m.new_owner_id === payee)), tax: sum(tax), taxMoves: tax.length };
  }
  return null;
}

// Every swap the Dragon made, checked against the coin's tax. A buy: DCL sent `gross` of the coin, the Dragon got
// `gross - tax` with `tax = gross * buy_bps / 10_000`, and `gross >= min_out`. A hunt sell: the Dragon sent its
// `amount_in` as gross, DCL got the net, and the wNEAR back met `min_out`, which was computed on that net.
export function taxedSwaps(receipts, config, dragon) {
  const { peers, coin, engine } = config;
  const out = [];
  for (const ev of wyrmEvents(receipts, dragon)) {
    if (!KIND[ev.name]) continue;
    const d = need(ev, ["amount_in", "min_out"]);
    const tx = ev.receipt.tx;
    const sell = ev.name === "hunt_slice";
    const bps = BigInt(sell ? engine.tax.sell_bps : engine.tax.buy_bps);
    const leg = taxedTransfer(receipts, tx, sell ? { coin: coin.token, sender: dragon, payee: peers.dcl } : { coin: coin.token, sender: peers.dcl, payee: dragon });
    const row = { tx, kind: KIND[ev.name], amount_in: BigInt(d.amount_in), min_out: BigInt(d.min_out), leg };
    const fail = (why) => out.push({ ...row, pass: false, why });
    if (!leg) {
      fail(sell ? `no transfer of ${coin.token} from the dragon to ${peers.dcl}` : `no transfer of ${coin.token} from ${peers.dcl} to the dragon`);
      continue;
    }
    const wantTax = (leg.gross * bps) / 10_000n;
    if (leg.taxMoves !== 1 || leg.tax !== wantTax) {
      fail(`tax ${leg.tax} in ${leg.taxMoves} transfer(s), expected one of ${wantTax} (${bps} bps of ${leg.gross})`);
      continue;
    }
    if (leg.net !== leg.gross - leg.tax) {
      fail(`net ${leg.net} is not gross ${leg.gross} less tax ${leg.tax}`);
      continue;
    }
    if (sell) {
      if (leg.gross !== row.amount_in) {
        fail(`sent ${leg.gross}, the slice logged ${row.amount_in}`);
        continue;
      }
      const back = taxedTransfer(receipts, tx, { coin: peers.wrap, sender: peers.dcl, payee: dragon });
      if (!back || back.gross < row.min_out) {
        fail(`wNEAR back ${back?.gross ?? "none"} is below min_out ${row.min_out}`);
        continue;
      }
      out.push({ ...row, back: back.gross, pass: true });
      continue;
    }
    if (leg.gross < row.min_out) {
      fail(`amount_out ${leg.gross} is below min_out ${row.min_out}`);
      continue;
    }
    out.push({ ...row, pass: true });
  }
  return out;
}

const pct = (part, whole) => (whole ? `${(Number((part * 1_000_000n) / whole) / 10_000).toFixed(2)}%` : "-");

export function describeSwap(s) {
  if (!s.leg) return `${s.kind} tx ${s.tx}: ${s.why}`;
  const legs = `amount_out ${s.leg.gross}, net ${s.leg.net} (${pct(s.leg.net, s.leg.gross)}), tax ${s.leg.tax}`;
  return `${s.kind} tx ${s.tx}: ${legs}${s.back !== undefined ? `, wNEAR back ${s.back} >= min_out ${s.min_out}` : ""}${s.why ? `: ${s.why}` : ""}`;
}

async function main() {
  const { context, options } = await import("./verify.mjs");
  const opts = options(process.argv.slice(2));
  const ctx = await context(opts);
  const config = await ctx.config();
  const receipts = await ctx.history(ctx.accounts.dragon);
  const { decisions, idle, unmade, refunded } = auditDragon(receipts, ctx.accounts.dragon, config);
  for (const x of decisions) {
    if (!x.pass || opts.verbose) {
      console.log(`${x.pass ? "PASS" : "FAIL"}  tx ${x.tx}  ${x.made.kind} ${x.made.amount_in}${x.why ? `: ${x.why}` : ""}`);
    }
  }
  const swaps = taxedSwaps(receipts, config, ctx.accounts.dragon);
  for (const s of swaps) console.log(`${s.pass ? "PASS" : "FAIL"}  ${describeSwap(s)}`);
  const bad = decisions.filter((x) => !x.pass).length + swaps.filter((s) => !s.pass).length;
  console.log(`${decisions.length} trades re-derived on the ${config.preset} cadence (gap ${config.preset_value.cadence.gap_seconds} s, spread ${config.preset_value.cadence.spread_minutes} min); ${swaps.length} swaps checked for the tax; ${bad} disagree; ${idle} observations with no trade due, ${unmade} where a trade was due and none was made, ${refunded} refunded swaps`);
  process.exit(bad ? 1 : 0);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((e) => { console.error(e.message); process.exit(2); });
}
