import crypto from "node:crypto";

// Peers (Nearly, DCL, wrap, ash) come from dragon.get_config().peers, never from this table. A sandbox has no
// indexer, so verify reads its history from the RPC.
export const NETWORKS = {
  mainnet: { rpc: "https://rpc.mainnet.fastnear.com", indexer: "https://tx.main.fastnear.com" },
  testnet: { rpc: "https://rpc.testnet.fastnear.com", indexer: "https://tx.test.fastnear.com" },
  sandbox: { rpc: null, indexer: null },
};

export function connect(opts) {
  const net = NETWORKS[opts.network];
  if (!net) throw new Error(`--network must be one of ${Object.keys(NETWORKS).join(", ")}`);
  const url = opts.rpc ?? net.rpc;
  if (!url) throw new Error(`--network ${opts.network} needs --rpc <url>`);
  return { net, rpc: httpRpc(url, { verbose: Boolean(opts.verbose) }) };
}

export class RpcError extends Error {
  constructor(method, error) {
    super(`${method}: ${typeof error === "string" ? error : typeof error?.data === "string" ? error.data : JSON.stringify(error)}`);
    this.rpcError = error;
  }
}

const backoff = (attempt) => Math.min(1000 * 2 ** attempt, 15_000);

// The transport is a function so tests can replay recorded responses: (method, params) => result.
// `rpc.requests` counts HTTP requests, retries included; --verbose logs each one to stderr.
// Overload arrives as HTTP 429 or, from the public RPC, as JSON-RPC error -429 inside an HTTP 200. Both are retried.
export function httpRpc(url, { verbose = false, wait = backoff } = {}) {
  const rpc = async (method, params) => {
    for (let attempt = 0; ; attempt++) {
      rpc.requests++;
      if (verbose) console.error(`rpc ${rpc.requests} ${method} ${params.request_type ?? ""} ${params.account_id ?? ""} ${params.method_name ?? params.block_id ?? params.tx_hash ?? ""}`.replace(/ +/g, " "));
      const res = await fetch(url, {
        signal: AbortSignal.timeout(60_000),
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", id: "wyrm", method, params }),
      });
      const text = await res.text();
      let body;
      try {
        body = JSON.parse(text);
      } catch {
        body = null;
      }
      if ((res.status === 429 || body?.error?.code === -429) && attempt < 8) {
        await new Promise((r) => setTimeout(r, wait(attempt)));
        continue;
      }
      if (!body) throw new RpcError(method, `HTTP ${res.status} ${text.slice(0, 200)}`);
      if (body.error) throw new RpcError(method, body.error);
      return body.result;
    }
  };
  rpc.requests = 0;
  return rpc;
}

export const rpcKey = (method, params) => JSON.stringify([method, params]);

// Keeps every answer, and every error, under its request (--record), so a run against a live node replays offline.
export function recording(rpc) {
  const log = {};
  const recorder = async (method, params) => {
    try {
      const result = await rpc(method, params);
      log[rpcKey(method, params)] = { result };
      return result;
    } catch (e) {
      log[rpcKey(method, params)] = { error: e.rpcError ?? e.message };
      throw e;
    }
  };
  recorder.log = log;
  Object.defineProperty(recorder, "requests", { get: () => rpc.requests });
  return recorder;
}

// Answers from a `recording` log (--replay), throwing for a request it never saw.
export function replaying(log) {
  const replayer = async (method, params) => {
    replayer.requests++;
    const entry = log[rpcKey(method, params)];
    if (!entry) throw new RpcError(method, `no recorded answer for ${rpcKey(method, params)}`);
    if ("error" in entry) throw new RpcError(method, entry.error);
    return entry.result;
  };
  replayer.requests = 0;
  return replayer;
}

// `fn` over `items` with at most `limit` in flight, so a public RPC is not asked for everything at once.
export async function mapLimit(items, limit, fn) {
  const out = new Array(items.length);
  let next = 0;
  await Promise.all(Array.from({ length: limit }, async () => {
    while (next < items.length) {
      const i = next++;
      out[i] = await fn(items[i]);
    }
  }));
  return out;
}

const finality = (blockId) => (blockId === undefined ? { finality: "final" } : { block_id: blockId });

export async function view(rpc, account, method, args = {}, blockId) {
  const result = await rpc("query", {
    request_type: "call_function", ...finality(blockId), account_id: account, method_name: method,
    args_base64: Buffer.from(JSON.stringify(args)).toString("base64"),
  });
  if (result.error) throw new RpcError(`${account}.${method}`, result.error);
  return JSON.parse(Buffer.from(result.result).toString() || "null");
}

export const isUnknownAccount = (e) => /UNKNOWN_ACCOUNT|does not exist while viewing/.test(e?.message ?? "");

// null when the account does not exist.
export async function viewAccount(rpc, account, blockId) {
  try {
    return await rpc("query", { request_type: "view_account", ...finality(blockId), account_id: account });
  } catch (e) {
    if (isUnknownAccount(e)) return null;
    throw e;
  }
}

// null when the account does not exist. view_access_key_list alone answers [] for a missing account, so a
// missing account would read as keyless; the existence check comes first.
export async function accessKeys(rpc, account) {
  if ((await viewAccount(rpc, account)) === null) return null;
  return (await rpc("query", { request_type: "view_access_key_list", finality: "final", account_id: account })).keys;
}

export async function finalBlock(rpc) {
  return (await rpc("block", { finality: "final" })).header;
}

const B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

export function b58decode(text) {
  let n = 0n;
  for (const ch of text) {
    const digit = B58.indexOf(ch);
    if (digit < 0) throw new Error(`"${ch}" is not a base58 character`);
    n = n * 58n + BigInt(digit);
  }
  const bytes = [];
  for (; n > 0n; n >>= 8n) bytes.unshift(Number(n & 0xffn));
  const zeros = text.length - text.replace(/^1+/, "").length;
  return Buffer.from([...new Array(zeros).fill(0), ...bytes]);
}

export function b58encode(bytes) {
  let n = BigInt("0x0" + Buffer.from(bytes).toString("hex"));
  let out = "";
  for (; n > 0n; n /= 58n) out = B58[Number(n % 58n)] + out;
  for (const b of bytes) {
    if (b !== 0) break;
    out = "1" + out;
  }
  return out;
}

const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const u64 = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };
const u128 = (n) => Buffer.concat([u64(BigInt(n) & ((1n << 64n) - 1n)), u64(BigInt(n) >> 64n)]);
const str = (s) => { const b = Buffer.from(s); return Buffer.concat([u32(b.length), b]); };

// "ed25519:<base58 of seed||public>" into its parts.
export function parseSecretKey(text) {
  const raw = b58decode(String(text).replace(/^ed25519:/, ""));
  if (raw.length !== 64) throw new Error(`not an ed25519 secret key (expected 64 bytes, got ${raw.length})`);
  return { seed: raw.subarray(0, 32), publicKey: raw.subarray(32), publicKeyText: "ed25519:" + b58encode(raw.subarray(32)) };
}

// A borsh-encoded Transaction (V0) with one FunctionCall action, signed, base64 for the RPC.
export function signedFunctionCall({ signer, seed, publicKey, nonce, receiver, blockHash, method, args, gas, deposit = 0n }) {
  const argBytes = Buffer.from(JSON.stringify(args));
  const action = Buffer.concat([Buffer.from([2]), str(method), u32(argBytes.length), argBytes, u64(gas), u128(deposit)]);
  const tx = Buffer.concat([str(signer), Buffer.from([0]), publicKey, u64(nonce), str(receiver), blockHash, u32(1), action]);
  const pkcs8 = Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]);
  const key = crypto.createPrivateKey({ key: pkcs8, format: "der", type: "pkcs8" });
  const hash = crypto.createHash("sha256").update(tx).digest();
  const signature = crypto.sign(null, hash, key);
  return { base64: Buffer.concat([tx, Buffer.from([0]), signature]).toString("base64"), hash: b58encode(hash) };
}

// Each key's nonce, by public key. Optimistic, not final: a nonce must include a call this signer sent a moment ago.
export async function readNonces(rpc, account) {
  const { keys } = await rpc("query", { request_type: "view_access_key_list", finality: "optimistic", account_id: account });
  return new Map(keys.map((k) => [k.public_key, BigInt(k.access_key.nonce)]));
}

// One request: the caller supplies the nonce and a recent block hash.
export async function sendFunctionCall(rpc, { signer, secret, nonce, blockHash, receiver, method, args, gas, deposit = 0n }) {
  const { seed, publicKey } = parseSecretKey(secret);
  const signed = signedFunctionCall({ signer, seed, publicKey, nonce, receiver, blockHash: b58decode(blockHash), method, args, gas, deposit });
  return rpc("send_tx", { signed_tx_base64: signed.base64, wait_until: "EXECUTED_OPTIMISTIC" });
}

// The transaction's own failure as text, or null when it succeeded.
export const failureOf = (outcome) => (outcome?.status?.Failure ? JSON.stringify(outcome.status.Failure) : null);

// Receipts that failed further down the chain (a refunded vault transfer, a failed swap), by id.
export const failedReceipts = (outcome) =>
  (outcome?.receipts_outcome ?? []).filter((r) => r.outcome?.status?.Failure).map((r) => r.id);
