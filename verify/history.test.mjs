import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { accountReceipts, accountTxHashes, chainReceipts, chainTransactions, fastnear, flatten, parseEvent, touching, wyrmEvents } from "./history.mjs";

const fixture = (name) => JSON.parse(fs.readFileSync(new URL(`./fixtures/${name}`, import.meta.url)));

test("a recorded FastNEAR transaction flattens into receipts with exact deposits, methods and Nearly's event", () => {
  const receipts = flatten(fixture("fastnear-tx-push-creator-fees.json").transactions[0]);
  const payout = receipts.find((r) => r.receiver === "nearlingsdevfees.near");
  assert.equal(payout.tx, "12Ni8fHqGsikhJPTo19c1ZmUqY4YsULv6gfA7R3frzvu");
  assert.equal(payout.predecessor, "nearlytrade.near");
  assert.deepEqual(payout.actions, [{ kind: "Transfer", deposit: 86399999999999999999999n }]);
  const call = receipts[0];
  assert.deepEqual([call.actions[0].method, call.actions[0].args], ["push_creator_fees", { account_id: "nearlingsdevfees.near" }]);
  assert.deepEqual(call.events.map((e) => [e.standard, e.event, e.data[0].amount]), [["nearpad", "creator_fees_claimed", "86399999999999999999999"]]);
  assert.ok(receipts.every((r) => r.ok && typeof r.at === "bigint" && r.height > 0));
});

test("the listing follows resume_token until a page comes back empty and yields each hash once", async () => {
  const seen = [];
  const post = async (_, body) => {
    seen.push(body.resume_token ?? null);
    if (!body.resume_token) return { account_txs: [{ transaction_hash: "A" }, { transaction_hash: "B" }], resume_token: "t1" };
    if (body.resume_token === "t1") return { account_txs: [{ transaction_hash: "B" }, { transaction_hash: "C" }], resume_token: "t2" };
    return { account_txs: [], resume_token: "t3" };
  };
  assert.deepEqual(await accountTxHashes(post, "x.near"), ["A", "B", "C"]);
  assert.deepEqual(seen, [null, "t1", "t2"]);
});

test("transactions are fetched in batches of at most 20", async () => {
  const hashes = Array.from({ length: 45 }, (_, i) => `H${i}`);
  const batches = [];
  const tx = fixture("fastnear-tx-push-creator-fees.json").transactions[0];
  const post = async (path, body) => {
    if (path === "/v0/account") return { account_txs: body.resume_token ? [] : hashes.map((transaction_hash) => ({ transaction_hash })), resume_token: "next" };
    batches.push(body.tx_hashes.length);
    return { transactions: body.tx_hashes.map(() => tx) };
  };
  await accountReceipts(post, "x.near");
  assert.deepEqual(batches, [20, 20, 5]);
});

test("only successful wyrm-standard events emitted by the account itself count", () => {
  const ev = (event, data) => `EVENT_JSON:${JSON.stringify({ standard: "wyrm", version: "1.0.0", event, data })}`;
  const r = (receiver, logs, ok = true) => ({ tx: "T", receiver, ok, events: logs.map(parseEvent).filter(Boolean) });
  const receipts = [
    r("tithe.near", [ev("tithe_round", [{ round: "1" }, { round: "2" }])]),
    r("tithe.near", [ev("tithe_round", [{ round: "3" }])], false),
    r("fake.near", [ev("tithe_round", [{ round: "4" }])]),
    r("tithe.near", ["not an event", 'EVENT_JSON:{"standard":"nep171","event":"nft_mint","data":[{}]}']),
  ];
  assert.deepEqual(wyrmEvents(receipts, "tithe.near").map((e) => e.data.round), ["1", "2"]);
});

const EMPTY = "11111111111111111111111111111111";

// A chain of `height + 1` blocks where each listed height holds one transaction, total supply falls one yocto in the
// block after each one, and every other block is idle.
function node({ height, txAt, missing = [], local = [] }) {
  const supply = (h) => String(1_000 - txAt.filter((t) => t < h).length);
  const block = (h) => ({
    header: { height: h, hash: `B${h}`, prev_hash: h === 0 ? EMPTY : `B${h - 1}`, timestamp: String(h * 1_000_000_000), total_supply: supply(h) },
    chunks: [{ chunk_hash: `C${h}`, tx_root: txAt.includes(h) ? "ROOT" : EMPTY }],
  });
  const calls = [];
  const rpc = async (method, params) => {
    calls.push(method);
    const id = params.block_id;
    const h = method === "block" && params.finality ? height : typeof id === "string" ? Number(id.slice(1)) : id;
    if (method === "block") {
      if (missing.includes(h) || h < 0 || h > height) throw new Error("UNKNOWN_BLOCK");
      return block(h);
    }
    if (method === "chunk") return { transactions: [{ hash: `T${params.chunk_id.slice(1)}`, signer_id: "s.near" }] };
    if (method === "EXPERIMENTAL_tx_status") {
      const h = Number(params.tx_hash.slice(1));
      const actions = [{ FunctionCall: { method_name: "poke", args: "e30=", deposit: "0" } }];
      const receiver = local.includes(h) ? "s.near" : "dragon.near";
      return {
        transaction: { signer_id: "s.near", receiver_id: receiver, actions },
        transaction_outcome: { outcome: { receipt_ids: [`R${h}`] } },
        receipts: local.includes(h) ? [] : [{ receipt_id: `R${h}`, predecessor_id: "s.near", receiver_id: receiver, receipt: { Action: { actions } } }],
        receipts_outcome: [{ id: `R${h}`, block_hash: `B${h}`, outcome: { logs: [], status: { SuccessValue: "" } } }, { id: "refund", block_hash: `B${h}`, outcome: { logs: [], status: {} } }],
      };
    }
    throw new Error(`unexpected ${method}`);
  };
  return { rpc, calls };
}

test("the sandbox walk finds every transaction back to genesis and jumps over a long idle stretch", async () => {
  const { rpc, calls } = node({ height: 100_000, txAt: [3, 40, 41, 90] });
  const { txs } = await chainTransactions(rpc);
  assert.deepEqual(txs.map((t) => t.hash).sort(), ["T3", "T40", "T41", "T90"]);
  assert.ok(calls.length < 300, `${calls.length} requests for 100,001 blocks`);
});

test("chainReceipts flattens each transaction's receipts with their block height and time", async () => {
  const { rpc } = node({ height: 50, txAt: [7, 20] });
  const receipts = await chainReceipts(rpc);
  assert.deepEqual(receipts.map((r) => [r.tx, r.receiver, r.height, r.at, r.actions[0].method]), [["T7", "dragon.near", 7, 7_000_000_000n, "poke"], ["T20", "dragon.near", 20, 20_000_000_000n, "poke"]]);
});

test("a transaction the signer sends to itself keeps its local receipt, rebuilt from the transaction", async () => {
  const { rpc } = node({ height: 50, txAt: [7, 20], local: [20] });
  const receipts = await chainReceipts(rpc);
  assert.deepEqual(receipts.map((r) => [r.tx, r.predecessor, r.receiver, r.actions[0].method]), [["T7", "s.near", "dragon.near", "poke"], ["T20", "s.near", "s.near", "poke"]]);
});

test("a garbage-collected block stops the walk with the reason, instead of an empty history", async () => {
  const { rpc } = node({ height: 30, txAt: [25], missing: Array.from({ length: 13 }, (_, h) => h) });
  await assert.rejects(chainTransactions(rpc), /no longer has the block before height 13/);
});

test("touching keeps every receipt of each transaction that reached or left the account", () => {
  const r = (tx, predecessor, receiver) => ({ tx, predecessor, receiver });
  const all = [r("A", "k.near", "dragon.near"), r("A", "dragon.near", "wrap.near"), r("B", "k.near", "wrap.near"), r("C", "wrap.near", "x.near"), r("C", "x.near", "dragon.near")];
  assert.deepEqual(touching(all, "dragon.near").map((x) => x.tx), ["A", "A", "C", "C"]);
});

const reply = (status, body = {}, headers = {}) => ({ status, ok: status < 400, headers: new Headers(headers), json: async () => body, text: async () => JSON.stringify(body) });

test("a rate-limited read waits and retries until the API answers, for as long as the API asks", async () => {
  const waits = [];
  const answers = [reply(429), reply(429, {}, { "retry-after": "7" }), reply(200, { ok: 1 })];
  const post = fastnear("https://api", async () => answers.shift(), async (ms) => waits.push(ms));
  assert.deepEqual(await post("/v0/account", {}), { ok: 1 });
  assert.deepEqual(waits, [1000, 7000]);
});

test("a rate limit that never lifts fails the read, and any other error fails it at once", async () => {
  let calls = 0;
  const limited = fastnear("https://api", async () => (calls++, reply(429)), async () => {});
  await assert.rejects(limited("/v0/account", {}), /HTTP 429/);
  assert.equal(calls, 12);
  calls = 0;
  const broken = fastnear("https://api", async () => (calls++, reply(500)), async () => {});
  await assert.rejects(broken("/v0/account", {}), /HTTP 500/);
  assert.equal(calls, 1);
});

test("requests go out one at a time, and a failed one does not block the next", async () => {
  let open = 0;
  let most = 0;
  const post = fastnear("https://api", async (url) => {
    most = Math.max(most, ++open);
    await new Promise((r) => setTimeout(r, 5));
    open--;
    return url.endsWith("/bad") ? reply(500) : reply(200, { url });
  });
  const results = await Promise.allSettled([post("/a", {}), post("/bad", {}), post("/c", {})]);
  assert.deepEqual(results.map((r) => r.status), ["fulfilled", "rejected", "fulfilled"]);
  assert.equal(most, 1);
});
