// The Council drill of design.md section 12, on a testnet system deployed with --preset drill: buy the Scales, close
// the sale, give the Dragon a hoard, then play the six scenarios with real votes. Each step reads the chain and sends
// only what is missing, so a stopped run resumes where it stopped. Without --send it prints what it would send.
import { pathToFileURL } from "node:url";
import { connect, parseSecretKey, view, viewAccount } from "./chain.mjs";
import { eventsOf, readDeployCredentials, sender, systemAccounts } from "./deploy.mjs";

const HELP = `Runs the Council drill on a drill system.

  npm run drill -- --network testnet --root council.thewyrm.testnet --budget 17 [--step buy,close,hoard,council,report]
                   [--scales 45] [--holders a,b] [--send]

  --root       the drill system's root, deployed with deploy.mjs --preset drill
  --scales     sale Scales the holders end up with, at least 45 (the Strict class's minimum electorate)
  --holders    accounts that buy, propose and vote, comma-separated, each holding the signer's key
               (default the signer and the root). Each holder can have one proposal open, so more holders run more
               scenarios at once, and each open proposal holds a 1 NEAR bond.
  --budget N   required with --send: the NEAR this run may spend, net. It counts what the run's own sends take from
               the signer and the holders together, so a bond counts until the finalize that returns it, and NEAR
               arriving from elsewhere between sends adds nothing.
  --step       the steps to run (default all): buy, close, hoard, council, report
  --send       send the transactions. Without it nothing is signed.

Credentials: WYRM_DEPLOY_CREDENTIALS, as for deploy.mjs.`;

export const NEAR = 10n ** 24n;
const MILLINEAR = NEAR / 1_000n;
const TGAS = 10n ** 12n;
const STORAGE_BYTE = 10n ** 19n;
// A send's gas at testnet prices, kept inside the budget with each spend.
const GAS_MARGIN = 20n * MILLINEAR;
// A signer must hold its attached gas at the pessimistic price, about 1e9 yocto a gas unit, before a send is accepted.
const PESSIMISTIC_GAS_PRICE = 1_000_000_000n;
export const GAS = { buy: 300n * TGAS, close: 50n * TGAS, mint: 30n * TGAS, poke: 1_000n * TGAS, propose: 100n * TGAS, vote: 100n * TGAS, finalize: 150n * TGAS };
export const MAX_VOTE_TOKENS = 25;
export const STEPS = ["buy", "close", "hoard", "council", "report"];
const TAG = "drill: ";

// wyrm-types order.rs: the class rules, gated by the minimum electorate first.
export const CLASS = {
  Light: { min: 10, passes: (y, n, e) => (y + n) * 10 >= e * 3 && y > n },
  Strict: { min: 45, passes: (y, n, e) => (y + n) * 3 >= e * 2 && y * 5 >= (y + n) * 4 },
};
export const bondReturns = (y, n, e) => (y + n) * 10 >= e * 3;

// What finalize decides from the votes alone: an Order that passes goes on to the Dragon, which answers Executed or
// LeashHeld.
export function decided(scenario, { yes, no }, electorate) {
  const c = CLASS[scenario.class];
  if (electorate < c.min || !c.passes(yes, no, electorate)) return "Rejected";
  return scenario.signal ? "Carried" : "Executing";
}

const ceil = (a, b) => Math.ceil(a / b);
const bps = (amount, n) => String((BigInt(amount) * BigInt(n)) / 10_000n);

// The scenarios in proposal order. `votes(e)` is the tally for an electorate of e; the Strict ones sit on the class's
// edges: two thirds turnout with 80% yes, and the same class missing at 79%. The leash scenario's Grant is the third
// inside 30 days, so it opens once both earlier Grants executed and finalizes against their window.
export const SCENARIOS = [
  { slug: "setslices", class: "Light", expect: "Executed", kind: () => ({ Order: { SetSlices: { cap_bps: 100 } } }), votes: (e) => ({ yes: ceil(4 * e, 9), no: ceil(e, 9) }) },
  { slug: "grant", class: "Strict", expect: "Executed", kind: ({ hoard, grantees }) => ({ Order: { Grant: { to: grantees[0], amount: bps(hoard, 40) } } }), votes: (e) => { const t = ceil(2 * e, 3); const yes = ceil(4 * t, 5); return { yes, no: t - yes }; } },
  { slug: "hunt-fails", class: "Strict", expect: "Rejected", kind: () => ({ Order: { SetPosture: { hunt: { sell_multiple_bps: 25_000, max_sell_bps: 1_000 } } } }), votes: (e) => { const t = e - 2; const yes = ceil(4 * t, 5) - 1; return { yes, no: t - yes }; } },
  { slug: "burn", class: "Light", expect: "Executed", kind: ({ hoard }) => ({ Order: { Burn: { amount: bps(hoard, 10) } } }), votes: (e) => ({ yes: ceil(4 * e, 9), no: 0 }) },
  { slug: "signal", class: "Light", signal: true, expect: "Carried", kind: () => "Signal", votes: (e) => ({ yes: ceil(3 * e, 10), no: 0 }) },
  { slug: "second-grant", class: "Strict", expect: "Executed", kind: ({ hoard, grantees }) => ({ Order: { Grant: { to: grantees[1], amount: bps(hoard, 40) } } }), votes: (e) => ({ yes: e - 5, no: 0 }) },
  { slug: "leash-held", class: "Strict", expect: "LeashHeld", after: ["grant", "second-grant"], kind: ({ hoard, grantees }) => ({ Order: { Grant: { to: grantees[2], amount: bps(hoard, 40) } } }), votes: (e) => ({ yes: e - 5, no: 0 }) },
];

export const statusName = (s) => (typeof s === "string" ? s : Object.keys(s)[0]);
const TERMINAL = new Set(["Rejected", "Carried", "Executed", "LeashHeld"]);

// The yes tokens are the first `yes` voting Scales by id, the no tokens the next `no`, so a rerun picks the same ones.
export function ballots(scales, votes, voted) {
  const ids = scales.map((s) => s.token_id).sort((a, b) => Number(a) - Number(b));
  const owner = new Map(scales.map((s) => [s.token_id, s.owner]));
  const done = new Set(voted);
  const out = [];
  for (const [approve, tokens] of [[true, ids.slice(0, votes.yes)], [false, ids.slice(votes.yes, votes.yes + votes.no)]]) {
    const byOwner = new Map();
    for (const t of tokens.filter((t) => !done.has(t))) byOwner.set(owner.get(t), [...(byOwner.get(owner.get(t)) ?? []), t]);
    for (const [signer, mine] of byOwner) {
      for (let i = 0; i < mine.length; i += MAX_VOTE_TOKENS) out.push({ signer, approve, tokens: mine.slice(i, i + MAX_VOTE_TOKENS) });
    }
  }
  return out;
}

// The next Council actions from one snapshot: finalize what is due, cast missing votes, then open the next scenario
// that has a free proposer, its dependencies executed and the budget for a bond. Pure.
//   snap: { now, closed, electorate, season, bond, proposals: [ProposalView], votes: { id: [token_id] }, scales: [ScaleView],
//           holders: [account], spendable: yocto left of the budget, hoard, grantees }
export function planCouncil(snap) {
  const bySlug = new Map();
  for (const p of snap.proposals) if (p.description.startsWith(TAG)) bySlug.set(p.description.slice(TAG.length), p);
  const voting = snap.scales.filter((s) => s.voting && snap.holders.includes(s.owner));
  const actions = [];
  for (const sc of SCENARIOS) {
    const p = bySlug.get(sc.slug);
    if (!p || statusName(p.status) !== "Open") continue;
    if (snap.now >= BigInt(p.ends_at)) actions.push({ type: "finalize", slug: sc.slug, id: p.id });
    else for (const b of ballots(voting, sc.votes(p.electorate), snap.votes[p.id] ?? [])) actions.push({ type: "vote", slug: sc.slug, id: p.id, ...b });
  }
  const busy = new Set(snap.proposals.filter((p) => ["Open", "Executing"].includes(statusName(p.status))).map((p) => p.proposer));
  const seasonedAt = voting.reduce((t, s) => (BigInt(s.hold_start) > t ? BigInt(s.hold_start) : t), 0n) + snap.season;
  let spendable = snap.spendable;
  for (const sc of SCENARIOS) {
    if (bySlug.has(sc.slug)) continue;
    if (!snap.closed) return { actions, wait: "the sale is still open, so the electorate is not final" };
    if (sc.after?.some((d) => statusName(bySlug.get(d)?.status ?? "Open") !== "Executed")) continue;
    if (voting.length < snap.electorate) return { actions, wait: `the holders vote with ${voting.length} of ${snap.electorate} sold Scales` };
    if (snap.now < seasonedAt) return { actions, wait: "the Scales are seasoning", until: seasonedAt };
    const proposer = snap.holders.find((h) => !busy.has(h) && voting.some((s) => s.owner === h));
    if (!proposer) break;
    if (spendable < snap.bond + GAS_MARGIN) return { actions, wait: `the next bond needs ${fmt(snap.bond + GAS_MARGIN - spendable)} more of the budget` };
    spendable -= snap.bond + GAS_MARGIN;
    busy.add(proposer);
    const token = voting.find((s) => s.owner === proposer).token_id;
    actions.push({ type: "propose", slug: sc.slug, proposer, token, kind: sc.kind(snap), description: `${TAG}${sc.slug}` });
  }
  const done = SCENARIOS.every((sc) => TERMINAL.has(statusName(bySlug.get(sc.slug)?.status ?? "Open")));
  return { actions, done };
}

export function fmt(yocto) {
  return `${(Number(BigInt(yocto) / (MILLINEAR / 1000n)) / 1e6).toFixed(4)} NEAR`;
}

const call = (method, args, gas, deposit = 0n) => ({ type: "call", method, args, gas, deposit });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export function options(argv) {
  const opts = {};
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i].startsWith("--")) throw new Error(`unexpected argument ${argv[i]}`);
    const name = argv[i].slice(2);
    opts[name] = ["send", "help"].includes(name) ? true : argv[++i];
    if (opts[name] === undefined) throw new Error(`--${name} needs a value`);
  }
  if (opts.help) return opts;
  if (opts.network === "mainnet") throw new Error("the drill preset is testnet only");
  if (!opts.network || !opts.root) throw new Error("--network and --root are required");
  if (opts.send && opts.budget === undefined) throw new Error("--send needs --budget: the NEAR this run may spend");
  opts.steps = opts.step ? opts.step.split(",") : STEPS;
  for (const s of opts.steps) if (!STEPS.includes(s)) throw new Error(`no step named ${s}; the steps are ${STEPS.join(", ")}`);
  opts.scales = Number(opts.scales ?? 45);
  if (!(opts.scales >= CLASS.Strict.min && opts.scales <= 90)) throw new Error("--scales is 45 to 90: Strict proposals need 45");
  opts.budget = opts.budget === undefined ? 0n : BigInt(Math.round(Number(opts.budget) * 1e6)) * (NEAR / 1_000_000n);
  return opts;
}

async function scalesOf(rpc, scales) {
  const out = [];
  for (let from = 0; ; from += 50) {
    const page = await view(rpc, scales, "get_scales", { from, limit: 50 });
    out.push(...page);
    if (page.length < 50) return out;
  }
}

async function proposalsOf(rpc, scales) {
  const out = [];
  for (let from = 0; ; from += 50) {
    const page = await view(rpc, scales, "get_proposals", { from, limit: 50 });
    out.push(...page);
    if (page.length < 50) return out;
  }
}

export function context(opts, { rpc, credentials, out = console.log, submit }) {
  const key = parseSecretKey(credentials.private_key);
  const accounts = systemAccounts(opts.root);
  const signer = credentials.account_id;
  const holders = opts.holders ? opts.holders.split(",") : [signer, accounts.root];
  submit ??= sender(rpc, key, out);
  const ctx = { rpc, opts, accounts, signer, holders, out, send: Boolean(opts.send), gasLog: [], grantees: () => [signer, accounts.root, signer] };
  const free = async (a) => {
    const v = await viewAccount(rpc, a);
    return v ? BigInt(v.amount) - BigInt(v.storage_usage) * STORAGE_BYTE : 0n;
  };
  const held = async () => {
    let total = 0n;
    for (const a of new Set([signer, ...holders])) total += await free(a);
    return total;
  };
  ctx.spent = 0n;
  ctx.spendable = () => opts.budget - ctx.spent;
  // Every send passes the budget first, then books what the signer and holders together hold less after it. NEAR from
  // elsewhere that lands while a send is in flight would book as a return; between sends it books nothing.
  ctx.submit = async (tx, cost = 0n) => {
    const room = ctx.spendable();
    if (room < cost + GAS_MARGIN) throw new Error(`BLOCKED: ${tx.receiver}.${tx.actions[0].method ?? tx.actions[0].type} needs ${fmt(cost + GAS_MARGIN)} and the budget leaves ${fmt(room)}; short ${fmt(cost + GAS_MARGIN - room)}`);
    if (!ctx.send) {
      out(`  would send ${tx.signer} -> ${tx.receiver}: ${tx.actions.map((a) => a.method ?? a.type).join(", ")}${cost ? ` (${fmt(cost)})` : ""}`);
      return null;
    }
    const before = await held();
    try {
      return await submit(tx);
    } finally {
      ctx.spent += before - (await held());
    }
  };
  // Tops a holder up from the signer so it can pay `deposit` and the gas it attaches.
  ctx.fund = async (holder, deposit, gas) => {
    if (holder === signer) return;
    const need = deposit + gas * PESSIMISTIC_GAS_PRICE;
    const have = await free(holder);
    if (have >= need + GAS_MARGIN) return;
    await ctx.submit({ signer, receiver: holder, actions: [{ type: "transfer", amount: need + GAS_MARGIN - have }] });
  };
  return ctx;
}

export const STEP_RUNS = {
  async buy(ctx) {
    const { rpc, accounts, holders, opts } = ctx;
    const sale = await view(rpc, accounts.scales, "get_sale");
    const held = (await scalesOf(rpc, accounts.scales)).filter((s) => s.voting && holders.includes(s.owner));
    const missing = opts.scales - held.length;
    if (missing <= 0) return `the holders own ${held.length} sale Scales, ${sale.sold} sold`;
    if (sale.state !== "Open") throw new Error(`the sale is ${sale.state} and the holders own ${held.length} of ${opts.scales} Scales`);
    const price = BigInt(sale.price);
    for (let i = 0; i < missing; i++) {
      const counts = holders.map((h) => held.filter((s) => s.owner === h).length);
      const buyer = holders[counts.indexOf(Math.min(...counts))];
      await ctx.fund(buyer, price, GAS.buy);
      const res = await ctx.submit({ signer: buyer, receiver: accounts.scales, actions: [call("buy_scale", {}, GAS.buy, price)] }, price);
      if (!res) return `would buy ${missing} Scales at ${fmt(price)}`;
      held.push({ owner: buyer, token_id: JSON.parse(Buffer.from(res.status.SuccessValue, "base64").toString()) });
    }
    return `bought ${missing}; the holders own ${held.length} sale Scales`;
  },

  async close(ctx) {
    const { rpc, accounts } = ctx;
    for (;;) {
      const sale = await view(rpc, accounts.scales, "get_sale");
      if (sale.state === "Closed" && sale.tithe_told) return `closed with ${sale.sold} sold, retired from ${sale.retired_from}`;
      const now = BigInt((await rpc("block", { finality: "final" })).header.timestamp);
      if (sale.state === "Open" && now < BigInt(sale.ends_at)) {
        const ms = Number((BigInt(sale.ends_at) - now) / 1_000_000n) + 5_000;
        if (!ctx.send) return `the sale ends in ${Math.round(ms / 60_000)} minutes`;
        ctx.out(`  waiting ${Math.round(ms / 60_000)} minutes for the sale to end`);
        await sleep(Math.min(ms, 10 * 60_000));
        continue;
      }
      if (!(await ctx.submit({ signer: ctx.signer, receiver: accounts.scales, actions: [call("close_sale", {}, GAS.close)] }))) return "would close the sale";
    }
  },

  // The mock coin mints to anyone, so the Dragon's hoard is a donation the next poke books, as the sandbox suite's
  // donate does. The hoard is many times the pool's coin, so a breath's buy cannot move the Council's bps limits.
  async hoard(ctx) {
    const { rpc, accounts } = ctx;
    const coin = await view(rpc, accounts.dragon, "get_coin");
    if (!coin) throw new Error("the Dragon is not bound; run deploy.mjs through bind");
    const peers = (await view(rpc, accounts.dragon, "get_config")).peers;
    if (!coin.token.endsWith(`.${peers.nearly}`)) throw new Error(`${coin.token} is not a mock Nearly token, so the drill cannot mint it`);
    const grantees = ctx.grantees();
    for (const g of new Set(grantees)) {
      if ((await view(rpc, coin.token, "storage_balance_of", { account_id: g })) === null) {
        await ctx.submit({ signer: ctx.signer, receiver: coin.token, actions: [call("mint", { account_id: g, amount: "0" }, GAS.mint)] });
      }
    }
    const pool = await view(rpc, peers.dcl, "get_pool", { pool_id: coin.pool_id });
    const poolCoin = BigInt(coin.wrap_is_x ? pool.total_y : pool.total_x);
    const want = poolCoin * 10n;
    const hoard = BigInt((await view(rpc, accounts.dragon, "get_state")).hoard.wyrm);
    if (hoard >= want) return `the hoard holds ${hoard}`;
    const held = BigInt(await view(rpc, coin.token, "ft_balance_of", { account_id: accounts.dragon }));
    if (held < want) await ctx.submit({ signer: ctx.signer, receiver: coin.token, actions: [call("mint", { account_id: accounts.dragon, amount: String(want - held) }, GAS.mint)] });
    for (let i = 0; i < 10; i++) {
      if (!ctx.send) return `would mint ${want - held} and poke`;
      await ctx.submit({ signer: ctx.signer, receiver: accounts.dragon, actions: [call("poke", {}, GAS.poke)] });
      const now = BigInt((await view(rpc, accounts.dragon, "get_state")).hoard.wyrm);
      if (now >= want) return `the hoard holds ${now}`;
      await sleep(5_000);
    }
    throw new Error("the poke did not book the donation");
  },

  // A failed read or send costs one round: the next snapshot shows what landed, so nothing is sent twice.
  async council(ctx) {
    let failures = 0;
    for (;;) {
      let snap, plan;
      try {
        snap = await snapshot(ctx);
        plan = planCouncil(snap);
        for (const a of plan.actions) await act(ctx, a, snap);
        failures = 0;
      } catch (e) {
        if (e.message.startsWith("BLOCKED") || ++failures > 20) throw e;
        ctx.out(`  retrying after: ${e.message}`);
        await sleep(15_000);
        continue;
      }
      const { actions, wait, until, done } = plan;
      if (done) return "every scenario reached its end";
      if (!ctx.send) return actions.length ? "dry run" : `waiting: ${wait ?? "votes are open"}`;
      if (actions.length) continue;
      const open = snap.proposals.filter((p) => statusName(p.status) === "Open").map((p) => BigInt(p.ends_at));
      const next = [until, ...open].filter(Boolean).reduce((a, b) => (b < a ? b : a), snap.now + 60_000_000_000n);
      const ms = Math.max(3_000, Math.min(60_000, Number((next - snap.now) / 1_000_000n) + 3_000));
      if (wait) ctx.out(`  waiting: ${wait}`);
      await sleep(ms);
    }
  },

  async report(ctx) {
    const { rpc, accounts } = ctx;
    const proposals = (await proposalsOf(rpc, accounts.scales)).filter((p) => p.description.startsWith(TAG));
    const lines = proposals.map((p) => {
      const sc = SCENARIOS.find((s) => `${TAG}${s.slug}` === p.description);
      const got = statusName(p.status);
      const reason = p.status.LeashHeld ? ` (${p.status.LeashHeld.error})` : "";
      return `  #${p.id} ${sc?.slug ?? p.description} ${p.class} ${p.yes} yes ${p.no} no of ${p.electorate}: ${got}${reason} ${got === sc?.expect ? "as planned" : `PLANNED ${sc?.expect}`}`;
    });
    for (const g of ctx.gasLog) lines.push(`  finalize #${g.id} ${g.slug}: ${g.tgas} Tgas burnt, tx ${g.hash}`);
    if (proposals.length) {
      const first = proposals.reduce((t, p) => (BigInt(p.created_at) < t ? BigInt(p.created_at) : t), BigInt(proposals[0].created_at));
      const last = ctx.gasLog.reduce((t, g) => (g.at > t ? g.at : t), 0n);
      if (last) lines.push(`  first proposal to last status: ${(Number(last - first) / 6e10).toFixed(1)} minutes`);
    }
    ctx.out(lines.join("\n"));
    const missed = SCENARIOS.filter((sc) => statusName(proposals.find((p) => p.description === `${TAG}${sc.slug}`)?.status ?? "missing") !== sc.expect);
    return missed.length ? `not yet as planned: ${missed.map((s) => s.slug).join(", ")}` : "all seven proposals ended as planned";
  },
};

async function snapshot(ctx) {
  const { rpc, accounts, holders } = ctx;
  const [header, sale, council, proposals, scales, state] = await Promise.all([
    rpc("block", { finality: "final" }).then((b) => b.header),
    view(rpc, accounts.scales, "get_sale"),
    view(rpc, accounts.scales, "get_council"),
    proposalsOf(rpc, accounts.scales),
    scalesOf(rpc, accounts.scales),
    view(rpc, accounts.dragon, "get_state"),
  ]);
  const votes = {};
  for (const p of proposals) if (statusName(p.status) === "Open") votes[p.id] = (await view(rpc, accounts.scales, "get_votes", { id: p.id })).map((v) => v.token_id);
  return {
    now: BigInt(header.timestamp), closed: sale.state === "Closed", electorate: sale.sold, season: BigInt(council.season_ns),
    bond: BigInt(council.bond) + BigInt(council.storage_fee), proposals, votes, scales, holders,
    spendable: ctx.spendable(), hoard: state.hoard.wyrm, grantees: ctx.grantees(),
  };
}

async function act(ctx, a, snap) {
  const scales = ctx.accounts.scales;
  if (a.type === "propose") {
    await ctx.fund(a.proposer, snap.bond, GAS.propose);
    ctx.out(`  propose ${a.slug} by ${a.proposer} with Scale ${a.token}: ${JSON.stringify(a.kind)}`);
    await ctx.submit({ signer: a.proposer, receiver: scales, actions: [call("propose", { kind: a.kind, description: a.description, token_id: a.token }, GAS.propose, snap.bond)] }, snap.bond);
  } else if (a.type === "vote") {
    await ctx.fund(a.signer, 1n, GAS.vote);
    ctx.out(`  vote ${a.approve ? "yes" : "no"} on #${a.id} ${a.slug}: ${a.tokens.length} Scales of ${a.signer}`);
    await ctx.submit({ signer: a.signer, receiver: scales, actions: [call("vote", { proposal_id: a.id, token_ids: a.tokens, approve: a.approve }, GAS.vote, 1n)] });
  } else {
    const res = await ctx.submit({ signer: ctx.signer, receiver: scales, actions: [call("finalize", { proposal_id: a.id }, GAS.finalize)] });
    if (!res) return;
    const tgas = [res.transaction_outcome, ...res.receipts_outcome].reduce((s, r) => s + BigInt(r.outcome.gas_burnt), 0n);
    const lastBlock = res.receipts_outcome[res.receipts_outcome.length - 1].block_hash;
    const at = BigInt((await ctx.rpc("block", { block_id: lastBlock })).header.timestamp);
    const ended = eventsOf(res).filter((e) => ["proposal_finalized", "proposal_executed", "order_refused"].includes(e.event)).map((e) => `${e.event} ${JSON.stringify(e.data?.[0] ?? e.data)}`);
    ctx.gasLog.push({ id: a.id, slug: a.slug, hash: res.transaction.hash, tgas: (Number(tgas) / 1e12).toFixed(2), at });
    ctx.out(`  finalized #${a.id} ${a.slug}: ${(Number(tgas) / 1e12).toFixed(2)} Tgas burnt of 150, tx ${res.transaction.hash}\n    ${ended.join("\n    ")}`);
  }
}

async function main() {
  const opts = options(process.argv.slice(2));
  if (opts.help) return console.log(HELP);
  const credentials = readDeployCredentials(process.env.WYRM_DEPLOY_CREDENTIALS);
  const { rpc } = connect({ network: opts.network, rpc: opts.rpc });
  const ctx = context(opts, { rpc, credentials });
  ctx.out(`drill ${opts.network} root ${opts.root} holders ${ctx.holders.join(", ")} ${ctx.send ? "SEND" : "dry run"}; ${fmt(ctx.spendable())} to spend`);
  for (const name of opts.steps) ctx.out(`${name}: ${await STEP_RUNS[name](ctx)}`);
  ctx.out(`spent ${fmt(ctx.spent)} net, ${fmt(ctx.spendable())} of the budget left`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((e) => { console.error(`drill: ${e.message}`); process.exit(1); });
}
