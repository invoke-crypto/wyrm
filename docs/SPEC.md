# WYRM contract spec

This file is the contract between the parts of WYRM v2: the four contracts, the keeper, `verify` and the Lair. It describes the code as built in `contracts/`. The method tables below list every method each contract exports, and `node docs/check-docs.mjs` fails when a table and the code disagree.

The design and its rationale are in [design.md](design/v2/design.md). Its invariants, with the place each one is enforced, are in [invariants.md](design/v2/invariants.md).

Units. NEAR amounts are yoctoNEAR as `U128` strings in JSON. Times are nanoseconds as `U64` strings. Percentages are basis points (bps, 10,000 = 100%). Token ids are the strings `"1"` to `"100"`.

Toolchain. Rust 1.99.0 pinned in `contracts/rust-toolchain.toml`, `near-sdk` 5.29.1, `near-contract-standards` 5.29.1, `near-workspaces` 0.23.0. Release builds keep `overflow-checks = true`. `contracts/build.sh --reproducible` builds the four deployed contracts with `cargo near build reproducible-wasm`, and `verify/releases.json` holds the code hashes of each release.

## Accounts

| Account | Contract | Keys after deploy |
|---|---|---|
| `tithe.<root>` | `tithe` | none |
| `dragon.<root>` | `dragon` | none |
| `scales.<root>` | `scales`: NFT collection, sale, market and Council | none |
| `001.scales.<root>` to `100.scales.<root>` | `scale-vault`, one global contract by code hash | none |
| `ash.<root>` | no code. The Dragon burns WYRM by sending it here. | none |

`<root>` is `thewyrm.near` for WYRM. The rehearsal system WYRMLING used `wyrmling.thewyrm.near`, and the testnet drill system uses `council.thewyrm.testnet`. Every contract takes its peers' account ids in `new`, so one build runs every system.

Rules every contract follows:

- Every callback is `#[private]`. `obey` and `release` check their caller by hand.
- State changes before a cross-contract call, and the callback restores it when the call fails.
- Methods that move a token or an offer need exactly 1 yoctoNEAR attached. Payable methods that take a price take the price.
- `new` is `#[init]`, and deploy plus init happen in one batch transaction.
- Text inputs have byte limits. Collections that grow with user input are bounded or paid for by the user.
- Every action emits an NEP-297 event: `EVENT_JSON:{"standard":"wyrm","version":"1.0.0","event":"<name>","data":[...]}`.

## Presets

Each system runs one preset from the compiled table in `contracts/wyrm-types/src/preset.rs`. `new` takes the preset's name. Scales and the Dragon both report the preset's hash and `ENGINE_VERSION` (200), and the Dragon's `launch` refuses unless they match.

| Field | hoarder (WYRM) |
|---|---|
| Daily breath budget | 1,000 bps of the day's opening reserve. Council range 1,000 to 4,000. |
| Slice cap | 200 bps of price impact. Council range 50 to 200. |
| Dip steps | 2,000, 3,500 and 5,000 bps below the reference, with no extra budget |
| Cadence | one breath a day, at least 60 s between slices |
| Hunt | off. A Strict vote may switch it on with a sell multiple of at least 20,000 bps of average cost and 1 to 1,500 bps of the hoard a day. |
| Auto-burn | 0 bps. Council range 0 to 5,000. |
| Sale | 90 Scales at 2 NEAR each, 1,000 bps to the creator, open 14 days |
| Royalty | 500 bps of every resale, to the creator |
| Council times | 7 days of seasoning, 2 days of voting |

The other presets are `steady`, `dip-hunter`, `swift`, `harvester` and `drill`. [design.md section 3](design/v2/design.md#3-the-presets) lists them. `drill` has a 0.1 NEAR price, a 2-hour sale, 10 minutes of seasoning and 30-minute votes, and `Preset::allowed_on` refuses it on any account that does not end in `.testnet`.

The token tax is not a preset field. Every launch uses `envelope::TAX`: 400 bps on buys and on sells, with the whole creator side (10,000 bps) paid to the tithe.

## tithe

Nearly's `fee_to` for the coin. It splits every inflow 50/50 between the Dragon and the 100 Scale vaults. After the sale closes, the shares of unsold Scales go to the Dragon.

| Method | Caller | Behaviour |
|---|---|---|
| `new(dragon, scales, wrap, min_round, reserve)` | init | Registers the tithe on `wrap`. `min_round` must be at least 200 yocto. |
| `distribute()` | anyone | When the free balance reaches `min_round`, sends half to the Dragon and one hundredth of the other half to each vault. Once `retired_from` is set, the Dragon's transfer also carries the hundredths of the retired ids, and retired vaults get nothing. Emits `tithe_round`. Otherwise panics `below min_round`. |
| `retire(first_unsold)` | only `scales` | `first_unsold` is 1 to 90. Sets `retired_from` once. A repeat with the same value returns quietly, and a different value panics. |
| `unwrap()` | anyone | Reads the tithe's wNEAR balance and withdraws all of it as NEAR for the next round. |
| `on_wrap_balance(balance)` | private | Panics `nothing to unwrap` on a zero balance. |
| `forward_ft(token)` | anyone | Reads the tithe's balance of `token` and sends all of it to the Dragon. wNEAR is unwrapped, not forwarded. |
| `on_ft_balance(token, balance)` | private | Panics `nothing to forward` on a zero balance. |
| `get_stats()` | view | `{ rounds, paid_dragon, paid_scales, min_round, distributable, retired_from }` |
| `get_config()` | view | `{ dragon, scales, wrap, min_round, reserve }` |

A transfer to a vault that does not exist refunds to the tithe and joins a later round. The deploy creates all 100 vaults before the launch names the tithe, so this happens only in tests.

## scale-vault

One vault per Scale. It is written against `near_sdk::env` without the `#[near]` macros, so its 16.8 KB of code costs 1.68 NEAR to publish as a global contract once per network.

| Method | Caller | Behaviour |
|---|---|---|
| `new()` | init | `parent = predecessor`. Panics `already initialized` on a repeat. |
| `release(to)` | only the parent | Sends `balance - VAULT_RESERVE` to `to` and returns the amount. Takes exactly `{"to":"<account>"}`. |
| `get_parent()` | view | |
| `get_claimable()` | view | `balance - VAULT_RESERVE` |

`VAULT_RESERVE` is 0.01 NEAR, and scales funds each new vault with exactly that, so a fresh vault has nothing to claim.

## scales

One contract holds the NEP-171 collection, the primary sale, the market and the Council, because a vote, a sale and a listing all need the token's owner and hold start in the same receipt.

Ids 1 to 90 are sale Scales and vote. Ids 91 to 100 are the creator's, minted to the creator in `new`, and never vote. A Scale's hold tier comes from the time since its last change of owner: Ash under 3 days, Ember from 3, Molten from 7 and Obsidian from 14. `nft_token` returns the tier's media.

### Collection and vaults

| Method | Caller | Behaviour |
|---|---|---|
| `new(preset, dragon, tithe, creator, vault_code_hash, collection)` | init | Runs `preset.check()`, `allowed_on` and `collection.check()`. Mints 91 to 100 to `creator`. The sale starts Waiting. |
| `publish_vault_code()` | anyone, payable | The input is the raw vault wasm. It must hash to `vault_code_hash`. Publishes it as a global contract, and the caller pays the burn. |
| `create_vaults()` | anyone | Creates the next 25 vaults, keyless, on the global vault code. Repeat until 100 exist, then panics `all 100 vaults exist`. |
| `on_vaults_created(last)` | private | Counts the batch when its last vault answers `get_parent`. |
| `claim(token_id)` | owner, 1 yocto | Delists the Scale, locks it, and asks its vault to release to the owner. |
| `on_claimed(token_id, by, started_at)` | private | Records the claim in history and `claimed_total`, or a `Swept` entry when the release went to the Dragon. Clears the lock. |
| `get_vault(token_id)` | view | the vault account |
| `nft_transfer`, `nft_transfer_call` | owner or approved, 1 yocto | NEP-171. Refused while a claim is in flight. |
| `nft_resolve_transfer(...)` | private | NEP-171. A returned token gets back its sender's hold start. |
| `nft_token`, `nft_metadata`, `nft_total_supply`, `nft_tokens`, `nft_supply_for_owner`, `nft_tokens_for_owner` | view | NEP-171, NEP-177 and NEP-181 |
| `nft_approve`, `nft_revoke`, `nft_revoke_all` | owner, payable | NEP-178. At most 3 approvals per token. The deposit pays the approval's storage. |
| `nft_is_approved` | view | NEP-178 |
| `nft_payout`, `nft_transfer_payout` | view, and owner or approved | NEP-199. 500 bps to the royalty receiver, the rest to the owner. |

A claim in flight locks the Scale for up to 10 minutes: no transfer, listing, sale or offer acceptance. Every change of owner runs one hook that resets the hold start, clears the listing, revokes approvals and appends history. History keeps the last 16 entries per Scale.

### Sale

| Method | Caller | Behaviour |
|---|---|---|
| `open_sale()` | only `dragon` | Called by the Dragon at bind. Waiting becomes Open for the preset's sale length. In any other state it changes nothing and does not panic. |
| `buy_scale()` | anyone, payable | Needs the sale Open, before `ends_at` and under 90 sold, and at least the price attached. Mints the next sale id to the caller, pays `price * creator_bps / 10,000` to the creator and the rest to the Dragon, refunds any excess, then sweeps the new Scale's vault to the Dragon under the claim lock. |
| `close_sale()` | anyone | When the sale is Open and past `ends_at` or sold out, sets Closed. Then, on this call and every later one until the tithe answers, calls `tithe.retire(sold + 1)`. Returns `not due` or `closed`. |
| `on_retired()` | private | Records that the tithe was told. A failed `retire` changes nothing, so the next `close_sale` retries. |
| `sweep(token_ids)` | anyone | 1 to 20 unminted ids. Releases each vault to the Dragon. |
| `get_sale()` | view | `{ state, price, creator_bps, sold, opened_at, ends_at, retired_from, tithe_told }` |
| `get_sweepable()` | view | unminted sale ids whose vault exists |

Unminted ids never count in any vote. Before the sale closes, a vault's income goes to the Dragon through the mint sweep. After it closes, the tithe pays retired ids' shares to the Dragon directly.

### Market

The market is built into the collection, so the Scale never leaves the seller until a sale.

| Method | Caller | Behaviour |
|---|---|---|
| `list(token_id, price)` | owner, 1 yocto | price at least 0.1 NEAR |
| `delist(token_id)` | owner, 1 yocto | |
| `buy(token_id)` | anyone, payable | The deposit must equal the listed price. The Scale moves first, then the seller gets the price less the royalty, and the royalty receiver gets the royalty. |
| `make_offer(token_id)` | anyone, payable | `token_id` null is an offer for any Scale. The deposit less the 0.01 NEAR storage fee is the offer, at least 0.1 NEAR, held in escrow. At most 10 open offers per account and 20 per Scale. |
| `cancel_offer(offer_id)` | buyer, 1 yocto | refunds the offer |
| `accept_offer(offer_id, token_id)` | owner, 1 yocto | For an any-Scale offer the owner names the Scale. Pays like `buy`. |
| `get_listings(from, limit)`, `get_offers(from, limit)`, `get_offers_for(token_id)` | view | `get_offers_for` returns offers named for that Scale. Any-Scale offers are in `get_offers`. |

Escrow is the sum of open offers and the bonds of open proposals. Every write that adds state checks that the balance less escrow still covers storage.

### Council

A proposal is an `Order` for the Dragon or a `Signal`, a text poll that calls nothing. Votes are cast per Scale. A sale Scale may vote on a proposal when its owner has held it for the preset's season before the proposal was created. The electorate is the number of sale Scales sold when the proposal opens, so a silent or unseasoned Scale counts as a no-show.

| Order | Class | Effect |
|---|---|---|
| `SetBreath { budget_bps }` | Light | the daily budget, inside the preset's bounds |
| `SetSlices { cap_bps }` | Light | the slice cap, inside the bounds |
| `SetPosture { hunt: null }` | Light | Hunt off |
| `SetPosture { hunt: { sell_multiple_bps, max_sell_bps } }` | Strict | Hunt on, or a new Hunt line |
| `Grant { to, amount }` | Strict | WYRM from the hoard to `to` |
| `Burn { amount }` | Light | WYRM from the hoard to ash |
| `SetAutoBurn { bps }` | Light | the share of each buy sent to ash |

| Class | Passes when | Can open when |
|---|---|---|
| Light | turnout at least 30% of the electorate and more yes than no | electorate at least 10 |
| Strict | turnout at least two thirds of the electorate and at least 80% yes | electorate at least 45 and the sale is over |

| Method | Caller | Behaviour |
|---|---|---|
| `propose(kind, description, token_id)` | holder of a seasoned sale Scale, 1 NEAR bond plus 0.01 NEAR | `Order::check` against the preset, the class's electorate rule, a description of at most 1,000 bytes, one open proposal per proposer. Voting ends after the preset's voting time. |
| `vote(proposal_id, token_ids, approve)` | holder, 1 yocto | The proposal must be Open and before `ends_at`. Up to 25 of the caller's seasoned Scales, each counted once. |
| `finalize(proposal_id)` | anyone, at least 150 Tgas | Open and past `ends_at`. Returns the bond when turnout reached 30%, otherwise sends it to the Dragon. A failed vote ends Rejected and a passed Signal ends Carried. A passed Order becomes Executing, and scales calls `dragon.obey` with 100 Tgas in the same transaction. |
| `on_executed(proposal_id)` | private | `Executed` on the Dragon's `Executed` verdict. `LeashHeld { error }` with the Dragon's reason on a refusal, or "dragon call failed" when the call failed. |
| `get_council()` | view | the enabled orders with their classes, the class rules, the bounds and the timings |
| `get_due()` | view | Open proposals past `ends_at` |
| `get_proposal(id)`, `get_proposals(from, limit)`, `get_votes(id)` | view | |

No order moves NEAR, and no order changes a rule, a bound or a timing. A passed order exists only inside its finalize transaction, so nothing can be banked.

### Views

| Method | Caller | Behaviour |
|---|---|---|
| `get_config()` | view | `{ preset, preset_hash, engine_version, dragon, tithe, creator, royalty_receiver, vault_code_hash, vaults_created, royalty_bps, season_ns, claim_timeout_ns, storage_fee, escrow, sale }` |
| `get_scale(token_id)`, `get_scales(from, limit)` | view | `{ token_id, owner, voting, hold_start, tier, claimed_total, claiming_since, listed_price, vault }` |
| `get_history(token_id)` | view | the last 16 `Minted`, `Transferred`, `Claimed` and `Swept` entries |
| `eligible_now()` | view | sale Scales minted, the electorate a proposal opened now would have |

## dragon

The treasury. It launches the coin on Nearly as its creator, binds to it, holds a NEAR reserve and the WYRM hoard, buys in slices through the DCL pool, and obeys only bounded orders from scales. The Dragon sends NEAR to only four places: into a swap, to Nearly for the launch quote, back to the creator from the deposit of the same `launch` call, and into the 1-yocto and storage deposits that tokens require.

Peers, fixed at init: `wrap` (`wrap.near`), `dcl` (`dclv2.ref-labs.near`), `nearly` (`nearlytrade.near`), `tithe`, `scales`, `ash` and `creator`.

### Launch and bind

| Method | Caller | Behaviour |
|---|---|---|
| `new(preset, peers)` | init | Runs `preset.check()` and `allowed_on`, registers on `wrap`, and starts with the preset's strategy and no coin. |
| `launch(coin)` | `creator`, payable, at least 500 Tgas | No coin bound, 10 minutes since the last launch, and `CoinArgs::parse` passes. Views the tithe's and scales' configs, Nearly's `quote_launch` and `get_slug_count` in parallel. |
| `on_wiring(symbol, args, deposit)` | private | Checks that the tithe and scales name this Dragon and its creator, share its preset hash and engine version, that 100 vaults exist, and that the deposit covers the quote plus 0.086 NEAR. Then checks the pool at the coin's predicted id. A refusal returns the whole deposit and emits `launch_refused`. |
| `on_pool_checked(args, deposit, cost)` | private | Refuses when a pool already exists. Otherwise returns `deposit - cost` to the creator and calls Nearly's `launch` with 300 Tgas, `fee_to` the tithe, fee mode creator and the 400 bps tax. |
| `on_launch_sent()` | private | Emits `launch_sent {ok}` and moves no NEAR. `ok: false` means the launch may be parked: resume it, do not launch again. |
| `nearly_resume(launch_id)` | anyone, at least 300 Tgas, before bind | Resumes a parked launch as its creator. |
| `nearly_cancel_stuck(launch_id)` | `creator` | Cancels a launch parked before its dev buy, so its NEAR returns to the Dragon. After bind, any launch but the bound one. |
| `bind(launch_id)` | `creator`, at least 150 Tgas | No coin bound. Views Nearly's `get_launch`, `get_fee_to`, `get_fee_opts` and `get_tax`, 5 Tgas each. Writes no state. |
| `on_launch_record(launch_id)` | private | Checks that the record names this Dragon as creator, the tithe as `fee_to`, fee mode creator, the 400 bps tax and step Done. Then views the pool. |
| `on_pool_seen(coin)` | private | Checks the pool runs wrap and the coin, sets `coin` once, and calls `scales.open_sale()`. A second bind finds the coin set and does nothing. |
| `get_coin()` | view | `{ launch_id, token, pool_id, wrap_is_x }` or null. The one mark of the real coin. |

### Buying

| Method | Caller | Behaviour |
|---|---|---|
| `poke()` | anyone, at least 760 Tgas | `not bound` before bind, `in flight` while a step younger than 5 minutes runs. Otherwise one step of the machine below. The keeper attaches 1,000 Tgas. |
| `on_observe(step)` | private | Reconciles the hoard and the wNEAR, records a price sample at most once a minute, rolls the day, and decides: Hunt sell, dip buy, breath buy or rest. A rest may send the auto-burn. |
| `on_quote(step, plan)` | private | Picks the largest of up to 6 halvings that passes the slice cap and sends the swap with `min_out` 150 bps under its quote. |
| `on_swap(step, ticket)` | private | Settles the swap and logs the slice, or logs `swap_failed` and returns the unused budget. |
| `ft_on_transfer` | NEP-141 receiver | The bound coin joins the hoard as donated, and wNEAR joins the reserve. Before bind only wNEAR is accepted. Any other token panics. |

The reserve is the Dragon's NEAR above its storage plus its wNEAR. Sale money, royalties and tithe rounds arrive as plain transfers. A buy spends wNEAR first and wraps only the shortfall.

The machine:

1. Observe. Batch the coin and wNEAR balances, `dcl.get_pool` and the quote for 1 NEAR, then `on_observe`.
2. Decide. The breath minute of each UTC day comes from `env::random_seed()` at the day's first observation. From that minute the day's budget may be spent, slice by slice, at least 60 s apart. A dip below a confirmed step (3 samples over at least 10 minutes) also buys, and the preset's extras add budget. No buy happens without a 24-hour reference, or while the price is more than 15% above it.
3. Quote. A slice of `a` NEAR is allowed when `quote(a) * 10,000 >= a * tokens_per_near * (10,000 - cap_bps)`. The smallest slice is 0.01 NEAR of value.
4. Swap. `wrap.ft_transfer_call` to DCL with 180 Tgas. A buy's `min_out` is on the gross quote, and the Dragon books only the net arrival, 4% less after the buy tax. A sell is quoted and its `min_out` set on the net input.

A step older than 5 minutes may be replaced. Its late callbacks see another step and change nothing, except that a late swap result still corrects the day's spend.

### Council and views

| Method | Caller | Behaviour |
|---|---|---|
| `obey(order)` | only `scales` | Runs the order through `Order::check` and the outflow windows. A refusal emits `order_refused {reason}` and answers `Refused { reason }` with no change. Settings orders write the strategy and answer `Executed`. Grant and Burn reserve the amount from the hoard and send it, and `on_outflow` settles them. |
| `on_outflow(seq)` | private | A failed transfer restores the amount to the hoard and frees its window share. |
| `get_config()` | view | `{ preset, preset_value, preset_hash, engine_version, peers, strategy, coin, engine }`. `engine` holds the tax, spike guard, slippage, dip confirmation, outflow caps, stale time and launch margin. |
| `get_state()` | view | `{ mood, phase, coin, reserve, reserve_near, reserve_wnear, spot, reference, hoard, day, strategy, dip, last_slice_at, last_launch_at, pending, samples, log_next }` |
| `get_log(from, limit)` | view | the last 256 events |
| `get_samples(limit)` | view | `[{ at, tokens_per_near }]`, at most 1,440 |

Grants and burns each take at most 50 bps of the hoard per order and 100 bps of it in any rolling 30 days, measured on the hoard at the time of each order. At most 3 outflows may be in flight. An outflow whose callback has not run after 10 minutes counts as settled at the next observation. `mood` is Sleeping, Stirring, Breathing, Gorged or Hunting.

## Events

| Contract | Events |
|---|---|
| tithe | `tithe_round {round, to_dragon, per_scale, dust}` |
| scales | `scale_minted`, `scale_swept`, `scale_claimed`, `scale_listed`, `scale_delisted`, `scale_sold`, `offer_made`, `offer_cancelled`, `sale_opened`, `sale_closed`, `tithe_told`, `proposal_created`, `vote_cast`, `proposal_finalized`, `proposal_executed`, and the NEP-171 `nft_mint` and `nft_transfer` |
| dragon | `observe`, `breath_slice`, `dip_slice`, `hunt_slice`, `swap_failed`, `grant_paid`, `burn_paid`, `auto_burn`, `outflow_failed`, `settings_changed`, `order_refused`, `launch_refused`, `launch_sent`, `bind_refused`, `bound` |

## Event fields read by verify

A missing field fails the check that reads it.

| Event | Fields |
|---|---|
| `tithe_round` | `round, to_dragon, per_scale, dust` |
| `breath_slice` | `at, breath_at, amount_in, quoted_out, min_out, tokens_per_near, reference, cap_bps, budget` |
| `dip_slice` | the `breath_slice` fields plus `step_bps, confirmations, confirmed_since` |
| `grant_paid`, `burn_paid` | `at, amount, hoard` (the hoard when the order was admitted), and `to` for a grant |
| `scale_minted` | `token_id, price` |
| `scale_swept` | `token_ids` |
| `proposal_finalized` | `proposal_id, status` |

## Refusal messages

The keeper reads these panics as "nothing to do".

| Contract | Method | Message |
|---|---|---|
| tithe | `distribute` | `below min_round` |
| tithe | `unwrap` | `nothing to unwrap` |
| tithe | `forward_ft` | `nothing to forward` |
| dragon | `poke` | `not bound` or `in flight` |
| scales | `create_vaults` | `all 100 vaults exist` |
| scales | `finalize` | `not open` or `voting still open` |

## Gas

Measured gas is in [GAS.md](GAS.md). On mainnet, WYRMLING's launch reached Done with 500 Tgas attached and 85.11 Tgas burnt, and its first buy chain against the real DCL completed within the 1,000 Tgas the keeper attaches ([THREAT-MODEL.md](THREAT-MODEL.md)).
