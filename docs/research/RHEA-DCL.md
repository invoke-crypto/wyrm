# Rhea DCL as the Dragon uses it

Research of 2026-11-03 on mainnet. Read-only: view calls on `https://rpc.mainnet.fastnear.com`, transaction status on `https://archival-rpc.mainnet.fastnear.com`, and the nearblocks API. A transaction is linked as `https://nearblocks.io/txns/<hash>`. A view is written as `account.method(args)` with the block height it answered at.

## The contract

- `dclv2.ref-labs.near.get_metadata()` returns version `2.4.1`, owner `ref-finance.sputnik-dao.near`, `protocol_fee_rate: 2000`, state `Running` (block 222922801).
- Nearly's audit measured gas against DCL 2.3.13 and 2.3.14 ([Nearly SPEC.md](https://github.com/sam3dsol/Nearlytrade/blob/3d59877/SPEC.md), section 1). DCL has been upgraded since.
- DCL code hash `3G34Nhjmir5JPYPtijaA9iCNnHDPR7h3wfC9H77woEGo`, version 2.4.2, 1,166,649 bytes (view_code, block 223362241, Nov 6). On Nov 3 it was version 2.4.1 with code hash `BzhtCCGJibs9ykCZUuMZ9Diq3gFAi4ZGeMHgEuVqvoiA` (block 222922050). The wasm has no `__contract_abi` export, so the shapes below come from live calls.

## get_pool

- Args are `{"pool_id": "<x>|<y>|<fee>"}`.
- `get_pool({"pool_id":"nearly-993927.nearlytrade.near|wrap.near|10000"})` (block 222922935) returns `pool_id, token_x, token_y, fee, point_delta, current_point, liquidity, liquidity_x, max_liquidity_per_point, total_fee_x_charged, total_fee_y_charged, volume_x_in, volume_y_in, volume_x_out, volume_y_out, total_liquidity, total_order_x, total_order_y, total_x, total_y, state, whitelist, tax`. Amounts are decimal strings. `current_point` is an integer. `state` is `"Running"`.
- An id with the tokens in the wrong order is not an error. `get_pool({"pool_id":"wrap.near|nearly-993927.nearlytrade.near|10000"})` returns `null` (block 222922945).
- `total_fee_*_charged` is the protocol part only. For NEARLINGS it is 0.2% of `volume_y_in` while the locker claimed 0.8% (see NEARLY.md).

## quote

- Args are `{"pool_ids":[id], "input_token", "output_token", "input_amount", "tag"}`. It returns `{"amount": "<u128>", "tag": <tag echoed>}`. `tag` may be omitted (block 222922947) or a string, which comes back unchanged (block 222922939).
- 1 NEAR into NEARLY returns `{"amount":"2880883214006427516440","tag":null}` (block 222922937).
- A wrong input token returns `{"amount":"0"}`, not a panic (block 222922943).
- A pool that does not exist panics with `E403: pool not exist` (no block, the view failed).
- An input larger than the pool can fill returns a partial amount, not zero. 1e9 NEAR into NEARLY returns 65,309,427 tokens, about all of `total_x` (block 222922941).
- Decimals. Nearly tokens have 18 decimals and wNEAR has 24. `nearlytrade.near.get_quotes()` lists wrap.near with `decimals: 24`. Every launch has `total_supply` `1e27`, which is 1e9 tokens with 18 decimals (`get_launches`, block 222922037).
- The quote includes the 1% pool fee. At 0.1 NEAR every pool costs 100 to 101 bps against `1.0001^current_point` (docs/research/dcl-impact.json).

### Cross-contract quote and its gas

`quote` is a read-only method, and contracts call it in receipts. Measured `gas_burnt` of the DCL receipt:

| Tx | Caller | Pool | Input | Attached | Burnt |
|---|---|---|---|---|---|
| [APvwcRvc…](https://nearblocks.io/txns/APvwcRvcShgsR8bVi47AgvyiJLzSy2GsnYax8pX8BXpz) | nekoorders.near | sherifchenko (Nearly) | 0.199 wNEAR | 20 T | 9.458 T |
| [37ihSB8d…](https://nearblocks.io/txns/37ihSB8dfTH83jQFt25GUPGZioumzCZLd2yk4Z3rFdQo) | nearpaid.near | npaid-831d2b | token in | 20 T | 12.498 T |
| [35ZFXJHv…](https://nearblocks.io/txns/35ZFXJHvUVS78gzWTqtK4V2Jj7QS45q1UE9AktHRNC2E) | nearpaid.near | npaid-831d2b | token in | 20 T | 12.299 T |
| [AGfLy9Qg…](https://nearblocks.io/txns/AGfLy9Qg9AGamahfTY4y6T5nLbQRPUcjnp7kBjJwu7JX) | fakeowl (Rhea launchpad) | fakeowl | token in | 240.6 T | 5.240 T |
| [Hg7HtxLi…](https://nearblocks.io/txns/Hg7HtxLiHXr855UKuEfAFcbvtxvKW5arJ9XTDjj7D1AY) | router.shardsmarket.near | qtc.omft | token in | 40 T | 4.855 T |

The Dragon attaches 10 Tgas (`GAS_QUOTE`, contracts/dragon/src/lib.rs:25). Two of the five real quotes burnt more than 10 Tgas. Quote gas grows with the points a swap crosses, so a large halving candidate costs more than these small quotes (inferred). Nothing above measures quotes of 25 to 100 NEAR.

Parallel quotes. `quote` does not write state, so nothing in DCL orders or locks parallel calls (inferred from the method being a view and from the five callers above). DCL has no on-chain rate limit for `quote` or swaps. Its only gates are the contract pause (`E004`), the pool pause (`E406`), a pool whitelist (`E407`, `whitelist: null` on NEARLY at block 222922935), frozen tokens (`E010`) and a user blacklist (`E017`). These are error strings in the DCL wasm. The public RPC rate-limits callers (fastnear answered `-429` during this research). That limits the keeper, not the contract.

## Swap by ft_transfer_call

The message, from the real calls below:

```json
{"Swap":{"pool_ids":["<pool_id>"],"output_token":"<token>","min_output_amount":"<u128>"}}
```

Optional fields seen in mainnet calls are `swap_out_recipient`, `skip_unwrap_near` and `referral_id`. `client_echo` is an error-string name in the wasm.

### A contract's swap: Nearly's buyback

[E6VWDuAY…](https://nearblocks.io/txns/E6VWDuAYU1rGnLaVcW4eRNRETLwLX8qzpQhreBkUN8Vk). nearlytrade.near spends 0.0853 NEAR on NEARLY.

| Receipt | Burnt | Note |
|---|---|---|
| wrap.near `near_deposit` + `ft_transfer_call` (one batch, 180 T attached) | 4.036 T | the same batch shape as the Dragon's buy |
| DCL `ft_on_transfer` (150 T it was given) | 8.099 T | returns `"0"`, nothing unused. `swap` event `total_fee` is 1% and `protocol_fee` is 20% of it |
| DCL to NEARLY `ft_transfer` to the buyer (131 T) | 1.523 T | the output, in its own receipt |
| DCL `callback_post_withdraw_asset` | 2.469 T | returns `true` |
| wrap.near `ft_resolve_transfer` | 1.323 T | returns the used amount |
| whole transaction | 42.533 T | includes Nearly's own callbacks and burn |

nearlytrade.near is not registered on DCL. `dclv2.ref-labs.near.storage_balance_of({"account_id":"nearlytrade.near"})` returns `null` (block 222922804). So a Swap by `ft_transfer_call` needs no DCL storage. DCL's own `storage_balance_bounds` min is 0.5 NEAR (block 222922806), and that applies only to accounts that keep balances or positions inside DCL.

### Ordinary user swaps

- [2ELxeSu6…](https://nearblocks.io/txns/2ELxeSu67hRyMYUuwnBBdRMme5QwPyjsF894TJetxL3N), elcsabito.near buys BATMAN for 130 NEAR. wrap batch 4.074 T, DCL `ft_on_transfer` 14.119 T, output `ft_transfer` 1.924 T, callback 2.503 T, `ft_resolve_transfer` 1.361 T, total 25.622 T. BATMAN is a tax token. The buyer got 364,901 tokens of the 368,587 `amount_out`, and 1% went to the token with memo `tax`.
- [4oUCVkmH…](https://nearblocks.io/txns/4oUCVkmHiucUe9osyk1eUmjtsWVvYWP2hHFjyFwrfpbr), a bot buys BATMAN for 15.9 NEAR. DCL `ft_on_transfer` 14.051 T, total 25.566 T.
- [GEuvALaU…](https://nearblocks.io/txns/GEuvALaUD4hK3qU3zXTYf98GkWmHKmEQ1GhnAa83JPFW), feeswap.near sells BATMAN for wNEAR with `skip_unwrap_near: true`. DCL `ft_on_transfer` 12.353 T. The output arrives as a wrap.near `ft_transfer`.

### Output delivery

The output is a separate receipt. DCL calls `<output_token>.ft_transfer(buyer)` and then its own `callback_post_withdraw_asset`. This runs beside wrap's `ft_resolve_transfer` (every trace above). The caller's callback on `ft_transfer_call` sees only the used input amount, never the output amount. The buyback reads the bought amount with a later `ft_balance_of` (receipt `D17tN339` in E6VWDuAY…).

The buyer must be registered on the output token, or the `ft_transfer` fails. DCL then has `callback_post_withdraw_asset` and `lostfound` methods for the failed case (method names in the wasm exports). I did not observe a failed delivery. `get_lostfound_count()` is `"0"` (block 222922810).

### min_output failure

[FAztisRM…](https://nearblocks.io/txns/FAztisRMbaCTRL3cpnBzwnEPvKXngE9ZR2KceyvDsiS6). DCL `ft_on_transfer` fails with `Smart contract panicked: E204: slippage error` after burning 13.047 T. wrap's `ft_resolve_transfer` logs `Refund … from dclv2.ref-labs.near to safenode8556.near` and returns `"0"`. The refund is wNEAR, not NEAR. When `near_deposit` ran in the same batch, it is not undone, because only the DCL receipt failed.

## Pool ids and fee tiers

- `get_fee_tiers()` returns `{"100":1,"400":8,"1000":20,"2000":40,"10000":200,"20000":200,"30000":200,"40000":200}`, fee to `point_delta` (block 222922808).
- Every Nearly pool uses fee 10000, which is 1% with point_delta 200. Nearly's config has `pool_fee: 10000` (`nearlytrade.near.get_config()`, block 222922088). The factory builds `format!("{}|{}|{}", a, b, pool_fee)` with the pair sorted lexicographically (Nearly factory source, contracts/factory/src/lib.rs:915 at e82bfa0).
- WYRM would be `wyrm.nearlytrade.near` while the slug is unused (`get_slug_count({"slug":"wyrm"})` is 0, block 222923503). `"wrap.near" < "wyrm.nearlytrade.near"`, so its pool is `wrap.near|wyrm.nearlytrade.near|10000` and wNEAR is token x. A launch named `wyrm-2` would sort the same way.

## Dragon and mock-dcl against real DCL

Ranked by severity.

1. **High. Quote gas is too low.** `GAS_QUOTE` is 10 T. Real cross-contract quotes burn 4.9 to 12.5 T at small sizes. A quote that runs out of gas reads as `None`. A failed 1-NEAR quote means no price and no buy. Failed halving quotes skip the slice. Fix: attach 30 T per quote (the observed callers attach 20 to 40 T). Then rederive `GAS_ON_OBSERVE`, `GAS_POKE_MIN` and the keeper's attached gas against the 1 PGas transaction limit. Measure quote gas on real DCL 2.4.1 wasm in the sandbox before you settle the numbers.
2. **Medium. The swap gets less gas than every real caller.** `GAS_SWAP` is 150 T. Nearly's buyback and the users attach 180 T to wrap's `ft_transfer_call`, and DCL's `ft_on_transfer` then had 150 T. With 150 T, DCL receives roughly 120 T (inferred). DCL burnt at most 14.1 T in these traces and passed 125 to 131 T on to the output `ft_transfer`. I cannot confirm from these traces that 120 T is enough. Fix: attach 180 T, and raise `GAS_ON_QUOTE` by the same 30 T.
3. **Medium. Sell output may arrive as native NEAR.** Every observed swap into wrap.near passes `"skip_unwrap_near": true` (GEuvALaU…, HPLmctuv…, the nearpaid SwapByOutput in 37ihSB8d…). The DCL wasm has `callback_post_withdraw_near` and `near_withdraw`. I did not prove DCL's default when the flag is absent. Fix: add `"skip_unwrap_near": true` to the Sell message in `dcl::swap_args`. Real DCL accepts the field, so this is safe either way.
4. **Low. Quote semantics on overflow.** Real `quote` returns a partial fill for an input the pool cannot fill. mock-dcl's constant-product pool never runs dry. The Dragon's cap check rejects such a slice because its impact is huge, so nothing breaks. Fix: update the comment "A quote of zero means DCL cannot fill the amount" in dcl.rs. Optionally make the mock cap output at reserves.
5. **Low. Missing pool.** Real `quote` panics `E403` and real `get_pool` returns `null`. mock-dcl returns `amount: 0`. The Dragon maps both to `None`, so no change is needed. Make mock `quote` panic for a missing pool to match.
6. **Low. Token tax is not modelled.** On a taxed Nearly token, DCL's `ft_transfer` delivers `amount_out` minus the tax (2ELxeSu6…). mock-ft has no tax. The Dragon reconciles from `ft_balance_of`, so it books what really arrived. Fix: launch WYRM without a tax, or add a tax switch to mock-ft.
7. **Low. Registration.** The Dragon attaches 0.125 NEAR to `storage_deposit` with `registration_only: true` (lib.rs:35). wrap.near and Nearly tokens need 0.00125 NEAR (`storage_balance_bounds`, blocks 222923530 and 222923532), and NEP-145 refunds the rest. No DCL registration is needed (see above).
8. **No change needed.** These all match real DCL: quote and get_pool argument names, the `{amount, tag}` result, the `total_x`/`total_y`/`state` fields, `"Running"`, the `Swap` message keys, `U128` strings, the `"0"` unused-amount return, the `E204` panic and refund, the separate output receipt, the sorted pool id, and 24 vs 18 decimals (the Dragon's math is decimal-free and 256-bit wide in math.rs).

## Price impact

See `docs/research/dcl-impact.json`, written by `docs/research/dcl-impact.mjs` at block 222923780. Buy impact in bps against a 0.01 NEAR quote:

| Coin | Pool wNEAR | 1 | 5 | 10 | 25 | 50 | 100 NEAR |
|---|---|---|---|---|---|---|---|
| NEARLY | 19,819 | 0 | 2 | 4 | 10 | 21 | 43 |
| BATMAN | 29,577 | 0 | 0 | 1 | 3 | 7 | 14 |
| SINGULARTY | 3,598 | 2 | 10 | 21 | 53 | 106 | 210 |
| NINU | 2,861 | 3 | 13 | 26 | 64 | 126 | 250 |
| DRAGON | 1,468 | 5 | 21 | 42 | 103 | 203 | 398 |
| XELA (no trades yet) | 0 | 9 | 48 | 96 | 236 | 462 | 884 |

Every pool except NEARLY has the same in-range `liquidity`, `50498695734733140569374`, which is Nearly's locked single-sided position. Impact depends on the price the coin has reached. A fresh coin is the thinnest. With the default 250 bps cap, a fresh WYRM pool allows a slice of about 25 NEAR (XELA row).
