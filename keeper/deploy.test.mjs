import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { ALONE, BULK_KEYS, STEPS, checkWiring, deliver, cleanArgs, encodeAction, makeContext, options, runStep, signTransaction, systemAccounts } from "./deploy.mjs";
import { b58encode, parseSecretKey, signedFunctionCall } from "./chain.mjs";

const NEAR = 10n ** 24n;
const TGAS = 10n ** 12n;
const HERE = path.dirname(fileURLToPath(import.meta.url));
const seed = Buffer.alloc(32, 7);
const pub = crypto.createPublicKey(crypto.createPrivateKey({ key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]), format: "der", type: "pkcs8" })).export({ format: "der", type: "spki" }).subarray(12);
const secret = `ed25519:${b58encode(Buffer.concat([seed, pub]))}`;
const PK = parseSecretKey(secret).publicKeyText;
const hash = (b) => b58encode(crypto.createHash("sha256").update(b).digest());

// Fake wasm files: each crate's code is its own name, so code hashes differ and stay stable.
const wasmDir = fs.mkdtempSync(path.join(os.tmpdir(), "deploy-test-"));
for (const crate of ["tithe", "dragon", "scales", "scale_vault", "state_cleanup", "mock_ft", "mock_dcl", "mock_nearly"]) {
  fs.mkdirSync(path.join(wasmDir, crate), { recursive: true });
  fs.writeFileSync(path.join(wasmDir, crate, `${crate}.wasm`), crate);
}
const code = (crate) => hash(Buffer.from(crate));

const ROOT = "rehearsal.thewyrm.testnet";
const A = systemAccounts(ROOT);
const MOCKS = { wrap: "wrap.mocks.thewyrm.testnet", dcl: "dcl.mocks.thewyrm.testnet", nearly: "nearly.mocks.thewyrm.testnet" };
const account = (codeHash = "11111111111111111111111111111111", amount = NEAR, storage = 500) => ({ amount: String(amount), code_hash: codeHash, storage_usage: storage });

// A chain answering the RPC requests deploy makes, from plain tables, and recording every send.
function chain({ accounts = {}, views = {}, keys = {}, state = {}, global = [] } = {}) {
  const world = { accounts, views, keys, state, global, sent: [] };
  world.rpc = async (method, params) => {
    if (method === "block") return { header: { height: 1, hash: b58encode(Buffer.alloc(32, 1)) } };
    if (method === "send_tx") throw new Error("the test sends through ctx.submit");
    const id = params.account_id;
    switch (params.request_type) {
      case "view_account":
        if (!accounts[id]) throw new Error(`account ${id} does not exist while viewing`);
        return accounts[id];
      case "call_function": {
        const answer = views[`${id}.${params.method_name}`];
        if (answer === undefined) throw new Error(`no view ${id}.${params.method_name}`);
        const args = JSON.parse(Buffer.from(params.args_base64, "base64").toString() || "{}");
        return { result: [...Buffer.from(JSON.stringify(typeof answer === "function" ? answer(args) : answer))] };
      }
      case "view_access_key_list": return { keys: (keys[id] ?? []).map((public_key) => ({ public_key, access_key: { nonce: 0, permission: "FullAccess" } })) };
      case "view_state":
        if (state[id] === "large") throw new Error(`query: State of contract ${id} is too large to be viewed`);
        return { values: (state[id] ?? []).map((k) => ({ key: Buffer.from(k).toString("base64"), value: "" })) };
      case "view_global_contract_code":
        if (!global.includes(params.code_hash)) throw new Error("query: global contract code not found");
        return {};
      default: throw new Error(`unexpected ${params.request_type}`);
    }
  };
  return world;
}

function context(world, flags = []) {
  const opts = options(["--network", "testnet", "--root", ROOT, "--wasm-dir", wasmDir, ...flags]);
  const lines = [];
  const ctx = makeContext(opts, { rpc: world.rpc, credentials: { account_id: "thewyrm.testnet", private_key: secret }, out: (l) => lines.push(l) });
  ctx.pollMs = 0;
  ctx.submit = async (tx) => { world.sent.push(tx); return world.onSend?.(tx) ?? {}; };
  return { ctx, lines };
}

const configs = () => ({
  tithe: { dragon: A.dragon, scales: A.scales, wrap: MOCKS.wrap },
  scales: { dragon: A.dragon, tithe: A.tithe, creator: ROOT, preset_hash: "H", engine_version: 200, vaults_created: 100, vault_code_hash: code("scale_vault") },
  dragon: { preset: "hoarder", preset_hash: "H", engine_version: 200, peers: { ...MOCKS, tithe: A.tithe, scales: A.scales, ash: A.ash, creator: ROOT } },
});
const deployed = (extra = {}) => ({
  accounts: {
    [ROOT]: account(), [A.tithe]: account(code("tithe")), [A.dragon]: account(code("dragon")), [A.scales]: account(code("scales")), [A.ash]: account(),
    ...extra.accounts,
  },
  views: {
    [`${A.tithe}.get_config`]: configs().tithe, [`${A.scales}.get_config`]: configs().scales, [`${A.dragon}.get_config`]: configs().dragon,
    ...extra.views,
  },
  keys: extra.keys ?? {},
  global: [code("scale_vault"), ...(extra.global ?? [])],
});

test("the steps run in design.md 1a's order, and delete-keys, mocks and teardown run only alone", () => {
  assert.deepEqual(STEPS, ["accounts", "publish-vault-code", "create-vaults", "check-wiring", "delete-keys", "launch", "bind"]);
  assert.deepEqual(ALONE, ["delete-keys", "mocks", "teardown"]);
  const byDefault = options(["--network", "testnet", "--root", ROOT]).steps;
  assert.deepEqual(byDefault, ["accounts", "publish-vault-code", "create-vaults", "check-wiring", "launch", "bind"]);
  for (const step of ALONE) {
    assert.ok(!byDefault.includes(step), `a default run never reaches ${step}`);
    assert.deepEqual(options(["--network", "testnet", "--root", ROOT, "--step", step]).steps, [step]);
    assert.throws(() => options(["--network", "testnet", "--root", ROOT, "--step", `check-wiring,${step}`]), /runs only when named alone/);
  }
  assert.throws(() => options(["--network", "testnet", "--root", ROOT, "--step", "deploy-everything"]), /no step named/);
});

test("delete-keys named alone deletes the deploy key of tithe, dragon, scales and ash", async () => {
  const world = chain(deployed({ keys: { [A.tithe]: [PK], [A.dragon]: [PK], [A.scales]: [PK], [A.ash]: [PK] } }));
  world.onSend = (tx) => { world.keys[tx.signer] = []; };
  const { ctx } = context(world, ["--send"]);
  assert.equal(await runStep("delete-keys", ctx), "done");
  assert.deepEqual(world.sent.map((t) => [t.signer, t.actions.map((a) => a.type)]), [A.tithe, A.dragon, A.scales, A.ash].map((a) => [a, ["delete-key"]]));
});

test("mainnet refuses --send without --operator-go, and the CLI exits before any RPC call", () => {
  assert.throws(() => options(["--network", "mainnet", "--root", "thewyrm.near", "--send"]), /refusing to sign on mainnet without --operator-go/);
  assert.equal(options(["--network", "mainnet", "--root", "thewyrm.near", "--send", "--operator-go"]).send, true);
  assert.equal(options(["--network", "mainnet", "--root", "thewyrm.near"]).send, undefined);
  assert.throws(() => options(["--network", "mainnet", "--root", "x.near", "--step", "mocks"]), /never deployed on mainnet/);
  const run = spawnSync(process.execPath, [path.join(HERE, "deploy.mjs"), "--network", "mainnet", "--root", "thewyrm.near", "--send", "--rpc", "http://127.0.0.1:9"], {
    encoding: "utf8", env: { ...process.env, WYRM_DEPLOY_CREDENTIALS: "" },
  });
  assert.equal(run.status, 1);
  assert.match(run.stderr, /refusing to sign on mainnet without --operator-go/);
  assert.equal(run.stdout, "");
});

test("a finished step is skipped and sends nothing, a rerun of the whole deploy included", async () => {
  const world = chain(deployed({ views: { [`${A.dragon}.get_coin`]: { token: "wyrm.nearly", launch_id: "4", pool_id: "p" } } }));
  const { ctx, lines } = context(world, ["--send"]);
  for (const step of ctx.opts.steps) assert.equal(await runStep(step, ctx), "skipped", step);
  assert.equal(world.sent.length, 0);
  assert.ok(lines.includes("publish-vault-code skipped: vault code " + code("scale_vault") + " is already global"));
  assert.ok(lines.some((l) => l.startsWith("check-wiring skipped: wiring ok")));
});

test("a missing root and system are created in rounds: the root first, then each account in one batch", async () => {
  const world = chain();
  world.onSend = (tx) => {
    const deploy = tx.actions.find((a) => a.type === "deploy");
    world.accounts[tx.receiver] = account(deploy && hash(deploy.code));
    world.views[`${tx.receiver}.get_config`] = {};
  };
  const { ctx } = context(world, ["--send"]);
  assert.equal(await runStep("accounts", ctx), "done");
  assert.deepEqual(world.sent.map((t) => [t.signer, t.receiver, t.actions.map((a) => a.type).join(" ")]), [
    ["thewyrm.testnet", ROOT, "create transfer add-key"],
    [ROOT, A.dragon, "create transfer add-key deploy call"],
    [ROOT, A.tithe, "create transfer add-key deploy call"],
    [ROOT, A.scales, "create transfer add-key deploy call"],
    [ROOT, A.ash, "create transfer add-key"],
  ]);
  const scales = world.sent[3].actions.at(-1);
  assert.equal(scales.args.vault_code_hash, code("scale_vault"));
  assert.equal(scales.args.creator, ROOT);
  const rootFund = world.sent[0].actions[1].amount;
  const children = world.sent.slice(1).reduce((sum, t) => sum + t.actions[1].amount, 0n);
  assert.ok(rootFund > children, "the root can fund its children");
  assert.equal(await runStep("accounts", ctx), "skipped");
});

test("an account running other code is never overwritten", async () => {
  const world = chain(deployed({ accounts: { [A.dragon]: account("SomeOtherCode") } }));
  const { ctx } = context(world, ["--send"]);
  await assert.rejects(runStep("accounts", ctx), /runs code SomeOtherCode, not this dragon build/);
  assert.equal(world.sent.length, 0);
});

test("create-vaults calls until 100 exist, re-reading the count after each batch", async () => {
  let created = 25;
  const world = chain(deployed({ views: { [`${A.scales}.get_config`]: () => ({ ...configs().scales, vaults_created: created }) } }));
  world.onSend = () => { created += 25; };
  const { ctx } = context(world, ["--send"]);
  assert.equal(await runStep("create-vaults", ctx), "done");
  assert.equal(world.sent.length, 3);
  assert.ok(world.sent.every((t) => t.signer === "thewyrm.testnet" && t.actions[0].method === "create_vaults" && t.actions[0].gas === 300n * TGAS));
});

test("a dry run prints what it would send and signs nothing", async () => {
  const world = chain({ ...deployed(), global: [] });
  const { ctx, lines } = context(world);
  assert.equal(await runStep("publish-vault-code", ctx), "planned");
  assert.equal(world.sent.length, 0);
  assert.match(lines[0], /would send thewyrm\.testnet -> scales\.rehearsal\.thewyrm\.testnet: publish_vault_code\(11 bytes\) \+0\.0011 NEAR 300 Tgas/);
});

test("check-wiring names what dragon.launch would refuse", () => {
  assert.deepEqual(checkWiring(configs(), A), []);
  const c = configs();
  c.scales.vaults_created = 75;
  c.tithe.dragon = "dragon.other.testnet";
  c.scales.preset_hash = "G";
  assert.deepEqual(checkWiring(c, A), [
    `tithe.dragon is dragon.other.testnet, expected ${A.dragon}`,
    "scales.preset_hash is G, expected H",
    "scales.vaults_created is 75, expected 100",
  ]);
});

test("delete-keys refuses while check-wiring fails, and deletes every key with the deploy key last", async () => {
  const bad = chain(deployed({ views: { [`${A.scales}.get_config`]: { ...configs().scales, vaults_created: 50 } }, keys: { [A.tithe]: [PK] } }));
  await assert.rejects(runStep("delete-keys", context(bad, ["--send"]).ctx), /check-wiring fails, so no key is deleted/);
  assert.equal(bad.sent.length, 0);
  const world = chain(deployed({ keys: { [A.tithe]: ["ed25519:Other", PK] } }));
  world.onSend = (tx) => { world.keys[tx.signer] = []; };
  const { ctx } = context(world, ["--send"]);
  assert.equal(await runStep("delete-keys", ctx), "done");
  assert.deepEqual(world.sent[0].actions, [{ type: "delete-key", publicKey: "ed25519:Other" }, { type: "delete-key" }]);
  const stranger = chain(deployed({ keys: { [A.dragon]: ["ed25519:Other"] } }));
  await assert.rejects(runStep("delete-keys", context(stranger, ["--send"]).ctx), /none of them the deploy key/);
});

const started = (id, creator) => `EVENT_JSON:${JSON.stringify({ standard: "nearly", event: "launch_started", data: [{ id, creator, symbol: "WYRM" }] })}`;
const launched = (id, creator) => ({ receipts_outcome: [{ outcome: { logs: [started(8, "sniper.testnet"), started(id, creator)] } }] });

test("launch attaches Nearly's quote plus 0.1 NEAR and 500 Tgas, takes the id from its own launch_started past a stranger's earlier launch of the symbol, resumes at 300 Tgas, and binds that id", async () => {
  const launches = { 3: { id: 3, creator: "sniper.testnet", step: "Done", inflight: false } };
  let lastLaunch = "0";
  const world = chain(deployed({
    accounts: { [ROOT]: account(undefined, 10n * NEAR) },
    views: {
      [`${A.dragon}.get_coin`]: null, [`${A.dragon}.get_state`]: () => ({ last_launch_at: lastLaunch }),
      [`${MOCKS.nearly}.quote_launch`]: { total: String(2n * NEAR) },
      [`${MOCKS.nearly}.get_launch_by_symbol`]: () => launches[3],
      [`${MOCKS.nearly}.get_launch`]: ({ launch_id }) => {
        const now = launches[launch_id];
        if (now.inflight) launches[launch_id] = { ...now, inflight: false, step: "Done" };
        return now;
      },
    },
  }));
  world.onSend = (tx) => {
    const method = tx.actions[0].method;
    if (method === "launch") {
      lastLaunch = "1";
      launches[9] = { id: 9, creator: A.dragon, step: "CreatePool", inflight: false };
      return launched(9, A.dragon);
    }
    if (method === "nearly_resume") launches[9] = { ...launches[9], step: "AddLiquidity", inflight: true };
    if (method === "bind") world.views[`${A.dragon}.get_coin`] = { token: "wyrm.nearly", launch_id: "9", pool_id: "p" };
  };
  const { ctx, lines } = context(world, ["--send"]);
  assert.equal(await runStep("launch", ctx), "done");
  assert.deepEqual(world.sent.map((t) => [t.signer, t.receiver, t.actions[0].method, t.actions[0].gas, t.actions[0].deposit]), [
    [ROOT, A.dragon, "launch", 500n * TGAS, 2n * NEAR + NEAR / 10n],
    [ROOT, A.dragon, "nearly_resume", 300n * TGAS, 0n],
  ]);
  assert.deepEqual(world.sent[1].actions[0].args, { launch_id: "9" });
  assert.ok(lines.includes("launch done: launch 9 is Done"));
  assert.equal(await runStep("bind", ctx), "done");
  assert.deepEqual(world.sent[2].actions[0].args, { launch_id: "9" });
});

test("a resumed run takes the symbol's first launch only when the Dragon created it, and --launch-id only for the Dragon's launch", async () => {
  const bySymbol = { id: 3, creator: "sniper.testnet", step: "Done", inflight: false };
  const world = chain(deployed({
    accounts: { [ROOT]: account(undefined, 10n * NEAR) },
    views: {
      [`${A.dragon}.get_coin`]: null, [`${A.dragon}.get_state`]: { last_launch_at: "5" },
      [`${MOCKS.nearly}.quote_launch`]: { total: String(2n * NEAR) },
      [`${MOCKS.nearly}.get_launch_by_symbol`]: () => bySymbol,
      [`${MOCKS.nearly}.get_launch`]: { id: 3, creator: "sniper.testnet", step: "Done" },
    },
  }));
  const stranger = /the first WYRM launch on Nearly is launch 3 by sniper\.testnet; pass --launch-id N/;
  await assert.rejects(runStep("launch", context(world, ["--send"]).ctx), stranger);
  await assert.rejects(runStep("bind", context(world, ["--send"]).ctx), stranger);
  await assert.rejects(runStep("bind", context(world, ["--send", "--launch-id", "3"]).ctx), new RegExp(`launch 3 is by sniper.testnet, not by ${A.dragon}`));
  assert.equal(world.sent.length, 0);
  const relaunch = context(world, ["--relaunch"]);
  assert.equal(await runStep("launch", relaunch.ctx), "planned");
  assert.match(relaunch.lines[0], /would send rehearsal\.thewyrm\.testnet -> dragon\.rehearsal\.thewyrm\.testnet: launch\(WYRM\)/);
  Object.assign(bySymbol, { id: 4, creator: A.dragon, step: "CreatePool" });
  const resumed = context(world);
  assert.equal(await runStep("launch", resumed.ctx), "planned");
  assert.match(resumed.lines[0], /nearly_resume\(launch 4 parked at CreatePool\) 300 Tgas/);
});

test("bind needs the Dragon's launch at Done and attaches 150 Tgas", async () => {
  const world = chain(deployed({ views: { [`${A.dragon}.get_coin`]: null, [`${MOCKS.nearly}.get_launch`]: { id: 7, creator: A.dragon, step: "AddLiquidity" } } }));
  await assert.rejects(runStep("bind", context(world, ["--send", "--launch-id", "7"]).ctx), /launch 7 is at AddLiquidity, not Done/);
  world.views[`${MOCKS.nearly}.get_launch`] = { id: 7, creator: A.dragon, step: "Done" };
  const { ctx, lines } = context(world, ["--launch-id", "7"]);
  assert.equal(await runStep("bind", ctx), "planned");
  assert.match(lines[0], /would send rehearsal\.thewyrm\.testnet -> dragon\.rehearsal\.thewyrm\.testnet: bind\(launch 7\) 150 Tgas/);
});

test("encodeAction matches the keeper's signed function call byte for byte", () => {
  const tx = { signer: "a.testnet", nonce: 5n, receiver: "b.testnet", blockHash: Buffer.alloc(32, 2) };
  const mine = signTransaction({ ...tx, seed, publicKey: pub, actions: [{ type: "call", method: "poke", args: {}, gas: 1000n * TGAS, deposit: 1n }] });
  const keeper = signedFunctionCall({ ...tx, seed, publicKey: pub, method: "poke", args: {}, gas: 1000n * TGAS, deposit: 1n });
  assert.equal(mine.base64, keeper.base64);
  assert.equal(mine.hash, keeper.hash);
  assert.deepEqual([...encodeAction({ type: "delete-account", beneficiary: "ab" }, pub)], [7, 2, 0, 0, 0, 97, 98]);
  assert.deepEqual([...encodeAction({ type: "add-key" }, pub).subarray(0, 2)], [5, 0]);
  assert.deepEqual([...encodeAction({ type: "add-key" }, pub).subarray(34)], [0, 0, 0, 0, 0, 0, 0, 0, 1]);
});

test("clean takes each key as a u16 length and its bytes, and the Dragon's rings are prefix plus u32 index", () => {
  assert.deepEqual([...cleanArgs([Buffer.from("s"), Buffer.from([1, 2, 3])])], [1, 0, 115, 3, 0, 1, 2, 3]);
  const dragon = BULK_KEYS.dragon();
  assert.equal(dragon.length, 1_440 + 256 + 32 + 32);
  assert.deepEqual([...dragon[1]], [115, 1, 0, 0, 0]);
  assert.deepEqual([...BULK_KEYS.scales()[0]], [0, 1]);
});

test("teardown refuses a root that is the signer, so it can never reach the operator's wallet", async () => {
  const world = chain(deployed());
  const opts = options(["--network", "testnet", "--root", "thewyrm.testnet", "--wasm-dir", wasmDir, "--step", "teardown"]);
  const ctx = makeContext(opts, { rpc: world.rpc, credentials: { account_id: "thewyrm.testnet", private_key: secret }, out: () => {} });
  await assert.rejects(runStep("teardown", ctx), /is the signer; refusing/);
});

const big = (refuse) => {
  const vaults = Object.fromEntries(Array.from({ length: 100 }, (_, i) => [A.vault(i + 1), account(code("scale_vault"), i < 2 ? NEAR / 2n : NEAR / 100n, 200)]));
  const wnear = { [A.dragon]: NEAR };
  const world = chain(deployed({
    accounts: { ...vaults, [A.dragon]: account(code("dragon"), 3n * NEAR, 300_000) },
    views: { [`${MOCKS.wrap}.ft_balance_of`]: ({ account_id }) => String(wnear[account_id] ?? 0n) },
    keys: Object.fromEntries([A.tithe, A.dragon, A.scales, A.ash, ROOT].map((a) => [a, [PK]])),
  }));
  world.state[A.dragon] = "large";
  world.onSend = (tx) => {
    const a = tx.actions[0];
    const acct = world.accounts[tx.signer];
    if (a.type === "delete-account" && refuse?.(tx, world)) {
      throw new Error(`H failed: {"ActionError":{"index":0,"kind":{"DeleteAccountWithLargeState":{"account_id":"${tx.signer}"}}}}`);
    }
    if (a.method === "release") world.accounts[tx.receiver] = account(code("scale_vault"), NEAR / 100n, 200);
    if (a.method === "near_withdraw") delete wnear[tx.signer];
    if (a.type === "deploy") world.accounts[tx.signer] = { ...acct, code_hash: code("state_cleanup"), storage_usage: acct.storage_usage - 100_000 };
    if (a.method === "clean") {
      world.state[tx.signer] = [];
      world.accounts[tx.signer] = { ...acct, storage_usage: 1_200 };
    }
    if (a.type === "delete-account") delete world.accounts[tx.signer];
  };
  return world;
};
const kindsOf = (world) => world.sent.map((t) => `${t.signer} ${t.actions[0].method ?? t.actions[0].type}`);
const inOrder = (kinds, ...steps) => {
  for (const s of steps) assert.ok(kinds.includes(s), `${s} was sent`);
  for (let i = 1; i < steps.length; i++) assert.ok(kinds.indexOf(steps[i - 1]) < kinds.indexOf(steps[i]), `${steps[i - 1]} before ${steps[i]}`);
};

test("teardown releases the vaults, unwraps, and deletes every account, root last, with no state wipe when the chain accepts", async () => {
  const world = big();
  const { ctx, lines } = context(world, ["--send", "--step", "teardown"]);
  assert.equal(await runStep("teardown", ctx), "done");
  const kinds = kindsOf(world);
  assert.deepEqual(kinds.filter((k) => k.endsWith("release")), [`${A.scales} release`, `${A.scales} release`]);
  inOrder(kinds, `${A.dragon} near_withdraw`, `${A.dragon} delete-account`);
  inOrder(kinds, `${A.scales} release`, `${A.scales} delete-account`);
  assert.ok(!kinds.some((k) => k.endsWith(" deploy") || k.endsWith(" clean")), "no state-cleanup when no deletion is refused");
  assert.equal(kinds.at(-1), `${ROOT} delete-account`);
  assert.ok(world.sent.filter((t) => t.actions[0].type === "delete-account").every((t) => t.actions[0].beneficiary === "thewyrm.testnet"));
  for (const a of [A.tithe, A.dragon, A.scales, A.ash, ROOT]) assert.equal(world.accounts[a], undefined, `${a} deleted`);
  assert.ok(lines.some((l) => /returns about 8\.9800 NEAR to thewyrm\.testnet/.test(l)), lines.join("\n"));
  world.sent.length = 0;
  assert.equal(await runStep("teardown", ctx), "skipped");
  assert.equal(world.sent.length, 0);
});

test("a deletion refused for large state gets state-cleanup after the unwrap, a wipe of the known keys, then a second deletion", async () => {
  let refusals = 0;
  const world = big((tx, w) => tx.signer === A.dragon && w.accounts[A.dragon].code_hash === code("dragon") && ++refusals);
  const { ctx, lines } = context(world, ["--send", "--step", "teardown"]);
  assert.equal(await runStep("teardown", ctx), "done");
  assert.equal(refusals, 1);
  const kinds = kindsOf(world);
  inOrder(kinds, `${A.dragon} near_withdraw`, `${A.dragon} delete-account`, `${A.dragon} deploy`, `${A.dragon} clean`);
  assert.equal(kinds.filter((k) => k === `${A.dragon} delete-account`).length, 2, "the refused deletion and the one after the wipe");
  assert.ok(kinds.lastIndexOf(`${A.dragon} delete-account`) > kinds.lastIndexOf(`${A.dragon} clean`));
  assert.deepEqual(kinds.filter((k) => k.endsWith(" deploy")), [`${A.dragon} deploy`], "only the refused account gets state-cleanup");
  const wiped = world.sent.filter((t) => t.signer === A.dragon && t.actions[0].method === "clean").reduce((n, t) => n + Number(t.actions[0].summary.split(" ")[0]), 0);
  assert.equal(wiped, BULK_KEYS.dragon().length, "the rings are wiped by their known keys while view_state refuses");
  assert.ok(lines.some((l) => /refused: .*DeleteAccountWithLargeState/.test(l)), lines.join("\n"));
  for (const a of [A.tithe, A.dragon, A.scales, A.ash, ROOT]) assert.equal(world.accounts[a], undefined, `${a} deleted`);
  let failures = 0;
  const other = big(() => {
    if (failures++ === 0) throw new Error("H failed: InvalidNonce");
    return false;
  });
  await assert.rejects(runStep("teardown", context(other, ["--send", "--step", "teardown"]).ctx), /InvalidNonce/, "only a large-state refusal is caught");
});

test("a teardown dry run lists every account with the NEAR it returns and signs nothing", async () => {
  const world = chain(deployed({ keys: Object.fromEntries([A.tithe, A.dragon, A.scales, A.ash, ROOT].map((a) => [a, [PK]])), views: { [`${MOCKS.wrap}.ft_balance_of`]: "0" } }));
  const { ctx, lines } = context(world, ["--step", "teardown"]);
  assert.equal(await runStep("teardown", ctx), "planned");
  assert.equal(world.sent.length, 0);
  for (const a of [A.tithe, A.dragon, A.scales, A.ash, ROOT]) assert.ok(lines.some((l) => l.trim().startsWith(a)), a);
  assert.ok(lines.some((l) => /0 vaults/.test(l)));
  assert.ok(lines.some((l) => /returns about 5\.0000 NEAR to thewyrm\.testnet/.test(l)), lines.join("\n"));
  const full = context(big(), ["--step", "teardown"]);
  assert.equal(await runStep("teardown", full.ctx), "planned");
  assert.ok(full.lines.some((l) => l.includes(`${A.scales}`) && l.endsWith("waits for the vault releases")), full.lines.join("\n"));
  assert.ok(!full.lines.some((l) => l.includes(`would send ${A.scales} -> ${A.scales}`)), "scales is not deleted in the round that releases its vaults");
});

test("a send the RPC drops with a timeout is looked up and sent again as the same signed bytes", async () => {
  const sent = [];
  let landed = false;
  const rpc = async (method, params) => {
    if (method === "send_tx") {
      sent.push(params.signed_tx_base64);
      if (sent.length === 1) throw new Error("send_tx: Timeout");
      landed = true;
      return { status: { SuccessValue: "" } };
    }
    if (!landed) throw new Error("UNKNOWN_TRANSACTION: Transaction X doesn't exist");
    return { status: { SuccessValue: "" } };
  };
  const outcome = await deliver(rpc, { base64: "AAAA", hash: "H" }, "a.testnet", "a call", 4, 0);
  assert.deepEqual(outcome.status, { SuccessValue: "" });
  assert.deepEqual(sent, ["AAAA", "AAAA"]);
  const failing = async () => { throw new Error("send_tx: InvalidNonce"); };
  await assert.rejects(deliver(failing, { base64: "B", hash: "H2" }, "a.testnet", "a call", 4, 0), /H2 a call: send_tx: InvalidNonce; read the transaction/);
});

test("a vault code publish is sent once and waited for while RPC does not yet return the code", async () => {
  const world = chain({ ...deployed(), global: [] });
  let reads = 0;
  const rpc = world.rpc;
  world.rpc = async (method, params) => {
    if (params?.request_type === "view_global_contract_code" && world.sent.length && ++reads > 3) return {};
    return rpc(method, params);
  };
  const { ctx, lines } = context(world, ["--send"]);
  ctx.rpc = world.rpc;
  assert.equal(await runStep("publish-vault-code", ctx), "done");
  assert.equal(world.sent.length, 1);
  assert.equal(lines.filter((l) => l.includes("not yet visible")).length, 3);
});
