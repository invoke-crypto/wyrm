# Nearly (nearly.trade) facts for WYRM

Research of 2026-11-03 on mainnet. Read-only. The views ran on `https://rpc.mainnet.fastnear.com` with the block height given. A transaction is linked as `https://nearblocks.io/txns/<hash>`.

## Source

- The factory is `nearlytrade.near` with code hash `9mwfrbtBbDr3y2t9rHY944qAQTU2PsB5smqsAHZJo4G5` (view_code, block 222922048).
- The source is public at [sam3dsol/Nearlytrade](https://github.com/sam3dsol/Nearlytrade). Its README maps that exact hash to commit `e82bfa0` ("fees in NEAR, 60 / 20 / 10 / 10 split"), deployed by DAO proposal #17 in tx [2uraygxP…](https://nearblocks.io/txns/2uraygxPDLGtiuY9rHhHGWx2AKG1KNWYCNDnNzk1dfTc) on 2026-10-19.
- The wasm's `__contract_abi` matches the source. The line numbers below are `contracts/factory/src/lib.rs` at `e82bfa0`.

## The market at launch

- There is no bonding curve. Each launch creates the DCL pool and puts the whole 1e9 supply into it as one single-sided position owned by a keyless locker. The steps are `CreateToken → (CreatePool) → AddLiquidity → (DevBuy) → Done` (Nearly SPEC.md section 2, README line 3).
- The DCL pool is created inside the launch transaction, in parallel with the token (SPEC "CreateToken"). Trading opens once the position is placed. `step: "Done"` and a non-null `lpt_id` mean live (`get_launches`, block 222922037, for example launch 3647 with `lpt_id "forever.nearlytrade.near|prl.omft.near|10000#24517"`).
- Grief risk. Anyone can pre-create the pool for a predictable token name at another price. The launch then fails (SPEC line 139).

## launch(args)

- `launch` is payable and takes `{ args: LaunchArgs }` (ABI). The required fields are `name` and `symbol`. The optional fields are `fee_to`, `dev_buy`, `quote`, `fee_mode` (`creator|burn|holders`), `creator_share_bps`, `creator_paid_in` and `holders_paid_in` (`near|pair`), `tax`, `init_point`, `icon`, `description` and `links`.
- Attach `quote_launch(icon_bytes, dev_buy, tax, text_bytes).total` and 300 TGas (SPEC "launch").
- `creator = env::predecessor_account_id()` (lib.rs:792). A contract can call `launch` and becomes the creator.
- `fee_to` may not be the factory or the tax seller. A `fee_to` equal to the creator is dropped (lib.rs:828-830). It is fixed for life (SPEC invariant 23).
- The dev buy is optional. `dev_buy` defaults to 0 (lib.rs:857). It works on the NEAR pair only. It is capped at the NEAR that buys `max_dev_buy_bps` (400, so 4%) of supply (`get_config`, block 222922088).
- In the launch transaction the dev buy runs whoever signed (SPEC invariant 14). From `resume` only the creator, the owner or a relayer may trigger it (lib.rs:4438). The bought tokens go to the creator (`swap_out_recipient = creator`, SPEC "Dev buy in-line"). So a launcher contract receives them and has to pass them on to the Dragon.
- `creator_share_bps`, when given, must equal the current share, 6000 (lib.rs:823). `get_creator_share_bounds()` returns `[6000, 6000]` (block 222922093).
- Tickers are not reserved. `unique_symbols: false` (`get_config`, block 222922088). The token account is `<slug>.nearlytrade.near`, then `<slug>-2`, `-3`, and so on (lib.rs:4773). `get_slug_count({"slug":"wyrm"})` returns 0 and `get_launch_by_symbol({"symbol":"WYRM"})` returns `null` (blocks 222923503 and 222923505). `dragon` is already at 7 (block 222923507). If anyone launches WYRM first, ours becomes `wyrm-2.nearlytrade.near`. So the Dragon's `pool_id` can only be fixed after our launch lands.

## Fees and the creator's share

- DCL takes 1% on every swap and keeps 20% of it (`protocol_fee_rate: 2000`). The locker's position earns the other 0.8%.
- Measured on claimed fees, `fees_near_total / volume_y_in` is 0.00800 for NEARLINGS (launch 3270), DRAGON (1502) and DRAGONS (3403). The pool's `total_fee_y_charged` is 0.002 of the same volume (`get_launch`, `get_pool`, block about 222923600).
- The creator gets `creator_share_bps` of what the locker claims, on both legs (SPEC "Split"). The rest goes to protocol, referral pot and holders pot, split 60 / 20 / 10 / 10 since e82bfa0 (FIXES.md 2026-10-19).
- Real share on recent launches. `get_fee_opts` gives `creator_share_bps: 6000` for 3643 through 3647 (block about 222923510). That is 60% of 0.8%, or 0.48% of the NEAR side of volume. Older launches keep their split: DRAGON (1502) is at 8000.
- The coin side of the creator's fees (0.8% of sells, times 60%) is sold to NEAR by Nearly's keeper and seller. "A creator is never paid the launch token: NEAR, or the pair token if chosen at launch" (FIXES.md, commit d4804bc). The NEAR arrives through `fee_proceeds`.
- `get_tax` returns `null` for a launch without tax (launch 3646). Otherwise it returns `{buy_bps, sell_bps, creator_bps, burn_bps, holders_bps, platform_bps, pending, holders_bucket, burned, paid_creator, paid_holders, paid_platform, selling, floor}`. Launches 3643, 3644 and 3647 use 200 to 300 bps a side with `platform_bps: 4000`, so the platform takes 40% of the tax. Launch 3645 splits its tax 3300 creator, 3300 burn, 3400 holders (block about 222923510).

## How and how often fees reach fee_to

- The claim runs about hourly. `claim_fees` is signed by `nearlytrade.near`, and 90 of its last 100 calls fall in minutes :05 to :09 of the hour across 22 distinct hours (nearblocks `txns?method=claim_fees`, 2026-11-02 22:35 to 2026-11-03 20:08).
- The push also runs about hourly, not every 15 minutes. 170 of the last 200 `push_creator_fees` calls fall in minutes :10 to :14, across 89 distinct hours in 104 h (2026-10-30 12:11 to 2026-11-03 20:12, all signed by `nearlytrade.near`).
- Anyone may call `push_creator_fees(account_id)`, unless the factory is paused (SPEC "Roles"). So our keeper can push to the tithe at any time. `claim_creator_fees` needs the payee itself as predecessor, so a keyless tithe cannot use it. `min_push` and `min_claim` are 0.05 NEAR (`get_fee_split`, block 222922090).
- The asset is native NEAR by plain `Transfer`. `internal_pay` with kind `near` is `Promise::new(to).transfer(amount)` (lib.rs:3697).
- Real fee_to example. `nearlingsdevfees.near` is `fee_to` of launch 3270 (`get_fee_to_many`, block about 222922400). Its incoming receipts are all plain transfers from `nearlytrade.near`. Two examples:
  - [12Ni8fHq…](https://nearblocks.io/txns/12Ni8fHqGsikhJPTo19c1ZmUqY4YsULv6gfA7R3frzvu) is `push_creator_fees`, event `creator_fees_claimed` amount 0.0864 NEAR, then `Transfer 86399999999999999999999`.
  - [H93VVuv1…](https://nearblocks.io/txns/H93VVuv1WB9bARpV9o1MNd2TKrujNVBEa2Afe1MXf7xb) pays 0.8832 NEAR the same way.
  - Six transfers between 10-19 and 10-24 total 1.54 NEAR.
- Another payee, [3Wpo9h54…](https://nearblocks.io/txns/3Wpo9h54bwj3Abf61DkyBBohQTwg8r6NLRsT9iZbkP9i), gets 0.0777 NEAR by `Transfer`. The whole transaction burns 14.5 T.
- wNEAR never reaches a NEAR-pair payee. Only an FT-pair launch pays its creator in the pair token (`creator_quote_fees`, SPEC "Split").
- One path can pay the launch token itself. `push_creator_token_fees(id, as_booked: true)` is keeper-only and pays the booked token fees as they are, by `ft_transfer` (lib.rs:1661). The factory registers the receiver on the token first: `nearlytrade.near → batman-4 storage_deposit,ft_transfer` in [6UEy5BLK…](https://nearblocks.io/txns/6UEy5BLKGKYQRUpHSohhB5beRufQtJqqgxGCVpj8yZaS).

## What this means for the tithe and the Dragon

- The tithe receives native NEAR, so `distribute` works as written. `unwrap` will normally have nothing to do on a NEAR-pair launch. It is harmless and covers wNEAR someone sends by hand.
- `forward_ft` covers the keeper's `as_booked` token path, and the factory registers the tithe on the token itself.
- Hourly pushes are 0.05 to 1 NEAR in the observed payees. With `min_round` of 1 NEAR, rounds will be hours or days apart at low volume.
- The keeper can call `push_creator_fees("tithe.thewyrm.near")` before `distribute`.
- The operator chose a 4% tax on Nov 4 (see "The token tax" below). A token tax cuts every Dragon buy on delivery, as BATMAN's 1% `tax` memo shows in [2ELxeSu6…](https://nearblocks.io/txns/2ELxeSu67hRyMYUuwnBBdRMme5QwPyjsF894TJetxL3N).
- Launch with `fee_mode: "creator"`, the default. Burn and holders modes divert the creator share away from `fee_to` (SPEC "Split").
- Initialize the Dragon only after the launch, with the token account that `get_launch_by_symbol` or the `launch` event reports.

## The token tax (checked 2026-11-04 in source at e82bfa0)

The line numbers are `contracts/token-tax/src/lib.rs` (token) and `contracts/factory/src/lib.rs` (factory).

- The tax is taken in the token itself, on transfers that touch the pool.
  - A transfer from a pair (the DCL) is a buy and pays `buy_bps`. A transfer to a pair is a sell and pays `sell_bps` (token `tax_for`, 583-599). `pairs` is the launch's DCL (factory 4496).
  - A wallet-to-wallet transfer pays nothing.
  - The admin (the factory), the token, the locker and the tax seller are exempt. Nobody else is, so the Dragon pays on both its buys and its sells.
- The tax is taken from the moving amount. The receiver gets `amount - tax`, and the tax tokens sit on the token account (token `internal_transfer`, 636-671). On a sell, the token takes 4% of what the Dragon sends into the pool, so the pool swaps only the net.
- `MAX_TAX_BPS` is 400 (factory 288), so 4% a side is the most Nearly allows. `creator_bps + burn_bps + holders_bps` must equal 10,000 (factory 842).
- The factory pulls the tax tokens with `tax_take` and sells them for NEAR. It splits the NEAR as follows (factory 4150-4186):
  - The platform gets `PLATFORM_TAX_BPS`, which is 4000 on observed launches.
  - The remaining 60% is split by `creator_bps`, `holders_bps` and `burn_bps`.
  - The creator part goes into `creator_fees[payee]`, where `payee` is `fee_to`, or the creator when `fee_to` is unset (factory 5209). This is the same bucket the pool-fee share uses, so `push_creator_fees` pays both to the tithe in one plain transfer.
  - If paying the creator part fails, it moves to the holders bucket (factory 1800).
- For WYRM use `tax: {buy_bps: 400, sell_bps: 400, creator_bps: 10000, burn_bps: 0, holders_bps: 0}`. The tithe then gets 60% of all tax, which is 2.4% of taxed volume, on top of 0.48% of the NEAR side from the pool fee.

## The launch id after `launch`

- `launch` returns the step chain's callback, `PromiseOrValue<bool>`, not the launch id. The id appears only in the `launch_started` event, which a contract cannot read.
- `get_launch(id)` returns the full `Launch` record, which includes `id`, `token`, `creator`, `pool_id` and `quote` (factory 394-441, 2818). `get_launch_by_token(token)` finds the same record by token account.
- So `bind` takes the launch id as an argument and verifies the record:
  - `creator` is the expected creator.
  - `get_fee_to(id)` is this tithe.
  - `get_fee_opts(id).mode` is `creator`.
  - `get_tax(id)` matches the preset.
  - `step` is `Done`.
  - It then takes `token` and `pool_id` from the record. A wrong id fails one of these checks, so the caller cannot bind to a coin that doesn't pay the tithe.

## Open

- I did not measure gas for a `launch` called by a contract.
- I saw no payout to a contract `fee_to`. `neariavault.near` (launch 2613) is a candidate to check.
- Nearly's seller sells the coin-side fees on its own schedule (`get_fee_selling`). I did not measure how often that NEAR reaches `creator_fees`.
