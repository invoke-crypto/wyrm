// Checks WYRM's promises against the chain. One PASS or FAIL line per check, with its evidence; exits 1 on any FAIL.
//
//   npm run verify -- --network mainnet --root thewyrm.near [--only 1,3,4] [--verbose] [--rpc URL] [--indexer URL]
//                     [--build DIR] [--releases PATH] [--record PATH | --replay PATH]
//
// Reads go to the RPC and, on mainnet and testnet, the FastNEAR transactions API (no API key). A sandbox has no
// indexer, so its history is walked from the RPC. Code must match a release in verify/releases.json; on sandbox and
// testnet, --build <contracts/target/near> adds the local build as an unreleased one. --verbose also logs every RPC
// request to stderr. --record writes every RPC answer to a file that --replay answers from.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { b58encode, connect, recording, replaying, view, viewAccount } from "../keeper/chain.mjs";
import { CHECKS, runCheck, wyrmAccounts } from "./checks.mjs";
import { accountReceipts, chainReceipts, fastnear, touching } from "./history.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const CRATES = ["tithe", "dragon", "scales", "scale_vault"];

export function options(argv) {
  const opts = { network: "mainnet" };
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i].startsWith("--")) continue;
    const name = argv[i].slice(2);
    opts[name] = name === "verbose" ? true : argv[++i];
  }
  if (!opts.root) throw new Error("--root is required, for example --root thewyrm.near");
  if (opts.build && opts.network === "mainnet") throw new Error("--build is for sandbox and testnet; mainnet code must be a release in verify/releases.json");
  return opts;
}

const sha256b58 = (file) => b58encode(crypto.createHash("sha256").update(fs.readFileSync(file)).digest());

// The releases code may match: verify/releases.json, plus the local build named by --build as "local build".
export function loadReleases(opts) {
  const table = JSON.parse(fs.readFileSync(opts.releases ?? path.join(HERE, "releases.json"), "utf8"));
  const releases = [...table.releases];
  if (opts.build) {
    const code = {};
    for (const c of CRATES) code[c] = sha256b58(path.join(opts.build, c, `${c}.wasm`));
    releases.push({ name: "local build", ...table.source, code });
  }
  return releases;
}

export async function context(opts) {
  const { net, rpc: live } = opts.replay ? { net: {}, rpc: replaying(JSON.parse(fs.readFileSync(opts.replay, "utf8"))) } : connect(opts);
  const rpc = opts.record ? recording(live) : live;
  const accounts = wyrmAccounts(opts.root);
  const releases = loadReleases(opts);
  const cache = new Map();
  const memo = (key, read) => {
    if (!cache.has(key)) cache.set(key, read());
    return cache.get(key);
  };
  const indexer = opts.indexer ?? net.indexer;
  // An empty history must not read as "no violations", so a missing account fails the read.
  const history = (account) => memo(`history ${account}`, async () => {
    if ((await viewAccount(rpc, account)) === null) throw new Error(`${account} does not exist`);
    if (indexer) return accountReceipts(fastnear(indexer), account);
    return touching(await memo("chain", () => chainReceipts(rpc)), account);
  });
  return {
    rpc, accounts, opts, cache, history, releases: () => releases,
    config: () => memo("config", () => view(rpc, accounts.dragon, "get_config")),
  };
}

export function render(result, verbose) {
  const lines = [`${result.pass ? "PASS" : "FAIL"}  ${String(result.id).padStart(2)}  ${result.title}: ${result.summary}`];
  const shown = verbose ? result.items : result.items.filter((i) => !i.pass).slice(0, 10);
  for (const i of shown) lines.push(`          ${i.pass ? "ok " : "bad"} ${i.evidence}`);
  const hidden = result.items.filter((i) => !i.pass).length - shown.filter((i) => !i.pass).length;
  if (hidden > 0) lines.push(`          ... and ${hidden} more failures (--verbose shows all)`);
  return lines.join("\n");
}

async function main() {
  const started = Date.now();
  const opts = options(process.argv.slice(2));
  const ctx = await context(opts);
  const only = opts.only ? new Set(opts.only.split(",").map(Number)) : null;
  let failed = 0;
  console.log(`verify ${opts.network} root ${opts.root}${opts.replay ? ` (replay of ${opts.replay})` : ""}`);
  for (const check of CHECKS) {
    if (only && !only.has(check.id)) continue;
    const result = await runCheck(check, ctx);
    if (!result.pass) failed++;
    console.log(render(result, opts.verbose));
  }
  if (opts.record) fs.writeFileSync(opts.record, JSON.stringify(ctx.rpc.log));
  console.log(`${failed ? `${failed} check(s) FAILED` : "all checks PASSED"}; ${ctx.rpc.requests} RPC requests in ${((Date.now() - started) / 1000).toFixed(1)} s`);
  process.exit(failed ? 1 : 0);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((e) => { console.error(e.message); process.exit(2); });
}
