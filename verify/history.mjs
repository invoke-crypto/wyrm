// Receipt history from the FastNEAR transactions API (tx.main.fastnear.com), which needs no API key:
//   POST /v0/account      { account_id, resume_token? }  -> every transaction touching the account, newest first
//   POST /v0/transactions { tx_hashes: [<= 20] }          -> each transaction with all its receipts, logs and outcomes
// Amounts stay exact strings here, unlike NearBlocks' receipt API, which returns deposits as floats.

const TX_BATCH = 20;

export function fastnear(base, fetchImpl = fetch) {
  return async (path, body) => {
    for (let attempt = 0; ; attempt++) {
      const res = await fetchImpl(base + path, {
        signal: AbortSignal.timeout(60_000), method: "POST",
        headers: { "content-type": "application/json" }, body: JSON.stringify(body),
      });
      if (res.status === 429 && attempt < 5) {
        await new Promise((r) => setTimeout(r, 1000 * 2 ** attempt));
        continue;
      }
      if (!res.ok) throw new Error(`${base}${path}: HTTP ${res.status} ${await res.text()}`);
      return res.json();
    }
  };
}

export async function accountTxHashes(post, account) {
  const hashes = [];
  let resume_token;
  do {
    const page = await post("/v0/account", { account_id: account, ...(resume_token ? { resume_token } : {}) });
    hashes.push(...page.account_txs.map((t) => t.transaction_hash));
    resume_token = page.account_txs.length ? page.resume_token : undefined;
  } while (resume_token);
  return [...new Set(hashes)];
}

const decodeArgs = (b64) => {
  try {
    return JSON.parse(Buffer.from(b64 ?? "", "base64").toString());
  } catch {
    return null;
  }
};

function action(raw) {
  if (raw === "CreateAccount") return { kind: "CreateAccount" };
  const [kind, body] = Object.entries(raw)[0];
  if (kind === "Transfer") return { kind, deposit: BigInt(body.deposit) };
  if (kind === "FunctionCall") return { kind, method: body.method_name, args: decodeArgs(body.args), deposit: BigInt(body.deposit) };
  return { kind };
}

export function parseEvent(log) {
  if (!log.startsWith("EVENT_JSON:")) return null;
  try {
    return JSON.parse(log.slice("EVENT_JSON:".length));
  } catch {
    return null;
  }
}

function flatReceipt(tx, receipt, outcome, height, at) {
  const { status, logs } = outcome;
  return {
    tx, id: receipt.receipt_id, predecessor: receipt.predecessor_id, receiver: receipt.receiver_id,
    actions: (receipt.receipt.Action?.actions ?? []).map(action),
    logs, events: logs.map(parseEvent).filter(Boolean),
    ok: !("Failure" in status),
    value: "SuccessValue" in status ? Buffer.from(status.SuccessValue, "base64").toString() : null,
    height, at,
  };
}

// One FastNEAR transaction into flat receipts: { tx, id, predecessor, receiver, actions, logs, events, ok, value, height, at }.
// `at` is the block timestamp in ns; FastNEAR sends it as a JSON number, so it is exact only to a microsecond.
export function flatten(t) {
  return t.receipts.map(({ receipt, execution_outcome: eo }) =>
    flatReceipt(t.transaction.hash, receipt, eo.outcome, eo.block_height, BigInt(Math.round(eo.block_timestamp))));
}

const EMPTY_HASH = "11111111111111111111111111111111";

// Every transaction on a node with no indexer (a sandbox), walked from the final block back to genesis.
// Total supply falls only when gas burns, so where it is flat no chunk ran anything and the walk jumps by height
// instead of stepping, which keeps a node that idled for hours as cheap as a fresh one.
export async function chainTransactions(rpc) {
  const blocks = new Map();
  const txs = [];
  let block = await rpc("block", { finality: "final" });
  let flat = 0;
  for (;;) {
    const h = block.header;
    blocks.set(h.hash, { height: h.height, at: BigInt(h.timestamp) });
    for (const c of block.chunks) {
      if (c.tx_root === EMPTY_HASH) continue;
      const chunk = await rpc("chunk", { chunk_id: c.chunk_hash });
      txs.push(...chunk.transactions.map((t) => ({ hash: t.hash, signer: t.signer_id })));
    }
    if (h.prev_hash === EMPTY_HASH) return { txs, blocks };
    // Short flat runs sit between the receipts of one transaction, where a failed jump only costs a request.
    const stride = flat >= 4 ? 2 ** flat : 0;
    const jumped = stride && h.height - stride > 0 ? await rpc("block", { block_id: h.height - stride }).catch(() => null) : null;
    if (jumped?.header.total_supply === h.total_supply) {
      block = jumped;
      flat++;
      continue;
    }
    block = await rpc("block", { block_id: h.prev_hash }).catch((e) => {
      throw new Error(`the node no longer has the block before height ${h.height} (${e.message.slice(0, 60)}), so its history is incomplete`);
    });
    flat =block.header.total_supply === h.total_supply ? Math.min(flat + 1, 4) : 0;
  }
}

// Every receipt of every transaction on the node, flattened as `flatten` does, oldest first.
export async function chainReceipts(rpc) {
  const { txs, blocks } = await chainTransactions(rpc);
  const out = [];
  for (const { hash, signer } of txs) {
    const t = await rpc("EXPERIMENTAL_tx_status", { tx_hash: hash, sender_account_id: signer, wait_until: "FINAL" });
    const byId = new Map(t.receipts.map((r) => [r.receipt_id, r]));
    // A transaction its signer sends to itself runs as a local receipt, which the node never lists.
    const first = t.transaction_outcome.outcome.receipt_ids[0];
    if (first && !byId.has(first)) {
      byId.set(first, { receipt_id: first, predecessor_id: t.transaction.signer_id, receiver_id: t.transaction.receiver_id, receipt: { Action: { actions: t.transaction.actions } } });
    }
    for (const o of t.receipts_outcome) {
      const receipt = byId.get(o.id);
      if (!receipt) continue;
      const block = blocks.get(o.block_hash);
      out.push(flatReceipt(hash, receipt, o.outcome, block?.height ?? 0, block?.at ?? 0n));
    }
  }
  return out.sort((a, b) => a.height - b.height);
}

// Receipts of the transactions that touched `account`, the shape FastNEAR's account history gives.
export function touching(receipts, account) {
  const txs = new Set(receipts.filter((r) => r.receiver === account || r.predecessor === account).map((r) => r.tx));
  return receipts.filter((r) => txs.has(r.tx));
}

export async function accountReceipts(post, account) {
  const hashes = await accountTxHashes(post, account);
  const out = [];
  for (let i = 0; i < hashes.length; i += TX_BATCH) {
    const { transactions } = await post("/v0/transactions", { tx_hashes: hashes.slice(i, i + TX_BATCH) });
    for (const t of transactions) if (t) out.push(...flatten(t));
  }
  return out.sort((a, b) => a.height - b.height);
}

// Every `wyrm` event an account emitted, each data record flattened with its receipt: { name, data, receipt }.
export function wyrmEvents(receipts, account) {
  const out = [];
  for (const r of receipts) {
    if (r.receiver !== account || !r.ok) continue;
    for (const e of r.events) {
      if (e.standard !== "wyrm") continue;
      for (const data of e.data ?? []) out.push({ name: e.event, data, receipt: r });
    }
  }
  return out;
}
