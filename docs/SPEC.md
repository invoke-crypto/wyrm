# WYRM contract spec

This file is the contract between the parts of WYRM. Each contract, the Lair, the keeper and `verify` build against the names and shapes here. Change a name here first, then in code.

Units. NEAR amounts are yoctoNEAR as `U128` strings in JSON. Times are nanoseconds as `U64` strings (`env::block_timestamp()`). Percentages are basis points (`u16`, 10,000 = 100%). Token ids are the strings `"1"` to `"100"`.

Toolchain. Rust 1.99.0 pinned in `contracts/rust-toolchain.toml`, `near-sdk` 5.29.1, `near-contract-standards` 5.29.1, `near-workspaces` 0.23.0, `cargo-near` 0.22. Release profile keeps `overflow-checks = true`.

## Accounts

| Account | Contract | Keys after launch |
|---|---|---|
| `tithe.<root>` | `tithe` | none |
| `dragon.<root>` | `dragon` | none |
| `scales.<root>` | `scales` (NFT collection, sale, market, Council) | none |
| `001.scales.<root>` to `100.scales.<root>` | `scale-vault`, one global contract by code hash | none |

`<root>` is `thewyrm.near` for WYRM and `wyrmling.thewyrm.near` for the rehearsal. Every contract takes its peers' account ids in `new`, so the same code runs both coins and the sandbox.

Every contract follows the same NEAR rules, taken from the Council-PM audit:

- Every callback is `#[private]`.
- State changes before any cross-contract call, and a failed call restores it in the callback.
- Methods that move value or change ownership need exactly 1 yoctoNEAR attached, except payable methods that take a price.
- `new` is `#[init]` and deploy plus init happen in one batch transaction.
- Text inputs have byte limits. Collections that grow with user input are bounded or paid for by the user.
- Every action emits an NEP-297 event `EVENT_JSON:{"standard":"wyrm","version":"1.0.0","event":"<name>","data":[...]}`.

## tithe

Nearly's `fee_to`. Splits every inflow 50/50 between the Dragon and the 100 Scale vaults.

```rust
struct Tithe {
    dragon: AccountId,
    scales: AccountId,        // vault i is format!("{:03}.{}", i, scales)
    wrap: AccountId,          // wrap.near
    min_round: NearToken,     // 1 NEAR
    reserve: NearToken,       // kept back for storage, set at init
    rounds: u64,
    paid_dragon: u128,
    paid_scales: u128,
}
```

| Method | Kind | Behaviour |
|---|---|---|
| `new(dragon, scales, wrap, min_round, reserve)` | init | registers the tithe on `wrap` with `storage_deposit` |
| `distribute()` | call, anyone | if `balance - reserve - locked >= min_round`: half to `dragon`, the other half split into 100 equal plain transfers to the vaults, dust stays. Emits `tithe_round`. Otherwise panics `"below min_round"`. |
| `unwrap()` | call, anyone | reads its own `wrap.ft_balance_of` and calls `near_withdraw` for all of it, so creator fees paid as wNEAR become NEAR for the next round |
| `forward_ft(token: AccountId)` | call, anyone | reads the tithe's balance of `token` and sends all of it to the Dragon with `ft_transfer` (1 yocto from the tithe). Covers any part of the creator fee Nearly pays in WYRM or another token. Anyone can register the tithe on a token with that token's `storage_deposit`. |
| `get_stats()` | view | `{ rounds, paid_dragon, paid_scales, min_round, distributable }`. `paid_scales` counts transfers sent; a bounce from a missing vault returns to the tithe. |

A transfer to a vault that does not exist yet refunds to the tithe and joins the next round. Gas of the 100-transfer round is measured in sandbox and written in `docs/PARAMETERS.md`. The launch order creates all 100 vaults before Nearly's launch names the tithe, so bounces only happen in tests.

## scale-vault

```rust
struct Vault { parent: AccountId }   // set to env::predecessor_account_id() at init
```

| Method | Kind | Behaviour |
|---|---|---|
| `new()` | init | `parent = predecessor` |
| `release(to: AccountId) -> U128` | call, only `parent` | transfers `balance - VAULT_RESERVE` to `to` and returns the amount |
| `get_parent()`, `get_claimable()` | view | |

Plain NEAR transfers to the vault need no method. The vault keeps a fixed storage reserve, so `claimable = balance - VAULT_RESERVE`.

## scales

One contract holds four things that all read the same token records: the NEP-171 collection, the primary sale, the market, and the Council. They live in one contract because a vote, a sale and a listing all need the token's owner and hold start in the same receipt.

### Tokens

```rust
struct Scale {
    owner: AccountId,
    voting: bool,            // false forever for the creator's Scales
    hold_start: u64,         // reset on every change of owner
    claimed_total: u128,
    claiming_since: Option<u64>, // a vault release is in flight; expires after 10 minutes
}
// history per token: a bounded ring of the last 16 entries
enum ScaleEvent { Minted{to, price, at}, Transferred{from, to, price: Option<U128>, at}, Claimed{by, amount, at} }
```

NEP-171 core, NEP-177 metadata, NEP-181 enumeration, NEP-178 approvals and NEP-199 payouts. Every change of owner, by any path, runs one internal hook. The hook resets `hold_start`, clears the token's listing, revokes approvals, and appends history. When `nft_resolve_transfer` returns a token to its sender, the sender's previous `hold_start` is restored. `nft_approve` takes a storage deposit as NEP-178 requires, and a token holds at most 3 approvals.

`nft_token` returns the metadata `media` and `extra` for the hold tier, computed at read time from `hold_start`: Ash under 3 days, Ember from 3, Molten from 7, Obsidian from 14.

`nft_payout` and `nft_transfer_payout` pay `ROYALTY_BPS` (500, 5%) to the Dragon and the rest to the owner, so a Scale sold on near.cooking or any NEP-199 market feeds the Dragon too.

### Vaults and claims

| Method | Kind | Behaviour |
|---|---|---|
| `create_vaults()` | call, anyone | creates the next 25 vault accounts with `use_global_contract(vault_code_hash)` and `new()`, no keys. Repeat until 100 exist. |
| `claim(token_id)` | call, owner, 1 yocto | delists the token, sets `claiming_since`, calls `vault.release(owner)`, and the callback `on_claimed` clears it and records the amount in history and `claimed_total`. The callback's static gas is tested against a full history ring. |

While a claim is in flight (under 10 minutes old) the token refuses every transfer, listing, sale and offer acceptance, so an old owner can never be paid after a sale. A claim also removes the listing, so a buyer never pays a price that was set while the vault was full. A late `on_claimed` after the 10 minutes still records the amount. A failed release clears the flag and records nothing.

### Primary sale

```rust
struct Tier { price: NearToken, remaining: u32 }
tiers: Vec<Tier>     // default 20 at 0.5, 30 at 1, 20 at 2 NEAR
sale_ends_at: u64    // set in new, default 14 days after init
```

`buy_scale()` is payable. It mints the next token id to the caller with `voting: true` at the cheapest tier that has stock, refunds any excess and sends the price to the Dragon as a plain transfer. The sale mints ids 1 to 70 in order and closes at `sale_ends_at`. Unsold ids are never minted, so they never count in any vote threshold. The creator's 30 Scales, ids 71 to 100, are minted to the `creator` account given to `new`, with `voting: false`.

### Market

The market is built into the collection. The NFT never leaves the seller until a sale, and no approval or second contract is involved.

```rust
listings: LookupMap<TokenId, NearToken>
struct Offer { id: u64, buyer: AccountId, token_id: Option<TokenId>, amount: NearToken, at: u64 }
offers: IterableMap<u64, Offer>          // token_id None means any Scale
offers_by_token: LookupMap<TokenId, Vec<u64>>   // at most 20 per token
escrow: u128                              // sum of open offers and in-flight bonds
```

Storage. Every offer and every proposal keeps a fixed `STORAGE_FEE` of 0.01 NEAR that is never refunded, and the contract is funded with 8 NEAR of storage before its keys are deleted. Every write that adds state asserts `account_balance - escrow >= storage_usage * byte_cost`, so escrowed NEAR never pays for storage. Bonds or payments that bounce from a deleted account return to the contract outside `escrow`.

| Method | Kind | Behaviour |
|---|---|---|
| `list(token_id, price)` | call, owner, 1 yocto | price at least 0.1 NEAR |
| `delist(token_id)` | call, owner, 1 yocto | |
| `buy(token_id)` | payable, anyone | attached must equal the listed price. Transfers the token, pays `price - royalty` to the seller and the royalty to the Dragon. |
| `make_offer(token_id: Option)` | payable | the attached NEAR minus `STORAGE_FEE` is the offer, at least 0.1 NEAR. It stays in the contract as escrow. One account may hold at most 10 open offers, and one token at most 20. |
| `cancel_offer(offer_id)` | call, buyer, 1 yocto | refunds the offer |
| `accept_offer(offer_id, token_id)` | call, owner, 1 yocto | for an any-Scale offer the owner names the token. Transfers, pays the seller minus royalty, royalty to the Dragon. |
| `get_listings(from, limit)`, `get_offers(from, limit)`, `get_offers_for(token_id)` | view | |

The internal transfer hook clears a token's listing, so a listing never survives a change of owner. The seller cannot be paid twice, because the token moves before any payment is sent.

### The Council

Votes are cast per Scale, not per account. That alone stops the vote-sell-vote attack, so there is no activation step.

```rust
enum ProposalKind {
    SetBreath { budget_bps: u16, steps_bps: [u16; 3], extra_bps: [u16; 3] },
    SetSlices { cap_bps: u16 },
    SetPosture { hunt: bool, sell_multiple_bps: u32, max_sell_bps: u16 },
    PauseBuys { hours: u16 },
    Grant { to: AccountId, amount: U128 },        // WYRM from the hoard
}
enum Status { Open, Passed, Rejected, Executing, Executed, LeashHeld { error: String }, Expired }
// Open -> Passed | Rejected (finalize); Passed -> Executing (execute) | Expired (deadline);
// Executing -> Executed | LeashHeld (callback). Rejected, Executed, LeashHeld, Expired are terminal.
struct Proposal {
    id: u64, kind: ProposalKind, proposer: AccountId, description: String,   // 1,000 bytes max
    bond: NearToken, created_at: u64, ends_at: u64,
    electorate: u32, yes: u32, no: u32, status: Status, execute_by: u64,
}
votes: LookupMap<(u64, TokenId), bool>
```

A Scale may vote on a proposal when it has `voting: true` and its current owner has held it since at least `SEASON` (7 days) before the proposal was created. Seasoning decides who may vote. It does not shrink the denominator: `electorate` is the number of minted Scales with `voting: true` when the proposal is created, so a Scale that just changed hands still counts as a silent no.

| Rule | Tune (everything but Grant) | Grant |
|---|---|---|
| Voting period | 3 days | 7 days |
| Passes when | `(yes + no) * 10 >= electorate * 3` and `yes > no` | `yes * 2 > electorate` and `no * 5 < electorate` |
| Can open only when | electorate >= 10 | the sale has closed, electorate >= 35, and no other Grant is Open, Passed or Executing |
| Early execution | no | no |
| Execution window | 72 hours after `ends_at`, then Expired | 72 hours after `ends_at`, then Expired |

With all 70 sold Scales minted, a Grant needs 36 yes votes, and 14 no votes block it. Only one Grant can be in play at a time, and a passed Grant that nobody executes within 72 hours expires, so a captured majority cannot bank grants for later.

| Method | Kind | Behaviour |
|---|---|---|
| `propose(kind, description, token_id)` | payable, 1 NEAR bond plus `STORAGE_FEE`, excess refunded | `token_id` must be an eligible Scale of the caller, which proves the proposer has stake. One Open proposal per proposer. |
| `vote(proposal_id, token_ids: Vec, approve: bool)` | call, 1 yocto | the proposal must be Open with `now < ends_at`. Each token must be the caller's, eligible, and not yet counted on this proposal. At most 25 tokens per call. |
| `finalize(proposal_id)` | call, anyone, Open only, after `ends_at` | sets Passed or Rejected and zeroes the bond before sending it. The bond returns to the proposer when turnout reached 30% of the electorate, otherwise it goes to the Dragon. |
| `execute(proposal_id)` | call, anyone, Passed only, before `execute_by` | sets Executing, then calls the matching Dragon method with 100 Tgas. The callback sets Executed, or `LeashHeld` when the call failed or a grant resolved to `false`. A Passed proposal past `execute_by` becomes Expired when anyone calls `execute` or `finalize` on it. |
| `get_proposals(from, limit)`, `get_proposal(id)`, `get_votes(id)` | view | |

There is no proposal kind that moves NEAR, and none that changes these rules. The thresholds are constants in code, not policy.

### Views

`get_config()`, `get_scale(token_id)` with owner, voting, hold_start, tier, claimed_total and the vault account, `get_scales(from, limit)`, `get_history(token_id)`, `get_sale()`, `eligible_now()`.

## dragon

The treasury. Holds a NEAR reserve and the WYRM hoard. No method sends NEAR anywhere except into a swap, apart from the 1 yocto deposits that `ft_transfer` and `near_withdraw` require.

### Peers

`wrap` (`wrap.near`), `dcl` (`dclv2.ref-labs.near`), `pool_id` (`"<token>|wrap.near|<fee>"`, sorted as DCL sorts), `wyrm` (the token), `council` (the scales account).

### DCL interface used

- `get_pool({pool_id})` returns `total_x`, `total_y`, `current_point`, `state`. NEAR depth is the wrap side of `total_x`/`total_y`.
- `quote({pool_ids:[pool_id], input_token, output_token, input_amount, tag: null})` returns `{amount}`.
- Swap: `wrap.near_deposit` then `wrap.ft_transfer_call({receiver_id: dcl, amount, msg})` with `msg = {"Swap":{"pool_ids":[pool_id],"output_token":wyrm,"min_output_amount":"<n>"}}`. The bought tokens arrive in a separate receipt.

### State

```rust
enum Phase { Idle, Observing { step: u64, since: u64 }, Quoting { step: u64, since: u64, side: Side, amount_in: u128 }, Swapping { step: u64, since: u64, side: Side, amount_in: u128, quoted_out: u128, min_out: u128 } }
// step grows by one per new step; every callback carries it and does nothing when it no longer matches
enum Side { Buy, Sell }
struct Settings { budget_bps, steps_bps: [u16;3], extra_bps: [u16;3], cap_bps, hunt: bool, sell_multiple_bps: u32, max_sell_bps, paused_until: u64, pause_cooldown_until: u64 }
struct Day { index: u64, breath_at: u64, opening_reserve: u128, budget: u128, spent: u128, sold_wyrm: u128 }
struct Hoard { wyrm: u128, near_spent: u128, wyrm_bought: u128, donated: u128, granted: u128, realized_near: u128 }
grants: ring of { at: u64, amount: u128 } for the rolling 30-day cap
samples: ring of 1,440 { at: u64, tokens_per_near: u128 }, at most one per minute
log: ring of 256 events for buys, sells, grants, Council changes and failures
```

Price is sampled as the quote for 1 NEAR in, in WYRM out (`tokens_per_near`). A dip of 10% in price is `tokens_per_near >= reference * 10_000 / 9_000`. The reference is the time-weighted mean of samples over 24 hours, where each sample is weighted by the time gap to the next sample, so one late sample cannot drag it.

### Hard limits (constants)

| Setting | Default | Council range |
|---|---|---|
| daily budget | 2,000 bps of the reserve at the day's first observation | 1,000 to 4,000 |
| dip steps | 2,000, 3,500, 5,000 bps below reference | 500 to 5,000, strictly increasing |
| extra budget per step | 500, 1,000, 1,500 bps of the day's opening reserve, cumulative: a confirmed step 3 adds all three, and an extra counts only while its step is confirmed in the current observation | each 0 to 2,500; budget plus the sum of extras at most 8,000 |
| slice cap | 250 bps of price impact | 50 to 300 |
| spike guard | no buy while `tokens_per_near * 11_500 < reference * 10_000` | fixed |
| slice gap | 60 s | fixed |
| slippage | 150 bps under the fresh quote | fixed |
| dip confirmation | 3 samples over at least 10 minutes | fixed |
| grant | 50 bps of hoard per grant; executed grants in any rolling 30 days at most 100 bps of the hoard at execution; a failed grant frees its share | fixed |
| Hunt | off; sell multiple at least 20,000 bps of average cost, at most 1,500 bps of hoard per day | default 25,000 and 1,000 |
| pause | at most 72 hours, then 7 days cooldown | fixed |

### Methods

The reserve is the Dragon's NEAR balance above its storage cost plus its wNEAR balance. Sale money, royalties and tithe rounds arrive as plain transfers. A buy spends wNEAR first and calls `near_deposit` only for the shortfall.

The slice cap is measured as price impact, not pool depth, because DCL's `total_x`/`total_y` include liquidity far from the current price that anyone can add for free. A slice of `a` NEAR is allowed when `quote(a) * 10_000 >= a * tokens_per_near * (10_000 - cap_bps)`, where `tokens_per_near` is the 1 NEAR quote taken in the same observation. The Dragon starts from the budget-limited amount and halves it until the check passes, at most 6 times, and skips the slice if it is still too large. Hunt sells obey the same cap, measured against the quote for selling 1 NEAR worth of WYRM.

The defaults come from the Gym's held-out replay (`docs/PARAMETERS.md`). Breath alone buys best; the dip ladder at these deeper steps and smaller extras costs about 0.5% on held-out coins and is kept for the Lair's Stirring mood and the Council to tune.

| Method | Kind | Behaviour |
|---|---|---|
| `new(wrap, dcl, pool_id, wyrm, council)` | init | registers on `wrap` and `wyrm` |
| `poke()` | call, anyone | one step of the machine below. Returns a short status string. Safe to call any time, refuses only when a step is in flight and younger than 5 minutes. |
| `ft_on_transfer` | NEP-141 receiver | accepts WYRM (adds to the hoard as `donated`, the dev buy arrives this way) and wNEAR. Refuses any other token. |
| `set_breath(budget_bps: u16, steps_bps: [u16;3], extra_bps: [u16;3])`, `set_slices(cap_bps: u16)`, `set_posture(hunt: bool, sell_multiple_bps: u32, max_sell_bps: u16)`, `pause_buys(hours: u16)` | call, only `council`, no deposit | each panics with a message naming the limit when out of range |
| `grant(to: AccountId, amount: U128)` | call, only `council`, no deposit | panics when over the cap. Otherwise reserves the amount and calls `wyrm.ft_transfer`. Resolves to `true` when paid; on failure the callback restores the hoard and resolves to `false`. |
| `get_config()`, `get_state()`, `get_log(from, limit)`, `get_samples(limit)` | view | `get_state` includes `mood`: Sleeping, Stirring, Breathing, Gorged or Hunting |

### poke() machine

1. Idle: start Observing with a new `step`. Batch `wyrm.ft_balance_of(self)`, `wrap.ft_balance_of(self)`, `dcl.get_pool(pool_id)`, `dcl.quote(1 NEAR)` and call back `on_observe(step)`.
2. `on_observe`: reconcile the hoard. A WYRM increase since the last observation is credited to `wyrm_bought` up to the `quoted_out` of the swap that ran since then, and any rest goes to `donated`, so donations never lower the average cost. Grants in flight are subtracted. Record a sample if a minute has passed, roll the day. Decide: Hunt sell, dip buy, breath buy, or nothing. A slice needs 60 s since the last one and no spike. If nothing, go Idle.
3. For a trade, go Quoting, quote the candidate amount and halve it per the slice cap, then swap with `min_out = quote * (10_000 - 150) / 10_000`, and go Swapping. `Day.spent` grows when the swap is sent.
4. `on_swap`: go Idle, log the slice. A refund leaves wNEAR in the Dragon, gives the amount back to `Day.spent`, and logs `swap_failed`.

A step older than 5 minutes may be replaced by `poke`. Its late callbacks see a different `step` and do nothing, except that a late swap result still frees or keeps `Day.spent` correctly.

The breath minute of each UTC day is drawn from `env::random_seed()` at the day's first `on_observe`. From that minute on, the remaining daily budget may be spent, slice by slice.

## Events

| Contract | Events |
|---|---|
| tithe | `tithe_round {round, to_dragon, per_scale, dust}` |
| scales | `scale_minted`, `scale_claimed`, `nft_transfer` (NEP-171), `scale_listed`, `scale_delisted`, `scale_sold {token_id, from, to, price, royalty}`, `offer_made`, `offer_cancelled`, `proposal_created`, `vote_cast`, `proposal_finalized`, `proposal_executed`, `leash_held` |
| dragon | `observe`, `breath_slice`, `dip_slice`, `hunt_slice`, `swap_failed`, `grant_paid`, `grant_failed`, `settings_changed` |

## Refusal messages

The keeper treats these panics as "nothing to do". Each contract panics with exactly this text.

| Contract | Method | Message |
|---|---|---|
| tithe | `distribute` | `below min_round` |
| tithe | `unwrap` | `nothing to unwrap` |
| tithe | `forward_ft` | `nothing to forward` |
| dragon | `poke` | `in flight` |
| scales | `create_vaults` | `all 100 vaults exist` |
| scales | `finalize` | `not open` or `voting still open` |
| scales | `execute` | `not passed` |

## Event fields read by verify and audit-dragon

A missing field fails the check that reads it. Amounts are yocto strings and times nanosecond strings.

| Event | Fields |
|---|---|
| `observe` | `at, tokens_per_near, reserve, hoard_wyrm, breath_at` |
| `breath_slice` | `at, breath_at, amount_in, quoted_out, tokens_per_near, reference, cap_bps, budget` |
| `dip_slice` | the `breath_slice` fields plus `step_bps, confirmations, confirmed_since` |
| `scale_minted` | `token_id, to, price` |
| `grant_paid` | `at, to, amount, hoard` (the hoard before this grant) |
| `settings_changed` | the changed setting fields by their `Settings` names |

View shapes: `get_proposals({from, limit})` and `get_scales({from, limit})` return arrays, and each `get_scales` item carries `vault`. `dragon.get_config()` returns `wrap, dcl, pool_id, wyrm, council` and a `settings` object.

## Shapes the Lair reads

- `dragon.get_state()` returns `{ mood, phase, reserve, spot, reference, hoard: {wyrm, near_spent, wyrm_bought, donated, granted, realized_near}, day: {index, breath_at, opening_reserve, budget, spent, sold_wyrm}, settings: {...}, dip: {step, samples, first_at} | null }`.
- `scales.get_sale()` returns `{ tiers: [{price, remaining}], sold, sale_ends_at }`.
- `scales.get_scale(token_id)` includes `claiming_since` (null when no claim is in flight).
- `get_listings` returns `[{token_id, price}]`. `get_offers` and `get_offers_for` return `[{id, buyer, token_id, amount, at}]`. `get_votes(id)` returns `[{token_id, approve}]`.
- `scales.eligible_now()` returns the current electorate: minted Scales with `voting: true`.
- `VAULT_RESERVE` is 0.01 NEAR. WYRM has 18 decimals, as every Nearly coin does.

## Decisions made while building scales (Nov 3)

- `create_vaults()` checks only the batch's last vault with `get_parent` after creation. Joining 25 results costs about 9.4 Tgas each and does not fit. A batch fails or succeeds as a whole, and a retry is safe because creating an existing vault fails and refunds. A batch in flight refuses with "the remaining vaults are being created" for up to 10 minutes.
- `publish_vault_code(code)` is payable. It checks the code against the `vault_code_hash` fixed in `new` and publishes it as a global contract. The caller pays the burn. This is the one-transaction publish step on mainnet, and the only way near-workspaces can publish global code.
- The vault is written without the `#[near]` macros, 17 KB instead of 95 KB, so publishing it burns about 1.7 NEAR. `release` takes exactly `{"to":"<account>"}`.
- Votes are stored as two 100-bit sets on each proposal. `get_votes` and the counts are unchanged.
- Proposal ids and offer ids are JSON numbers.
- `execute` on a Passed proposal past `execute_by` sets Expired and returns `false` instead of panicking, so the status change sticks.
- `get_offers_for(token_id)` returns offers named for that Scale. Any-Scale offers are in `get_offers`.
- `new(dragon, creator, vault_code_hash, metadata, tier_media: [String; 4], tiers: Option, sale_ends_at: Option)`.
- History keeps the last 16 entries per Scale. The full history is in the events. The scales account is funded with 8 NEAR before its keys are deleted: about 3.9 NEAR for the 394 KB of code and the rest for tokens, history, offers and proposals.

## Decisions made while building the dragon (Nov 3)

- `poke` refuses below 600 Tgas, and the keeper attaches 1,000 Tgas. The chain of a buy (observe batch, parallel halving quotes, deposit, swap) uses about 570 Tgas including what it attaches. Mainnet protocol 86 allows 1,000 Tgas per transaction (checked with `EXPERIMENTAL_protocol_config` on Nov 3).
- The halving candidates are quoted in one parallel round and the largest that passes is used. The smallest slice is 0.01 NEAR of value.
- No buy happens until a reference exists. Hunt needs no reference and ignores the spike guard and `pause_buys`.
- `realized_near` is gross Hunt proceeds, not profit.
- `get_state().dip` is `{step, step_bps, samples, first_at, confirmed}` for the newest run below a step, where `step` is the deepest step crossed, 1 to 3, or `null` with no dip.
- Mood precedence: a swap in flight sets the mood (sell is Hunting, buy is Breathing); otherwise Hunting, Gorged, Breathing, Stirring, Sleeping in that order.
- The grant ring holds 32 entries, so a 33rd grant in 30 days is refused. Grants in flight count toward the window. A reconcile that lands between a grant's transfer and its callback can undercount the hoard by that grant, which errs toward smaller caps.
- The reserve holds back storage for ring slots not yet filled, about 2.1 NEAR at launch.
- `pause_buys` takes 1 to 72 hours. `max_sell_bps` takes 1 to 1,500.

## Decisions from the sandbox run (Nov 3)

- A `LeashHeld` status carries "the dragon refused the call" or "the grant resolved to false". NEAR gives a callback no failure payload, so the Dragon's own limit message is visible only in the receipt failures of the `execute` transaction. The Lair links that transaction.
- A new vault starts with a few attoNEAR claimable, from the gas reward of its `new()` call. The first claim pays it out.
- Measured gas is in `docs/GAS.md`. A poke that ends in a buy burns about 41 Tgas on the mocks, far under the 600 Tgas the Dragon requires attached. The 600 Tgas floor stays, because static gas for the chained callbacks must be prepaid and unused gas is refunded.
