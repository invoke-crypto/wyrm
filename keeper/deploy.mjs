// Deploys a WYRM system as design.md section 1a lays it out, one named step at a time. Each step reads the chain first
// and sends only what is missing, so a rerun skips finished work and resumes an interrupted one. Without --send it is
// a dry run: it prints the first transactions it would send, then stops.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { b58decode, b58encode, connect, failedReceipts, failureOf, finalBlock, isUnknownAccount, mapLimit, parseSecretKey, view, viewAccount } from "./chain.mjs";
import { accountsFor } from "./keeper.mjs";

const HELP = `Deploys, launches, binds and tears down a WYRM system.

  npm run deploy -- --network testnet --root rehearsal.thewyrm.testnet [--preset hoarder] [--step a,b] [--send]

  --network      mainnet, testnet or sandbox (sandbox needs --rpc)
  --rpc URL[,URL]  the RPCs to read and send through, in place of the network's list; a rate limit moves to the next
  --root         the system's root account. It is the creator: it signs launch and bind and gets the creator Scales.
                 deploy creates it under the signer when it is missing.
  --preset       hoarder (default), steady, dip-hunter, swift, harvester, or drill (testnet only)
  --step         the steps to run, comma-separated. The default runs accounts to bind, skipping delete-keys.
  --send         send the transactions. Without it nothing is signed.
  --operator-go  required with --network mainnet --send
  --mocks ACCT   testnet and sandbox: the account holding wrap, dcl and nearly (testnet default mocks.thewyrm.testnet)
  --wasm-dir DIR the cargo-near output (default contracts/target/near)
  --coin FILE    the coin as dragon.launch takes it: { name, symbol, icon, description, links }. Required on mainnet.
  --symbol S     testnet and sandbox, without --coin: the coin's name and symbol (default WYRM)
  --collection FILE  the Scale collection as scales.new takes it: { name, symbol, icon, tier_media }. Required on mainnet.
  --launch-id N  the Dragon's launch to resume or bind. A run that sends the launch reads the id from its
                 launch_started event. A resumed run otherwise looks up the first launch of the symbol on Nearly,
                 which may be anyone's, and stops unless the Dragon created it.
  --relaunch     launch again although the Dragon launched before and no launch of its own is found

Steps:
  accounts            the root, tithe, dragon and scales (each created, funded, keyed, deployed and initialized in one
                      batch) and ash (keyed, no code)
  publish-vault-code  the vault code as global code, unless RPC already returns code for its hash (once per network)
  create-vaults       scales.create_vaults until 100 vaults exist
  check-wiring        reads the three get_config views and compares them the way dragon.launch will
  delete-keys         deletes every key of tithe, dragon, scales and ash, after check-wiring passes. Runs only alone.
  launch              dragon.launch with Nearly's quote plus 0.1 NEAR, then nearly_resume until get_launch says Done
  bind                dragon.bind, which opens the Scale sale
  mocks               testnet and sandbox only: wrap, dcl and nearly under --mocks. Runs only alone.
  teardown            releases what the vaults hold, unwraps wNEAR and deletes tithe, dragon, scales, ash and the root,
                      with the signer as beneficiary. When the chain refuses to delete an account for its large state,
                      the account gets contracts/state-cleanup, its state is wiped and the deletion is sent again.
                      The 100 vaults have no keys and cannot be deleted. Runs only alone.

Credentials. WYRM_DEPLOY_CREDENTIALS is the path to a JSON file outside the repo, in the near-cli shape:
  { "account_id": "thewyrm.near", "private_key": "ed25519:..." }
The key must be a full access key of that account. Every account deploy creates gets the same key.`;

export const STEPS = ["accounts", "publish-vault-code", "create-vaults", "check-wiring", "delete-keys", "launch", "bind"];
export const ALONE = ["delete-keys", "mocks", "teardown"];
const DEFAULT_STEPS = STEPS.filter((s) => !ALONE.includes(s));
const ALL = [...STEPS, "mocks", "teardown"];

const NEAR = 10n ** 24n;
const MILLINEAR = NEAR / 1_000n;
const TGAS = 10n ** 12n;
const STORAGE_BYTE = 10n ** 19n;
const GLOBAL_CODE_BYTE = 10n ** 20n;
const VAULT_RESERVE = 10n * MILLINEAR;
// Older protocols refused DeleteAccount above 10 KB of storage. Testnet at protocol 88 deleted a 100 KB account in
// V9, and mainnet's protocol is inferred, not read, so teardown tries the deletion and wipes only on this refusal.
const LARGE_STATE = /DeleteAccountWithLargeState/;
const NO_CODE = "11111111111111111111111111111111";

export const MAINNET_PEERS = { wrap: "wrap.near", dcl: "dclv2.ref-labs.near", nearly: "nearlytrade.near" };
const TESTNET_MOCKS = "mocks.thewyrm.testnet";
const POOL_NEAR = 1_000n * NEAR;
const DCL_WNEAR = 1_000_000n * NEAR;

// NEAR each account gets beyond its code's storage. scales pays 0.01 NEAR into each of its 100 vaults.
const EXTRA = { tithe: 50n * MILLINEAR, dragon: 300n * MILLINEAR, scales: 1_600n * MILLINEAR, ash: 10n * MILLINEAR, wrap: 300n * MILLINEAR, dcl: 300n * MILLINEAR, nearly: 500n * MILLINEAR };
const ROOT_EXTRA = 500n * MILLINEAR;
const LAUNCH_MARGIN = 100n * MILLINEAR;
// The signer must hold the attached gas at the pessimistic gas price before a transaction is accepted.
const PREPAID_GAS = NEAR;

// docs/GAS.md.
export const GAS = {
  init: 300n * TGAS, publish: 300n * TGAS, create_vaults: 300n * TGAS, launch: 500n * TGAS, nearly_resume: 300n * TGAS,
  bind: 150n * TGAS, release: 10n * TGAS, near_withdraw: 30n * TGAS, clean: 300n * TGAS, mock: 100n * TGAS,
};
export const CLEAN_CHUNK = 1_000;

export function options(argv) {
  const opts = {};
  const flags = new Set(["send", "operator-go", "relaunch", "verbose", "help"]);
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i].startsWith("--")) throw new Error(`unexpected argument ${argv[i]}`);
    const name = argv[i].slice(2);
    opts[name] = flags.has(name) ? true : argv[++i];
    if (opts[name] === undefined) throw new Error(`--${name} needs a value`);
  }
  if (opts.help) return opts;
  if (!opts.network) throw new Error("--network is required: mainnet, testnet or sandbox");
  if (!opts.root) throw new Error("--root is required, for example --root thewyrm.near");
  if (opts.network === "mainnet" && opts.send && !opts["operator-go"]) {
    throw new Error("refusing to sign on mainnet without --operator-go");
  }
  opts.steps = opts.step ? opts.step.split(",") : DEFAULT_STEPS;
  for (const s of opts.steps) if (!ALL.includes(s)) throw new Error(`no step named ${s}; the steps are ${ALL.join(", ")}`);
  for (const s of ALONE) {
    if (opts.steps.includes(s) && opts.steps.length > 1) throw new Error(`${s} runs only when named alone: --step ${s}`);
  }
  if (opts.network === "mainnet" && opts.steps.includes("mocks")) throw new Error("the mocks are never deployed on mainnet");
  opts.preset ??= "hoarder";
  return opts;
}

export function peersFor(opts) {
  if (opts.network === "mainnet") return MAINNET_PEERS;
  const mocks = opts.mocks ?? (opts.network === "testnet" ? TESTNET_MOCKS : null);
  if (!mocks) throw new Error("--network sandbox needs --mocks <account>");
  return { mocks, wrap: `wrap.${mocks}`, dcl: `dcl.${mocks}`, nearly: `nearly.${mocks}` };
}

export function systemAccounts(root) {
  return { root, ...accountsFor(root), ash: `ash.${root}` };
}

// Borsh, as nearcore's Transaction V0 and Action enum lay it out.
const u8 = (n) => Buffer.from([n]);
const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const u64 = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };
const u128 = (n) => Buffer.concat([u64(BigInt(n) & ((1n << 64n) - 1n)), u64(BigInt(n) >> 64n)]);
const str = (s) => { const b = Buffer.from(s); return Buffer.concat([u32(b.length), b]); };
const ed25519 = (publicKey) => Buffer.concat([u8(0), publicKey]);

// An action is { type, ... }; `key` is the signer's public key, which a created account gets as its full access key.
export function encodeAction(a, key) {
  switch (a.type) {
    case "create": return u8(0);
    case "deploy": return Buffer.concat([u8(1), u32(a.code.length), a.code]);
    case "call": {
      const args = Buffer.isBuffer(a.args) ? a.args : Buffer.from(JSON.stringify(a.args));
      return Buffer.concat([u8(2), str(a.method), u32(args.length), args, u64(a.gas), u128(a.deposit ?? 0n)]);
    }
    case "transfer": return Buffer.concat([u8(3), u128(a.amount)]);
    case "add-key": return Buffer.concat([u8(5), ed25519(key), u64(0), u8(1)]);
    case "delete-key": return Buffer.concat([u8(6), ed25519(a.publicKey ? b58decode(a.publicKey.replace(/^ed25519:/, "")) : key)]);
    case "delete-account": return Buffer.concat([u8(7), str(a.beneficiary)]);
    default: throw new Error(`unknown action ${a.type}`);
  }
}

export function signTransaction({ signer, seed, publicKey, nonce, receiver, blockHash, actions }) {
  const body = actions.map((a) => encodeAction(a, publicKey));
  const tx = Buffer.concat([str(signer), ed25519(publicKey), u64(nonce), str(receiver), blockHash, u32(body.length), ...body]);
  const pkcs8 = Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]);
  const hash = crypto.createHash("sha256").update(tx).digest();
  const signature = crypto.sign(null, hash, crypto.createPrivateKey({ key: pkcs8, format: "der", type: "pkcs8" }));
  return { base64: Buffer.concat([tx, u8(0), signature]).toString("base64"), hash: b58encode(hash) };
}

const near = (yocto) => `${(Number(BigInt(yocto) / (MILLINEAR / 1000n)) / 1e6).toFixed(4)} NEAR`;
const short = (a) => {
  if (a.type === "deploy") return `deploy ${a.name} (${a.code.length} bytes)`;
  if (a.type === "call") return `${a.method}${a.summary ? `(${a.summary})` : ""}${a.deposit ? ` +${near(a.deposit)}` : ""} ${Number(a.gas / TGAS)} Tgas`;
  if (a.type === "transfer") return `transfer ${near(a.amount)}`;
  if (a.type === "delete-account") return `delete account to ${a.beneficiary}`;
  if (a.type === "delete-key") return `delete key ${a.publicKey ?? "(deploy key)"}`;
  return a.type;
};
export const describe = (tx) => `${tx.signer} -> ${tx.receiver}: ${tx.actions.map(short).join(", ")}`;

const call = (method, args, gas, deposit = 0n, summary) => ({ type: "call", method, args, gas, deposit, summary });
const codeHash = (code) => b58encode(crypto.createHash("sha256").update(code).digest());

function createAndInit(signer, account, { code, name, init, fund }) {
  const actions = [{ type: "create" }, { type: "transfer", amount: fund }, { type: "add-key" }];
  if (code) actions.push({ type: "deploy", code, name }, call("new", init, GAS.init));
  return { signer, receiver: account, actions };
}

export async function viewOrNull(rpc, account, method, args) {
  try {
    return await view(rpc, account, method, args);
  } catch (e) {
    if (isUnknownAccount(e) || /MethodNotFound|CodeDoesNotExist|wasm execution failed|not initialized|CompilationError/i.test(e.message)) return null;
    throw e;
  }
}

async function globalCodeExists(rpc, hash) {
  try {
    await rpc("query", { request_type: "view_global_contract_code", finality: "final", code_hash: hash });
    return true;
  } catch (e) {
    if (/never been observed|not found|NoGlobalContractCode|does not exist|UNKNOWN/i.test(e.message)) return false;
    throw e;
  }
}

// The Dragon's launch: --launch-id, or the id in launch_started from this run's own launch transaction. Only a
// resumed run falls back to get_launch_by_symbol, which answers the symbol's first launch on Nearly, anyone's.
async function launchOf(ctx) {
  const { rpc, peers, accounts, opts } = ctx;
  const id = opts["launch-id"] ?? ctx.launchId;
  if (id !== undefined) {
    const record = await view(rpc, peers.nearly, "get_launch", { launch_id: String(id) });
    if (record?.creator !== accounts.dragon) throw new Error(`launch ${id} is ${record ? `by ${record.creator}` : "unknown to Nearly"}, not by ${accounts.dragon}`);
    return record;
  }
  const state = await view(rpc, accounts.dragon, "get_state");
  if (Number(state.last_launch_at) === 0) return null;
  const symbol = ctx.coin().symbol;
  const record = await view(rpc, peers.nearly, "get_launch_by_symbol", { symbol });
  if (record?.creator === accounts.dragon) return record;
  if (opts.relaunch) return null;
  const first = record ? `launch ${record.id} by ${record.creator}` : "none";
  throw new Error(`the Dragon launched before, and the first ${symbol} launch on Nearly is ${first}; pass --launch-id N with the Dragon's launch, or --relaunch to launch again`);
}

export function launchStarted(outcome, dragon) {
  const started = eventsOf(outcome).filter((ev) => ev.event === "launch_started").map((ev) => ev.data?.[0] ?? ev.data).find((d) => d.creator === dragon);
  return started ? String(started.id) : undefined;
}

// Each step: plan(ctx) answers the transactions still owed, [] when the step is finished, or { wait } while a chain
// it started is still running. `note` is what a finished step prints.
export const STEP_PLANS = {
  async accounts(ctx) {
    const { rpc, accounts, signer } = ctx;
    const views = Object.fromEntries(await Promise.all(["root", "dragon", "tithe", "scales", "ash"].map(async (r) => [r, await viewAccount(rpc, accounts[r])])));
    const roles = ctx.roles();
    if (!views.root) {
      if (!accounts.root.endsWith(`.${signer}`) || accounts.root.slice(0, -signer.length - 1).includes(".")) {
        throw new Error(`${accounts.root} does not exist and is not a direct subaccount of ${signer}`);
      }
      const fund = Object.values(roles).reduce((sum, r) => sum + r.fund, ROOT_EXTRA);
      return [{ signer, receiver: accounts.root, actions: [{ type: "create" }, { type: "transfer", amount: fund }, { type: "add-key" }] }];
    }
    const txs = [];
    for (const [role, spec] of Object.entries(roles)) {
      const v = views[role];
      if (!v) txs.push(createAndInit(accounts.root, accounts[role], spec));
      else if (!spec.code) continue;
      else if (v.code_hash === NO_CODE) txs.push({ signer: accounts[role], receiver: accounts[role], actions: [{ type: "deploy", code: spec.code, name: spec.name }, call("new", spec.init, GAS.init)] });
      else if (v.code_hash !== codeHash(spec.code)) throw new Error(`${accounts[role]} runs code ${v.code_hash}, not this ${spec.name} build (${codeHash(spec.code)})`);
      else if (!(await viewOrNull(rpc, accounts[role], "get_config", {}))) txs.push({ signer: accounts[role], receiver: accounts[role], actions: [call("new", spec.init, GAS.init)] });
    }
    return txs;
  },

  async "publish-vault-code"(ctx) {
    const code = ctx.wasm("scale_vault");
    const hash = codeHash(code);
    const config = await view(ctx.rpc, ctx.accounts.scales, "get_config");
    if (config.vault_code_hash !== hash) throw new Error(`scales expects vault code ${config.vault_code_hash}, the build is ${hash}`);
    if (await globalCodeExists(ctx.rpc, hash)) return { note: `vault code ${hash} is already global` };
    // Global code reaches every shard some blocks after its transaction, so a fresh publish is waited for, not repeated.
    if (ctx.published.has(hash)) return { wait: `vault code ${hash} is published and not yet visible` };
    ctx.published.add(hash);
    const deposit = BigInt(code.length) * GLOBAL_CODE_BYTE;
    return [{ signer: ctx.signer, receiver: ctx.accounts.scales, actions: [call("publish_vault_code", code, GAS.publish, deposit, `${code.length} bytes`)] }];
  },

  async "create-vaults"(ctx) {
    const { vaults_created } = await view(ctx.rpc, ctx.accounts.scales, "get_config");
    if (vaults_created >= 100) return { note: "100 of 100 vaults exist" };
    return [{ signer: ctx.signer, receiver: ctx.accounts.scales, actions: [call("create_vaults", {}, GAS.create_vaults, 0n, `${vaults_created} of 100 exist`)] }];
  },

  async "check-wiring"(ctx) {
    const configs = await readConfigs(ctx);
    const problems = checkWiring(configs, ctx.accounts);
    if (problems.length) throw new Error(`wiring: ${problems.join("; ")}`);
    return { note: `wiring ok: preset ${configs.dragon.preset} ${configs.dragon.preset_hash}, engine ${configs.dragon.engine_version}, 100 vaults` };
  },

  async "delete-keys"(ctx) {
    const problems = checkWiring(await readConfigs(ctx), ctx.accounts);
    if (problems.length) throw new Error(`check-wiring fails, so no key is deleted: ${problems.join("; ")}`);
    const txs = [];
    for (const role of ["tithe", "dragon", "scales", "ash"]) {
      const account = ctx.accounts[role];
      const keys = (await ctx.keys(account)).map((k) => k.public_key);
      if (!keys.length) continue;
      if (!keys.includes(ctx.key.publicKeyText)) throw new Error(`${account} holds keys, none of them the deploy key: ${keys.join(", ")}`);
      const others = keys.filter((k) => k !== ctx.key.publicKeyText).map((publicKey) => ({ type: "delete-key", publicKey }));
      txs.push({ signer: account, receiver: account, actions: [...others, { type: "delete-key" }] });
    }
    return txs.length ? txs : { note: "tithe, dragon, scales and ash hold no keys" };
  },

  async launch(ctx) {
    const { rpc, accounts, peers } = ctx;
    const coin = await view(rpc, accounts.dragon, "get_coin");
    if (coin) return { note: `bound to ${coin.token}, launch ${coin.launch_id}` };
    const record = await launchOf(ctx);
    if (record) {
      if (record.step === "Done") return { note: `launch ${record.id} is Done` };
      if (record.step === "Failed") throw new Error(`launch ${record.id} Failed and was refunded; the operator decides what comes next`);
      if (record.inflight) return { wait: `launch ${record.id} has a step in flight at ${record.step}` };
      return [{ signer: accounts.root, receiver: accounts.dragon, actions: [call("nearly_resume", { launch_id: String(record.id) }, GAS.nearly_resume, 0n, `launch ${record.id} parked at ${record.step}`)] }];
    }
    const args = ctx.coin();
    const quote = await view(rpc, peers.nearly, "quote_launch", { icon_bytes: Buffer.byteLength(args.icon ?? ""), dev_buy: null, tax: true });
    const deposit = BigInt(quote.total) + LAUNCH_MARGIN;
    const txs = [];
    if (accounts.root !== ctx.signer) {
      const root = await viewAccount(rpc, accounts.root);
      const free = BigInt(root.amount) - BigInt(root.storage_usage) * STORAGE_BYTE;
      const need = deposit + PREPAID_GAS;
      if (free < need) txs.push({ signer: ctx.signer, receiver: accounts.root, actions: [{ type: "transfer", amount: need - free }] });
    }
    txs.push({
      signer: accounts.root, receiver: accounts.dragon, actions: [call("launch", { coin: args }, GAS.launch, deposit, args.symbol)],
      after: (outcome) => { ctx.launchId = launchStarted(outcome, accounts.dragon); },
    });
    return txs;
  },

  async bind(ctx) {
    const { rpc, accounts } = ctx;
    const coin = await view(rpc, accounts.dragon, "get_coin");
    if (coin) return { note: `bound to ${coin.token}, launch ${coin.launch_id}, pool ${coin.pool_id}` };
    const record = await launchOf(ctx);
    if (!record) throw new Error("no launch by the Dragon was found; run the launch step");
    if (record.step !== "Done") throw new Error(`launch ${record.id} is at ${record.step}, not Done; run the launch step`);
    return [{ signer: accounts.root, receiver: accounts.dragon, actions: [call("bind", { launch_id: String(record.id) }, GAS.bind, 0n, `launch ${record.id}`)] }];
  },

  async mocks(ctx) {
    const { rpc, peers, signer } = ctx;
    const specs = {
      wrap: { code: ctx.wasm("mock_ft"), name: "mock_ft", init: { symbol: "wNEAR", decimals: 24 } },
      dcl: { code: ctx.wasm("mock_dcl"), name: "mock_dcl", init: {} },
      nearly: { code: ctx.wasm("mock_nearly"), name: "mock_nearly", init: { dcl: peers.dcl, wrap: peers.wrap, pool_near: String(POOL_NEAR) } },
    };
    const tokenCode = ctx.wasm("mock_ft");
    for (const [role, s] of Object.entries(specs)) s.fund = BigInt(s.code.length) * STORAGE_BYTE + EXTRA[role] + (role === "nearly" ? BigInt(tokenCode.length) * GLOBAL_CODE_BYTE : 0n);
    if (!(await viewAccount(rpc, peers.mocks))) {
      if (!peers.mocks.endsWith(`.${signer}`)) throw new Error(`${peers.mocks} does not exist and is not a subaccount of ${signer}`);
      const fund = Object.values(specs).reduce((sum, s) => sum + s.fund, ROOT_EXTRA);
      return [{ signer, receiver: peers.mocks, actions: [{ type: "create" }, { type: "transfer", amount: fund }, { type: "add-key" }] }];
    }
    const missing = [];
    for (const [role, s] of Object.entries(specs)) if (!(await viewAccount(rpc, peers[role]))) missing.push(createAndInit(peers.mocks, peers[role], s));
    if (missing.length) return missing;
    const txs = [];
    if (!(await globalCodeExists(rpc, codeHash(tokenCode)))) {
      if (ctx.published.has(codeHash(tokenCode))) return { wait: "the mock-ft code is published and not yet visible" };
      ctx.published.add(codeHash(tokenCode));
      txs.push({ signer: peers.mocks, receiver: peers.nearly, actions: [call("set_token_code", tokenCode, GAS.init, 0n, `${tokenCode.length} bytes`)] });
    }
    if (BigInt(await view(rpc, peers.wrap, "ft_balance_of", { account_id: peers.dcl })) === 0n) {
      txs.push({ signer: peers.mocks, receiver: peers.wrap, actions: [call("mint", { account_id: peers.dcl, amount: String(DCL_WNEAR) }, GAS.mock, 0n, "the pool's wNEAR inventory")] });
    }
    return txs.length ? txs : { note: `wrap, dcl and nearly run under ${peers.mocks}` };
  },

  teardown: (ctx) => teardownPlan(ctx),
};

async function readConfigs(ctx) {
  const { rpc, accounts } = ctx;
  const [tithe, scales, dragon] = await Promise.all(["tithe", "scales", "dragon"].map((r) => view(rpc, accounts[r], "get_config")));
  return { tithe, scales, dragon };
}

// What dragon.launch's on_wiring would refuse, and what verify reads: the three contracts name each other, run one
// preset and engine version, and scales has its 100 vaults. Pure.
export function checkWiring({ tithe, scales, dragon }, accounts) {
  const problems = [];
  const expect = (what, got, want) => { if (got !== want) problems.push(`${what} is ${got}, expected ${want}`); };
  const p = dragon.peers;
  expect("dragon.peers.tithe", p.tithe, accounts.tithe);
  expect("dragon.peers.scales", p.scales, accounts.scales);
  expect("dragon.peers.ash", p.ash, accounts.ash);
  expect("dragon.peers.creator", p.creator, accounts.root);
  expect("tithe.dragon", tithe.dragon, accounts.dragon);
  expect("tithe.scales", tithe.scales, accounts.scales);
  expect("tithe.wrap", tithe.wrap, p.wrap);
  expect("scales.dragon", scales.dragon, accounts.dragon);
  expect("scales.tithe", scales.tithe, accounts.tithe);
  expect("scales.creator", scales.creator, p.creator);
  expect("scales.preset_hash", scales.preset_hash, dragon.preset_hash);
  expect("scales.engine_version", scales.engine_version, dragon.engine_version);
  expect("scales.vaults_created", scales.vaults_created, 100);
  return problems;
}

const ring = (prefix, cap) => Array.from({ length: cap }, (_, i) => Buffer.concat([Buffer.from(prefix), u32(i)]));
const ids = Array.from({ length: 100 }, (_, i) => i + 1);
// view_state refuses an account holding over 50 KB of state, and the Dragon's rings alone reach about 330 KB. These
// are the keys of each contract's big collections (near_sdk store::Vector is prefix + u32 index, LookupMap is prefix
// + borsh key), removed first so view_state can list what is left.
export const BULK_KEYS = {
  dragon: () => [...ring("s", 1_440), ...ring("l", 256), ...ring("g", 32), ...ring("b", 32)],
  scales: () => [0, 1, 2, 4].flatMap((prefix) => ids.map((id) => Buffer.from([prefix, id]))),
  tithe: () => [],
};

// The keys `clean` takes: each as a little-endian u16 length, then its bytes (contracts/state-cleanup).
export function cleanArgs(keys) {
  return Buffer.concat(keys.flatMap((k) => { const len = Buffer.alloc(2); len.writeUInt16LE(k.length); return [len, k]; }));
}

async function listKeys(rpc, account) {
  try {
    const { values } = await rpc("query", { request_type: "view_state", finality: "final", account_id: account, prefix_base64: "" });
    return values.map((v) => Buffer.from(v.key, "base64"));
  } catch (e) {
    if (/too large/i.test(e.message)) return null;
    throw e;
  }
}

const cleanTxs = (account, keys) => {
  const txs = [];
  for (let i = 0; i < keys.length; i += CLEAN_CHUNK) {
    const chunk = keys.slice(i, i + CLEAN_CHUNK);
    txs.push({ signer: account, receiver: account, actions: [call("clean", cleanArgs(chunk), GAS.clean, 0n, `${chunk.length} keys`)] });
  }
  return txs;
};

// One teardown round: the next transactions for each account, read from the chain now. Within an account the
// actions run in order (vault releases from scales, then unwrap, then deletion), so each account owes at most one
// kind of transaction per round. The root goes last, once its children are gone. Only an account whose deletion the
// chain refused gets contracts/state-cleanup, a wipe and a second deletion.
export async function teardownPlan(ctx) {
  const { rpc, accounts, peers, signer } = ctx;
  if (accounts.root === signer) throw new Error(`teardown deletes the root, and ${accounts.root} is the signer; refusing`);
  const cleanup = ctx.wasm("state_cleanup");
  const rows = [];
  const txs = [];
  const scales = await viewAccount(rpc, accounts.scales);
  let releasing = 0;
  if (scales) {
    const vaults = await mapLimit(ids, 4, (id) => viewAccount(rpc, accounts.vault(id)));
    const full = ids.filter((id, i) => vaults[i] && BigInt(vaults[i].amount) > VAULT_RESERVE + MILLINEAR);
    const claimable = full.reduce((sum, id) => sum + BigInt(vaults[id - 1].amount) - VAULT_RESERVE, 0n);
    const kept = vaults.filter(Boolean).length;
    rows.push({ account: `${kept} vaults`, returns: claimable, next: full.length ? `release ${full.length}` : "nothing to release", stays: BigInt(kept) * VAULT_RESERVE });
    for (const id of full) txs.push({ signer: accounts.scales, receiver: accounts.vault(id), actions: [call("release", { to: signer }, GAS.release, 0n, `to ${signer}`)] });
    releasing = full.length;
  }
  const children = ["tithe", "dragon", "scales", "ash"];
  let alive = 0;
  for (const role of [...children, "root"]) {
    const account = accounts[role];
    const v = await viewAccount(rpc, account);
    if (!v) {
      rows.push({ account, returns: 0n, next: "gone" });
      continue;
    }
    if (role !== "root") alive++;
    const row = { account, returns: BigInt(v.amount), storage: v.storage_usage };
    rows.push(row);
    const hasKey = (await ctx.keys(account)).some((k) => k.public_key === ctx.key.publicKeyText);
    if (!hasKey) throw new Error(`${account} does not hold the deploy key, so it cannot be torn down`);
    const deletion = { signer: account, receiver: account, actions: [{ type: "delete-account", beneficiary: signer }] };
    const wnear = role === "tithe" || role === "dragon" ? BigInt((await viewOrNull(rpc, peers.wrap, "ft_balance_of", { account_id: account })) ?? 0) : 0n;
    row.returns += wnear;
    if (role === "scales" && releasing) row.next = "waits for the vault releases";
    else if (role === "root" && alive) row.next = "waits for its children";
    else if (wnear > 0n) {
      row.next = `unwrap ${near(wnear)}`;
      txs.push({ signer: account, receiver: peers.wrap, actions: [call("near_withdraw", { amount: String(wnear) }, GAS.near_withdraw, 1n)] });
    } else if (v.code_hash === codeHash(cleanup)) {
      const listed = await listKeys(rpc, account);
      if (listed?.length === 0) {
        row.next = "delete";
        txs.push(deletion);
      } else {
        if (!listed && ctx.bulkCleaned.has(role)) throw new Error(`${account} still holds over 50 KB of state after its known collections were wiped`);
        const keys = listed ?? (BULK_KEYS[role]?.() ?? []);
        if (!listed) ctx.bulkCleaned.add(role);
        if (!keys.length) throw new Error(`${account} holds over 50 KB of state that view_state will not list, and no known keys`);
        row.next = `wipe ${keys.length} ${listed ? "keys" : "known keys"}, delete`;
        txs.push(...cleanTxs(account, keys));
      }
    } else if (!ctx.deleteRefused.has(account)) {
      row.next = "delete, or wipe its state first if the chain refuses";
      const refused = (e) => {
        if (!LARGE_STATE.test(e.message)) return false;
        ctx.deleteRefused.add(account);
        return true;
      };
      txs.push({ ...deletion, refused });
    } else {
      row.next = "deletion refused: deploy state-cleanup, wipe, delete";
      txs.push({ signer: account, receiver: account, actions: [{ type: "deploy", code: cleanup, name: "state_cleanup" }] });
    }
  }
  const total = rows.reduce((sum, r) => sum + r.returns, 0n);
  return Object.assign(txs, { rows, total });
}

function printTeardown(out, plan, beneficiary) {
  for (const r of plan.rows) {
    const storage = r.storage === undefined ? "" : ` ${r.storage} B`;
    const stays = r.stays ? `, ${near(r.stays)} stays in keyless vaults that cannot be deleted` : "";
    out(`  ${r.account.padEnd(36)} ${near(r.returns).padStart(16)}${storage.padStart(10)}  ${r.next}${stays}`);
  }
  out(`  returns about ${near(plan.total)} to ${beneficiary}, before gas`);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const ROUNDS = { accounts: 3, "publish-vault-code": 40, "create-vaults": 6, launch: 60, teardown: 30, mocks: 40 };

// Runs one step to its end: plans, sends, and plans again from what the chain then holds, until nothing is owed.
// Answers "skipped" when the first plan owed nothing, "done" when it sent, "planned" on a dry run.
export async function runStep(name, ctx) {
  const plan = STEP_PLANS[name];
  let sent = 0;
  for (let round = 0; round < (ROUNDS[name] ?? 2); round++) {
    const owed = await plan(ctx);
    if (name === "teardown" && round === 0) printTeardown(ctx.out, owed, ctx.signer);
    if (owed.wait) {
      ctx.out(`  waiting: ${owed.wait}`);
      await sleep(ctx.pollMs);
      continue;
    }
    if (!Array.isArray(owed) || !owed.length) {
      ctx.out(`${name} ${sent ? "done" : "skipped"}${owed.note ? `: ${owed.note}` : ""}`);
      return sent ? "done" : "skipped";
    }
    if (!ctx.send) {
      for (const tx of owed) ctx.out(`  would send ${describe(tx)}`);
      ctx.out(`${name} planned: ${owed.length} transaction(s); dry run, add --send`);
      return "planned";
    }
    for (const tx of owed) {
      let outcome;
      try {
        outcome = await ctx.submit(tx);
      } catch (e) {
        if (!tx.refused?.(e)) throw e;
        ctx.out(`  refused: ${e.message}`);
        continue;
      }
      tx.after?.(outcome);
    }
    sent += owed.length;
  }
  throw new Error(`${name} still owes transactions after ${ROUNDS[name] ?? 2} rounds`);
}

const EVENT = /^EVENT_JSON:/;
export function eventsOf(outcome) {
  return (outcome?.receipts_outcome ?? []).flatMap((r) => r.outcome.logs).filter((l) => EVENT.test(l)).map((l) => JSON.parse(l.replace(EVENT, "")));
}
const burnt = (outcome) => [outcome.transaction_outcome, ...outcome.receipts_outcome].reduce((sum, r) => sum + BigInt(r.outcome.gas_burnt), 0n);

// The public RPC sometimes answers send_tx with a timeout and never forwards the transaction. The same signed bytes
// are safe to send again: one nonce lands at most once.
export async function deliver(rpc, signed, signer, what, tries = 4, waitMs = 5_000) {
  let last;
  for (let attempt = 0; attempt < tries; attempt++) {
    try {
      return await rpc("send_tx", { signed_tx_base64: signed.base64, wait_until: "FINAL" });
    } catch (e) {
      last = e;
      if (!/timeout/i.test(e.message)) break;
    }
    await sleep(waitMs);
    try {
      return await rpc("EXPERIMENTAL_tx_status", { tx_hash: signed.hash, sender_account_id: signer, wait_until: "FINAL" });
    } catch (e) {
      if (!/UNKNOWN_TRANSACTION|doesn't exist|timeout/i.test(e.message)) { last = e; break; }
    }
  }
  throw new Error(`${signed.hash} ${what}: ${last.message}; read the transaction before rerunning`);
}

// The signer for every account deploy touches. Each send reads that account's nonce for the deploy key, waits for
// the transaction to be final, and throws on any failed receipt, so a rerun starts from what really happened.
export function sender(rpc, key, out) {
  return async (tx) => {
    const { nonce } = await rpc("query", { request_type: "view_access_key", finality: "optimistic", account_id: tx.signer, public_key: key.publicKeyText });
    const { hash: blockHash } = await finalBlock(rpc);
    const signed = signTransaction({ ...tx, seed: key.seed, publicKey: key.publicKey, nonce: BigInt(nonce) + 1n, blockHash: b58decode(blockHash) });
    const outcome = await deliver(rpc, signed, tx.signer, describe(tx));
    const failure = failureOf(outcome);
    // A deleted account cannot take its own gas refund, so that refund receipt fails by design.
    const deleted = tx.actions.some((a) => a.type === "delete-account");
    const refundToDeleted = (r) => deleted && r.outcome.executor_id === tx.signer && JSON.stringify(r.outcome.status.Failure).includes("AccountDoesNotExist");
    const failed = failedReceipts(outcome).filter((id) => !refundToDeleted(outcome.receipts_outcome.find((r) => r.id === id)));
    out(`  sent ${signed.hash} ${describe(tx)} (burnt ${(Number(burnt(outcome)) / 1e12).toFixed(2)} Tgas)`);
    for (const ev of eventsOf(outcome)) {
      if (["launch_started", "launch_sent", "launch_refused", "bound", "bind_refused"].includes(ev.event)) out(`    event ${ev.event} ${JSON.stringify(ev.data?.[0] ?? ev.data)}`);
    }
    if (failure) throw new Error(`${signed.hash} failed: ${failure}`);
    if (failed.length) {
      const why = outcome.receipts_outcome.filter((r) => failed.includes(r.id)).map((r) => JSON.stringify(r.outcome.status.Failure));
      throw new Error(`${signed.hash}: receipts failed: ${why.join("; ")}`);
    }
    const refused = eventsOf(outcome).find((ev) => ev.event === "launch_refused" || ev.event === "bind_refused");
    if (refused) throw new Error(`${signed.hash}: ${refused.event} ${JSON.stringify(refused.data)}`);
    return outcome;
  };
}

const HERE = path.dirname(fileURLToPath(import.meta.url));

export function defaultCoin(symbol = "WYRM") {
  return { name: symbol, symbol, icon: null, description: "", links: {} };
}

export function defaultCollection(coinSymbol) {
  const media = ["ash", "ember", "molten", "obsidian"].map((t) => `https://wyrm.example/${t}.svg`);
  return { name: `${coinSymbol} Scales`, symbol: "SCALE", icon: null, tier_media: media };
}

export function makeContext(opts, { rpc, credentials, out = console.log }) {
  const key = parseSecretKey(credentials.private_key);
  const accounts = systemAccounts(opts.root);
  const peers = peersFor(opts);
  const dir = opts["wasm-dir"] ?? path.join(HERE, "..", "contracts", "target", "near");
  const wasmCache = new Map();
  const wasm = (crate) => {
    if (!wasmCache.has(crate)) wasmCache.set(crate, fs.readFileSync(path.join(dir, crate, `${crate}.wasm`)));
    return wasmCache.get(crate);
  };
  const readJson = (flag) => {
    if (opts[flag]) return JSON.parse(fs.readFileSync(opts[flag], "utf8"));
    if (opts.network === "mainnet") throw new Error(`--${flag} FILE is required on mainnet`);
    return null;
  };
  const coin = () => readJson("coin") ?? defaultCoin(opts.symbol);
  const ctx = {
    rpc, opts, peers, accounts, key, wasm, coin, out,
    signer: credentials.account_id,
    send: Boolean(opts.send),
    pollMs: 3_000,
    bulkCleaned: new Set(),
    deleteRefused: new Set(),
    published: new Set(),
    keys: async (account) => (await ctx.rpc("query", { request_type: "view_access_key_list", finality: "final", account_id: account })).keys,
    submit: sender(rpc, key, out),
    roles: () => {
      const vaultHash = codeHash(wasm("scale_vault"));
      const collection = readJson("collection") ?? defaultCollection(coin().symbol);
      const all = { ...peers, tithe: accounts.tithe, scales: accounts.scales, ash: accounts.ash, creator: accounts.root };
      const roles = {
        dragon: { name: "dragon", init: { preset: opts.preset, peers: { wrap: all.wrap, dcl: all.dcl, nearly: all.nearly, tithe: all.tithe, scales: all.scales, ash: all.ash, creator: all.creator } } },
        tithe: { name: "tithe", init: { dragon: accounts.dragon, scales: accounts.scales, wrap: peers.wrap, min_round: String(NEAR), reserve: "0" } },
        scales: { name: "scales", init: { preset: opts.preset, dragon: accounts.dragon, tithe: accounts.tithe, creator: accounts.root, vault_code_hash: vaultHash, collection } },
        ash: { name: "ash" },
      };
      for (const [role, r] of Object.entries(roles)) {
        r.code = role === "ash" ? null : wasm(r.name);
        r.fund = BigInt(r.code?.length ?? 0) * STORAGE_BYTE + EXTRA[role];
      }
      return roles;
    },
  };
  return ctx;
}

export function readDeployCredentials(file) {
  if (!file) throw new Error("WYRM_DEPLOY_CREDENTIALS is not set: point it at a { account_id, private_key } JSON file outside the repo");
  const c = JSON.parse(fs.readFileSync(file, "utf8"));
  if (!c.account_id || !c.private_key) throw new Error(`${file} needs account_id and private_key`);
  return c;
}

async function main() {
  const opts = options(process.argv.slice(2));
  if (opts.help) return console.log(HELP);
  const credentials = readDeployCredentials(process.env.WYRM_DEPLOY_CREDENTIALS);
  const { rpc } = connect({ network: opts.network, rpc: opts.rpc, verbose: opts.verbose });
  const ctx = makeContext(opts, { rpc, credentials });
  const balance = async () => near((await viewAccount(rpc, ctx.signer)).amount);
  ctx.out(`deploy ${opts.network} root ${opts.root} preset ${opts.preset} signer ${ctx.signer} ${ctx.send ? "SEND" : "dry run"}; signer holds ${await balance()}`);
  for (const name of opts.steps) {
    const status = await runStep(name, ctx);
    if (status === "planned") {
      const rest = opts.steps.slice(opts.steps.indexOf(name) + 1);
      if (rest.length) ctx.out(`not read yet: ${rest.join(", ")}, which follow ${name}`);
      break;
    }
  }
  ctx.out(`signer holds ${await balance()}`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((e) => { console.error(`deploy: ${e.message}`); process.exit(1); });
}
