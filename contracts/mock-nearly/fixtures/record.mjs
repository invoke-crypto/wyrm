// Records the mainnet answers mock-nearly copies: Nearly's views for the newest taxed launch, and its DCL pool.
// Run: node contracts/mock-nearly/fixtures/record.mjs   (read-only view calls against public RPC; signs nothing)
import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const RPC = process.env.RPC ?? 'https://rpc.mainnet.fastnear.com';
const NEARLY = 'nearlytrade.near';
const DCL = 'dclv2.ref-labs.near';
const OUT = dirname(fileURLToPath(import.meta.url));

async function rpc(method, params) {
  for (let attempt = 0; ; attempt++) {
    const res = await fetch(RPC, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
      signal: AbortSignal.timeout(30_000),
    });
    // Public RPC rate-limits with HTTP 429, sometimes with a body that is not JSON.
    const r = res.status === 429 ? { error: { code: -429 } } : await res.json();
    if (!r.error) return r.result;
    if (r.error.code !== -429 || attempt === 8) throw new Error(JSON.stringify(r.error));
    await new Promise((s) => setTimeout(s, 2_000 * (attempt + 1)));
  }
}

async function view(account_id, method_name, args, block_id) {
  const res = await rpc('query', {
    request_type: 'call_function',
    account_id,
    method_name,
    args_base64: Buffer.from(JSON.stringify(args)).toString('base64'),
    block_id,
  });
  if (res.error) throw new Error(`${account_id}.${method_name}: ${res.error}`);
  return JSON.parse(Buffer.from(res.result).toString());
}

// Nearly's sanitize_slug (factory lib.rs:4776 at e82bfa0).
const slug = (symbol) =>
  [...symbol].filter((c) => /[A-Za-z0-9]/.test(c)).map((c) => c.toLowerCase()).slice(0, 12).join('') || 't';

const block = (await rpc('block', { finality: 'final' })).header.height;

let launch;
for (let from = 0; !launch; from += 50) {
  const page = await view(NEARLY, 'get_launches', { from_index: from, limit: 50 }, block);
  if (page.length === 0) throw new Error('no taxed launch found');
  for (const l of page) {
    if ((await view(NEARLY, 'get_tax', { launch_id: String(l.id) }, block)) !== null) {
      launch = l;
      break;
    }
  }
}

const id = { launch_id: String(launch.id) };
const views = [
  ['get_launch', NEARLY, id],
  ['get_fee_to', NEARLY, id],
  ['get_fee_opts', NEARLY, id],
  ['get_tax', NEARLY, id],
  ['get_slug_count', NEARLY, { slug: slug(launch.symbol) }],
  ['quote_launch', NEARLY, { icon_bytes: 0, dev_buy: null, tax: true, text_bytes: null }],
  ['get_dev_buy_cap', NEARLY, {}],
  ['get_pool', DCL, { pool_id: launch.pool_id }],
];
for (const [method, account_id, args] of views) {
  const result = await view(account_id, method, args, block);
  const file = join(OUT, `${method}.json`);
  writeFileSync(file, JSON.stringify({ account_id, method, args, block_height: block, result }, null, 2) + '\n');
  console.log(`${account_id}.${method} block ${block} -> ${file}`);
}
