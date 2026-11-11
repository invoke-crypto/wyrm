# WYRM v2 design

This is the merged design from an arena between two candidate designs, A and B, revised after an adversarial review. Candidate B is the base. The grafts from A, the review's changes and the reason for each are in [the synthesis decision](#synthesis-decision). [invariants.md](invariants.md) lists every invariant and where it is enforced.

Part 7 of the WYRM master plan, which is not in this repository, wins over everything here. Nearly facts come from `docs/research/NEARLY.md` and from Nearly's factory source at `e82bfa0` (`contracts/factory/src/lib.rs`, read on Nov 4). DCL facts come from `docs/research/RHEA-DCL.md`. Strategy numbers come from `docs/PARAMETERS.md` and `gym/out/presets.json`. Every size, gas and cost figure is an estimate unless it cites one of those files or `docs/GAS.md`. This section was written before the v2 build; "Implementation reconciliation" records where the built contracts differ.

## The shape in six sentences

1. The Dragon calls Nearly's `launch` itself, so Nearly records the Dragon as the coin's creator. `bind(launch_id)` then checks that record and the DCL pool, and `dragon.get_coin()` is the one mark of the real coin.
2. Presets are one compiled table in a new `wyrm-types` crate, keyed by name. The five Gym presets are there, plus a `drill` preset that only a `.testnet` account can use. Every contract takes the preset's name in `new`, and every bound is a constant or a preset value.
3. The Council reaches the Dragon through one method, `obey(order: Order)`, which answers with a `Verdict` instead of panicking. A proposal that passes executes inside `finalize`, so there is no execute step, no execution window, no Expired status and no one-grant lock.
4. The Scale sale opens when the Dragon binds and runs 14 days. Each mint first sweeps the new Scale's vault to the Dragon. When the sale closes, scales tells the tithe once, and from then on the tithe pays the unsold Scales' shares straight to the Dragon.
5. The token tax is an engine constant, 400 bps a side with the whole creator side to the tithe. A buy's `min_out` stays on the gross quote and the Dragon books only the net arrival. A sell is quoted, and its `min_out` set, on the net input.
6. The factory checks every input before it deploys anything, then deploys a fresh copy of the three engines for each launch from code bytes it stores. Every launch's 100 vaults use the one shared vault code by hash, and the creator's deposit pays for all of it.

## 1. Caller's usage

### 1a. The creator deploys WYRM directly (Nov 9)

```sh
# Four accounts. Each is one batch of CreateAccount, Transfer, DeployContract and FunctionCall new. ash gets no code.
deploy tithe.thewyrm.near  tithe.wasm  new '{"dragon":"dragon.thewyrm.near","scales":"scales.thewyrm.near","wrap":"wrap.near","min_round":"1000000000000000000000000","reserve":"..."}'
deploy dragon.thewyrm.near dragon.wasm new '{"preset":"hoarder","peers":{"wrap":"wrap.near","dcl":"dclv2.ref-labs.near","nearly":"nearlytrade.near","tithe":"tithe.thewyrm.near","scales":"scales.thewyrm.near","ash":"ash.thewyrm.near","creator":"thewyrm.near"}}'
deploy scales.thewyrm.near scales.wasm new '{"preset":"hoarder","dragon":"dragon.thewyrm.near","tithe":"tithe.thewyrm.near","creator":"thewyrm.near","vault_code_hash":"...","collection":{"name":"WYRM Scales","symbol":"SCALE","icon":"https://...","tier_media":[...]}}'
create_keyless ash.thewyrm.near      # CreateAccount and Transfer of 0.01 NEAR. No AddKey, no code.

# Once per network. The 16.8 KB vault code becomes global code by hash, burning 1.68 NEAR. Every later system on the
# network, WYRM's and every factory launch, uses the same hash for free. The script skips this when RPC already
# returns code for the hash. scales must exist first, because the method lives on it.
near call scales.thewyrm.near publish_vault_code --base64 "$(base64 vault.wasm)" --deposit 1.7 --accountId thewyrm.near

near call scales.thewyrm.near create_vaults --gas 300Tgas    # four times, 25 vaults each, as v1
check_wiring        # reads the three get_config views and compares them the way dragon.launch will
delete_keys tithe dragon scales

# The coin. Attach at least Nearly's quote_launch total plus 0.086 NEAR. The Dragon reads the quote itself, checks
# that no pool exists yet for the coin's predicted token, sends Nearly the quote plus 0.086 NEAR and returns the
# rest to the caller. It writes fee_to, fee_mode and the tax into Nearly's args.
near call dragon.thewyrm.near launch '{"coin":{"name":"WYRM","symbol":"WYRM","icon":"https://...","description":"...","links":{...}}}' \
  --deposit 10 --gas 450Tgas --accountId thewyrm.near
# The launch id is in Nearly's launch_started event in this transaction. Wait for get_launch(id).step == "Done".
near call dragon.thewyrm.near bind '{"launch_id":"3712"}' --gas 150Tgas --accountId thewyrm.near
# bind ends by opening the Scale sale on scales. The sale runs 14 days from that block.
```

The creator account `thewyrm.near` keeps its keys. It receives the 10 creator Scales, the 10% sale cut and the 5% resale royalty. It alone may call `launch`, `bind` and `nearly_cancel_stuck`.

### 1b. A stranger launches through the factory (from Nov 12)

```ts
// The Lair's Summon wizard. summon, create_vaults and bind are safe to retry. launch is not (see below).
const label = "kobold";
await wallet.call("summon.thewyrm.near", "summon",
  { label, preset: "hoarder", collection: { name: "Kobold Scales", symbol: "KSCALE", icon, tier_media } },
  { deposit: await view("summon.thewyrm.near", "quote_summon", { preset: "hoarder" }), gas: TGAS(300) });
// Accounts: kobold-tithe, kobold-dragon, kobold-scales and kobold-ash, each .summon.thewyrm.near
for (let i = 0; i < 4; i++) await wallet.call("kobold-scales.summon.thewyrm.near", "create_vaults", {}, { gas: TGAS(300) });
const tx = await wallet.call("kobold-dragon.summon.thewyrm.near", "launch",
  { coin: { name: "Kobold", symbol: "KOBOLD", icon, description, links, dev_buy: "2000000000000000000000000" } },
  { deposit: nearlyQuoteTotal + LAUNCH_MARGIN + slack, gas: TGAS(450) });
const started = nearlyLaunchStarted(tx);          // Nearly's launch_started event, or null
if (!started) { /* Nearly refused at once. The deposit stayed in the Dragon. A new launch is safe. */ }
else {
  // The launch exists. Never launch again. Wait for Done, and resume it if it parked.
  await waitUntilDoneOrResume(started.id);        // nearly.resume(id), or dragon.nearly_resume(id) for a dev buy
  await wallet.call("kobold-dragon.summon.thewyrm.near", "bind", { launch_id: started.id }, { gas: TGAS(150) });
}
```

`launch` is not safe to retry blindly. A Nearly launch whose step chain stopped early stays parked and resumable (an AddLiquidity failure keeps its step, lib.rs:4259-4267 and 1243), and anyone may resume a launch with no dev buy. A second launch would leave the first one as an orphan coin that also names our Dragon as creator.

The stranger is `creator` in scales and dragon. Nearly registers the creator on the new token in its CreateToken step (lib.rs:4508-4513), so the dev buy is delivered straight into the Kobold Dragon's hoard.

### 1c. A holder votes

```ts
// Propose with a Scale held 7 days. 1 NEAR bond plus a 0.01 NEAR storage fee.
const id = await wallet.call(SCALES, "propose", {
  kind: { Order: { SetAutoBurn: { bps: 1000 } } }, description: "Burn 10% of every buy", token_id: "7",
}, { deposit: "1010000000000000000000000" });
await wallet.call(SCALES, "vote", { proposal_id: id, token_ids: ["7", "12", "40"], approve: true }, { deposit: "1" });
// Two days later anyone finalizes. A passed order reaches the Dragon in the same transaction.
await wallet.call(SCALES, "finalize", { proposal_id: id }, { gas: TGAS(150) });
// -> proposal_finalized {status: "Executing"}, then proposal_executed {status: "Executed"} from the callback,
//    or {status: {LeashHeld: {error: "auto_burn_bps must be 0 to 5000, got 6000"}}} when the Dragon refuses
// A text poll. It can only end Carried or Rejected, and it calls nothing.
await wallet.call(SCALES, "propose", { kind: "Signal", description: "Should the Lair add a leaderboard?", token_id: "7" }, ...);
```

`scales.get_council()` returns what the Lair renders. That is the preset's enabled kinds, each kind's class, the class thresholds, the Council bounds and the vote timings.

### 1d. The keeper loop

```ts
// keeper/keeper.mjs plan(snapshot). Every refusal string below means "nothing to do".
every(60_000, () => call(DRAGON, "poke", {}, TGAS(1000)));                   // "not bound", "in flight"
everyHourAt(":15", async () => {
  await call(NEARLY, "push_creator_fees", { account_id: TITHE });             // pool-fee share and tax share, as NEAR
  await call(TITHE, "distribute", {}, TGAS(300));                              // "below min_round"
});
everyHour(async () => {
  for (const id of await view(SCALES, "get_due")) await call(SCALES, "finalize", { proposal_id: id }, TGAS(150));
});
everyDay(async () => {
  await call(SCALES, "close_sale", {}, TGAS(50));                              // "not due", then "closed" once the tithe knows
  const ids = (await view(SCALES, "get_sweepable")).filter(id => vaultBalance(id) >= 0.05);
  for (const chunk of chunks(ids, 20)) await call(SCALES, "sweep", { token_ids: chunk }, TGAS(300));
});
```

There is no `execute` call and no `open_sale` call. After the sale closes and the last sweep runs, unsold Scales need no keeper work at all.

## 2. Type sketch

### 2a. `contracts/wyrm-types` (new library crate, linked by scales, dragon and factory)

```rust
// lib.rs
/// Bumped on every release of the engine contracts. scales and dragon report it, and Dragon::launch requires the
/// two to match its own, so engines from different builds cannot pair.
pub const ENGINE_VERSION: u32 = 2_00;

// preset.rs
/// The one list of presets. JSON names are kebab-case: "hoarder", "dip-hunter".
#[near(serializers = [borsh, json])]
pub enum PresetName { Hoarder, Steady, DipHunter, Swift, Harvester, Drill }
pub const ALL: [PresetName; 6];

/// Everything a launch fixes for life, plus the launch values of what the Council can move.
#[near(serializers = [borsh, json])]
pub struct Preset {
    pub name: PresetName,
    pub network: Network,            // Any, or TestnetOnly (Drill)
    pub strategy: Strategy,          // launch values; the Dragon copies them into state at init
    pub bounds: StrategyBounds,      // the Council's range for each Strategy field
    pub ladder: DipLadder,           // fixed for life
    pub cadence: Cadence,            // fixed for life
    pub sale: SaleTerms,
    pub royalty_to: RoyaltyTo,
    pub features: Features,
    pub times: CouncilTimes,
}
pub enum Network { Any, TestnetOnly }

/// What the Council can move. One value, so the engine reads one thing.
pub struct Strategy {
    pub budget_bps: u16,             // daily breath budget, bps of the day's opening reserve
    pub cap_bps: u16,                // slice price-impact cap
    pub hunt: Option<HuntLine>,      // None = Hunt off
    pub auto_burn_bps: u16,          // share of each buy's WYRM sent to ash
}
pub struct HuntLine { pub sell_multiple_bps: u32, pub max_sell_bps: u16 }
pub struct StrategyBounds {
    pub budget_bps: BpsRange,
    pub cap_bps: BpsRange,
    pub sell_multiple_min_bps: u32,
    pub max_sell_bps: BpsRange,
    pub auto_burn_bps: BpsRange,
}
pub struct DipLadder { pub steps_bps: [u16; 3], pub extra_bps: [u16; 3] }
/// spread_minutes 0 opens the whole budget at the breath minute, as v1. A positive value opens it linearly over
/// that many minutes, and the breath minute is drawn so the spread ends inside the UTC day (Gym `allowance`).
pub struct Cadence { pub spread_minutes: u16, pub gap_seconds: u16 }
pub struct SaleTerms { pub price: NearToken, pub creator_bps: u16, pub length: u64 }
/// Who receives the 5% resale royalty. Holders pays it to the tithe, which splits it like any other income, half
/// to the Dragon and a hundredth to each vault, because one NEP-199 payout cannot carry 100 vault entries.
pub enum RoyaltyTo { Creator, Holders }
pub struct Features { pub grant: bool, pub burn: bool, pub hunt: bool, pub auto_burn: bool }
pub struct CouncilTimes { pub season: u64, pub voting: u64 }
pub struct BpsRange { pub lo: u16, pub hi: u16 }
/// Nearly's tax args.
pub struct TaxPolicy { pub buy_bps: u16, pub sell_bps: u16, pub creator_bps: u16, pub burn_bps: u16, pub holders_bps: u16 }

/// The engine walls. No preset may cross them. Constants, the same in every build.
pub mod envelope {
    /// The token tax every launch asks Nearly for, and that bind requires back. 4% a side is Nearly's maximum.
    /// A preset without this tax would need a 0.5% slice-cap ceiling (the Gym's sandwich break-even), and that
    /// rule is added the day such a preset exists.
    pub const TAX: TaxPolicy = TaxPolicy { buy_bps: 400, sell_bps: 400, creator_bps: 10_000, burn_bps: 0, holders_bps: 0 };
    pub const BUDGET_BPS: BpsRange = BpsRange { lo: 1_000, hi: 4_000 };
    pub const CAP_BPS: BpsRange = BpsRange { lo: 50, hi: 200 };          // Set slices: 0.5% to 2%
    pub const STEP_BPS: BpsRange = BpsRange { lo: 500, hi: 5_000 };
    pub const EXTRA_BPS: BpsRange = BpsRange { lo: 0, hi: 2_500 };
    pub const DAY_TOTAL_MAX_BPS: u16 = 8_000;                           // budget hi + sum of extras
    pub const SELL_MULTIPLE_MIN_BPS: u32 = 20_000;
    pub const MAX_SELL_BPS: BpsRange = BpsRange { lo: 1, hi: 1_500 };
    pub const AUTO_BURN_BPS: BpsRange = BpsRange { lo: 0, hi: 5_000 };
    pub const SPREAD_MINUTES_MAX: u16 = 1_439;
    pub const GAP_SECONDS: RangeInclusive<u16> = 60..=3_600;
    pub const CREATOR_SALE_MAX_BPS: u16 = 2_000;
    pub const MIN_SCALE_PRICE: NearToken = NearToken::from_millinear(100);
    pub const MAINNET_TIMES: CouncilTimes = CouncilTimes { season: 7 * DAY, voting: 2 * DAY };
    pub const MAINNET_SALE_LENGTH: u64 = 14 * DAY;
}

/// The compiled table. A match, so a new name without a value fails to compile.
pub const fn preset(name: PresetName) -> Preset;

impl Preset {
    /// The preset rules, in one function. A unit test runs it on every name in ALL, contracts/build.sh runs the
    /// tests before it builds release wasm, and each engine's `new` runs it again.
    ///  1. Strategy inside bounds, bounds inside the envelope.
    ///  2. budget_bps.hi + sum(extra_bps) <= DAY_TOTAL_MAX_BPS. Steps strictly increasing.
    ///  3. A disabled feature's launch value is off: hunt None, auto_burn 0.
    ///  4. Hunt may be on at launch only when features.hunt.
    ///  5. Network::Any implies times == MAINNET_TIMES and sale.length == MAINNET_SALE_LENGTH.
    ///  6. spread_minutes <= SPREAD_MINUTES_MAX, gap_seconds inside GAP_SECONDS.
    ///  7. sale.price >= MIN_SCALE_PRICE, sale.creator_bps <= CREATOR_SALE_MAX_BPS.
    pub fn check(&self) -> Result<(), String>;
    /// sha256 of the borsh encoding. scales and dragon report it; Dragon::launch compares the two.
    pub fn hash(&self) -> CryptoHash;
    /// Refuses TestnetOnly unless `account` ends in ".testnet".
    pub fn allowed_on(&self, account: &AccountId) -> Result<(), String>;
}

// collection.rs
/// The Scale collection's text. scales `new` runs `check`, and the factory runs the same function in `summon`
/// before it deploys anything, so a summon batch cannot fail on its arguments.
pub struct CollectionArgs {
    pub name: String,                // 1..=64 bytes
    pub symbol: String,              // 1..=16 bytes, ASCII letters and digits
    pub icon: Option<String>,        // https:// or ipfs://, at most 512 bytes
    pub tier_media: [String; 4],     // each https:// or ipfs://, at most 512 bytes
}
impl CollectionArgs { pub fn check(&self) -> Result<(), String>; pub fn metadata(&self) -> NFTContractMetadata; }

// order.rs
/// Everything the Council can make the Dragon do. One enum, one Dragon method.
#[near(serializers = [borsh, json])]
pub enum Order {
    SetBreath { budget_bps: u16 },
    SetSlices { cap_bps: u16 },
    SetPosture { hunt: Option<HuntLine> },
    Grant { to: AccountId, amount: U128 },
    Burn { amount: U128 },
    SetAutoBurn { bps: u16 },
}
pub enum Feature { Grant, Burn, Hunt, AutoBurn }
pub enum VoteClass { Light, Strict }
/// What `obey` answers. The Dragon never panics on a refused order, because a PromiseError carries no message.
#[near(serializers = [json])]
pub enum Verdict { Executed, Refused { reason: String } }

impl Order {
    pub fn feature(&self) -> Option<Feature>;      // SetPosture -> Hunt, SetAutoBurn -> AutoBurn, Grant, Burn
    pub fn class(&self) -> VoteClass {
        match self {
            Order::Grant { .. } | Order::SetPosture { hunt: Some(_) } => VoteClass::Strict,
            _ => VoteClass::Light,
        }
    }
    /// Feature enabled, value inside the preset's bounds, and amount > 0 for Grant and Burn. Pure. scales runs it
    /// at propose, the Dragon in admit.
    pub fn check(&self, preset: &Preset) -> Result<(), String>;
}
impl VoteClass {
    /// Light: (yes + no) * 10 >= electorate * 3 && yes > no.
    /// Strict: (yes + no) * 3 >= electorate * 2 && yes * 5 >= (yes + no) * 4.
    pub const fn passes(self, yes: u32, no: u32, electorate: u32) -> bool;
    pub const fn min_electorate(self) -> u32 { match self { Light => 10, Strict => 45 } }
    pub const fn needs_closed_sale(self) -> bool { matches!(self, Strict) }
}
/// The bond returns at 30% turnout, whatever the class.
pub const fn bond_returns(yes: u32, no: u32, electorate: u32) -> bool;

// coin.rs
/// The creator's coin text, parsed with Nearly's own synchronous rules (launch, lib.rs:794-809) so that Nearly
/// cannot refuse these fields. fee_to, fee_mode, quote and tax are absent because the Dragon writes them.
pub struct CoinArgs {
    pub name: String,            // 1..=32 chars after trim
    pub symbol: String,          // 1..=12 ASCII letters and digits after trim
    pub icon: Option<String>,    // https:// or ipfs://, at most 512 bytes, with no '"', '\' or control
                                 // characters, so Nearly's json_len(icon) equals its byte length and the quote fits
    pub description: String,     // at most 500 chars
    pub links: Links,            // each at most 200 bytes
    pub dev_buy: Option<U128>,
}
impl CoinArgs { pub fn parse(self) -> Result<CoinArgs, String>; pub fn text_bytes(&self) -> u32; pub fn icon_bytes(&self) -> u32; }
```

### 2b. tithe (v1 plus retirement and one view)

```rust
struct Tithe {
    dragon, scales, wrap, min_round, reserve, rounds, paid_dragon, paid_scales,   // v1
    /// The first unsold sale id once the sale has closed. Ids first..=90 are retired: their hundredths go to the
    /// Dragon inside the round. A suffix of the sale ids by construction, so a sold Scale can never be retired.
    retired_from: Option<u8>,
}
```

### 2c. scale-vault

Unchanged. Same code, same code hash, same global code. One published copy serves every system on the network.

### 2d. scales

```rust
pub const SUPPLY: u8 = 100;
pub const SALE_IDS: RangeInclusive<u8> = 1..=90;     // sold, vote
pub const CREATOR_IDS: RangeInclusive<u8> = 91..=100; // minted to the creator at init, never vote
pub const ROYALTY_BPS: u16 = 500;
pub const VAULT_BATCH: u8 = 25;                      // as v1, measured at 254 Tgas (docs/GAS.md)
pub const MAX_SWEEP: usize = 20;
pub const FINALIZE_GAS: Gas = Gas::from_tgas(150);   // finalize refuses with less prepaid gas
pub const OBEY_GAS: Gas = Gas::from_tgas(100);

#[near(contract_state)]
pub struct Scales {
    preset: PresetName,
    dragon: AccountId,
    tithe: AccountId,                          // close_sale tells it which ids retired; Holders royalties go here
    creator: AccountId,
    royalty_receiver: AccountId,               // creator or tithe, resolved from preset.royalty_to in new
    vault_code_hash: CryptoHash,
    vaults_created: u8,
    vaults_batch_since: Option<u64>,
    metadata: NFTContractMetadata,             // from CollectionArgs::metadata
    tier_media: [String; 4],
    sale: Sale,
    scales: LookupMap<ScaleId, Scale>,         // v1 Scale minus `voting`, which ScaleId::votes() derives from the id
    history: LookupMap<ScaleId, Vec<ScaleEvent>>,
    listings, offers, offers_by_token, open_offers, next_offer_id, escrow,   // market unchanged
    proposals: Vector<Proposal>,
    open_proposal: LookupMap<AccountId, u64>,
}

/// Opens once, when the Dragon binds. Closed is terminal.
pub enum Sale {
    Waiting,                                   // before the Dragon is bound
    Open { opened_at: u64, ends_at: u64, sold: u8 },
    Closed { sold: u8, tithe_told: bool },
}
// Derived, never stored:
//   minted(id)  = CREATOR_IDS.contains(id) || id <= sold
//   retired(id) = SALE_IDS.contains(id) && id > sold && sale is Closed

pub enum ScaleEvent { Minted { .. }, Transferred { .. }, Claimed { .. }, Swept { amount, at } }   // Swept is new

pub enum ProposalKind { Order(Order), Signal }
// Open -> Rejected | Carried (a Signal that passed) | Executing (an Order that passed), all in finalize
// Executing -> Executed | LeashHeld (callback). Every status but Open and Executing is terminal.
pub enum Status { Open, Rejected, Carried, Executing, Executed, LeashHeld { error: String } }
pub struct Proposal {
    id: u64, kind: ProposalKind, class: VoteClass,   // class fixed at creation
    proposer: AccountId, description: String,         // 1,000 bytes max
    bond: NearToken, created_at: u64, ends_at: u64,
    electorate: u32,                                  // sold count at creation
    yes: ScaleSet, no: ScaleSet, status: Status,
}
```

Deleted from v1: `Tier`, `default_tiers`, `assert_valid_tiers`, `Track`, `ProposalKind::dragon_call`, `Status::Passed`, `Status::Expired`, `execute_by`, `EXECUTION_WINDOW`, `grant_in_play`, `execute`, and `Scale.voting`.

### 2e. dragon

```rust
#[near(contract_state)]
pub struct Dragon {
    preset: PresetName,                 // ladder, cadence, bounds and features are read from the table
    peers: Peers,
    strategy: Strategy,                 // replaces v1 Settings; only obey changes it
    /// The coin is unknown until Nearly's launch lands, so it is state and not an init argument. Set once.
    coin: Option<Coin>,
    last_launch_at: u64,                // launch refuses within LAUNCH_COOLDOWN (10 min) of the last one
    // The engine below is v1's, unchanged in shape.
    phase: Phase, step: u64, day: Day, hoard: Hoard, unsettled: Unsettled, wnear: u128, last_slice_at: u64,
    grants: Window, burns: Window,      // two rolling 30-day windows: 50 bps each, 100 bps per window
    pending: Vec<Pending>,              // grants, burns and auto-burns sent and not settled, at most 3
    samples: Ring<Sample>, log: Ring<LogEntry>,
}
/// One outflow in flight. on_outflow matches `seq`, so a callback for a dropped entry changes nothing.
pub struct Pending { pub seq: u64, pub kind: Outflow, pub amount: u128, pub at: u64 }
pub struct Peers { pub wrap, pub dcl, pub nearly, pub tithe, pub scales, pub ash, pub creator: AccountId }
pub struct Coin { pub launch_id: u64, pub token: AccountId, pub pool_id: String, pub wrap_is_x: bool }

/// hoard.wyrm is the WYRM the Dragon owns: its token balance less ash_due. Grant and Burn caps and Hunt's
/// sell_room are measured on it, so WYRM owed to ash is never granted, burned twice or sold.
pub struct Hoard { wyrm, near_spent, wyrm_bought, donated, granted, realized_near,   // v1
                   burned: u128, auto_burned: u128, ash_due: u128 }                  // new
pub struct Window { ring: Ring<Spent> }          // Spent { at, amount }, 32 slots, as v1's grant ring
pub enum Outflow { Grant { to: AccountId }, Burn, AutoBurn }
```

`dragon/src/nearly.rs` is the whole boundary with Nearly. Nearly's JSON is parsed here and goes no further.

```rust
pub const LAUNCH_GAS: Gas = Gas::from_tgas(300);    // the floor sent to Nearly; unused gas weight 1 adds the rest
pub const RESUME_GAS: Gas = Gas::from_tgas(260);    // Nearly's floor for a first buy that opens the pool (resume)
/// DCL_POOL_CREATE 0.1 less DCL_POOL_CHARGE 0.014 (lib.rs:50-55). Nearly's cost rises by this much when its pool
/// float runs low (pool_fronted, lib.rs:4252-4257). Whatever Nearly does not use returns to the Dragon as `extra`.
pub const LAUNCH_MARGIN: NearToken = NearToken::from_millinear(86);
pub const POOL_FEE: u32 = 10_000;                   // Nearly's config pool_fee, read at block 222922088

/// Nearly's `launch` args: the coin text plus fee_to = tithe, fee_mode = "creator", envelope::TAX, no quote
/// override (wNEAR), no creator_share_bps, no init_point.
pub fn launch_args(coin: &CoinArgs, tithe: &AccountId) -> Vec<u8>;
/// The token account and pool id Nearly will give this symbol, from get_slug_count. Mirrors sanitize_slug,
/// short_name and RESERVED_SLUGS (lib.rs:4765-4779, 893-905).
pub fn predicted_pool(symbol: &str, slug_count: u32, nearly: &AccountId, wrap: &AccountId) -> (AccountId, String);

/// The answers of Nearly's get_launch, get_fee_to, get_fee_opts and get_tax, and DCL's get_pool, for one id.
/// Unknown fields are ignored.
pub struct LaunchRecord {
    pub creator: AccountId, pub token: AccountId, pub pool_id: String, pub quote: AccountId,
    pub step: String, pub lpt_id: Option<String>,
    pub fee_to: Option<AccountId>, pub fee_mode: String, pub tax: Option<TaxSeen>,
    pub pool: Option<PoolSeen>,
}
pub struct TaxSeen { pub buy_bps: u16, pub sell_bps: u16, pub creator_bps: u16, pub burn_bps: u16, pub holders_bps: u16 }
pub struct PoolSeen { pub token_x: AccountId, pub token_y: AccountId, pub state: String }

/// Pure. Every reason bind can refuse, in checking order, in one place.
pub fn check_record(r: &LaunchRecord, me: &AccountId, peers: &Peers) -> Result<Coin, BindRefusal>;
pub enum BindRefusal { ViewFailed, NoRecord, NotDone, NotOurs, NotNearPair, WrongFeeTo, WrongFeeMode, WrongTax,
                       BadPoolId(String), NoPool, WrongPoolTokens, PoolNotRunning }
```

`dragon/src/rules.rs` changes only where the tax, auto-burn and cadence touch it.

```rust
/// Buy: DCL checks min_output_amount before the token takes the buy tax, so min_out stays quote * (1 - 150 bps).
/// What the Dragon expects to receive is the net.
pub fn expected_net(quoted_out: u128) -> u128;       // quoted_out * (BPS - TAX.buy_bps) / BPS
/// Sell: the token takes the sell tax from the Dragon's transfer into the pool, so DCL swaps only the net.
pub fn sell_quote_input(gross_in: u128) -> u128;     // gross_in * (BPS - TAX.sell_bps) / BPS
/// Never called while `pending` holds an outflow, because an outflow's transfer can land before its callback.
/// seen = balance - ash_due. A rise is credited to wyrm_bought up to unsettled.wyrm_out (the expected net) and to
/// donated beyond it. auto_burn_bps of the bought part, never of the donated part, moves from the hoard to ash_due.
pub fn reconcile_wyrm(hoard: &mut Hoard, unsettled: &mut Unsettled, balance: u128, auto_burn_bps: u16);
/// v1's day_budget, with the breath part opened linearly over cadence.spread_minutes when it is positive.
pub fn day_budget(s: &Strategy, ladder: &DipLadder, cadence: &Cadence, day: &Day, now: u64, dip: u8) -> u128;
```

`Day::starting` draws the breath minute from `0..(1440 - spread_minutes)`, and `decide` reads the slice gap from `cadence.gap_seconds` instead of the `SLICE_GAP` constant.

`dragon/src/council.rs` holds the Council side, out of `lib.rs`.

```rust
/// Bounds and features through Order::check, then the hoard windows on hoard.wyrm. Returns the change to apply.
pub fn admit(order: &Order, preset: &Preset, s: &Strategy, hoard: &Hoard, grants: u128, burns: u128) -> Result<Change, String>;
// obey, on_outflow, the Window logic and the outflow send live here.
```

`dragon/src/launch.rs` holds `launch`, `on_wiring`, `on_pool_checked`, `on_launch_sent`, `bind`, `on_launch_record` and the two Nearly proxies, next to `nearly.rs`.

### 2f. factory (new, `summon.<root>`)

```rust
#[near(contract_state)]
pub struct Factory {
    wrap: AccountId, dcl: AccountId, nearly: AccountId,
    vault_code_hash: CryptoHash,             // the network's published vault code
    code: [CodeSlot; 3],                     // tithe, dragon, scales: hash fixed in new, bytes stored once
    summons: LookupMap<Label, Summon>,
    labels: Vector<Label>,
    owed_total: u128,                        // sum of every Broken summon's unpaid `owed`
}
pub enum Role { Tithe, Dragon, Scales, Ash }
pub struct CodeSlot { hash: CryptoHash, len: Option<u32> }   // bytes under storage key "code/<role>"
/// [a-z0-9]{1,16}, not in RESERVED ("wyrm", "wyrmling", "thewyrm", "summon", "drill")
pub struct Label(String);
pub struct Summon { creator: AccountId, preset: PresetName, at: u64, status: SummonStatus }
pub enum SummonStatus {
    Assembling,
    Assembled,
    /// `owed` is the funding of the failed batches. The runtime refunds it to the factory in its own receipts,
    /// which may land after on_assembled, so it is paid by `refund(label)`, not in the callback.
    Broken { failed: Vec<Role>, owed: u128 },
}
```

## 3. The presets

The values come from `docs/PARAMETERS.md` and `gym/out/presets.json`. WYRM uses `hoarder`, the Gym's default. Every preset shares the rows marked "all".

| Field | hoarder (WYRM) | steady | dip-hunter | swift | harvester | drill (testnet only) |
|---|---|---|---|---|---|---|
| Budget at launch | 1,000 | 1,000 | 1,000 | 4,000 | 1,000 | 1,000 |
| Budget bounds | 1,000 to 4,000 | 1,000 to 4,000 | 1,000 to 3,500 | 1,000 to 4,000 | 1,000 to 4,000 | 1,000 to 4,000 |
| Dip steps | 2,000 / 3,500 / 5,000 | same | 1,000 / 2,000 / 3,000 | same as hoarder | same as hoarder | same as hoarder |
| Dip extras | 0 / 0 / 0 | 0 / 0 / 0 | 1,000 / 1,500 / 2,000 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| Spread | 0 (one breath) | 360 min | 0 | 0 | 0 | 0 |
| Slice gap | 60 s | 900 s | 60 s | 60 s | 60 s | 60 s |
| Hunt at launch | off | off | off | off | 2x cost, 500 bps a day | off |
| Slice cap | all: 200 at launch, bounds 50 to 200 | | | | | |
| Hunt bounds | all: multiple at least 20,000, max sell 1 to 1,500 | | | | | |
| Auto-burn | all: 0 at launch, bounds 0 to 5,000 | | | | | |
| Features | all: Grant, Burn, Hunt and auto-burn exist | | | | | |
| Sale | 2 NEAR, 10% to the creator, 14 days | same | same | same | same | 0.1 NEAR, 10%, 2 hours |
| Royalty | all: 5% to the creator (`RoyaltyTo::Creator`) | | | | | |
| Seasoning, vote | 7 days, 2 days | same | same | same | same | 10 min, 30 min |

The tax is not a preset field. It is `envelope::TAX`, 400 bps a side with the creator side 10,000, for every launch. The spike guard (15%), slippage (150 bps), dip confirmation (3 samples over 10 minutes), reference window (24 hours), halvings (6) and minimum slice (0.01 NEAR) stay engine constants too. The Gym found nothing to change in them, so a preset field would add choice without a reason.

Decisions taken in this table.

- **Steady is added.** The spread costs two fields in `Cadence`, a changed minute draw in `Day::starting`, a linear factor in `day_budget` and the gap read in `decide`. That is about 15 lines and two tests, and under 1 KB of wasm (estimate). It mirrors the Gym's `allowance` function line for line, so the replay that scored Steady is the rule the contract runs.
- **Dip-hunter's budget ceiling is 3,500, not 4,000.** Its extras sum to 4,500 bps, and v1's rule caps the budget plus the extras at 8,000. The other presets keep v1's 1,000 to 4,000.
- **Harvester launches with Hunt on.** It is the only reason the preset exists. Part 7 item 7 turns Hunt off at launch for WYRM, and hoarder does that. A harvester Council can switch Hunt off with a Light vote.
- **Every preset pays the royalty to the creator.** `RoyaltyTo::Holders` exists for later presets, as Part 7 item 12 allows.
- **Drill exists only for the Council demo.** `Preset::allowed_on` refuses it on any account that does not end in `.testnet`, the factory refuses it the same way, and verify fails any mainnet system that reports it. Section 12 explains why it is needed.
- **The budget floor stays at 10%.** PARAMETERS.md shows 5% and 7% scoring higher but leaving 34% to 46% of income unspent, and the 10% value is "the best value the Council may set, not a measured optimum". Lowering the floor would be a new decision for the operator, so this design does not take it.

`wyrm-types` holds the table as hand-written Rust. A unit test reads `gym/out/presets.json` with `include_str!` and fails when a strategy number in the table differs from the Gym's file. The Gym and the contracts then cannot drift, and no code generator is needed.

## 4. Public methods and who may call them

Every callback is `#[private]`. "1y" means exactly 1 yoctoNEAR attached.

### tithe

| Method | Caller | Behaviour |
|---|---|---|
| `new(dragon, scales, wrap, min_round, reserve)` | init | as v1 |
| `distribute()` | anyone | As v1, except that once `retired_from` is set the Dragon's transfer carries half plus one hundredth of the other half for each retired id, and retired vaults get no transfer |
| `retire(first_unsold: u8)` | only `scales` | `first_unsold` must be 1 to 90. Sets `retired_from` once. A repeat with the same value returns quietly, and a different value panics. |
| `unwrap()`, `forward_ft(token)` | anyone | as v1 |
| `get_stats()` | view | adds `retired_from` |
| `get_config()` | view | new: `{ dragon, scales, wrap, min_round, reserve }` |

### scale-vault

`new()`, `release(to)` (parent only), `get_parent()`, `get_claimable()`. Unchanged.

### scales

| Method | Caller | Behaviour |
|---|---|---|
| `new(preset, dragon, tithe, creator, vault_code_hash, collection)` | init | `preset.check()`, `allowed_on(current_account_id)` and `collection.check()`. Mints 91 to 100 to `creator`. Resolves `royalty_receiver` to the creator or the tithe. Sale is Waiting. |
| `publish_vault_code()` | anyone, payable | unchanged |
| `create_vaults()` | anyone | unchanged, 25 per call |
| `open_sale()` | only `dragon` | Waiting: sets `Open { opened_at: now, ends_at: now + sale.length, sold: 0 }`. Otherwise returns without a change. It never panics after the caller check, so the Dragon's kick cannot be lost to a refusal. |
| `buy_scale()` | anyone, payable | Requires Open, `now < ends_at` and `sold < 90`. Mints the next id to the caller. Pays `price * creator_bps / 10,000` to the creator and the rest to the Dragon, refunds any excess, then sweeps the new Scale's vault to the Dragon under the claim lock (section 7). |
| `close_sale()` | anyone | When the sale is Open and past `ends_at` or sold out, it sets `Closed { sold, tithe_told: sold == 90 }`. Then, and on any later call while `tithe_told` is false, calls `tithe.retire(sold + 1)` and `on_retired`. Returns `"not due"` or `"closed"` otherwise. |
| `on_retired()` | private | Success sets `tithe_told = true`. Failure changes nothing, so the next `close_sale` retries. |
| `sweep(token_ids)` | anyone | 1 to 20 ids. Each must be unminted. Calls `vault.release(dragon)` for each with no callback, and emits `scale_swept`. |
| `claim(token_id)` | owner, 1y | unchanged |
| `on_claimed(...)` | private | Unchanged, except that a mint sweep records `Swept` and adds nothing to `claimed_total`. |
| `list`, `delist`, `buy`, `make_offer`, `cancel_offer`, `accept_offer` | as v1 | The royalty goes to `royalty_receiver`. |
| NEP-171/177/178/181/199 | as v1 | `nft_payout` has two entries, the owner and `royalty_receiver`. |
| `propose(kind, description, token_id)` | holder of a seasoned sale Scale, 1 NEAR + 0.01 | `Order::check(preset)`. The electorate (sold count) must reach `class.min_electorate()`. A Strict kind also needs the sale Closed. One Open proposal per proposer. `ends_at = now + times.voting`. |
| `vote(proposal_id, token_ids, approve)` | holder, 1y | Unchanged. Each Scale must be a sale Scale held since `created_at - times.season`. |
| `finalize(proposal_id)` | anyone, at least 150 Tgas | Open and `now >= ends_at`. Settles the bond. A failed vote ends Rejected, and a passing Signal ends Carried. A passing Order becomes Executing, and scales calls `dragon.obey({order})` with 100 Tgas, then `on_executed`. |
| `on_executed(proposal_id)` | private | `Verdict::Executed` sets Executed. `Verdict::Refused { reason }` sets `LeashHeld { error: reason }`. A failed call sets `LeashHeld { error: "dragon call failed" }`. |
| `get_config()` | view | adds `preset`, `preset_hash`, `engine_version`, `tithe`, `creator`, `royalty_receiver`, `sale`, `vaults_created` |
| `get_council()` | view | enabled kinds, each kind's class, thresholds, bounds, `times` |
| `get_sale()` | view | `{ state, price, creator_bps, sold, ends_at, retired }` |
| `get_sweepable()` | view | Unminted ids. The keeper reads vault balances from RPC. |
| `get_due()` | view | Open proposals past `ends_at` |
| v1 views | view | unchanged, `voting` derived from the id |

### dragon

| Method | Caller | Behaviour |
|---|---|---|
| `new(preset, peers)` | init | `preset.check()` and `allowed_on`. Registers on wrap. `strategy = preset.strategy`. `coin = None`. |
| `launch(coin: CoinArgs)` | `creator`, payable, at least 450 Tgas | `coin` is None, `CoinArgs::parse`, and 10 minutes since `last_launch_at`. Sets `last_launch_at`. Views `tithe.get_config()`, `scales.get_config()`, `nearly.quote_launch(...)` and `nearly.get_slug_count(slug)` in parallel, then `on_wiring`. |
| `on_wiring(coin, deposit)` | private | Section 5a, step 3 |
| `on_pool_checked(coin, deposit, cost)` | private | Section 5a, step 4 |
| `on_launch_sent()` | private | Emits `launch_sent {ok}`. Moves no NEAR. `ok: false` means "the launch may be parked: read get_launch and resume it, do not launch again". |
| `nearly_resume(launch_id)` | anyone, at least 280 Tgas, while `coin` is None | Calls `nearly.resume(launch_id)` with at least 260 Tgas. Nearly lets only the creator resume a parked dev buy or a locker first buy, and the Dragon is the creator. |
| `nearly_cancel_stuck(launch_id)` | `creator` | Calls `nearly.cancel_stuck_launch(launch_id)`. Before bind, any id. After bind, any id except the bound one, so an orphan launch's held dev buy comes back. Nearly allows it 24 hours after a launch parked before its dev buy and refunds to the creator, the Dragon. |
| `bind(launch_id: U64)` | `creator`, at least 150 Tgas | `coin` is None. Batches Nearly's `get_launch`, `get_fee_to`, `get_fee_opts` and `get_tax` (5 Tgas each), then `on_launch_record`. Writes no state. |
| `on_launch_record(launch_id)` | private | Section 5b, step 3. Checks the record, then views `peers.dcl.get_pool(record.pool_id)` and calls back `on_pool_seen`. |
| `on_pool_seen(coin)` | private | Section 5b, step 4. Checks the pool, sets `coin`, kicks `scales.open_sale()`. |
| `poke()` | anyone, at least 760 Tgas | `"not bound"` before bind. Then v1's machine with quote 30 Tgas, swap 180 Tgas, `skip_unwrap_near: true` on sells, the tax rules of section 6, the reconcile rule of section 6, and an auto-burn send when `ash_due > 0` and `pending` is empty. |
| `on_observe`, `on_quote`, `on_swap` | private | v1, with the section 6 changes |
| `on_outflow(seq, kind, amount) -> Verdict` | private | Settles a grant, burn or auto-burn. A failure restores the amount to the hoard (Grant, Burn) or to `ash_due` (AutoBurn), frees the window share, and answers `Refused { reason: "transfer failed" }`. |
| `ft_on_transfer` | NEP-141 | Before bind it accepts wNEAR only. After, it accepts the bound token (donated) and wNEAR. |
| `obey(order: Order) -> PromiseOrValue<Verdict>` | only `scales` | `council::admit`. A refusal emits `order_refused {reason}` and answers `Refused { reason }` with no change. Grant sends `ft_transfer` to the grantee. Burn sends `storage_deposit(ash)` then `ft_transfer(ash)` in one batch. Both resolve through `on_outflow`. Settings orders write the strategy, emit `settings_changed` and answer `Executed`. |
| `get_coin()` | view | `Option<Coin>`. The one mark of the real coin for verify and the Lair. |
| `get_config()` | view | `preset`, the full `Preset` value, `preset_hash`, `engine_version`, `peers`, `strategy`, `coin`, `envelope` |
| `get_state()`, `get_log()`, `get_samples()` | view | v1 plus `coin` and the new hoard fields |

Removed from v1: `set_breath`, `set_slices`, `set_posture`, `pause_buys` and `grant`. `obey` replaces all five.

### factory

| Method | Caller | Behaviour |
|---|---|---|
| `new(wrap, dcl, nearly, vault_code_hash, tithe_hash, dragon_hash, scales_hash)` | init | |
| `store_code(role)` | anyone, payable | The raw wasm is the input. It must hash to the slot's hash. The caller pays its storage. Once per role. |
| `summon(label, preset, collection)` | anyone, payable | Section 5c |
| `on_assembled(label)` | private | Section 5c |
| `refund(label)` | anyone | Broken with `owed > 0`. Requires `account_balance - storage_cost >= owed_total`, so the factory pays only NEAR it certainly holds. Pays `owed` to the summon's creator and sets it to 0. |
| `quote_summon(preset)` | view | Section 8b, from the stored code lengths |
| `get_presets()`, `get_preset(name)`, `get_summon(label)`, `get_summons(from, limit)`, `get_code()` | view | `get_presets` lists only the presets this account may summon |

## 5. Receipt sequences

### 5a. Launch (the creator, both paths)

1. `creator -> dragon.launch(coin)` with at least Nearly's quote plus 0.086 NEAR attached and 450 Tgas. The Dragon checks the caller, that no coin is bound, the cooldown and `CoinArgs::parse`, and sets `last_launch_at`.
   *On failure.* A refusal panics, so the runtime refunds the whole deposit to the creator.
2. The Dragon calls, in parallel, `tithe.get_config()`, `scales.get_config()`, `nearly.quote_launch(icon_bytes, dev_buy, tax: Some(true), text_bytes)` and `nearly.get_slug_count(sanitize_slug(symbol))`, then `on_wiring(coin, deposit)`.
   *On failure.* A failed view reaches `on_wiring` as an error in its slot.
3. `on_wiring` requires every view to have answered and:
   - the tithe names this Dragon, these scales and this wrap;
   - scales names this Dragon, this tithe and `peers.creator`, reports this `preset_hash` and this `ENGINE_VERSION`, and has `vaults_created == 100`;
   - `deposit >= quote.total + LAUNCH_MARGIN`.

   *On failure.* The Dragon sends the whole `deposit` back to the creator as a plain transfer and emits `launch_refused {reason}`. Nothing was sent to Nearly, so this is the creator's own money in the same call.

   Otherwise it computes the token and pool id Nearly will use with `predicted_pool(symbol, slug_count, nearly, wrap)` and calls `peers.dcl.get_pool(pool_id)`, then `on_pool_checked(coin, deposit, cost = quote.total + LAUNCH_MARGIN)`.
4. `on_pool_checked`. If a pool already exists at the predicted id, the Dragon sends the whole `deposit` back and emits `launch_refused {reason: "pool exists"}`, before any NEAR reaches Nearly. A pool at Nearly's own starting point would be adopted rather than fail the launch, but the Dragon cannot read that point, so it refuses any existing pool. If the view failed, it refuses the same way.

   Otherwise it sends `deposit - cost` back to the creator, then calls `nearly.launch({args: launch_args(coin, tithe)})` with `cost` attached, 300 Tgas and unused gas weight 1, then `on_launch_sent`.
5. Nearly's `launch` receipt runs its synchronous checks (lib.rs:790-1004). Text, fee mode, fee_to and tax cannot fail, because `CoinArgs::parse` and `launch_args` follow the same rules. A cost rise from the `pool_fronted` flip fits in the margin. What remains are settings Nearly's owner can change at any time:
   - the factory is paused;
   - `unique_symbols` is switched on and the symbol is taken;
   - `tax_code_hash` is unset ("tax launches are not enabled yet");
   - `max_icon_bytes` is lowered below the icon;
   - the wNEAR quote asset is disabled or removed;
   - `set_config` raises the launch fee by more than the margin between the quote view and the call.

   *On failure.* The runtime refunds the deposit to the predecessor, which is the Dragon. The Dragon keeps it as reserve. See "When Nearly refuses" below.
6. Nearly records the launch with `creator = dragon`, `fee_to = tithe` and the tax, emits `launch_started {id, token, pool_id, ...}`, returns the unused part of the margin to the creator (`extra`, lib.rs:993), and runs `CreateToken -> CreatePool -> AddLiquidity -> (DevBuy) -> Done` on its own receipts. CreateToken registers the creator, the Dragon, on the new token (lib.rs:4508-4513).
   - Short of gas after the first step, Nearly parks the launch (`launch_step_parked`). An AddLiquidity failure also leaves the step in place to resume (lib.rs:4259-4267, 1243). Anyone may call `nearly.resume(id)` for a launch without a dev buy. A parked dev buy or locker first buy needs the creator, so anyone calls `dragon.nearly_resume(id)`.
   - A pool created between step 4 and Nearly's CreatePool, at another price, marks the launch Failed (lib.rs:1214). Nearly refunds the pool deposit to the creator, the Dragon, at once. Anyone may call `nearly.refund_failed(id)` to send the held dev buy to the Dragon too. The creator launches again and gets the next slug.
7. `on_launch_sent` emits `launch_sent {ok}`. `ok: false` does not mean the launch failed. It means the step chain did not reach Done in this transaction. The launch may be parked, so the creator reads `get_launch(id)` with the id from `launch_started` and resumes it, and does not launch again. Only a transaction with no `launch_started` event means Nearly refused at once, and only then is a new launch safe.

**When Nearly refuses.** Every NEAR that Nearly returns during a launch goes to its creator, which is the Dragon. That covers the synchronous refund in step 5, the unused margin (`extra`), the pool deposit after a griefed launch, and `refund_failed` and `cancel_stuck_launch`. All of it stays in the Dragon as reserve and is spent only into swaps after bind. The Dragon never sends Nearly's refunds on to the creator, because its callback cannot tell a refusal in Nearly's first receipt from a failure later in the step chain after Nearly kept the money. A refund there could pay the creator twice out of the treasury. The money then sits in the creator's own Dragon and buys the creator's own coin, so no third party gains from the case. The margin costs the creator at most 0.086 NEAR per launch, paid into their own Dragon.

### 5b. Bind

1. `creator -> dragon.bind(launch_id)` with 150 Tgas. The Dragon requires `coin` to be None. It writes no state.
2. Four parallel views on Nearly with that id: `get_launch`, `get_fee_to`, `get_fee_opts` and `get_tax`, at 5 Tgas each. The record holds no icon after CreateToken (lib.rs:1151), so it is small. Then `on_launch_record(launch_id)`.
3. `on_launch_record`. If `coin` is already Some, a concurrent bind won, and the callback does nothing. Otherwise `nearly::check_record` checks, in this order:
   1. every view answered;
   2. `get_launch` is not null;
   3. `step == "Done"` and `lpt_id` is set;
   4. `creator == current_account_id`;
   5. `quote == peers.wrap`;
   6. `get_fee_to == Some(peers.tithe)`;
   7. `get_fee_opts.mode == "creator"`;
   8. `get_tax` equals `envelope::TAX` in buy, sell, creator, burn and holders bps;
   9. `pool_id` parses with `dcl::parse_pool_id(wrap, token)` and matches the record's `token`.

   *On failure.* Nothing changes, and the Dragon emits `bind_refused {launch_id, reason}`. "not done" is the usual reason, and the creator retries a little later.

   When checks 1 to 9 pass, the Dragon views `peers.dcl.get_pool(record.pool_id)` (10 Tgas), then `on_pool_seen(coin)`.
4. `on_pool_seen`. If `coin` is already Some, it does nothing. Otherwise it requires:
   10. the pool is not null;
   11. its `token_x` and `token_y` are wrap and the coin, in DCL order;
   12. its `state` is `"Running"`.

   This catches a launch Nearly placed on another DCL. Nearly picks the DCL per launch with `dcl_for(id)`, and its owner can change it with `set_dcl` (lib.rs:2939-2950, 3613-3615).

   *On failure.* Nothing changes, and the Dragon emits `bind_refused {launch_id, reason}`.

   When all twelve checks pass, the Dragon sets `coin = Some(coin)`, emits `bound`, and calls `scales.open_sale()` with 10 Tgas.
5. `scales.open_sale()` accepts the Dragon as caller and opens the sale. It cannot refuse after the caller check, and its static gas is tested, so the kick cannot be lost.

After bind the Dragon never calls Nearly again, apart from `cancel_stuck_launch` on an orphan launch. A later change to Nearly's views cannot break a bound Dragon. The Dragon does not register itself on the token, because Nearly already did. It registers ash lazily, in the same batch as each Burn or auto-burn (`storage_deposit` refunds a repeat).


### 5c. Factory summon

1. `stranger -> factory.summon(label, preset, collection)` with `quote_summon(preset)` attached and 300 Tgas. Before anything deploys, the factory checks:
   - the label syntax, the RESERVED list, and that the label is unused;
   - that the preset exists and is `allowed_on` the factory's account;
   - `collection.check()`, the same function scales `new` runs;
   - that all three code slots hold their bytes;
   - that the deposit covers the quote.

   It writes `Summon { creator: stranger, status: Assembling }`. Every argument of every `new` is either checked here or computed by the factory from the label, so no batch can fail on its arguments. Only the factory can create its own sub-accounts, so the reservation guarantees nobody else takes these four names.
   *On failure.* A refusal panics and the deposit returns to the stranger.
2. Four batches go out in parallel, each atomic:
   - `<label>-tithe`: CreateAccount, Transfer(funding), DeployContract(stored tithe bytes), FunctionCall `new(...)`
   - `<label>-dragon`: the same with the dragon bytes and `new(preset, peers { creator: stranger, ... })`
   - `<label>-scales`: the same with the scales bytes and `new(preset, dragon, tithe, creator: stranger, vault_code_hash, collection)`
   - `<label>-ash`: CreateAccount and Transfer(0.01 NEAR). No key and no code.

   *On failure.* A failing batch reverts its own account creation, and the runtime refunds its transfer to the factory in a separate receipt.
3. `on_assembled(label)` joins the four results. If all succeeded, the status is Assembled, and the deposit above the quote goes back to the stranger at once, because the factory has held it since step 1. If any failed, the status is `Broken { failed, owed }`, where `owed` is the funding of the failed batches, and `owed_total` grows by it. The excess goes back at once. `owed` is paid later by anyone's `refund(label)`, once the runtime's refund receipts have landed and the factory's free balance covers `owed_total`.

   With every argument checked in step 1, two causes are left for a failed batch. One is gas exhaustion, and the other is funding below the deployed code's storage. Both are deterministic for a given build. The factory sandbox test pins the gas and checks `quote_summon` against the stored code lengths, so neither should happen in a tested build.

   The deployed parts of a Broken summon keep their funding as storage. They are keyless and have no withdraw method, so no design returns that part. They are inert, because the Dragon's `launch` wiring check fails without a matching tithe and scales. The label stays used, because its accounts exist and `CreateAccount` refuses an existing account.
4. The stranger calls `create_vaults` four times, then follows 5a and 5b on their own Dragon. The factory takes no further part. It never calls Nearly and never holds a coin.

### 5d. Council finalize

1. `anyone -> scales.finalize(id)` with at least 150 Tgas. The proposal must be Open with `now >= ends_at`. Scales zeroes the bond and sends it to the proposer when `bond_returns`, otherwise to the Dragon.
2. A proposal that fails `class.passes(yes, no, electorate)` ends Rejected. A Signal that passes ends Carried. An Order that passes becomes Executing, and scales calls `dragon.obey({order})` with 100 Tgas, then `on_executed(id)`.
   *On failure.* A finalize with less than 150 Tgas prepaid panics before any change, so nobody can starve a passed order into LeashHeld.
3. `dragon.obey` refuses every caller but scales by panicking. Every other refusal is an answer. `admit` checks bounds, features and the window, and a refusal emits `order_refused {reason}` and answers `Refused { reason }`, with a reason that names the limit. Grant and Burn reserve the amount from `hoard.wyrm`, push it to their window, record it in `pending`, send the transfer (Burn first deposits ash's storage in the same batch), and call back `on_outflow`. Settings orders write the strategy and answer `Executed`.
4. `on_executed` sets Executed, or `LeashHeld { error }` with the Dragon's reason.

A passed order exists only inside this one transaction, so nothing can be banked. Two Grants open at the same time are each checked against the window when they finalize, and the one that would break it ends LeashHeld.

### 5e. Closing the sale

1. `anyone -> scales.close_sale()` when the sale is Open and past `ends_at`, or sold out. Scales sets `Closed { sold, tithe_told: sold == 90 }`.
2. If `tithe_told` is false, scales calls `tithe.retire(sold + 1)`, then `on_retired`.
3. The tithe requires scales as the caller and a value of 1 to 90, and sets `retired_from` once. A repeat of the same value succeeds without a change, so two racing `close_sale` calls agree.
4. `on_retired` sets `tithe_told`. If the call failed, the next `close_sale` sends it again. Until it lands, retired vaults keep receiving hundredths, and `sweep` still moves them to the Dragon.
5. The keeper runs one last `sweep` over the retired ids to empty what they received before the tithe knew.

## 6. The tax and the hoard in the engine

The tax is `envelope::TAX`. `launch` writes it into Nearly's args and `bind` requires the same values back from `get_tax`. The token takes the tax on transfers to or from the pool (`tax_for`, token-tax lib.rs:583-599), after DCL has priced the swap, and a wallet-to-wallet transfer pays nothing. Each side is handled where the tax happens.

| Path | What happens on the token | What the Dragon does |
|---|---|---|
| Buy | DCL sends `amount_out` and the token keeps 4%, so the Dragon receives the net | The quote, the cap check and `min_out = quote * (1 - 150 bps)` stay on gross amounts, because DCL checks `min_output_amount` before the tax. Lowering `min_out` by the tax would hand that gap to a sandwich. `unsettled.wyrm_out = expected_net(quoted_out)`, so reconcile books the net as bought and anything above it as donated. |
| Sell (Hunt) | The Dragon sends `amount` and the token keeps 4%, so DCL swaps the net | Candidates are gross amounts of `hoard.wyrm`. Each is quoted at `sell_quote_input(gross)`, and `min_out` is that net quote less 150 bps. The cap's unit quote is also taken on net input. A failed swap returns the transfer through `ft_resolve_transfer`, and reconcile reads the balance. |
| Grant, Burn, auto-burn | Neither side is a pool, so there is no tax | none |
| Dev buy | DCL to the creator is a pool transfer, so it is taxed | Arrives before bind and is booked as donated, net, at the first observation |
| Price samples and reference | Gross quotes | The tax is a constant factor, so dips, spikes and the cap ratio are unchanged and the Gym's tuning carries over |

**The hoard and outflows.** v1's reconcile subtracted grants in flight from the balance. That double-books an outflow whose transfer lands before its callback. The balance has already dropped and the in-flight amount is subtracted again, and when the callback clears it, the next reconcile books the amount as a donation. v2 changes the rule.

- Sending an outflow moves its amount out of `hoard.wyrm` (Grant, Burn) or out of `ash_due` (auto-burn) and records it in `pending` with a sequence number and the time.
- `on_observe` skips the WYRM reconcile while `pending` is not empty. Buys and the wNEAR reconcile go on. A buy that lands meanwhile stays in `unsettled` and is booked at the next observation after the outflow settles.
- Otherwise `seen = balance - ash_due`, and a rise is split into bought and donated as before. `auto_burn_bps` of the bought part moves from the hoard to `ash_due`. Donations never feed auto-burn.
- `on_outflow` removes its entry from `pending`. A failure returns the amount to `hoard.wyrm` (Grant, Burn) or to `ash_due` (auto-burn) and frees the window share.
- An entry older than 10 minutes is dropped at the next observation, so a lost callback cannot stop the reconcile for good. Its late callback finds no matching `seq` and changes nothing, as v1's late swap callbacks do. A failed transfer found this way is booked as donated, which keeps the hoard's amount right but leaves its window share used.
- The Grant and Burn caps and Hunt's `sell_room` are measured on `hoard.wyrm`, which never includes `ash_due`.

Average cost is `near_spent / wyrm_bought`. `wyrm_bought` is net, so the tax shows up as a 4% higher cost, and Hunt's sell line uses that true cost. The creator side of the Dragon's own tax returns through the tithe, and half of that comes back to the Dragon.

## 7. Unsold Scales

The operator's default holds. The sale runs 14 days or until sold out, anyone may sweep an unminted vault to the Dragon at any time, and unsold Scales retire at close. This design changes three things.

1. **The sale opens at bind, not at init.** Nobody pays 2 NEAR for a Scale of a coin that does not exist yet. The 14 days count from the coin's first live block, and a failed or griefed launch strands no buyer.
2. **Each mint sweeps first.** `buy_scale` mints id n, sets `claiming_since` on it, and calls `vault(n).release(dragon)`, which settles through `on_claimed` and records `Swept`. Income from before the sale goes to the Dragon by the same rule as unsold income, so a late buyer cannot collect weeks of rounds by buying a full vault. The lock is v1's claim lock. The token cannot move or be claimed until the sweep answers or 10 minutes pass. A round that lands between the mint and the sweep goes to the Dragon too, which errs toward the Dragon.
3. **At close the tithe learns once.** Retirement is derived in scales (`retired(id)`), and `close_sale` passes the first unsold id to the tithe. From the next round on, the tithe adds the retired hundredths to the Dragon's transfer and sends nothing to those vaults. The round has fewer transfers and costs less. The Dragon's income from unsold Scales then needs no keeper for as long as the system lives.

**Rejected, sweeping retired vaults forever.** That is the brief's default. Every round would leave up to 90 vault balances for a keeper to sweep, at about 7 Tgas each (`claim` burns 6.93 Tgas, docs/GAS.md), and the Dragon's income would depend on a keeper outliving the hackathon.

**Rejected, a tithe that reads the sold count on every round.** That puts a cross-contract read and a new failure mode into the contract whose address Nearly holds forever. The push happens once, carries one number, and the tithe accepts only a suffix of the sale ids from scales.

**Rejected, minting unsold Scales to the Dragon at close.** The Dragon could not vote, sell or claim with them without new methods, and it adds up to 90 writes at close.

**Why the sale still closes.** A flat price and an open-ended sale would let an attacker buy most of the voting supply later at the launch price and capture the Strict class after seasoning. A closed sale caps the electorate at what sold in 14 days, as in v1.

## 8. Wasm, storage and who pays

### 8a. Wasm size

| Contract | Today | v2 estimate | Why |
|---|---|---|---|
| scales | 398 KB | 390 to 405 KB | Tiers, `execute`, expiry and the grant lock go. Sale state, sweep, close, Signal and `wyrm-types` come in. |
| dragon | 254 KB | 260 to 275 KB | `nearly.rs`, `launch.rs`, `council.rs` and the cadence go in. Four setters, pause and the bind state machine go out. |
| tithe | 116 KB | 117 to 119 KB | `retire`, one field, one view |
| scale-vault | 16.8 KB | 16.8 KB | unchanged |
| factory | none | 150 to 200 KB (guess) | `Preset` and `CollectionArgs` serde and the code slots |

Measure each after Phase D and put the numbers in `docs/GAS.md`. `quote_summon` reads the stored code lengths, so the deposit follows the real sizes with no constant to update.

### 8b. A factory launch, paid by the creator

Storage costs 1 NEAR per 100 KB locked. Global code by hash costs 10 times that and is burned (`GLOBAL_CODE_BYTE_COST`, 0.0001 NEAR a byte, `contracts/scales/src/vaults.rs`).

| Item | Size or count | NEAR | Source |
|---|---|---|---|
| tithe code | 116 KB | 1.16 locked | today's wasm |
| dragon code | 254 KB | 2.54 locked | today's wasm |
| scales code | 398 KB | 3.98 locked | today's wasm |
| scales state headroom | | about 4.0 locked | v1 funds scales with 8 NEAR, code included (SPEC "Market") |
| dragon rings (1,440 samples, 256 log entries) | | about 2.1 locked | estimate |
| tithe state and reserve | | about 0.1 locked | v1 `reserve` argument |
| 100 vaults on the shared vault code | 100 x 0.01 | 1.0 locked | `VAULT_FUNDING` |
| ash | | 0.01 locked | |
| **Total per launch** | | **about 14.9** | plus Nearly's `quote_launch` total and 0.086 NEAR margin at `launch`, plus gas |

The code rows are 7.68 NEAR, which is the "about 8 NEAR more per launch" of Part 7 item 15.

**The vaults use one global copy, published once per network, by whoever deploys first.** Copying the vault code into each vault would lock 100 x 16.8 KB = 1.68 MB, or 16.8 NEAR, on every launch. One copy by hash burns 16,800 bytes x 0.0001 = 1.68 NEAR once, and every system on the network then uses it for free, because code by hash is immutable and shared. WYRMLING or WYRM publishes it as part of its own costs, 1.68 NEAR against the 16.8 NEAR that system would otherwise lock in its own vaults. The factory publishes nothing and pays nothing for vaults. If the hash were ever missing on a network, the first creator's `scales.publish_vault_code()` would publish it, and `create_vaults` fails until someone does.

**The factory's own storage is the one cost on our side.** The factory must hold the engine bytes to deploy them. That locks 768 KB, or about 7.7 NEAR, plus the factory's own code, about 1.5 to 2 NEAR (guess). That is about 9 to 10 NEAR locked once, not burned, against the 70 to 85 NEAR burn Part 7 item 15 rejected. `store_code` is payable by anyone, but in practice we call it.

### 8c. WYRM's direct deploy, paid by us as WYRM's creator

The same 14.9 NEAR, plus 1.68 NEAR to publish the vault code if no one has, plus Nearly's launch quote and the margin. WYRMLING's rehearsal on Nov 8 costs the same again and publishes the vault code for both.

## 9. Gas

Estimates, to be measured in sandbox against DCL 2.4.1 wasm and a mock Nearly. Every chain stays under the 1,000 Tgas transaction limit. The poke floor is 760 Tgas everywhere in these documents.

| Call | Attached by the caller | How it adds up |
|---|---|---|
| `poke` | 1,000 Tgas, floor 760 | The floor is the chain's static gas. 4 views (5 + 5 + 10 + 30) and `on_observe` 600. `on_observe` covers 7 halving quotes at 30 Tgas each with their callbacks (about 310), `on_quote` 230 (deposit 5, swap 180, `on_swap` 10, fees) and an auto-burn batch of 40 (ash deposit 5, `ft_transfer` 10, `on_outflow` 10, fees). The rest is the Dragon's own receipts. |
| `launch` | 450 Tgas | 4 views at 10, `on_wiring` 15 with `get_pool` 10, and `on_pool_checked` 335 (Nearly 300, `on_launch_sent` 10, the change refund and its own work). Nearly gets any gas left over. |
| `bind` | 150 Tgas | 4 Nearly views at 5, `on_launch_record` 15 with `get_pool` 10, and `on_pool_seen` 30 with `open_sale` 10 |
| `nearly_resume` | 280 Tgas | `resume` 260, Nearly's own floor for a first buy (lib.rs:1029) |
| `finalize` | 150 Tgas, enforced | `obey` 100 (a burn's deposit 5, `ft_transfer` 10 and `on_outflow` 10 inside it) and `on_executed` 10 |
| `create_vaults` | 300 Tgas | 25 vaults, 254 Tgas measured (docs/GAS.md) |
| `summon` | 300 Tgas | 3 deploys of 0.12 to 0.4 MB at about 71 Mgas a byte (about 55 Tgas for 768 KB), 4 creates, 3 `new` calls and `on_assembled` |
| `close_sale` | 50 Tgas | `retire` 10 and `on_retired` 5 |

Nearly's `launch` needs only about 87 Tgas to pass its synchronous gas check at CreateToken (6 + 8 + 10 step gas, 45 callback, 18 reserve; lib.rs:79-150 and 4463-4467). Past that, a short-gassed launch parks and anyone resumes it. So a wrong 300 Tgas cannot lose a launch. It can only park one. The real gas of a contract-called launch is still unmeasured, and it is a gate before mainnet.

## 10. What verify, audit-dragon, the keeper and the Lair expect

These are the v1 checks (master plan Part 5) as v2 changes them.

- **Check 1, keys.** Adds ash, which must have no keys and no code.
- **Check 2, code hashes.** Adds the factory and the preset. `get_config().preset` must not be `drill` on mainnet. `preset_hash` and `engine_version` on scales and dragon must equal each other, and the hash must equal the factory's `get_preset(name)` hash.
- **Check 3, the coin.** The real coin is `dragon.get_coin()`, and nothing else. A fake WYRM can exist, and an orphan launch by our own Dragon can exist, but neither is the bound coin. verify then checks the bound record: `get_launch(coin.launch_id).creator == dragon` and `get_fee_to == tithe`.
- **Check 4, the fee terms.** `get_tax(launch_id)` equals `envelope::TAX` (400 / 400, creator 10,000) and `get_fee_opts.mode == "creator"`. `get_pool(coin.pool_id)` on our DCL is Running with wrap and the coin.
- **Check 5, tithe rounds.** Before `retired_from` a round is half to the Dragon and 1/100 to each vault. After, the Dragon's transfer adds one hundredth of the vault half for each retired id, and retired vaults get nothing.
- **Checks 6 and 7, buys.** Unchanged. The budget and cap are on gross NEAR in and gross quotes, so the tax does not enter them.
- **New check, taxed buys.** For each buy slice the WYRM delivered is `amount_out * (1 - 4%)`, and `amount_out >= min_out`. audit-dragon pairs the token's two transfer events, the net and the one with memo `tax`.
- **New check, taxed sells.** Each Hunt sell sent `gross`, DCL received `gross * (1 - 4%)`, and the wNEAR out met `min_out` computed on that net.
- **Check 8, outgoing receipts.** Adds Nearly (`launch`, `resume`, `cancel_stuck_launch`, the launch and bind views), DCL's `get_pool`, ash (`storage_deposit`, `ft_transfer`), and the creator (only the unsent part of a launch deposit, in `on_wiring` or `on_pool_checked`).
- **Check 9, the Council.** Every executed proposal's order passes `Order::check` against the preset, and each one went from Open to Executing in its finalize transaction. Every LeashHeld proposal has a matching `order_refused` event or a failed transfer.
- **Check 10, the collection.** 100 ids. 91 to 100 belong to the creator and never vote.
- **Check 11, the sale.** Every `buy_scale` paid `price * creator_bps` to the creator and the rest to the Dragon, and emitted `Swept`.
- **Check 12, the dev wallet.** Now structural. Nearly's dev-buy recipient is the Dragon.
- **Check 13, outflows.** Grants and burns each stay within 50 bps a proposal and 100 bps per 30 days of `hoard.wyrm`.

The keeper is section 1d. It drops `execute`, the Passed and Expired handling, and `open_sale`, and adds `push_creator_fees`, `close_sale` and `sweep`. It pokes only when `get_coin()` is `Some`. Before bind it watches the creator's launch and resumes it if it parks.

The Lair names the real coin only by `dragon.get_coin()`. It gains the Summon wizard, a sale page that says "opens when the coin is bound", retired Scales in the grid, Council forms generated from `Order` that offer only the preset's enabled kinds, a LeashHeld reason on each refused proposal, and an estimator that counts the tax share, so 0.48% of buy volume plus 60% of the 4% tax on all volume reaches the tithe (PARAMETERS.md, estimator). Every page of a `drill` system carries a "testnet" banner.

## 11. Module map

No source file may pass 1,000 lines. Today `dragon/src/lib.rs` has 805 lines and `dragon/src/rules.rs` 962, of which 466 are tests.

| Path | Status | What changes |
|---|---|---|
| `contracts/wyrm-types/src/{lib,preset,collection,order,coin}.rs` | new | `ENGINE_VERSION`, `PresetName`, `Preset`, `envelope` (with `TAX`), the table, `Preset::check`, `CollectionArgs`, `Order`, `Verdict`, `VoteClass`, `CoinArgs`, the test against `gym/out/presets.json` |
| `contracts/tithe/src/lib.rs` | changed | `retired_from`, `retire`, distribute pays retired shares to the Dragon, `get_config` |
| `contracts/scale-vault/src/lib.rs` | unchanged | |
| `contracts/scales/src/ids.rs` | changed | `SALE_IDS` 1 to 90, `CREATOR_IDS` 91 to 100, `ScaleId::votes()` |
| `contracts/scales/src/sale.rs` | rewritten | `Sale` enum, flat price and creator cut, `open_sale` from the Dragon, mint sweep, `sweep`, `close_sale` and `on_retired`, `get_sweepable` |
| `contracts/scales/src/council.rs` | rewritten | `ProposalKind = Order or Signal`, classes and timings from the preset, `finalize` executes, `on_executed` reads `Verdict`, `get_council`, `get_due`. `execute`, `Expired`, `execute_by`, `grant_in_play`, `Track` and `dragon_call` deleted. |
| `contracts/scales/src/lib.rs` | changed | `preset`, `tithe`, `royalty_receiver`, `CollectionArgs` in `new`, `Swept`, `Scale.voting` removed |
| `contracts/scales/src/{market,nft}.rs` | changed | royalty to `royalty_receiver` |
| `contracts/scales/src/vaults.rs` | changed | `release_to(id, payee)` shared by claim, the mint sweep and `sweep` |
| `contracts/dragon/src/lib.rs` | shrinks | state, `new`, `poke` and its callbacks, `ft_on_transfer`, views wiring, gas constants. The five Council setters move out and are replaced. |
| `contracts/dragon/src/launch.rs` | new | `launch`, `on_wiring`, `on_pool_checked`, `on_launch_sent`, `bind`, `on_launch_record`, `on_pool_seen`, `nearly_resume`, `nearly_cancel_stuck` |
| `contracts/dragon/src/nearly.rs` | new | `launch_args`, `predicted_pool`, `LaunchRecord`, `check_record` |
| `contracts/dragon/src/council.rs` | new | `obey`, `admit`, the two windows, the outflow send (with the ash deposit) and `on_outflow` |
| `contracts/dragon/src/limits.rs` | shrinks | Engine-fixed constants stay (spike, slippage, dip confirmation, halvings, stale, ring caps). Council ranges move to `wyrm-types`. Pause deleted. |
| `contracts/dragon/src/rules.rs` | changed | `Strategy` replaces `Settings`, `expected_net`, `sell_quote_input`, the new reconcile, cadence in `Day::starting`, `day_budget` and `decide`. Its tests move out. |
| `contracts/dragon/src/rules_tests.rs` | new | the 466 lines of rules tests, moved, plus the new ones |
| `contracts/dragon/src/dcl.rs` | changed | quote 30 Tgas, swap 180 Tgas, `skip_unwrap_near: true` on sells, the partial-fill comment fixed |
| `contracts/dragon/src/{events,views}.rs` | changed | `launch_sent`, `launch_refused`, `bind_refused`, `bound`, `order_refused`, `burn_paid`, `auto_burn`, `outflow_failed`, `settings_changed`. Pause fields gone. |
| `contracts/dragon/src/{ring,math}.rs` | unchanged | |
| `contracts/factory/src/{lib,code}.rs` | new | factory, code slots, `refund` |
| `contracts/mock-nearly/src/lib.rs` | new | `launch` that records the predecessor as creator, registers it on the token, and can park, fail on a pre-created pool or panic; `get_launch`, `get_fee_to`, `get_fee_opts`, `get_tax`, `get_slug_count`, `quote_launch`, `resume`, `refund_failed`, `cancel_stuck_launch`, `push_creator_fees`. Its JSON shapes are recorded from mainnet answers. |
| `contracts/mock-ft/src/lib.rs` | changed | a tax switch with `pairs`, `buy_bps` and `sell_bps`, and the token-tax refund on resolve |
| `contracts/mock-dcl/src/lib.rs` | changed | `quote` panics `E403` for a missing pool and caps output at reserves. `get_pool` returns `null` for a missing pool. |
| `contracts/tests/tests/{bind,tax,unsold,factory}_sandbox.rs` | new | bind refusals, the coin's tax, unsold Scales and the factory |
| `contracts/build.sh` | changed | builds factory and mock-nearly, runs the `wyrm-types` tests first, and fails when any source file passes 1,000 lines |
| `keeper/`, `verify/`, `lair/` | changed | section 10 |
| `docs/SPEC.md` | rewritten for v2 | after Phase D |

No file is deleted.

## 12. Showing the Council before the deadline

**The problem.** Seasoning stays at 7 days and every vote lasts 2 days. A WYRM proposal needs a Scale bought in the sale, which opens at bind on Nov 9, plus 7 days of seasoning. The first proposal can open on Nov 16 and finalize on Nov 18. Development ends 2026-11-14T23:59Z. No WYRM Council vote can happen on mainnet inside the window. Moving the deploy earlier does not save it, because a proposal opened by the deadline needs a bind by Nov 7 and a finalized one needs a bind by Nov 5, and the v2 contracts do not exist yet.

**The proposal.** Run a `drill` system on testnet with the same wasm, the same code hashes and the same engine, against our mock Nearly, mock DCL and mock taxed token deployed on testnet (Nearly is mainnet only). The `drill` preset differs from `hoarder` in four values. Seasoning is 10 minutes, a vote 30 minutes, the sale 2 hours, and a Scale costs 0.1 NEAR. Everything else, the classes, the thresholds, the minimum electorates of 10 and 45, the bounds and the windows, is the same code path. One testnet account buys 50 Scales for 5 testnet NEAR, waits for the seasoning and the 2-hour sale to end, and runs one of each kind in an afternoon. The set is a Light SetSlices that passes, a Strict Grant at 2/3 turnout and 80% yes, a Strict Hunt-on that fails at 79%, a Burn to ash, a Signal that ends Carried, and a third Grant inside the same 30 days that would pass 1% of the hoard and ends LeashHeld with the Dragon's reason.

**Keeping it honest.**

- `drill` refuses any account that does not end in `.testnet`, in each engine's `new` and in the factory. A mainnet system cannot run it, and verify fails any mainnet system that reports it.
- The video and the README label every Council scene "testnet, mock Nearly", with its testnet explorer link. The Lair shows a testnet banner on every drill page.
- On mainnet the judges see the real Council surface. The proposal form, the bounds from `get_council()`, and a countdown to Nov 16, when the first Scale bought on Nov 9 becomes eligible.
- The sandbox suite runs the same scenarios on the mainnet timings, and the README links its output.

## 13. Rationale

### Problem

v2 turns four contracts built for one coin into an engine that anyone can launch, while every bound stays fixed for life after the keys are deleted. Four facts make the shape non-obvious. The coin's token account and pool id are unknown until Nearly's launch lands, because tickers are not reserved and the slug may be `wyrm-2`. Nearly treats whoever calls `launch` as the creator, and that account alone receives dev-buy tokens and refunds and may resume a parked dev buy (lib.rs:792, 1009-1035). Presets must be named and shared by a direct deploy and a factory, with more added later. And the 4% tax, Nearly's maximum, is taken by the token on pool transfers, not by DCL. Four constraints crossed into this design from the existing system. The tithe is `fee_to` forever and must stay small. The Council needs token ownership in the same receipt, so it stays in scales. DCL delivers swap output in a separate receipt. The operator decided that the creator pays the whole cost of a launch.

### Usage (caller's view)

Section 1. The creator deploys four accounts, makes the vaults, then calls `dragon.launch(coin)` and `dragon.bind(launch_id)`. A stranger calls `factory.summon(label, preset, collection)` and then the same calls on their own Dragon. A holder calls `propose`, `vote` and `finalize`. The keeper calls `poke`, `push_creator_fees`, `distribute`, `finalize` from `get_due`, `close_sale` and `sweep`.

### Shape

**The Dragon is Nearly's creator.** `launch` writes `fee_to`, `fee_mode`, the tax and the wNEAR quote itself, so the caller supplies only text and an optional dev buy. Bind's ownership check is `creator == current_account_id`, which no other account can satisfy. The dev buy and every Nearly refund land in the hoard with no forwarding code. The Dragon returns to the creator only the unsent part of the deposit of the same call, computed from callback arguments, never from its balance.

**Launch checks the wiring and the pool, and bind checks the record and the pool.** `fee_to` is permanent, so before Nearly is asked the Dragon proves that the tithe and scales name it and its creator, share its preset hash and engine version, and have all 100 vaults, that the deposit covers the quote and its margin, and that no pool sits at the coin's predicted id. `bind` turns Nearly's four answers and DCL's pool into one `LaunchRecord` and runs one pure `check_record`, with every refusal reason in one enum (per boundary-discipline). After bind the Dragon has no dependency on Nearly.

**The coin is an `Option`.** Only `Some(Coin)` carries a pool id, so `poke` cannot trade an unbound coin (per type-system-discipline). Bind writes no in-flight state, so a lost callback leaves nothing to clean up, and a concurrent bind's callback finds `Some` and does nothing (per make-operations-idempotent).

**Presets are a compiled table, keyed by name.** `new` takes `"preset": "hoarder"`. Every value lives once, in `wyrm-types`, and a unit test ties the table to the Gym's output file. The engines run `Preset::check` again in `new`, and scales and dragon report the preset hash and engine version that `launch` compares. The tax is a single constant, because every preset uses Nearly's maximum (per laziness-protocol: one source of truth).

**The Council has one verb, no waiting state and no panics.** `ProposalKind = Order(Order) | Signal`, and the Dragon exposes `obey(order) -> Verdict`. The class is a match on the order. `finalize` executes, so the Passed state, the 72-hour window, Expired and the one-grant lock all disappear with the problem they guarded, a passed proposal waiting to be used (per subtract-before-you-add). The windows in the Dragon still bound concurrent payouts. A floor of 150 Tgas on `finalize` stops anyone from starving a passed order. A refusal comes back as a reason, so LeashHeld says which limit held.

**The hoard counts what the Dragon owns.** `hoard.wyrm` is the balance less `ash_due`, and the reconcile waits while an outflow is in flight, so an outflow is counted once whichever of its transfer and its callback lands first.

**Unsold income reaches the Dragon without a keeper.** The mint sweep takes pre-sale income, and the tithe's one-time `retire` routes retired shares from the round itself. The tithe stores one number, `retired_from`, which can only describe a suffix of the sale ids.

**What the system deliberately does not do.** The factory never calls Nearly and holds no coin. The Dragon never refunds money Nearly might have kept. The tithe never reads another contract. No order moves NEAR, and no order changes a rule, a bound or a timing.

**Interface depth.** The Dragon's Council surface shrinks from five setters to `obey`. It gains `launch` and `bind`, which hide Nearly's argument format, the slug prediction, the record and pool checks, the preset and version checks and the deposit split. The creator never handles `fee_to`, `fee_mode`, `quote`, `tax`, `pool_id` or the token account. The tithe gains `retire`, which only scales can call. The factory exposes `summon` and `refund`.

### Synthesis decision

**Base: candidate B.** The Dragon calling Nearly's `launch` gives the strongest bind check, because no impostor launch can name our Dragon as creator. It also puts the dev buy straight into the hoard and gives the direct deploy and the factory one path. B's launch shape was checked against Nearly's source on Nov 4:

- `fee_to` equal to the creator is dropped (lib.rs:828), and the tithe is never the Dragon, so the case cannot occur.
- `launch` returns the step chain's result, not the id. `bind(launch_id)` verifies `get_launch(id).creator`, `get_fee_to(id)`, fee mode, `get_tax(id)` and `step == Done`, then takes `token` and `pool_id` from the record.
- The Dragon attaches 300 Tgas to `launch` with unused gas weight. The synchronous floor is about 87 Tgas, and a short-gassed launch parks rather than fails. The real gas is a mainnet gate.
- B's open problem, the deposit after Nearly refuses, is settled in section 5a. Every NEAR Nearly returns goes to the Dragon and stays as reserve. Nearly's own `refund_failed` (anyone may call it) and `cancel_stuck_launch` (creator only, proxied) recover dev-buy NEAR into the Dragon.

**Grafted from A.**

- **Finalize executes.** It deletes `execute`, the 72-hour window, Expired and the one-grant lock, and the keeper loses a step. It holds up because the Dragon's windows still cap payouts and a 150 Tgas floor on `finalize` closes the one new hole.
- **The tithe routes retired shares at close.** It needs one method and one field on the tithe, and it removes a keeper sweep that would otherwise run for the life of the system. A's ScaleSet argument became a single `first_unsold` number, so a non-suffix is not representable.
- **Compiled preset table by name.** It reads simpler than B's preset passed as a JSON value. Init takes a name, every value is in one Rust file, and verify compares a name, a hash and a version.
- **Creator Scales derived from the id.** `Scale.voting` duplicated what the id already says.
- **The bind pool check.** A's `get_pool` at bind came back through the review, because Nearly can place a launch on another DCL.
- **Tax handling.** Both candidates agreed. A's wording of the sandwich reason for keeping buy `min_out` on the gross quote is kept.

**Changed by the adversarial review.**

- Bind checks the DCL pool, and launch refuses a pool that already sits at the predicted id.
- `dragon.get_coin()` is the only mark of the real coin. An orphan launch can be cancelled after bind, and `launch_sent {ok: false}` means "resume, do not relaunch".
- The reconcile skips while an outflow is in flight and excludes `ash_due`, which fixes a double-booking of outflows that v1's formula had. This synthesis added a 10-minute expiry for a pending outflow, so a lost callback cannot stop the reconcile for good.
- The factory checks every `new` argument before it deploys, records what a Broken summon is owed, and pays it through `refund(label)`.
- Launch sends a 0.086 NEAR margin above the quote, icons refuse characters that `json_len` counts twice, and the owner-mutable refusals are listed.
- `RoyaltyTo` is `Creator | Holders`, with Holders paid to the tithe.
- Bind is `Option<Coin>` with no in-flight states and no registration step. Ash is registered with each outflow. scales opens the sale on the Dragon's call.
- The tax is a constant.
- The Dragon is split into `launch.rs`, `nearly.rs` and `council.rs`, and the rules tests move out, so no file passes 1,000 lines.
- `ENGINE_VERSION` joins the preset hash in the wiring check.
- `obey` answers with a `Verdict`.
- The runbook publishes the vault code after scales exists.

**Rejected from A.** The `den` per-launch contract. B's Dragon-as-creator removes the need for a launcher contract and its `forward` step. A's global-code engines, which the operator ruled out. A's hand launch for WYRM, because B's single path is the same for WYRM and strangers. A's `tune` and `pay` split, because one `obey(Order)` is a smaller surface with the same checks.

**Rejected from B.** `breaths_per_day`, replaced by the Gym's own spread rule in `Cadence`. Vault batches of 50, because v1's 25 is measured and tested. The Checking and Registering bind states and the `Checking` sale state, because an `Option` and idempotent callbacks do the same work. Global-contract engines as a future switch, because the operator decided per-launch copies.

### Tradeoffs accepted

- We accept that a launch Nearly refuses in its first receipt leaves its deposit in the creator's own Dragon, and that the unused margin of every launch does too, in exchange for a callback that can never pay the creator twice.
- We accept that the Dragon mirrors Nearly's slug rule and pool fee to predict the pool id, in exchange for refusing a griefed launch before any NEAR leaves. If Nearly changes either, the prediction misses, the check passes without effect, and bind's pool check still guards the result.
- We accept a new engine build and a new factory for each new preset, in exchange for presets that live in one file, cannot differ between scales and dragon, and are named in `new`.
- We accept a tax that no preset can change, in exchange for deleting the tax-dependent cap rule until a preset without the tax exists.
- We accept one more method on the permanent tithe, in exchange for unsold income that needs no keeper after the sale.
- We accept that a passed proposal cannot wait for a better moment, in exchange for no banking, no expiry and no grant lock.
- We accept a WYRM reconcile that waits while an outflow is in flight, and a lost outflow callback that expires after 10 minutes with its window share still used, in exchange for an outflow that is counted once whichever receipt lands first.
- We accept 0.00125 NEAR for an ash deposit on every Burn and auto-burn (refunded when ash is already registered), in exchange for no registration step at bind.
- We accept that the deployed parts of a Broken summon keep their funding as storage, in exchange for keyless engines with no withdraw method. The failed parts' funding is paid back by `refund(label)`.
- We accept that every mint costs a vault release and a 10-minute claim lock on the new Scale, in exchange for pre-sale income going to the Dragon with no race.
- We accept that turning Hunt on, or moving its line while it is on, is always Strict, in exchange for a class that is a pure function of the proposal.
- We accept that the Dragon trusts the tax it saw at bind for life, in exchange for no Nearly read on every poke. A later rise would make sells fail closed on `min_out`.
- We accept about 9 to 10 NEAR locked by us in the factory for the engine bytes and its own code, in exchange for no global-code burn and a creator-paid launch.
- We accept https and ipfs icons only, with no quote, backslash or control characters, in exchange for an icon whose quoted size equals Nearly's charge and that Nearly's size check never refuses at 512 bytes.
- We accept a testnet-only preset compiled into mainnet code, in exchange for demonstrating the real Council code before the deadline. The account-suffix check keeps it off mainnet.

### Alternatives considered

- **The creator calls Nearly's `launch` by hand, and bind checks `creator == peers.creator`.** It exposes more to callers. The creator must assemble `fee_to`, `fee_mode` and the tax with a permanent penalty for a mistake, and a contract creator must forward dev-buy tokens. The check then depends on the creator account staying honest. It lost on interface depth and on trust.
- **A per-launch launcher contract (A's den).** It keeps the Dragon from ever receiving a Nearly refund, but adds a fifth contract, a forward step for dev-buy tokens, and a second creator identity to check. The Dragon receiving refunds is harmless when it never sends them on.
- **Presets passed as a value at init (B).** Engine code stays the same across preset additions, but every deploy carries a 30-field JSON value, and both engines keep copies that only a hash comparison ties together. It hid less for the same new-factory cost.
- **Global-contract engines.** It saves about 8 NEAR a launch and burns about 77 NEAR once. The operator ruled it out on Nov 4.
- **Execute as a separate step (v1).** It needs Passed, Expired, `execute_by`, the grant lock and a keeper action, all to guard a state that exists only because execution is separate.

### Implementation reconciliation

The contracts as built, release v2.0 in `verify/releases.json`, differ from this design in these ways. Each entry names its source. [SPEC.md](../../SPEC.md) describes the code as built.

- **The factory and the Summon wizard are not built** (operator, Nov 8). Sections 1b, 2f, 5c and 8b, the factory rows of section 4 and risks 33 to 36 describe a design. WYRM deploys directly as section 1a lays out, with `keeper/deploy.mjs`. The Council demo used the `deploy.mjs --preset drill` path on testnet, at `council.thewyrm.testnet`.
- **A Scale costs 2 NEAR** on every mainnet preset (operator, Nov 8), not 5 NEAR. The creator still gets 10% of each sale and the Dragon 90%. Section 3 already shows the built value.
- **`launch` requires 500 Tgas**, not 450. It pays for four views, the wiring callback with its `get_pool`, the pool callback and Nearly's 300 Tgas (`contracts/dragon/src/launch.rs`). On mainnet, WYRMLING's launch reached Done with 500 attached and 85.11 Tgas burnt.
- **`nearly_resume` requires 300 Tgas**, not 280, because Nearly's own resume refuses less than 222 Tgas at AddLiquidity, 235 at DevBuy and 263 for a first buy that opens the pool.
- **The ash deposit is 0.0125 NEAR**, ten times the token's minimum storage deposit of 0.00125 NEAR. The token refunds what it does not use, and all of it once ash is registered.
- **The grant and burn caps are measured on the hoard at each order** (operator, Nov 6), as in the code before v2. Two maximal grants in one 30-day window add up to 0.995% of the starting hoard, never quite 1%. This is stricter than the design, never looser.
- **Nearly's overpayment refund stays in the Dragon.** About 0.087 NEAR of each launch deposit comes back from Nearly to the Dragon, on mainnet too, and joins its NEAR reserve.
- **WYRMLING keeps its keys and is torn down** (operator, Nov 8). It runs the mainnet gates and is then emptied with the `state-cleanup` contract and deleted by `deploy.mjs --step teardown`, which recovers its storage deposits. Key deletion was proven on testnet. WYRMLING published the network's vault code on mainnet for 1.681 NEAR, and WYRM uses it by hash.
- **Nearly pays `fee_to` on its own schedule.** Its relayer pushed the pool-fee share to WYRMLING's tithe by plain transfer, so the keeper's hourly `push_creator_fees` is best-effort. The tax share is paid only after Nearly's own keeper sells the tax tokens ([THREAT-MODEL.md](../../THREAT-MODEL.md), gate 6).
- **The RPC client fails over** (operator, Nov 10). `keeper/chain.mjs` lists three free mainnet RPCs and moves to the next one on a rate limit, and `--rpc` takes a comma-separated list. verify against WYRMLING went from 205 s, with one check lost to rate limits, to 8.7 s with every read answered.
- **The Lair shows only the past** (operator, Nov 9). It does not show the next breath time, the dip-step thresholds or the reference-price chart, so it does not announce when the Dragon will buy. The views stay public on the contracts.

### Open questions and risks

- What gas does a contract-called `launch` burn on mainnet, and does WYRMLING on Nov 8 bind cleanly with 450 Tgas on `launch` and 150 on `bind`?
- Do Nearly's mainnet view answers match the field names mock-nearly records?
- Does the creator part of the tax reach a contract `fee_to` through `push_creator_fees` as one plain transfer, as the source says (factory 4150-4186, 5209)? No payout to a contract `fee_to` has been observed yet.
- Should the Council's budget floor drop below 10%, where the Gym scores higher but leaves more unspent?
- Should Strict proposals keep the closed-sale rule on top of the 45-Scale minimum? Both candidates kept v1's rule, and this design does too.
- Should a failed launch's money ever leave the Dragon? This design says no.

### Next implementation step

Write `wyrm-types` with the preset table, `Preset::check`, `CollectionArgs::check`, `Order::check`, `VoteClass`, `Verdict` and `CoinArgs::parse`, plus the test against `gym/out/presets.json`, because every other contract's `new`, the Council rules and the factory depend on it.
