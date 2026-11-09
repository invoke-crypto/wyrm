// The WYRM keeper (design.md 1d). Each tick reads the chain at one final block, plans what is due, and sends it.
//   every minute       dragon.poke()                    once the Dragon is bound (get_config().coin)
//                      dragon.nearly_resume(id)         before bind, while the creator's launch is parked
//   fees, from :15     nearly.push_creator_fees(tithe), hourly and sent once per slot whatever the answer. It is
//                      best-effort: Nearly's own relayer pushes at :10 to :14, so the keeper's push is usually refused
//   hourly, from :15   tithe.unwrap() and forward_ft(coin) when it holds them, tithe.distribute(),
//                      scales.finalize(id) for every id in scales.get_due()
//   daily, from 00:00  scales.create_vaults() until 100 exist, scales.close_sale() until the tithe is told,
//                      and once the sale is closed a sweep pass over the unsold vaults (before that, buy_scale
//                      sweeps each vault as it mints)
//   every tick         up to 20 more vault balances of the sweep pass, and scales.sweep(ids) over those holding
//                      at least 0.05 NEAR
// A fresh keeper owes every step. A step is done for its slot once every call it planned was sent or refused as a
// no-op. A tick where one of its calls fails (an unknown refusal, a node that is down, a lost answer, a lost nonce
// race) is one attempt; the next tick plans the step again from what the chain holds then, and after MAX_TRIES
// attempts the step closes for its slot and logs the failure once, so an unknown refusal does not burn gas every
// tick. The sweep pass counts its attempts per slice and, closing, moves past the slice.
//
// A tick stays at or under 30 RPC requests, sends included, because the public RPC answers -429 under load. The
// nonces are one read per tick and each send is one request, so the vault reads get what is left, and a pass over
// up to 90 vaults spans several ticks. A pass that moves nothing once the tithe is told is the last: unsold Scales
// then need no keeper work.
//
// Several copies may run at once. Every action is safe to repeat: the contracts refuse a duplicate, a known refusal
// is logged as a no-op, and a nonce collision with another copy is a lost race the next tick retries.
import fs from "node:fs";
import { pathToFileURL } from "node:url";
import { accessKeys, connect, failedReceipts, failureOf, finalBlock, mapLimit, parseSecretKey, readNonces, sendFunctionCall, view, viewAccount } from "./chain.mjs";

const HELP = `The WYRM keeper pokes the Dragon, pushes creator fees, distributes, finalizes, closes the sale and sweeps.

  npm run keeper -- --network mainnet --root thewyrm.near [--once] [--dry-run] [--interval 20] [--verbose]
                    [--rpc URL] [--launch-id N | --symbol WYRM]

  --network       mainnet, testnet or sandbox (sandbox needs --rpc)
  --rpc URL       the RPC to read and send through, in place of the network's FastNEAR RPC
  --root          the system's root account
  --interval      seconds between ticks (default 20)
  --once          run one tick and exit. The sweep pass lives in memory, so --once reads only the first vaults of
                  a pass (at most 20) and never finishes it. Production runs the long-running loop.
  --dry-run       log the planned calls with their gas, and send nothing
  --verbose       log each RPC request to stderr
  --launch-id N   before bind, the launch to watch; else Nearly's get_launch_by_symbol(--symbol), default WYRM.
                  The keeper resumes it only when the Dragon created it, and reads Done from get_launch(id).step.

Credentials. WYRM_KEEPER_CREDENTIALS is the path to a JSON file outside the repo:
  { "account_id": "keeper.example.near",
    "keys": { "tithe.thewyrm.near": "ed25519:...", "dragon.thewyrm.near": "ed25519:...",
              "scales.thewyrm.near": "ed25519:...", "nearlytrade.near": "ed25519:..." } }
A function call access key names one receiver, so each receiver has its own key. "private_key" in place of "keys"
(the near-cli file shape) serves the one receiver its key names. A full access key stops the keeper before its
first tick: no call here attaches a deposit.`;

const TGAS = 10n ** 12n;
const NEAR = 10n ** 24n;
// docs/GAS.md. poke's floor is 760 Tgas; nearly_resume needs 300 because Nearly's resume panics below 222 at
// AddLiquidity and 235 at DevBuy.
export const GAS = {
  poke: 1_000n * TGAS, nearly_resume: 300n * TGAS, push_creator_fees: 50n * TGAS, unwrap: 100n * TGAS, forward_ft: 100n * TGAS,
  distribute: 300n * TGAS, finalize: 150n * TGAS, create_vaults: 300n * TGAS, close_sale: 50n * TGAS, sweep: 300n * TGAS,
};
export const SWEEP_MIN = NEAR / 20n;
export const SWEEP_CHUNK = 20;
export const VAULT_READS = 20;
export const TICK_REQUESTS = 30;
const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const FEES_AT = 15 * MINUTE;
const PARKED = (step) => step !== "Done" && step !== "Failed";

// Each step's slot index at a wall-clock time; a step is due once per slot.
const hourSlot = (t) => Math.floor((t - FEES_AT) / HOUR);
const SLOT = { minute: (t) => Math.floor(t / MINUTE), fees: hourSlot, hourly: hourSlot, daily: (t) => Math.floor(t / (24 * HOUR)) };
export const STEPS = Object.keys(SLOT);
export const MAX_TRIES = { minute: 3, fees: 1, hourly: 3, daily: 3, sweep: 3 };
export const newState = () => ({ lastRun: {}, sweep: null, tries: {} });
export const dueSteps = (nowMs, lastRun) => STEPS.filter((s) => lastRun[s] === undefined || SLOT[s](nowMs) > SLOT[s](lastRun[s]));

const pad = (i) => String(i).padStart(3, "0");
export function accountsFor(root) {
  const scales = `scales.${root}`;
  return { tithe: `tithe.${root}`, dragon: `dragon.${root}`, scales, vault: (id) => `${pad(id)}.${scales}` };
}

// The panic text each contract uses when a repeat finds nothing left to do. Matching one is a no-op, not a fault.
export const KNOWN_REFUSALS = {
  poke: /not bound|in flight/,
  nearly_resume: /already bound|nothing to resume|step in flight/,
  push_creator_fees: /nothing to push/,
  distribute: /below min_round/,
  unwrap: /nothing to unwrap/,
  forward_ft: /nothing to forward/,
  finalize: /not open|voting still open/,
  create_vaults: /all 100 vaults exist/,
  sweep: /is minted/,
};

// What is due in `steps`, from a snapshot read at one final block. Pure.
//   snapshot: { coin, peers, launch: { id, creator, step, inflight } | null,
//               tithe: { wnear, wyrm: BigInt }, due: [id], vaultsCreated, sale: { state, tithe_told },
//               sweepable: [{ id, balance: BigInt }] (this tick's slice of the sweep pass) }
// Each action names its step, or "sweep" for the pass that runs every tick.
export function plan(snapshot, steps, accounts) {
  const actions = [];
  let step = "minute";
  const add = (receiver, method, args, why) => actions.push({ step, receiver, method, args, gas: GAS[method], why });
  const { coin, peers, launch } = snapshot;
  if (steps.includes("minute")) {
    if (coin) add(accounts.dragon, "poke", {}, "every minute while bound");
    else if (launch?.creator === accounts.dragon && PARKED(launch.step) && !launch.inflight) {
      add(accounts.dragon, "nearly_resume", { launch_id: launch.id }, `launch ${launch.id} is parked at ${launch.step}`);
    }
  }
  step = "fees";
  if (steps.includes("fees")) add(peers.nearly, "push_creator_fees", { account_id: accounts.tithe }, "hourly from :15, before distribute");
  step = "hourly";
  if (steps.includes("hourly")) {
    if (snapshot.tithe.wnear > 0n) add(accounts.tithe, "unwrap", {}, `tithe holds ${snapshot.tithe.wnear} wNEAR`);
    if (snapshot.tithe.wyrm > 0n) add(accounts.tithe, "forward_ft", { token: coin.token }, `tithe holds ${snapshot.tithe.wyrm} of ${coin.token}`);
    add(accounts.tithe, "distribute", {}, "hourly from :15");
    for (const id of snapshot.due) add(accounts.scales, "finalize", { proposal_id: id }, `proposal ${id} is due`);
  }
  step = "daily";
  if (steps.includes("daily")) {
    if (snapshot.vaultsCreated < 100) add(accounts.scales, "create_vaults", {}, `${snapshot.vaultsCreated} of 100 vaults exist`);
    if (snapshot.sale.state !== "Waiting" && !snapshot.sale.tithe_told) add(accounts.scales, "close_sale", {}, `sale ${snapshot.sale.state}, tithe not told`);
  }
  step = "sweep";
  const full = snapshot.sweepable.filter((v) => v.balance >= SWEEP_MIN).map((v) => v.id);
  for (let i = 0; i < full.length; i += SWEEP_CHUNK) {
    const ids = full.slice(i, i + SWEEP_CHUNK);
    add(accounts.scales, "sweep", { token_ids: ids }, `${ids.length} unsold vaults hold at least 0.05 NEAR`);
  }
  return actions;
}

// The launch the keeper watches before bind: --launch-id, or the newest with --symbol. null when there is none.
async function readLaunch(q, nearly, { launchId, symbol }) {
  const record = launchId === undefined ? await q(nearly, "get_launch_by_symbol", { symbol }) : await q(nearly, "get_launch", { launch_id: String(launchId) });
  return record && { id: String(record.id), creator: record.creator, step: record.step, inflight: record.inflight };
}

// state.sweep is a pass (the unsold ids, how many are read and swept, whether the tithe was told when it began), null
// between passes, or "done" once the scan has ended.
const startPass = (ids, titheTold) => ({ ids, next: 0, found: 0, titheTold });
export function advancePass(pass, read, found) {
  const next = { ...pass, next: pass.next + read, found: pass.found + found };
  if (next.next < pass.ids.length) return next;
  return pass.titheTold && next.found === 0 ? "done" : null;
}

// Also returns the final block (`at`, `hash`) and `pass`, the sweep pass this tick reads from: state.sweep, or a new
// one the daily step starts. `sweepable` stays empty until readVaults fills it.
export async function readSnapshot(rpc, accounts, steps, watch = { symbol: "WYRM" }, sweep = null) {
  const { height: at, hash } = await finalBlock(rpc);
  const q = (account, method, args = {}) => view(rpc, account, method, args, at);
  const { coin, peers } = await q(accounts.dragon, "get_config");
  const snapshot = { at, hash, coin, peers, launch: null, sweepable: [] };
  if (steps.includes("minute") && !coin) snapshot.launch = await readLaunch(q, peers.nearly, watch);
  if (steps.includes("hourly")) {
    const [wnear, wyrm, due] = await Promise.all([
      q(peers.wrap, "ft_balance_of", { account_id: accounts.tithe }),
      coin ? q(coin.token, "ft_balance_of", { account_id: accounts.tithe }) : "0",
      q(accounts.scales, "get_due"),
    ]);
    Object.assign(snapshot, { tithe: { wnear: BigInt(wnear), wyrm: BigInt(wyrm) }, due });
  }
  if (steps.includes("daily")) {
    const config = await q(accounts.scales, "get_config");
    Object.assign(snapshot, { vaultsCreated: config.vaults_created, sale: config.sale });
    if (config.sale.state === "Closed" && sweep === null) sweep = startPass(await q(accounts.scales, "get_sweepable"), config.sale.tithe_told);
  }
  snapshot.pass = sweep === "done" ? null : sweep;
  return snapshot;
}

export async function readVaults(rpc, accounts, { pass, at }, count) {
  const ids = pass ? pass.ids.slice(pass.next, pass.next + count) : [];
  const views = await mapLimit(ids, 4, (id) => viewAccount(rpc, accounts.vault(Number(id)), at));
  return ids.map((id, i) => ({ id, balance: BigInt(views[i]?.amount ?? 0) }));
}

// Classifies a send: "sent", "noop" (a known refusal), "race" (nonce taken by another copy) or "failed".
export function classify(action, { outcome, error }) {
  if (error) return /InvalidNonce|NonceTooLarge/.test(error) ? "race" : "failed";
  const failure = failureOf(outcome);
  if (!failure) return "sent";
  return KNOWN_REFUSALS[action.method]?.test(failure) ? "noop" : "failed";
}

export function readCredentials(path) {
  if (!path) throw new Error("WYRM_KEEPER_CREDENTIALS is not set: point it at the keeper's credentials JSON (see the top of keeper.mjs)");
  if (process.platform !== "win32" && fs.statSync(path).mode & 0o077) throw new Error(`${path} can be read by group or others; run chmod 600 on it`);
  const file = JSON.parse(fs.readFileSync(path, "utf8"));
  if (!file.account_id) throw new Error(`${path} has no account_id`);
  if (!file.keys && !file.private_key) throw new Error(`${path} has neither keys nor private_key`);
  return { account: file.account_id, keys: file.keys ?? {}, fallback: file.private_key };
}

// Reads the account's keys once, at startup, and returns the signer the sends use:
// { account, keys: { receiver: { secret, publicKeyText, methods } } }. Every key must be a function call key of the
// account, listed under the receiver it names.
export async function checkCredentials(rpc, credentials) {
  const onChain = (await accessKeys(rpc, credentials.account)) ?? [];
  const scoped = (secret) => {
    const { publicKeyText } = parseSecretKey(secret);
    const key = onChain.find((k) => k.public_key === publicKeyText);
    if (!key) throw new Error(`${publicKeyText} is not a key of ${credentials.account}`);
    if (key.access_key.permission === "FullAccess") throw new Error(`${publicKeyText} is a full access key of ${credentials.account}; the keeper runs on function call keys only`);
    const { receiver_id, method_names } = key.access_key.permission.FunctionCall;
    return { secret, publicKeyText, receiver: receiver_id, methods: method_names };
  };
  const keys = {};
  if (credentials.fallback) {
    const key = scoped(credentials.fallback);
    keys[key.receiver] = key;
  }
  for (const [receiver, secret] of Object.entries(credentials.keys)) {
    const key = scoped(secret);
    if (key.receiver !== receiver) throw new Error(`${key.publicKeyText} may call ${key.receiver}, not ${receiver}`);
    keys[receiver] = key;
  }
  return { account: credentials.account, keys };
}

export function signingKey(signer, action) {
  const key = signer.keys[action.receiver];
  if (!key) throw new Error(`no key for ${action.receiver}`);
  if (key.methods.length && !key.methods.includes(action.method)) throw new Error(`${key.publicKeyText} may not call ${action.method}`);
  return key;
}

const replacer = (_, v) => (typeof v === "bigint" ? v.toString() : v);
export const log = (event, fields = {}) => console.log(JSON.stringify({ ts: new Date().toISOString(), event, ...fields }, replacer));

const launchNote = (launch, dragon) => {
  if (!launch) return "no launch found; pass --launch-id";
  if (launch.creator !== dragon) return `created by ${launch.creator}, not the dragon; pass --launch-id`;
  if (launch.inflight) return "a step is in flight; wait";
  if (launch.step === "Done") return "Done; the creator binds it";
  if (launch.step === "Failed") return "Failed and refunded; the operator decides";
  return "parked; resume";
};

// `requests` in the tick log counts every RPC request of the tick, sends included. Calls past the cap are deferred,
// and their step stays due.
export async function tick({ rpc, accounts, signer, dryRun, state, watch, send = sendFunctionCall, now = Date.now() }) {
  const before = rpc.requests;
  const used = () => rpc.requests - before;
  const steps = dueSteps(now, state.lastRun);
  const snapshot = await readSnapshot(rpc, accounts, steps, watch, state.sweep);
  if (steps.includes("minute") && !snapshot.coin) log("unbound", { launch: snapshot.launch, note: launchNote(snapshot.launch, accounts.dragon) });
  const sendCost = (calls) => (calls ? 1 + calls : 0);
  const room = TICK_REQUESTS - used() - sendCost(plan(snapshot, steps, accounts).length + 1);
  snapshot.sweepable = await readVaults(rpc, accounts, snapshot, Math.max(0, Math.min(VAULT_READS, room)));
  const planned = plan(snapshot, steps, accounts);
  const actions = planned.slice(0, Math.max(0, TICK_REQUESTS - used() - 1));
  const nonces = dryRun || !actions.length ? null : await readNonces(rpc, signer.account);
  const done = new Set();
  const failures = new Map();
  for (const action of actions) {
    const call = `${action.receiver}.${action.method}`;
    const tgas = Number(action.gas / TGAS);
    if (dryRun) {
      log("would_send", { call, args: action.args, tgas, why: action.why });
      done.add(action);
      continue;
    }
    let result;
    try {
      const key = signingKey(signer, action);
      if (!nonces.has(key.publicKeyText)) throw new Error(`${key.publicKeyText} is no longer a key of ${signer.account}`);
      const nonce = nonces.get(key.publicKeyText) + 1n;
      nonces.set(key.publicKeyText, nonce);
      result = { outcome: await send(rpc, { ...action, signer: signer.account, secret: key.secret, nonce, blockHash: snapshot.hash }) };
    } catch (e) {
      result = { error: e.message };
    }
    const kind = classify(action, result);
    const failure = kind === "sent" ? undefined : result.error ?? failureOf(result.outcome);
    if (kind === "sent" || kind === "noop") done.add(action);
    else failures.set(action.step, failure);
    log(kind, {
      call, args: action.args, tgas, why: action.why, tx: result.outcome?.transaction?.hash ?? result.outcome?.transaction_outcome?.id,
      failure,
      failed_receipts: result.outcome ? failedReceipts(result.outcome) : undefined,
    });
  }
  const finished = (step) => planned.every((a) => a.step !== step || done.has(a));
  const slotOf = (step) => (step === "sweep" ? snapshot.pass.next : SLOT[step](now));
  const closes = (step) => {
    if (finished(step)) return true;
    if (!failures.has(step)) return false;
    const tries = (state.tries[step]?.slot === slotOf(step) ? state.tries[step].n : 0) + 1;
    if (tries < MAX_TRIES[step]) {
      state.tries[step] = { slot: slotOf(step), n: tries };
      return false;
    }
    log("gave_up", { step, tries, failure: failures.get(step) });
    return true;
  };
  for (const s of steps) {
    if (!closes(s)) continue;
    state.lastRun[s] = now;
    delete state.tries[s];
  }
  if (snapshot.pass) {
    const swept = planned.filter((a) => a.step === "sweep").flatMap((a) => a.args.token_ids).length;
    state.sweep = closes("sweep") ? advancePass(snapshot.pass, snapshot.sweepable.length, swept) : snapshot.pass;
    if (state.sweep !== snapshot.pass) delete state.tries.sweep;
  }
  const scan = state.sweep === "done" ? "done" : state.sweep && `${state.sweep.next} of ${state.sweep.ids.length} vaults read`;
  const deferred = planned.length - actions.length;
  log("tick", { steps, actions: actions.length, deferred: deferred || undefined, sweep_scan: scan ?? undefined, requests: used() });
  return actions;
}

export function options(argv) {
  const opts = { network: "mainnet", interval: "20", symbol: "WYRM" };
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i].startsWith("--")) continue;
    const name = argv[i].slice(2);
    opts[name] = ["once", "dry-run", "verbose"].includes(name) ? true : argv[++i];
  }
  if (!opts.root) throw new Error("--root is required, for example --root thewyrm.near");
  return opts;
}

async function main() {
  if (process.argv.includes("--help")) return console.log(HELP);
  const opts = options(process.argv.slice(2));
  const { rpc } = connect(opts);
  const accounts = accountsFor(opts.root);
  const dryRun = Boolean(opts["dry-run"]);
  const credentials = dryRun ? null : readCredentials(process.env.WYRM_KEEPER_CREDENTIALS);
  log("start", { network: opts.network, root: opts.root, dry_run: dryRun, signer: credentials?.account });
  const signer = credentials && (await checkCredentials(rpc, credentials));
  const state = newState();
  const watch = { launchId: opts["launch-id"], symbol: opts.symbol };
  for (;;) {
    try {
      await tick({ rpc, accounts, signer, dryRun, state, watch });
    } catch (e) {
      log("tick_error", { error: e.message });
      if (opts.once) process.exit(1);
    }
    if (opts.once) return;
    await new Promise((r) => setTimeout(r, Number(opts.interval) * 1000));
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((e) => { log("fatal", { error: e.message }); process.exit(1); });
}
