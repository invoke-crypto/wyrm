// The verify checks. Each has `read(ctx)`, which fetches what it needs from the chain and the indexer, and a pure
// `judge(data, accounts)`, which returns { summary, items: [{ pass, evidence }] }. A check passes when every item does.
// The coin is only ever `dragon.get_coin()`; peers (Nearly, DCL, wrap, ash, the creator) come from its get_config.
import { mapLimit, view, viewAccount } from "../keeper/chain.mjs";
import { wyrmEvents } from "./history.mjs";
import { auditDragon, describeSwap, taxedSwaps } from "./audit-dragon.mjs";

const DAY_NS = 86_400n * 10n ** 9n;
const NEAR = 10n ** 24n;
const EMPTY_HASH = "11111111111111111111111111111111";
const FIRST_CREATOR_ID = 91;
const ok = (evidence) => ({ pass: true, evidence });
const bad = (evidence) => ({ pass: false, evidence });
const pad = (i) => String(i).padStart(3, "0");
const IDS = Array.from({ length: 100 }, (_, i) => i + 1);

// The tax every WYRM system promises: 4% each side, all of it on the creator side, which Nearly pays to the tithe.
export const TAX = { buy_bps: 400, sell_bps: 400, creator_bps: 10_000, burn_bps: 0, holders_bps: 0 };

export function wyrmAccounts(root) {
  const scales = `scales.${root}`;
  return { root, tithe: `tithe.${root}`, dragon: `dragon.${root}`, scales, vaults: IDS.map((i) => `${pad(i)}.${scales}`) };
}

function shared(ctx, key, read) {
  if (!ctx.cache.has(key)) ctx.cache.set(key, read());
  return ctx.cache.get(key);
}

async function pages(rpc, account, method) {
  const out = [];
  for (let from = 0; ; from += 50) {
    const page = await view(rpc, account, method, { from, limit: 50 });
    out.push(...page);
    if (page.length < 50) return out;
  }
}

const coinOf = (ctx) => shared(ctx, "coin", () => view(ctx.rpc, ctx.accounts.dragon, "get_coin"));

const systemViews = (ctx) => shared(ctx, "views", async () => {
  const { peers } = await ctx.config();
  const list = [ctx.accounts.tithe, ctx.accounts.dragon, ctx.accounts.scales, peers.ash, ...ctx.accounts.vaults];
  const views = await mapLimit(list, 8, (a) => viewAccount(ctx.rpc, a));
  return { ash: peers.ash, rows: list.map((account, i) => ({ account, view: views[i] })) };
});

const boundLaunch = (ctx) => shared(ctx, "launch", async () => {
  const coin = await coinOf(ctx);
  if (!coin) return { coin };
  const { peers } = await ctx.config();
  const launch_id = coin.launch_id;
  const [record, feeTo, tax, feeOpts, pool] = await Promise.all([
    view(ctx.rpc, peers.nearly, "get_launch", { launch_id }),
    view(ctx.rpc, peers.nearly, "get_fee_to", { launch_id }),
    view(ctx.rpc, peers.nearly, "get_tax", { launch_id }),
    view(ctx.rpc, peers.nearly, "get_fee_opts", { launch_id }),
    view(ctx.rpc, peers.dcl, "get_pool", { pool_id: coin.pool_id }),
  ]);
  return { coin, record, feeTo, tax, feeOpts, pool, peers };
});

const codeOf = (v) => v.global_contract_hash ?? v.code_hash;
const crateOf = (account, accounts) => ({ [accounts.tithe]: "tithe", [accounts.dragon]: "dragon", [accounts.scales]: "scales" })[account] ?? "scale_vault";
const launchName = (r) => `launch ${r.id} (${r.symbol}, token ${r.token})`;
const notBound = { summary: "the dragon is not bound: get_coin is null", items: [bad("the dragon is not bound: get_coin is null")] };

function fields(ev, names) {
  for (const n of names) if (ev.data[n] === undefined) throw new Error(`${ev.name} in tx ${ev.receipt.tx} has no "${n}" field`);
  return ev.data;
}

// Runs `fn` per event, turning a malformed event into a failing item instead of an exception.
const each = (events, fn) => events.map((ev) => {
  try {
    return fn(ev);
  } catch (e) {
    return bad(e.message);
  }
});

export const variant = (x) => (typeof x === "string" ? [x, {}] : Object.entries(x)[0]);

// Why an order breaks the preset, or null: wyrm-types Order::check, the rule the Dragon's obey applies.
export function orderViolation(order, preset) {
  const [kind, body] = variant(order);
  const feature = { SetPosture: "hunt", Grant: "grant", Burn: "burn", SetAutoBurn: "auto_burn" }[kind];
  if (feature && !preset.features[feature]) return `${kind} is disabled in the ${preset.name} preset`;
  const b = preset.bounds;
  const range = (what, r, v) => (v >= r.lo && v <= r.hi ? null : `${what} must be ${r.lo} to ${r.hi}, got ${v}`);
  switch (kind) {
    case "SetBreath":
      return range("budget_bps", b.budget_bps, body.budget_bps);
    case "SetSlices":
      return range("cap_bps", b.cap_bps, body.cap_bps);
    case "SetPosture":
      if (!body.hunt) return null;
      if (body.hunt.sell_multiple_bps < b.sell_multiple_min_bps) return `sell_multiple_bps must be at least ${b.sell_multiple_min_bps}, got ${body.hunt.sell_multiple_bps}`;
      return range("max_sell_bps", b.max_sell_bps, body.hunt.max_sell_bps);
    case "Grant":
    case "Burn":
      return BigInt(body.amount) === 0n ? "amount must be above 0" : null;
    case "SetAutoBurn":
      return range("auto_burn_bps", b.auto_burn_bps, body.bps);
    default:
      return `${kind} is not an order`;
  }
}

const SLICE_FIELDS = ["at", "breath_at", "amount_in", "quoted_out", "min_out", "tokens_per_near", "reference", "cap_bps", "budget"];
const BUYS = new Set(["breath_slice", "dip_slice"]);

// Buy slices. A slice event is logged only once its swap filled.
export const buySlices = (dragonReceipts, dragon) => wyrmEvents(dragonReceipts, dragon).filter((e) => BUYS.has(e.name));

// Why one buy slice broke the rules it claims, or null. Pure arithmetic on the logged values and the engine's constants.
export function sliceRuleBreak(ev, engine) {
  const d = fields(ev, ev.name === "dip_slice" ? [...SLICE_FIELDS, "step_bps", "confirmations", "confirmed_since"] : SLICE_FIELDS);
  const at = BigInt(d.at);
  const tpn = BigInt(d.tokens_per_near);
  const ref = d.reference === null ? null : BigInt(d.reference);
  if (ref !== null && tpn * BigInt(10_000 + engine.spike_bps) < ref * 10_000n) return `bought during a spike: spot more than ${engine.spike_bps} bps above the reference`;
  if (ev.name === "breath_slice") {
    const breathAt = BigInt(d.breath_at);
    if (at < breathAt) return `breath slice at ${at} before the breath minute ${breathAt}`;
    if (at / DAY_NS !== breathAt / DAY_NS) return `breath minute ${breathAt} is not on the slice's UTC day`;
    return null;
  }
  if (ref === null) return "dip slice without a reference price";
  const step = BigInt(d.step_bps);
  if (tpn * (10_000n - step) < ref * 10_000n) return `dip step ${step} bps not reached: ${tpn} tokens/NEAR against reference ${ref}`;
  if (Number(d.confirmations) < engine.dip_samples) return `dip confirmed by ${d.confirmations} samples, fewer than ${engine.dip_samples}`;
  const span = BigInt(engine.dip_span_ns);
  if (at - BigInt(d.confirmed_since) < span) return `dip confirmed over ${(at - BigInt(d.confirmed_since)) / 10n ** 9n} s, under ${span / 10n ** 9n} s`;
  return null;
}

// Price impact: quote(a) * 10_000 >= a * tokens_per_near * (10_000 - cap_bps), with a in NEAR.
export function impactOk(d) {
  return BigInt(d.quoted_out) * 10_000n * NEAR >= BigInt(d.amount_in) * BigInt(d.tokens_per_near) * (10_000n - BigInt(d.cap_bps));
}

// The wNEAR the Dragon actually sent to DCL in a transaction.
const swapIn = (receipts, tx, { dragon, wrap, dcl }) => receipts
  .filter((r) => r.tx === tx && r.predecessor === dragon && r.receiver === wrap && r.ok)
  .flatMap((r) => r.actions)
  .filter((a) => a.method === "ft_transfer_call" && a.args?.receiver_id === dcl)
  .reduce((sum, a) => sum + BigInt(a.args.amount), 0n);

const transfersFrom = (receipts, tx, from) => receipts.filter((r) => r.tx === tx && r.predecessor === from)
  .flatMap((r) => r.actions.filter((a) => a.kind === "Transfer").map((a) => [r.receiver, a.deposit]));

// One rolling-window check per outflow kind: each at most 50 bps of `hoard`, and the kind's paid outflows in the
// 30 days up to it at most 100 bps of that hoard. Failed outflows emit outflow_failed and free their share.
function outflowItems(events, kind, engine) {
  const parsed = events.map((ev) => {
    try {
      const d = fields(ev, ["at", "amount", "hoard"]);
      return { ev, at: BigInt(d.at), amount: BigInt(d.amount), hoard: BigInt(d.hoard) };
    } catch (e) {
      return { error: e.message };
    }
  });
  const window = BigInt(engine.outflow_window_ns);
  return parsed.map((g) => {
    if (g.error) return bad(g.error);
    if (g.amount * 10_000n > g.hoard * BigInt(engine.outflow_bps)) return bad(`${kind} tx ${g.ev.receipt.tx}: ${g.amount} is above ${engine.outflow_bps} bps of the hoard ${g.hoard}`);
    const sum = parsed.filter((x) => !x.error && x.at <= g.at && x.at > g.at - window).reduce((s, x) => s + x.amount, 0n);
    return sum * 10_000n <= g.hoard * BigInt(engine.outflow_window_bps) ? ok(`${kind} tx ${g.ev.receipt.tx}: 30-day total ${sum} within ${engine.outflow_window_bps} bps of ${g.hoard}`)
      : bad(`${kind} tx ${g.ev.receipt.tx}: 30-day total ${sum} above ${engine.outflow_window_bps} bps of the hoard ${g.hoard}`);
  });
}

const swapCheck = (kind) => async (ctx) => ({ receipts: await ctx.history(ctx.accounts.dragon), config: await ctx.config(), kind });
function judgeSwaps({ receipts, config, kind }, accounts) {
  if (!config.coin) return notBound;
  const swaps = taxedSwaps(receipts, config, accounts.dragon).filter((s) => (s.kind === "hunt") === (kind === "sell"));
  const items = swaps.map((s) => (s.pass ? ok(describeSwap(s)) : bad(describeSwap(s))));
  const bps = kind === "sell" ? config.engine.tax.sell_bps : config.engine.tax.buy_bps;
  return { summary: `${swaps.length} ${kind === "sell" ? "hunt sells" : "buys"} against a ${bps} bps ${kind} tax`, items };
}

export const CHECKS = [
  {
    id: 1, title: "zero access keys on tithe, dragon, scales, ash and the 100 vaults; ash has no code",
    read: async (ctx) => {
      const { ash, rows } = await systemViews(ctx);
      const keys = await mapLimit(rows, 8, async ({ view: v, account }) => (v ? (await ctx.rpc("query", { request_type: "view_access_key_list", finality: "final", account_id: account })).keys : null));
      return { ash, rows: rows.map((r, i) => ({ ...r, keys: keys[i] })) };
    },
    judge: ({ ash, rows }) => {
      const items = rows.map(({ account, view: v, keys }) => {
        if (!v) return bad(`${account} does not exist`);
        if (keys.length) return bad(`${account} has ${keys.length} key(s): ${keys.map((k) => `${k.public_key} ${k.access_key.permission === "FullAccess" ? "FullAccess" : "FunctionCall"}`).join(", ")}`);
        if (account === ash && (v.code_hash !== EMPTY_HASH || v.global_contract_hash)) return bad(`${account} runs code ${codeOf(v)}; ash has none`);
        return ok(`${account} 0 keys${account === ash ? ", no code" : ""}`);
      });
      return { summary: `${rows.length} accounts, ${items.filter((i) => i.pass).length} keyless`, items };
    },
  },
  {
    id: 2, title: "the code is a release in verify/releases.json, and scales and dragon run that release's preset",
    read: async (ctx) => {
      const [{ ash, rows }, config, scalesConfig] = await Promise.all([systemViews(ctx), ctx.config(), view(ctx.rpc, ctx.accounts.scales, "get_config")]);
      return { network: ctx.opts.network, releases: ctx.releases(), rows: rows.filter((r) => r.account !== ash), config, scalesConfig };
    },
    judge: ({ network, releases, rows, config, scalesConfig }, accounts) => {
      const items = [];
      if (network === "mainnet" && (config.preset === "drill" || config.preset_value.network !== "Any")) {
        items.push(bad(`the mainnet system runs the ${config.preset} preset, which is testnet only (drill)`));
      }
      const dragonCode = rows.find((r) => r.account === accounts.dragon)?.view;
      const release = dragonCode && releases.find((r) => r.code.dragon === codeOf(dragonCode));
      for (const { account, view: v } of rows) {
        const crate = crateOf(account, accounts);
        if (!v) items.push(bad(`${account} does not exist`));
        else if (!release) items.push(bad(`${account} runs ${codeOf(v)}, and no release in verify/releases.json has that dragon`));
        else if (codeOf(v) !== release.code[crate]) items.push(bad(`${account} runs ${codeOf(v)}, ${release.name} ${crate}.wasm is ${release.code[crate]}`));
        else items.push(ok(`${account} ${codeOf(v)} = ${release.name} ${crate}.wasm`));
      }
      const same = (what, a, b) => items.push(a === b ? ok(`${what} ${a} on dragon and scales`) : bad(`${what}: dragon ${a}, scales ${b}`));
      same("preset", config.preset, scalesConfig.preset);
      same("preset_hash", config.preset_hash, scalesConfig.preset_hash);
      same("engine_version", config.engine_version, scalesConfig.engine_version);
      if (release) {
        const want = release.presets[config.preset];
        items.push(want === config.preset_hash ? ok(`${config.preset} hash ${want} is ${release.name}'s`) : bad(`${config.preset} hash ${config.preset_hash}, ${release.name} holds ${want ?? "no such preset"}`));
        items.push(release.engine_version === config.engine_version ? ok(`engine_version ${config.engine_version} is ${release.name}'s`) : bad(`engine_version ${config.engine_version}, ${release.name} is ${release.engine_version}`));
      }
      return { summary: `${release ? release.name : "no release"}, preset ${config.preset} ${config.preset_hash}, ${items.filter((i) => i.pass).length}/${items.length} items`, items };
    },
  },
  {
    id: 3, title: "the coin is dragon.get_coin(): launched by the dragon, Done, paying the tithe",
    read: async (ctx) => {
      const launch = await boundLaunch(ctx);
      const bySymbol = launch.record ? await view(ctx.rpc, launch.peers.nearly, "get_launch_by_symbol", { symbol: launch.record.symbol }) : null;
      return { ...launch, bySymbol };
    },
    judge: ({ coin, record, feeTo, bySymbol }, accounts) => {
      if (!coin) return notBound;
      if (!record) return { summary: `Nearly has no launch ${coin.launch_id}`, items: [bad(`Nearly has no launch ${coin.launch_id}, the one get_coin names`)] };
      const items = [
        record.creator === accounts.dragon ? ok(`${launchName(record)} was launched by ${record.creator}`) : bad(`${launchName(record)} was launched by ${record.creator}, not ${accounts.dragon}`),
        record.token === coin.token && record.pool_id === coin.pool_id ? ok(`get_coin names token ${coin.token} and pool ${coin.pool_id}, as the record does`)
          : bad(`get_coin names ${coin.token} / ${coin.pool_id}, the record ${record.token} / ${record.pool_id}`),
        record.step === "Done" ? ok("step Done") : bad(`step ${record.step}, not Done`),
        feeTo === accounts.tithe ? ok(`fee_to ${feeTo}`) : bad(`fee_to ${feeTo}, not ${accounts.tithe}`),
      ];
      if (bySymbol && String(bySymbol.id) !== String(record.id)) {
        const whose = bySymbol.creator === accounts.dragon ? "an orphan launch by the dragon" : `a launch by ${bySymbol.creator}`;
        items.push(ok(`${launchName(bySymbol)} is ${whose}, not bound`));
      }
      return { summary: `the coin is ${launchName(record)}, from get_coin`, items };
    },
  },
  {
    id: 4, title: "tax 400 bps each side, all to the creator side; pool fee mode creator; the pool runs wrap and the coin",
    read: boundLaunch,
    judge: ({ coin, tax, feeOpts, pool, peers }) => {
      if (!coin) return notBound;
      if (!tax) return { summary: `launch ${coin.launch_id} has no tax`, items: [bad(`get_tax(${coin.launch_id}) is null`)] };
      const items = Object.entries(TAX).map(([k, want]) => ((tax[k] ?? 0) === want ? ok(`${k} ${want}`) : bad(`${k} ${tax[k]}, expected ${want}`)));
      items.push(feeOpts.mode === "creator" ? ok("pool fee mode creator") : bad(`pool fee mode ${feeOpts.mode}, expected creator`));
      const tokens = pool ? [pool.token_x, pool.token_y].sort().join(" ") : null;
      const want = [peers.wrap, coin.token].sort().join(" ");
      items.push(!pool ? bad(`${peers.dcl} has no pool ${coin.pool_id}`)
        : pool.state === "Running" && tokens === want ? ok(`pool ${coin.pool_id} Running with ${want}`)
        : bad(`pool ${coin.pool_id} is ${pool.state} with ${tokens}, expected Running with ${want}`));
      return { summary: `${tax.buy_bps}/${tax.sell_bps} bps, creator ${tax.creator_bps}, platform ${tax.platform_bps}, fee mode ${feeOpts.mode}`, items };
    },
  },
  {
    id: 5, title: "every tithe_round paid half to the dragon and 1/100 of the rest to each unretired vault",
    read: async (ctx) => ({ receipts: await ctx.history(ctx.accounts.tithe), stats: await view(ctx.rpc, ctx.accounts.tithe, "get_stats") }),
    judge: ({ receipts, stats }, accounts) => {
      const rounds = wyrmEvents(receipts, accounts.tithe).filter((e) => e.name === "tithe_round");
      const retiredFrom = stats.retired_from;
      const shapes = [0, ...(retiredFrom === null ? [] : [FIRST_CREATOR_ID - retiredFrom])];
      const items = each(rounds, (ev) => {
        const d = fields(ev, ["round", "to_dragon", "per_scale", "dust"]);
        const [toDragon, perScale, dust] = [BigInt(d.to_dragon), BigInt(d.per_scale), BigInt(d.dust)];
        const where = `round ${d.round} tx ${ev.receipt.tx}`;
        if (dust >= 100n) return bad(`${where}: dust ${dust} is not under 100 yocto`);
        const other = perScale * 100n + dust;
        const retired = shapes.find((n) => [other, other - 1n].includes(toDragon - perScale * BigInt(n)));
        if (retired === undefined) return bad(`${where}: dragon ${toDragon} is neither half nor half plus the share of ${shapes.at(-1)} retired vaults`);
        const sent = transfersFrom(receipts, ev.receipt.tx, accounts.tithe);
        const toD = sent.filter(([to]) => to === accounts.dragon);
        if (toD.length !== 1 || toD[0][1] !== toDragon) return bad(`${where}: dragon transfers ${toD.map((x) => x[1])} instead of one of ${toDragon}`);
        for (const [i, v] of accounts.vaults.entries()) {
          const paid = !retired || i + 1 < retiredFrom || i + 1 >= FIRST_CREATOR_ID;
          const got = sent.filter(([to]) => to === v);
          if (paid && (got.length !== 1 || got[0][1] !== perScale)) return bad(`${where}: ${v} got ${got.map((x) => x[1]).join("+") || "nothing"}, expected ${perScale}`);
          if (!paid && got.length) return bad(`${where}: retired ${v} got ${got.map((x) => x[1]).join("+")}`);
        }
        if (sent.length !== 101 - retired) return bad(`${where}: ${sent.length} transfers, expected ${101 - retired}`);
        return ok(`${where}: ${toDragon} to the dragon, ${perScale} to each of ${100 - retired} vaults, dust ${dust}`);
      });
      return { summary: `${rounds.length} rounds, retired from ${retiredFrom ?? "none"}`, items };
    },
  },
  {
    id: 6, title: "every dragon buy followed the breath or a confirmed dip",
    read: async (ctx) => ({ receipts: await ctx.history(ctx.accounts.dragon), config: await ctx.config() }),
    judge: ({ receipts, config }, accounts) => {
      const slices = buySlices(receipts, accounts.dragon);
      const items = each(slices, (ev) => {
        const why = sliceRuleBreak(ev, config.engine);
        return why ? bad(`${ev.name} tx ${ev.receipt.tx}: ${why}`) : ok(`${ev.name} tx ${ev.receipt.tx}`);
      });
      let replay;
      try {
        replay = auditDragon(receipts, accounts.dragon, config);
        for (const x of replay.decisions) items.push(x.pass ? ok(`replay tx ${x.tx}: ${x.made.kind} agrees`) : bad(`replay tx ${x.tx}: ${x.why}`));
      } catch (e) {
        items.push(bad(`replay: ${e.message}`));
      }
      const n = (name) => slices.filter((s) => s.name === name).length;
      const agreed = replay ? `, ${replay.decisions.filter((x) => x.pass).length}/${replay.decisions.length} re-derived by gym/dragon-rules.mjs on the ${config.preset} cadence` : "";
      return { summary: `${slices.length} buys (${n("breath_slice")} breath, ${n("dip_slice")} dip)${agreed}`, items };
    },
  },
  {
    id: 7, title: "daily spend within budget and every slice within the price-impact cap",
    read: async (ctx) => ({ receipts: await ctx.history(ctx.accounts.dragon), config: await ctx.config() }),
    judge: ({ receipts, config }, accounts) => {
      const peers = { dragon: accounts.dragon, wrap: config.peers.wrap, dcl: config.peers.dcl };
      const slices = buySlices(receipts, accounts.dragon);
      const days = new Map();
      const items = each(slices, (ev) => {
        const d = fields(ev, SLICE_FIELDS);
        const paid = swapIn(receipts, ev.receipt.tx, peers);
        const day = BigInt(d.at) / DAY_NS;
        const book = days.get(day) ?? { spent: 0n, budget: 0n };
        book.spent += paid;
        book.budget = BigInt(d.budget) > book.budget ? BigInt(d.budget) : book.budget;
        days.set(day, book);
        if (paid !== BigInt(d.amount_in)) return bad(`${ev.name} tx ${ev.receipt.tx}: logged ${d.amount_in}, sent ${paid} to ${peers.dcl}`);
        if (!impactOk(d)) return bad(`${ev.name} tx ${ev.receipt.tx}: ${d.amount_in} in for ${d.quoted_out} out exceeds ${d.cap_bps} bps impact at ${d.tokens_per_near} tokens/NEAR`);
        return ok(`${ev.name} tx ${ev.receipt.tx}: ${d.amount_in} within ${d.cap_bps} bps impact`);
      });
      for (const [day, { spent, budget }] of days) {
        const date = new Date(Number(day * 86_400_000n)).toISOString().slice(0, 10);
        items.push(spent <= budget ? ok(`${date}: spent ${spent} of ${budget}`) : bad(`${date}: spent ${spent}, budget ${budget}`));
      }
      return { summary: `${slices.length} slices over ${days.size} days`, items };
    },
  },
  {
    id: 8, title: "the dragon's outgoing receipts went only to its peers and its coin, wiring reads to the tithe, refunds to the creator",
    read: async (ctx) => ({ receipts: await ctx.history(ctx.accounts.dragon), config: await ctx.config() }),
    judge: ({ receipts, config }, accounts) => {
      const { peers, coin } = config;
      const any = () => true;
      const rules = new Map([[peers.wrap, any], [peers.dcl, any], [peers.nearly, any], [peers.scales, any], ...(coin ? [[coin.token, any]] : []),
        [peers.tithe, (a) => a.method === "get_config"], [peers.creator, (a) => a.kind === "Transfer"]]);
      const out = receipts.filter((r) => r.predecessor === accounts.dragon && r.receiver !== accounts.dragon);
      const items = out.filter((r) => !r.actions.every(rules.get(r.receiver) ?? (() => false)))
        .map((r) => bad(`receipt ${r.id} tx ${r.tx} to ${r.receiver}: ${r.actions.map((a) => a.method ?? a.kind).join(", ")}`));
      if (!items.length) items.push(ok(`${out.length} outgoing receipts, to ${[peers.wrap, peers.dcl, peers.nearly, peers.scales, coin?.token].filter(Boolean).join(", ")}, get_config to ${peers.tithe} and transfers to ${peers.creator}`));
      return { summary: `${out.length} outgoing receipts`, items };
    },
  },
  {
    id: 9, title: "the Council: every executed order passes the preset, went Open to Executing at finalize, and every leash hold has its refusal",
    read: async (ctx) => {
      const [config, scalesConfig, proposals, receipts] = await Promise.all([
        ctx.config(), view(ctx.rpc, ctx.accounts.scales, "get_config"), pages(ctx.rpc, ctx.accounts.scales, "get_proposals"), ctx.history(ctx.accounts.scales),
      ]);
      return { config, scalesConfig, proposals, receipts };
    },
    judge: ({ config, scalesConfig, proposals, receipts }, accounts) => {
      const preset = config.preset_value;
      const items = [
        config.peers.scales === accounts.scales && scalesConfig.dragon === accounts.dragon ? ok(`dragon's council is ${accounts.scales}, scales' dragon is ${accounts.dragon}`)
          : bad(`dragon's council is ${config.peers.scales}, scales' dragon is ${scalesConfig.dragon}`),
      ];
      const s = config.strategy;
      const live = orderViolation({ SetBreath: { budget_bps: s.budget_bps } }, preset) ?? orderViolation({ SetSlices: { cap_bps: s.cap_bps } }, preset)
        ?? (s.hunt ? orderViolation({ SetPosture: { hunt: s.hunt } }, preset) : null) ?? (s.auto_burn_bps ? orderViolation({ SetAutoBurn: { bps: s.auto_burn_bps } }, preset) : null);
      items.push(live ? bad(`live strategy breaks the ${preset.name} preset: ${live}`) : ok(`live strategy budget ${s.budget_bps}, cap ${s.cap_bps}, hunt ${s.hunt ? "on" : "off"}, auto-burn ${s.auto_burn_bps} within the ${preset.name} bounds`));
      const finalizeTx = new Map();
      for (const ev of wyrmEvents(receipts, accounts.scales)) if (ev.name === "proposal_finalized") finalizeTx.set(Number(ev.data.proposal_id), ev);
      const dragonEvents = wyrmEvents(receipts, accounts.dragon);
      for (const p of proposals) {
        const [status, detail] = variant(p.status);
        const [kind, order] = variant(p.kind);
        const fin = finalizeTx.get(Number(p.id));
        const label = kind === "Order" ? variant(order)[0] : kind;
        if (status === "Executed") {
          const why = orderViolation(order, preset);
          items.push(why ? bad(`proposal ${p.id} ${label} executed although ${why}`) : ok(`proposal ${p.id} ${label} passes the ${preset.name} preset`));
          items.push(fin && fin.data.status === "Executing" ? ok(`proposal ${p.id} went to Executing in finalize tx ${fin.receipt.tx}`) : bad(`proposal ${p.id} has no finalize to Executing`));
        }
        if (status === "LeashHeld") {
          const tx = fin?.receipt.tx;
          const refusal = tx && (dragonEvents.find((e) => e.receipt.tx === tx && (e.name === "order_refused" || e.name === "outflow_failed")) ?? receipts.find((r) => r.tx === tx && !r.ok));
          items.push(refusal ? ok(`proposal ${p.id} ${label} held by the leash (${detail.error}), ${refusal.name ?? `failed receipt ${refusal.id}`} in tx ${tx}`)
            : bad(`proposal ${p.id} ${label} held by the leash (${detail.error}) with no order_refused, outflow_failed or failed receipt in its finalize`));
        }
      }
      const by = (st) => proposals.filter((p) => variant(p.status)[0] === st).length;
      return { summary: `${proposals.length} proposals, ${by("Executed")} executed, ${by("LeashHeld")} held by the leash`, items };
    },
  },
  {
    id: 10, title: "100 Scales at most, every vault NNN.scales.<root> with parent scales; 91-100 are the creator's and never vote",
    read: async (ctx) => {
      const [{ rows }, scales, supply, sale, config] = await Promise.all([systemViews(ctx), pages(ctx.rpc, ctx.accounts.scales, "get_scales"),
        view(ctx.rpc, ctx.accounts.scales, "nft_total_supply"), view(ctx.rpc, ctx.accounts.scales, "get_sale"), ctx.config()]);
      const vaults = rows.filter((r) => ctx.accounts.vaults.includes(r.account));
      const parents = await mapLimit(vaults, 8, ({ account, view: v }) => (v ? view(ctx.rpc, account, "get_parent") : null));
      return { supply, scales, sale, creator: config.peers.creator, parents };
    },
    judge: ({ supply, scales, sale, creator, parents }, accounts) => {
      const items = [];
      const byId = new Map(scales.map((s) => [Number(s.token_id), s]));
      items.push(Number(supply) <= 100 && Number(supply) === scales.length ? ok(`nft_total_supply ${supply}`) : bad(`nft_total_supply ${supply}, listed ${scales.length}`));
      for (const s of scales) if (!(Number(s.token_id) >= 1 && Number(s.token_id) <= 100)) items.push(bad(`token id ${s.token_id} outside 1..100`));
      for (const i of IDS) {
        const s = byId.get(i);
        const vault = accounts.vaults[i - 1];
        if (parents[i - 1] !== accounts.scales) items.push(bad(`${vault} ${parents[i - 1] === null ? "does not exist" : `has parent ${parents[i - 1]}`}`));
        if (i >= FIRST_CREATOR_ID) {
          if (!s) items.push(bad(`creator Scale ${i} is not minted`));
          else if (s.voting !== false) items.push(bad(`creator Scale ${i} has voting ${s.voting}`));
          continue;
        }
        if (!s) {
          if (i <= sale.sold) items.push(bad(`sold Scale ${i} is not minted`));
          continue;
        }
        if (s.voting !== true) items.push(bad(`sold Scale ${i} has voting ${s.voting}`));
        if (sale.retired_from !== null && i >= sale.retired_from) items.push(bad(`Scale ${i} is minted, but the sale retired ids from ${sale.retired_from}`));
      }
      for (const s of scales) if (s.vault !== accounts.vaults[Number(s.token_id) - 1]) items.push(bad(`Scale ${s.token_id} vault ${s.vault}, expected ${accounts.vaults[Number(s.token_id) - 1]}`));
      const creatorHeld = scales.filter((s) => Number(s.token_id) >= FIRST_CREATOR_ID && s.owner === creator).length;
      if (!items.some((x) => !x.pass)) {
        items.push(ok(`${scales.length} Scales (${sale.sold} sold, ${sale.retired_from === null ? "none" : `${FIRST_CREATOR_ID - sale.retired_from}`} retired, 10 creator, ${creatorHeld} still with ${creator}), 100 vaults with parent ${accounts.scales}, 91-100 voting false`));
      }
      return { summary: `${supply} Scales minted, ${sale.sold} of 90 sold, sale ${sale.state}`, items };
    },
  },
  {
    id: 11, title: "every Scale sale paid the creator's cut to the creator, the rest to the dragon, and swept the vault",
    read: async (ctx) => ({ receipts: await ctx.history(ctx.accounts.scales), sale: await view(ctx.rpc, ctx.accounts.scales, "get_sale"), config: await ctx.config() }),
    judge: ({ receipts, sale, config }, accounts) => {
      const events = wyrmEvents(receipts, accounts.scales);
      const mints = events.filter((e) => e.name === "scale_minted" && BigInt(e.data.price ?? 0) > 0n);
      const creator = config.peers.creator;
      let total = 0n;
      const items = each(mints, (ev) => {
        const d = fields(ev, ["token_id", "price"]);
        const price = BigInt(d.price);
        total += price;
        const toCreator = (price * BigInt(sale.creator_bps)) / 10_000n;
        const sent = transfersFrom(receipts.filter((r) => r.ok), ev.receipt.tx, accounts.scales);
        const got = (who) => sent.filter(([to]) => to === who).map(([, amount]) => amount);
        const where = `Scale ${d.token_id} tx ${ev.receipt.tx}`;
        if (toCreator > 0n && !got(creator).includes(toCreator)) return bad(`${where}: ${toCreator} not sent to ${creator} (sent ${got(creator).join(", ") || "nothing"})`);
        if (!got(accounts.dragon).includes(price - toCreator)) return bad(`${where}: ${price - toCreator} not sent to the dragon (sent ${got(accounts.dragon).join(", ") || "nothing"})`);
        const swept = events.some((e) => e.name === "scale_swept" && e.receipt.tx === ev.receipt.tx && e.data.token_ids?.includes(String(d.token_id)));
        if (!swept) return bad(`${where}: no scale_swept for its vault`);
        return ok(`${where}: ${toCreator} to the creator, ${price - toCreator} to the dragon, vault swept`);
      });
      return { summary: `${mints.length} sales, ${total} yocto, creator cut ${sale.creator_bps} bps`, items };
    },
  },
  {
    id: 12, title: "no dev wallet: Nearly's dev-buy recipient is the dragon",
    read: boundLaunch,
    judge: ({ coin, record }, accounts) => {
      if (!coin) return notBound;
      const item = record?.creator === accounts.dragon ? ok(`${launchName(record)} creator ${record.creator} receives its dev buy of ${record.dev_buy_tokens} tokens`)
        : bad(`launch ${coin.launch_id} creator ${record?.creator}, so its dev buy goes there, not to ${accounts.dragon}`);
      return { summary: item.evidence, items: [item] };
    },
  },
  {
    id: 13, title: "grants and burns each stay within 50 bps of the hoard per order and 100 bps per rolling 30 days",
    read: async (ctx) => ({ receipts: await ctx.history(ctx.accounts.dragon), config: await ctx.config() }),
    judge: ({ receipts, config }, accounts) => {
      const events = wyrmEvents(receipts, accounts.dragon);
      const grants = events.filter((e) => e.name === "grant_paid");
      const burns = events.filter((e) => e.name === "burn_paid");
      const items = [...outflowItems(grants, "grant", config.engine), ...outflowItems(burns, "burn", config.engine)];
      return { summary: `${grants.length} grants and ${burns.length} burns paid`, items };
    },
  },
  {
    id: 14, title: "every buy delivered amount_out less the buy tax, as a net transfer and a `tax` transfer, and amount_out met min_out",
    read: swapCheck("buy"),
    judge: judgeSwaps,
  },
  {
    id: 15, title: "every hunt sell sent its gross, DCL got it less the sell tax, and the wNEAR back met min_out on that net",
    read: swapCheck("sell"),
    judge: judgeSwaps,
  },
];

export async function runCheck(check, ctx) {
  let data;
  try {
    data = await check.read(ctx);
  } catch (e) {
    return { id: check.id, title: check.title, pass: false, summary: `could not read: ${e.message}`, items: [] };
  }
  try {
    const { summary, items } = check.judge(data, ctx.accounts);
    return { id: check.id, title: check.title, pass: items.every((i) => i.pass), summary, items };
  } catch (e) {
    return { id: check.id, title: check.title, pass: false, summary: `unexpected data shape: ${e.message}`, items: [] };
  }
}
