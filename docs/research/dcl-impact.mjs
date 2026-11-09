// Quotes Rhea DCL for the largest Nearly coins at one block and writes docs/research/dcl-impact.json.
// Run: node docs/research/dcl-impact.mjs   (read-only view calls against public RPC)
import { writeFileSync } from 'node:fs';

const RPC = process.env.RPC ?? 'https://rpc.mainnet.fastnear.com';
const DCL = 'dclv2.ref-labs.near';
const WRAP = 'wrap.near';
const COINS = [
  'nearly-993927.nearlytrade.near',
  'batman-4.nearlytrade.near',
  'singularty.nearlytrade.near',
  'ninu-4.nearlytrade.near',
  'dragon-3.nearlytrade.near',
  // A launch with no trades yet: what a fresh coin's pool looks like.
  'xela.nearlytrade.near',
];
const SIZES_NEAR = [0.1, 0.5, 1, 2, 5, 10, 25, 50, 100];
const REF_NEAR = 0.01;
const YOCTO = 10n ** 24n;

async function rpc(method, params) {
  for (let attempt = 0; ; attempt++) {
    const r = await fetch(RPC, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) }).then((r) => r.json());
    if (!r.error) return r.result;
    // Public RPC answers -429 when called too fast.
    if (r.error.code !== -429 || attempt === 8) throw new Error(JSON.stringify(r.error));
    await new Promise((s) => setTimeout(s, 2_000 * (attempt + 1)));
  }
}
async function view(account_id, method_name, args, block_id) {
  const res = await rpc('query', { request_type: 'call_function', account_id, method_name, args_base64: Buffer.from(JSON.stringify(args)).toString('base64'), block_id });
  if (res.error) throw new Error(res.error);
  return JSON.parse(Buffer.from(res.result).toString());
}
const yocto = (near) => (BigInt(Math.round(near * 1e6)) * YOCTO) / 1_000_000n;

const block = (await rpc('block', { finality: 'final' })).header.height;
const coins = [];
for (const token of COINS) {
  const launch = await view('nearlytrade.near', 'get_launch_by_token', { token }, block);
  const pool_id = launch.pool_id;
  const pool = await view(DCL, 'get_pool', { pool_id }, block);
  const tokenIsX = pool.token_x === token;
  const quote = (near) => view(DCL, 'quote', { pool_ids: [pool_id], input_token: WRAP, output_token: token, input_amount: yocto(near).toString(), tag: null }, block).then((q) => BigInt(q.amount));
  const refOut = await quote(REF_NEAR);
  const refPerNear = Number(refOut) / REF_NEAR;
  // 1.0001^point is the price of x in units of y (raw). Tokens per wNEAR at spot, before the pool fee:
  const spotPerNear = (tokenIsX ? 1 / Math.pow(1.0001, pool.current_point) : Math.pow(1.0001, pool.current_point)) * 1e24;
  const rows = [];
  for (const near of SIZES_NEAR) {
    const out = await quote(near);
    const perNear = Number(out) / near;
    rows.push({
      in_near: near,
      out_raw: out.toString(),
      out_tokens: Number(out) / 1e18,
      impact_bps_vs_0_01_near: Math.round((1 - perNear / refPerNear) * 10_000),
      cost_bps_vs_spot_incl_fee: Math.round((1 - perNear / spotPerNear) * 10_000),
    });
  }
  coins.push({
    symbol: launch.symbol, token, launch_id: launch.id, pool_id, fee: pool.fee, current_point: pool.current_point,
    pool_total_wnear: Number(BigInt(tokenIsX ? pool.total_y : pool.total_x)) / 1e24,
    liquidity: pool.liquidity, tax_on_token: await view('nearlytrade.near', 'get_tax', { launch_id: String(launch.id) }, block),
    ref_tokens_per_near_at_0_01: refPerNear / 1e18, spot_tokens_per_near: spotPerNear / 1e18, rows,
  });
}
const out = {
  generated_by: 'docs/research/dcl-impact.mjs', rpc: RPC, block_height: block, dcl: DCL,
  method: 'quote({pool_ids:[pool_id], input_token:"wrap.near", output_token:<coin>, input_amount, tag:null}) at one block. impact_bps_vs_0_01_near compares tokens per NEAR with a 0.01 NEAR quote (the 1% pool fee cancels out). cost_bps_vs_spot_incl_fee compares with 1.0001^current_point, so it includes the 1% fee. A token buy tax (get_tax.buy_bps) is taken on delivery and is not in the quote.',
  coins,
};
writeFileSync(new URL('./dcl-impact.json', import.meta.url), JSON.stringify(out, null, 2) + '\n');
for (const c of coins) console.log(c.symbol, c.pool_total_wnear.toFixed(0), c.rows.map((r) => `${r.in_near}:${r.impact_bps_vs_0_01_near}`).join(' '));
console.log('block', block);
