// Test-only stand-in for Nearly's factory (nearlytrade.near, Nearlytrade e82bfa0, contracts/factory/src/lib.rs).
// `launch` records the caller as creator and walks Nearly's steps against mock-ft and mock-dcl: CreateToken (the
// coin as a mock-ft account by global code hash, the creator registered on it, and the pool created in parallel),
// CreatePool when that pool already existed, AddLiquidity (the pool seeded, with the dev buy as the locker's first buy)
// and DevBuy after a refunded first buy. Each step parks, fails and returns true or false where mainnet does, against
// mainnet's gas floors. `set_mode` makes later launches find create_pool refused, or pauses the factory as Nearly's
// set_paused does: launch and resume panic, and a step already in flight parks at its first buy or dev buy.
// The views answer in the shapes recorded from mainnet in fixtures/. Never deploy this on mainnet.
mod rules;
mod types;

use rules::{emit, json_len, sanitize_slug, short_name};
pub use types::*;

use near_sdk::json_types::{U128, U64};
use near_sdk::serde_json::{self, json};
use near_sdk::store::LookupMap;
use near_sdk::{env, near, require, AccountId, CryptoHash, Gas, NearToken, PanicOnDefault, Promise, PromiseOrValue};

const BPS: u128 = 10_000;
const MAX_TAX_BPS: u16 = 400;
const CREATOR_SHARE_BPS: u16 = 6_000;
const PLATFORM_TAX_BPS: u16 = 4_000;
const POOL_FEE: u32 = 10_000;
const FEE_DENOMINATOR: u128 = 1_000_000;
const TOTAL_SUPPLY: u128 = 1_000_000_000 * 10u128.pow(18);
const MAX_DEV_BUY_BPS: u128 = 400;
const DEV_BUY_MIN_OUT_BPS: u128 = 9_700;
const STUCK_LAUNCH_MS: u64 = 24 * 60 * 60 * 1000;
const STORAGE_PRICE_PER_BYTE: u128 = 10u128.pow(19);
const FT_STORAGE_REG: NearToken = NearToken::from_yoctonear(1_250_000_000_000_000_000_000);
// Nearly's storage pricing: TOKEN_BASE_STORAGE_BYTES, TAX_EXTRA_BYTES, RECORD_BASE_BYTES and the longest text
// quote_launch assumes when told none (MAX_TEXT_BYTES + MAX_LINKS_BYTES).
const TOKEN_BASE_BYTES: u128 = 1_000;
const TAX_EXTRA_BYTES: u128 = 600;
const MAX_TOKEN_TEXT_BYTES: u128 = 32 * 4 + 12;
const RECORD_BASE_BYTES: u128 = 1_500;
const MAX_RECORD_TEXT: u32 = 2_740;
// DCL_POOL_CHARGE when Nearly's pool float fronts the pool, DCL_POOL_CREATE when it does not.
const POOL_CHARGE: u128 = 14 * 10u128.pow(21);
const POOL_CREATE: u128 = 100 * 10u128.pow(21);
const POOL_FLOAT_MIN: u128 = 2 * 10u128.pow(24);
const DCL_STORAGE: u128 = 20 * 10u128.pow(21);

// Nearly's gas floors (factory lib.rs:79-200), so a short-gassed launch parks where mainnet's would.
const GAS_LAUNCH_FLOOR: Gas = Gas::from_tgas(6 + 8 + 10 + 45 + 18);
const GAS_POOL_STEP_FLOOR: Gas = Gas::from_tgas(8 + 5 + 45 + 10);
const GAS_ADD_LIQUIDITY_NEED: Gas = Gas::from_tgas(180 + 32 + 10);
const GAS_ADD_BUY_STEP: Gas = Gas::from_tgas(230 + 14 + 5);
const GAS_ADD_BUY_MARGIN: Gas = Gas::from_tgas(3);
const GAS_DEV_BUY_INLINE_NEED: Gas = Gas::from_tgas(5 + 5 + 80 + 36 + 5);
const GAS_DEV_BUY_FULL_NEED: Gas = Gas::from_tgas(5 + 5 + 160 + 55 + 10);
const MAX_ICON_BYTES: usize = 16 * 1024;

const GAS_TOKEN_CALL: Gas = Gas::from_tgas(5);
const GAS_DCL_CALL: Gas = Gas::from_tgas(8);
const GAS_DEV_SWAP: Gas = Gas::from_tgas(80);
const GAS_CALLBACK: Gas = Gas::from_tgas(15);
const GAS_RELAY_KEEP: Gas = Gas::from_tgas(10);

#[near(serializers = [borsh])]
pub struct Record {
    launch: Launch,
    fee_to: Option<AccountId>,
    fee_mode: String,
    tax: Option<TaxArgs>,
    // The expected dev buy output less 3%, the swap's min_output.
    dev_out: u128,
    dev_buy_done: bool,
    // A refunded first buy leaves its wNEAR here, so the DevBuy step skips the wrap.
    dev_buy_wrapped: bool,
    // Nearly's pool float did not front the pool, so the creator paid DCL_POOL_CREATE.
    pool_full: bool,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MockNearly {
    owner: AccountId,
    dcl: AccountId,
    wrap: AccountId,
    // The pool's starting wNEAR reserve. Tests mint that much wNEAR to the DCL so sells can be paid.
    pool_near: u128,
    token_code: Option<CryptoHash>,
    mode: Mode,
    next_id: u64,
    launches: LookupMap<u64, Record>,
    slugs: LookupMap<String, u32>,
    symbols: LookupMap<String, u64>,
    creator_fees: LookupMap<AccountId, u128>,
}

#[near]
impl MockNearly {
    #[init]
    pub fn new(dcl: AccountId, wrap: AccountId, pool_near: U128) -> Self {
        Self {
            owner: env::predecessor_account_id(),
            dcl,
            wrap,
            pool_near: pool_near.0,
            token_code: None,
            mode: Mode::Normal,
            next_id: 0,
            launches: LookupMap::new(b"l"),
            slugs: LookupMap::new(b"s"),
            symbols: LookupMap::new(b"y"),
            creator_fees: LookupMap::new(b"f"),
        }
    }

    /// Publishes the mock-ft wasm every launch uses, by hash. The call's raw input is the code. Publishing burns ten
    /// times the code's storage cost from this account, about 17 NEAR, once per network.
    pub fn set_token_code(&mut self) -> Promise {
        self.assert_owner();
        let code = env::input().expect("code");
        self.token_code = Some(env::sha256_array(&code));
        Promise::new(env::current_account_id()).deploy_global_contract(code)
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.assert_owner();
        self.mode = mode;
    }

    /// Books the attached NEAR as fees owed to `account_id`, as Nearly's claims and tax sales do.
    #[payable]
    pub fn credit_creator_fees(&mut self, account_id: AccountId) {
        let owed = self.creator_fees.get(&account_id).copied().unwrap_or(0);
        self.creator_fees
            .insert(account_id, owed + env::attached_deposit().as_yoctonear());
    }

    /// Lets a test launch from a contract caller, as the Dragon does. The attached NEAR goes to `nearly.launch`.
    #[payable]
    pub fn call_launch(&mut self, nearly: AccountId, args: LaunchArgs) -> Promise {
        relay(nearly, "launch", json!({ "args": args }))
    }

    /// Lets a test resume from a contract caller, as the Dragon's `nearly_resume` does.
    pub fn call_resume(&mut self, nearly: AccountId, launch_id: U64) -> Promise {
        relay(nearly, "resume", json!({ "launch_id": launch_id }))
    }

    #[payable]
    pub fn launch(&mut self, args: LaunchArgs) -> Promise {
        require!(self.mode != Mode::Panic, "launches paused");
        let creator = env::predecessor_account_id();
        let deposit = env::attached_deposit().as_yoctonear();

        let name = args.name.trim().to_string();
        let symbol = args.symbol.trim().to_string();
        require!((1..=32).contains(&name.chars().count()), "name: 1-32 chars");
        require!((1..=12).contains(&symbol.chars().count()), "symbol: 1-12 chars");
        require!(
            symbol.chars().all(|c| c.is_ascii_alphanumeric()),
            "symbol: ascii letters/digits only"
        );
        let description = args.description.unwrap_or_default();
        require!(description.chars().count() <= 500, "description: max 500 chars");
        let links = args.links.unwrap_or_default();
        let link_list = [&links.website, &links.twitter, &links.telegram];
        for l in link_list.into_iter().flatten() {
            require!(l.len() <= 200, "link: max 200 chars");
        }
        if let Some(icon) = &args.icon {
            require!(
                icon.starts_with("data:image/") || icon.starts_with("https://") || icon.starts_with("ipfs://"),
                "icon: data:image/, https:// or ipfs://"
            );
            require!(icon.len() <= MAX_ICON_BYTES, "icon too large");
        }
        let quote = args.quote.unwrap_or_else(|| self.wrap.clone());
        require!(quote == self.wrap, "quote asset not approved");
        let fee_mode = args.fee_mode.unwrap_or_else(|| "creator".into());
        require!(
            matches!(fee_mode.as_str(), "creator" | "burn" | "holders"),
            "fee_mode: creator, burn or holders"
        );
        if let Some(b) = args.creator_share_bps {
            require!(
                b == CREATOR_SHARE_BPS,
                "creator_share_bps: the launchpad's current share, or leave it out"
            );
        }
        let fee_to = args.fee_to.filter(|a| *a != creator);
        require!(
            fee_to.as_ref() != Some(&env::current_account_id()),
            "fee_to: not the launchpad itself"
        );
        let tax = args.tax.filter(|t| t.buy_bps > 0 || t.sell_bps > 0);
        if let Some(t) = &tax {
            require!(
                t.buy_bps <= MAX_TAX_BPS && t.sell_bps <= MAX_TAX_BPS,
                "tax: max 4% a side (1% pool fee + 4% = 5%)"
            );
            require!(
                t.creator_bps as u32 + t.burn_bps as u32 + t.holders_bps as u32 == 10_000,
                "tax shares must total 100%"
            );
        }
        let dev_buy = args.dev_buy.map_or(0, |d| d.0).min(self.dev_buy_cap());
        let dev_out = if dev_buy == 0 {
            0
        } else {
            self.fresh_pool_out(dev_buy) * DEV_BUY_MIN_OUT_BPS / BPS
        };
        // Nearly charges the icon and name as JSON-escaped, which quote_launch cannot know (lib.rs:878-880).
        let icon_bytes = args.icon.as_deref().map_or(0, json_len);
        let token_text = json_len(&name) + symbol.len();
        let link_bytes: usize = link_list.into_iter().flatten().map(|l| l.len()).sum();
        let record_text = name.len() + symbol.len() + description.len() + link_bytes;
        let full = !self.pool_fronted();
        let cost = self.cost(icon_bytes, token_text, record_text, dev_buy, tax.is_some(), full);
        require!(
            deposit >= cost.total.0,
            "attached deposit below quote_launch(...).total"
        );

        let slug = sanitize_slug(&symbol);
        let mut taken = self.slugs.get(&slug).copied().unwrap_or(0);
        if taken == 0 && slug == "lock" {
            taken = 1;
        }
        self.slugs.insert(slug.clone(), taken + 1);
        let token: AccountId = format!("{}.{}", short_name(&slug, taken), env::current_account_id())
            .parse()
            .expect("token account id");
        let token_is_x = token < quote;
        let pool_id = if token_is_x {
            format!("{token}|{quote}|{POOL_FEE}")
        } else {
            format!("{quote}|{token}|{POOL_FEE}")
        };
        let (left_point, right_point) = if token_is_x { (200, 500_000) } else { (-500_000, -200) };
        let id = self.next_id;
        self.next_id += 1;
        if !self.symbols.contains_key(&slug) {
            self.symbols.insert(slug, id);
        }
        let launch = Launch {
            id,
            token,
            creator: creator.clone(),
            name,
            symbol,
            icon: args.icon,
            description,
            links,
            created_at_ms: env::block_timestamp_ms(),
            total_supply: U128(TOTAL_SUPPLY),
            pool_id,
            token_is_x,
            init_point: 0,
            left_point,
            right_point,
            lpt_id: None,
            step: Step::CreateToken,
            inflight: false,
            dev_buy_near: U128(dev_buy),
            dev_buy_exact: false,
            dev_buy_tokens: U128(0),
            dev_buy_held: U128(dev_buy),
            claims: 0,
            fees_near_total: U128(0),
            fees_token_total: U128(0),
            creator_token_fees: U128(0),
            protocol_token_fees: U128(0),
            quote,
            fees_quote_total: U128(0),
            creator_quote_fees: U128(0),
            protocol_quote_fees: U128(0),
        };
        if let Some(t) = &tax {
            emit(
                "tax_opts",
                json!({ "id": id, "buy_bps": t.buy_bps, "sell_bps": t.sell_bps, "creator_bps": t.creator_bps,
                        "burn_bps": t.burn_bps, "holders_bps": t.holders_bps, "platform_bps": PLATFORM_TAX_BPS }),
            );
        }
        if let Some(to) = &fee_to {
            emit("fee_to", json!({ "id": id, "creator": creator, "to": to }));
        }
        let extra = deposit - cost.total.0;
        if extra > 0 {
            Promise::new(creator.clone())
                .transfer(NearToken::from_yoctonear(extra))
                .detach();
        }
        emit(
            "launch_started",
            json!({ "id": id, "token": launch.token, "creator": creator, "name": launch.name, "symbol": launch.symbol,
                    "pool_id": launch.pool_id, "quote": launch.quote, "init_point": launch.init_point,
                    "left_point": launch.left_point, "right_point": launch.right_point, "dev_buy": U128(dev_buy),
                    "ts_ms": launch.created_at_ms, "dev_buy_min_out": U128(dev_out) }),
        );
        self.launches.insert(
            id,
            Record {
                launch,
                fee_to,
                fee_mode,
                tax,
                dev_out,
                dev_buy_done: false,
                dev_buy_wrapped: false,
                pool_full: full,
            },
        );
        self.run_step(id, true)
    }

    pub fn resume(&mut self, launch_id: U64) -> Promise {
        let who = env::predecessor_account_id();
        let r = self.rec(launch_id.0);
        let l = &r.launch;
        require!(!l.inflight, "step in flight");
        require!(!matches!(l.step, Step::Done | Step::Failed), "nothing to resume");
        if l.step == Step::DevBuy {
            require!(
                who == l.creator || who == self.owner,
                "dev buy: creator, owner or relayer only"
            );
        }
        require!(self.mode != Mode::Panic, "paused");
        if l.step == Step::AddLiquidity && first_buy(r) {
            require!(
                who == l.creator || who == self.owner,
                "first buy pending: only the creator, the owner or a relayer can open this pool"
            );
            require!(
                gas_left() >= GAS_ADD_BUY_STEP.saturating_add(GAS_ADD_BUY_MARGIN),
                "attach at least 260 TGas: this launch's first buy goes out with its position"
            );
        }
        // Nearly's internal_run_step and its full-budget internal_dev_buy, which only a resume reaches.
        match l.step {
            Step::AddLiquidity => require!(
                gas_left() >= GAS_ADD_LIQUIDITY_NEED,
                "attach more gas: adding liquidity needs 300 TGas"
            ),
            Step::DevBuy => {
                require!(
                    l.dev_buy_held.0 > 0 && l.dev_buy_held.0 == l.dev_buy_near.0,
                    "dev buy not funded"
                );
                require!(gas_left() >= GAS_DEV_BUY_FULL_NEED, "attach more gas for the dev buy");
            }
            _ => {}
        }
        self.run_step(launch_id.0, false)
    }

    pub fn refund_failed(&mut self, launch_id: U64) -> Promise {
        let r = self.rec_mut(launch_id.0);
        require!(r.launch.step == Step::Failed, "launch not failed");
        let amount = std::mem::take(&mut r.launch.dev_buy_held.0);
        require!(amount > 0, "nothing to refund");
        r.launch.dev_buy_near = U128(0);
        let creator = r.launch.creator.clone();
        emit(
            "dev_buy_refunded",
            json!({ "id": launch_id.0, "creator": creator, "amount": U128(amount) }),
        );
        Promise::new(creator).transfer(NearToken::from_yoctonear(amount))
    }

    pub fn cancel_stuck_launch(&mut self, launch_id: U64) -> bool {
        let r = self.rec_mut(launch_id.0);
        let l = &mut r.launch;
        require!(env::predecessor_account_id() == l.creator, "creator only");
        require!(
            matches!(l.step, Step::CreatePool | Step::AddLiquidity),
            "only a launch parked before its dev buy"
        );
        require!(!l.inflight, "step in flight");
        require!(
            env::block_timestamp_ms() >= l.created_at_ms + STUCK_LAUNCH_MS,
            "a stuck launch can be cancelled 24h after it was created"
        );
        let amount = std::mem::take(&mut l.dev_buy_held.0);
        l.dev_buy_near = U128(0);
        l.step = Step::Failed;
        emit(
            "launch_failed",
            json!({ "id": l.id, "token": l.token, "refundable": U128(amount), "cancelled": true }),
        );
        if amount > 0 {
            emit(
                "dev_buy_refunded",
                json!({ "id": l.id, "creator": l.creator, "amount": U128(amount) }),
            );
            Promise::new(l.creator.clone())
                .transfer(NearToken::from_yoctonear(amount))
                .detach();
        }
        true
    }

    pub fn push_creator_fees(&mut self, account_id: AccountId) -> Promise {
        let amount = self.creator_fees.remove(&account_id).unwrap_or(0);
        require!(amount > 0, "nothing to push");
        emit(
            "creator_fees_claimed",
            json!({ "creator": account_id, "amount": U128(amount), "pushed": true }),
        );
        Promise::new(account_id).transfer(NearToken::from_yoctonear(amount))
    }

    pub fn get_launch(&self, launch_id: U64) -> Option<Launch> {
        self.launches.get(&launch_id.0).map(|r| r.launch.clone())
    }

    pub fn get_launch_by_symbol(&self, symbol: String) -> Option<Launch> {
        let id = self.symbols.get(&sanitize_slug(&symbol))?;
        self.get_launch(U64(*id))
    }

    pub fn get_fee_to(&self, launch_id: U64) -> Option<AccountId> {
        self.launches.get(&launch_id.0).and_then(|r| r.fee_to.clone())
    }

    pub fn get_fee_opts(&self, launch_id: U64) -> FeeOpts {
        FeeOpts {
            mode: self
                .launches
                .get(&launch_id.0)
                .map_or_else(|| "creator".into(), |r| r.fee_mode.clone()),
            creator_share_bps: CREATOR_SHARE_BPS,
            bucket: U128(0),
            burned: U128(0),
            paid_holders: U128(0),
        }
    }

    pub fn get_tax(&self, launch_id: U64) -> Option<TaxInfo> {
        let t = self.launches.get(&launch_id.0)?.tax.as_ref()?;
        Some(TaxInfo {
            buy_bps: t.buy_bps,
            sell_bps: t.sell_bps,
            creator_bps: t.creator_bps,
            burn_bps: t.burn_bps,
            holders_bps: t.holders_bps,
            platform_bps: PLATFORM_TAX_BPS,
            pending: U128(0),
            holders_bucket: U128(0),
            burned: U128(0),
            paid_creator: U128(0),
            paid_holders: U128(0),
            paid_platform: U128(0),
            selling: U128(0),
            floor: U128(0),
        })
    }

    pub fn get_slug_count(&self, slug: String) -> u32 {
        self.slugs.get(&slug).copied().unwrap_or(0)
    }

    /// Prices the icon at the byte count the caller passes and the token text at its longest, as Nearly does.
    pub fn quote_launch(
        &self,
        icon_bytes: u32,
        dev_buy: Option<U128>,
        tax: Option<bool>,
        text_bytes: Option<u32>,
    ) -> LaunchCost {
        let record_text = text_bytes.unwrap_or(MAX_RECORD_TEXT).min(MAX_RECORD_TEXT);
        self.cost(
            icon_bytes as usize,
            MAX_TOKEN_TEXT_BYTES as usize,
            record_text as usize,
            dev_buy.map_or(0, |d| d.0),
            tax.unwrap_or(false),
            !self.pool_fronted(),
        )
    }

    /// The NEAR that buys MAX_DEV_BUY_BPS of supply from a fresh pool, after the 1% pool fee.
    pub fn get_dev_buy_cap(&self) -> U128 {
        U128(self.dev_buy_cap())
    }

    pub fn get_creator_fees(&self, account_id: AccountId) -> U128 {
        U128(self.creator_fees.get(&account_id).copied().unwrap_or(0))
    }

    /// CreateToken's batch and the parallel create_pool (Nearly's on_created, lib.rs:1121-1168).
    #[private]
    pub fn on_created(&mut self, launch_id: U64, inline: bool) -> PromiseOrValue<bool> {
        let id = launch_id.0;
        let token_ok = env::promise_result_checked(0, 1_000).is_ok();
        let pool_ok = self.pool_answer(id, 1);
        let storage = self.token_storage(self.rec(id));
        let r = self.rec_mut(id);
        r.launch.inflight = false;
        r.launch.icon = None;
        if !token_ok {
            if inline {
                let creator = r.launch.creator.clone();
                emit(
                    "token_storage_refunded",
                    json!({ "id": id, "creator": creator, "amount": U128(storage) }),
                );
                Promise::new(creator)
                    .transfer(NearToken::from_yoctonear(storage))
                    .detach();
            }
            return self.fail(id, "token create");
        }
        if pool_ok {
            self.rec_mut(id).launch.step = Step::AddLiquidity;
            return self.advance(id, inline);
        }
        self.rec_mut(id).launch.step = Step::CreatePool;
        emit("launch_step_failed", json!({ "id": id, "step": "create_pool" }));
        PromiseOrValue::Promise(self.pool_check(id, inline))
    }

    #[private]
    pub fn on_pool_created(&mut self, launch_id: U64, inline: bool) -> PromiseOrValue<bool> {
        let id = launch_id.0;
        let ok = self.pool_answer(id, 0);
        self.rec_mut(id).launch.inflight = false;
        if ok {
            self.rec_mut(id).launch.step = Step::AddLiquidity;
            return self.advance(id, inline);
        }
        emit("launch_step_failed", json!({ "id": id, "step": "create_pool" }));
        PromiseOrValue::Promise(self.pool_check(id, inline))
    }

    /// DCL's get_pool after create_pool was refused (Nearly's on_pool_checked, lib.rs:1194-1222).
    #[private]
    pub fn on_pool_checked(&mut self, launch_id: U64, inline: bool) -> PromiseOrValue<bool> {
        let id = launch_id.0;
        let pool = env::promise_result_checked(0, 10_000)
            .ok()
            .map(|b| serde_json::from_slice::<serde_json::Value>(&b).unwrap_or_default());
        let r = self.rec_mut(id);
        r.launch.inflight = false;
        let init_point = r.launch.init_point as i64;
        match pool {
            Some(p) if !p.is_null() && p["current_point"].as_i64() == Some(init_point) => {
                emit(
                    "pool_adopted",
                    json!({ "id": id, "pool_id": r.launch.pool_id, "point": init_point }),
                );
                r.launch.step = Step::AddLiquidity;
                self.advance(id, inline)
            }
            Some(p) if !p.is_null() => {
                let fee = if r.pool_full { POOL_CREATE } else { POOL_CHARGE };
                let creator = r.launch.creator.clone();
                emit(
                    "pool_deposit_refunded",
                    json!({ "id": id, "creator": creator, "amount": U128(fee) }),
                );
                Promise::new(creator).transfer(NearToken::from_yoctonear(fee)).detach();
                self.mark_failed(id, "pool exists at another price")
            }
            other => {
                let reason = if other.is_none() {
                    "pool read failed"
                } else {
                    "create_pool refused and no pool exists"
                };
                self.park(id, reason, false)
            }
        }
    }

    #[private]
    pub fn on_liquidity_added(&mut self, launch_id: U64, inline: bool) -> PromiseOrValue<bool> {
        let id = launch_id.0;
        let ok = env::promise_result_checked(0, 1_000).is_ok();
        let r = self.rec_mut(id);
        r.launch.inflight = false;
        if !ok {
            return self.fail(id, "add_liquidity");
        }
        r.launch.lpt_id = Some(format!("{}#{}", r.launch.pool_id, id));
        if r.launch.dev_buy_near.0 > 0 && !r.dev_buy_done {
            r.launch.step = Step::DevBuy;
            return self.advance(id, inline);
        }
        self.done(id);
        PromiseOrValue::Value(true)
    }

    /// The pool seeded and the first buy sent with it (Nearly's settle_add_buy, lib.rs:1258-1300).
    #[private]
    pub fn on_liquidity_added_buy(&mut self, launch_id: U64) -> PromiseOrValue<bool> {
        let id = launch_id.0;
        let used = promise_u128(0);
        let r = self.rec_mut(id);
        r.launch.inflight = false;
        let Some(used) = used else {
            return self.park(id, "add_buy failed", false);
        };
        r.launch.lpt_id = Some(format!("{}#{}", r.launch.pool_id, id));
        r.dev_buy_wrapped = true;
        if used > 0 {
            r.launch.dev_buy_near = U128(used);
            r.launch.dev_buy_held = U128(0);
            r.dev_buy_done = true;
            self.done(id);
            return PromiseOrValue::Value(true);
        }
        r.launch.step = Step::DevBuy;
        self.park(id, "swap refunded", false)
    }

    /// The DevBuy step's swap (Nearly's on_dev_bought, lib.rs:1333-1373).
    #[private]
    pub fn on_dev_bought(&mut self, launch_id: U64) -> PromiseOrValue<bool> {
        let id = launch_id.0;
        let used = promise_u128(0);
        let r = self.rec_mut(id);
        r.launch.inflight = false;
        match used {
            Some(u) if u > 0 => {
                r.launch.dev_buy_near = U128(u);
                r.launch.dev_buy_held = U128(0);
                r.dev_buy_done = true;
                self.done(id);
                PromiseOrValue::Value(true)
            }
            Some(_) => {
                r.dev_buy_wrapped = true;
                self.park(id, "swap refunded", true)
            }
            None => self.park(id, "wrap reverted", false),
        }
    }
}

impl MockNearly {
    fn assert_owner(&self) {
        require!(env::predecessor_account_id() == self.owner, "owner only");
    }

    fn rec(&self, id: u64) -> &Record {
        self.launches.get(&id).expect("no such launch")
    }

    fn rec_mut(&mut self, id: u64) -> &mut Record {
        self.launches.get_mut(&id).expect("no such launch")
    }

    // Nearly's pool_fronted (lib.rs:4252): its own float, beyond locked storage, pays the pool creation.
    fn pool_fronted(&self) -> bool {
        let locked = env::storage_usage() as u128 * STORAGE_PRICE_PER_BYTE;
        let own = env::account_balance().as_yoctonear() - env::attached_deposit().as_yoctonear();
        own.saturating_sub(locked) >= POOL_FLOAT_MIN
    }

    fn dev_buy_cap(&self) -> u128 {
        // Constant product: buying a share s of the token side takes x * s / (1 - s) wNEAR after the fee.
        self.pool_near * MAX_DEV_BUY_BPS / (BPS - MAX_DEV_BUY_BPS) * FEE_DENOMINATOR
            / (FEE_DENOMINATOR - POOL_FEE as u128)
    }

    /// What `near` buys from the pool AddLiquidity seeds, after the pool fee.
    fn fresh_pool_out(&self, near: u128) -> u128 {
        const SCALE: u128 = 10u128.pow(12);
        let net = near / FEE_DENOMINATOR * (FEE_DENOMINATOR - POOL_FEE as u128);
        TOTAL_SUPPLY / 1_000_000 * (net / SCALE) / ((self.pool_near + net) / SCALE) * 1_000_000
    }

    fn cost(
        &self,
        icon_bytes: usize,
        token_text: usize,
        record_text: usize,
        dev_buy: u128,
        tax: bool,
        full: bool,
    ) -> LaunchCost {
        let tax_bytes = if tax { TAX_EXTRA_BYTES } else { 0 };
        let token_storage = (TOKEN_BASE_BYTES + icon_bytes as u128 + token_text as u128 + tax_bytes)
            * STORAGE_PRICE_PER_BYTE
            + FT_STORAGE_REG.as_yoctonear();
        let record_storage = (RECORD_BASE_BYTES + record_text as u128) * STORAGE_PRICE_PER_BYTE;
        let pool_create = if full { POOL_CREATE } else { POOL_CHARGE };
        LaunchCost {
            launch_fee: U128(0),
            token_storage: U128(token_storage),
            record_storage: U128(record_storage),
            pool_create: U128(pool_create),
            dcl_storage: U128(DCL_STORAGE),
            dev_buy: U128(dev_buy),
            total: U128(token_storage + record_storage + pool_create + DCL_STORAGE + dev_buy),
        }
    }

    fn token_storage(&self, r: &Record) -> u128 {
        let l = &r.launch;
        let icon = l.icon.as_deref().map_or(0, json_len);
        let cost = self.cost(icon, json_len(&l.name) + l.symbol.len(), 0, 0, r.tax.is_some(), false);
        cost.token_storage.0
    }

    /// Nearly's internal_advance (lib.rs:4343-4389): the dev buy and first-buy gates, then the gas floor.
    fn advance(&mut self, id: u64, inline: bool) -> PromiseOrValue<bool> {
        let r = self.rec(id);
        let l = &r.launch;
        let may_trigger = || {
            let signer = env::signer_account_id();
            signer == l.creator || signer == self.owner
        };
        if l.step == Step::DevBuy {
            let reason = if gas_left() < GAS_DEV_BUY_INLINE_NEED {
                Some("not enough gas for the swap in this transaction")
            } else if !inline && !may_trigger() {
                Some("only the creator, the owner or a relayer triggers the dev buy")
            } else if self.mode == Mode::Panic {
                Some("paused")
            } else if l.dev_buy_held.0 == 0 || l.dev_buy_held.0 != l.dev_buy_near.0 {
                Some("dev buy not funded")
            } else {
                None
            };
            if let Some(reason) = reason {
                return self.park(id, reason, true);
            }
        } else if l.step == Step::AddLiquidity && first_buy(r) {
            let margin = if inline { Gas::from_tgas(0) } else { GAS_ADD_BUY_MARGIN };
            let reason = if self.mode == Mode::Panic {
                Some("paused")
            } else if !inline && !may_trigger() {
                Some("first buy: resume by the creator, the owner or a relayer")
            } else if gas_left() < GAS_ADD_BUY_STEP.saturating_add(margin) {
                Some("first buy: resume with at least 260 TGas")
            } else {
                None
            };
            if let Some(reason) = reason {
                return self.park(id, reason, true);
            }
        } else if l.step == Step::AddLiquidity && gas_left() < GAS_ADD_LIQUIDITY_NEED {
            emit("launch_step_parked", json!({ "id": id, "step": l.step }));
            return PromiseOrValue::Value(true);
        }
        PromiseOrValue::Promise(self.run_step(id, inline))
    }

    fn run_step(&mut self, id: u64, inline: bool) -> Promise {
        let me = env::current_account_id();
        let r = self.rec(id);
        let l = r.launch.clone();
        let then = Self::ext(me).with_static_gas(GAS_CALLBACK);
        let p = match l.step {
            Step::CreateToken => {
                require!(
                    gas_left() >= GAS_LAUNCH_FLOOR,
                    "attach more gas: this step needs 300 TGas"
                );
                // Park mode sends create_pool an id DCL refuses, so the launch takes mainnet's refused-pool path.
                let pool_id = if inline && self.mode == Mode::Park {
                    ""
                } else {
                    l.pool_id.as_str()
                };
                self.create_token(r)
                    .and(self.create_pool(pool_id))
                    .then(then.on_created(U64(id), inline))
            }
            Step::CreatePool => {
                require!(
                    gas_left() >= GAS_POOL_STEP_FLOOR,
                    "attach more gas: this step needs 300 TGas"
                );
                self.create_pool(&l.pool_id).then(then.on_pool_created(U64(id), inline))
            }
            Step::AddLiquidity if first_buy(r) => self
                .seed_pool(&l)
                .then(self.dev_buy(r))
                .then(then.on_liquidity_added_buy(U64(id))),
            Step::AddLiquidity => self.seed_pool(&l).then(then.on_liquidity_added(U64(id), inline)),
            Step::DevBuy => self.dev_buy(r).then(then.on_dev_bought(U64(id))),
            _ => env::panic_str("nothing to resume"),
        };
        self.rec_mut(id).launch.inflight = true;
        p
    }

    fn create_token(&self, r: &Record) -> Promise {
        let l = &r.launch;
        let code = self.token_code.expect("set_token_code first");
        let storage = self.token_storage(r) - FT_STORAGE_REG.as_yoctonear();
        let mut p = Promise::new(l.token.clone())
            .create_account()
            .transfer(NearToken::from_yoctonear(storage))
            .use_global_contract(code)
            .function_call(
                "new",
                json!({ "symbol": l.symbol, "decimals": 18 }).to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                GAS_TOKEN_CALL,
            )
            .function_call(
                "mint",
                json!({ "account_id": self.dcl, "amount": U128(TOTAL_SUPPLY) })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(0),
                GAS_TOKEN_CALL,
            );
        if let Some(t) = &r.tax {
            p = p.function_call(
                "set_tax",
                json!({ "tax": { "pairs": [self.dcl], "buy_bps": t.buy_bps, "sell_bps": t.sell_bps } })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(0),
                GAS_TOKEN_CALL,
            );
        }
        p.function_call(
            "storage_deposit",
            json!({ "account_id": l.creator, "registration_only": true })
                .to_string()
                .into_bytes(),
            FT_STORAGE_REG,
            GAS_TOKEN_CALL,
        )
    }

    fn create_pool(&self, pool_id: &str) -> Promise {
        dcl_call(&self.dcl, "create_pool", json!({ "pool_id": pool_id }))
    }

    fn pool_check(&mut self, id: u64, inline: bool) -> Promise {
        self.rec_mut(id).launch.inflight = true;
        let pool_id = self.rec(id).launch.pool_id.clone();
        dcl_call(&self.dcl, "get_pool", json!({ "pool_id": pool_id })).then(
            Self::ext(env::current_account_id())
                .with_static_gas(GAS_CALLBACK)
                .on_pool_checked(U64(id), inline),
        )
    }

    /// Whether create_pool at promise result `idx` answered this launch's pool id.
    fn pool_answer(&self, id: u64, idx: u64) -> bool {
        let pool_id = &self.rec(id).launch.pool_id;
        env::promise_result_checked(idx, 1_000)
            .ok()
            .and_then(|b| serde_json::from_slice::<String>(&b).ok())
            .is_some_and(|p| p == *pool_id)
    }

    /// AddLiquidity: the whole supply on one side, `pool_near` of wNEAR on the other.
    fn seed_pool(&self, l: &Launch) -> Promise {
        let (x, y) = if l.token_is_x {
            (TOTAL_SUPPLY, self.pool_near)
        } else {
            (self.pool_near, TOTAL_SUPPLY)
        };
        dcl_call(
            &self.dcl,
            "set_pool",
            json!({ "pool_id": l.pool_id, "total_x": U128(x), "total_y": U128(y) }),
        )
    }

    fn dev_buy(&self, r: &Record) -> Promise {
        let l = &r.launch;
        let amount = l.dev_buy_near.0;
        let mut p = Promise::new(self.wrap.clone());
        if !r.dev_buy_wrapped {
            p = p
                .function_call(
                    "storage_deposit",
                    json!({ "account_id": env::current_account_id(), "registration_only": true })
                        .to_string()
                        .into_bytes(),
                    FT_STORAGE_REG,
                    GAS_TOKEN_CALL,
                )
                .function_call(
                    "near_deposit",
                    b"{}".to_vec(),
                    NearToken::from_yoctonear(amount),
                    GAS_TOKEN_CALL,
                );
        }
        let msg = json!({ "Swap": { "pool_ids": [l.pool_id], "output_token": l.token,
                                    "min_output_amount": U128(r.dev_out.max(1)), "swap_out_recipient": l.creator } });
        p.function_call(
            "ft_transfer_call",
            json!({ "receiver_id": self.dcl, "amount": U128(amount), "msg": msg.to_string() })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            GAS_DEV_SWAP,
        )
    }

    /// Emits launch_step_parked and returns what mainnet returns for that park.
    fn park(&mut self, id: u64, reason: &str, ret: bool) -> PromiseOrValue<bool> {
        let step = self.rec(id).launch.step;
        emit(
            "launch_step_parked",
            json!({ "id": id, "step": step, "reason": reason }),
        );
        PromiseOrValue::Value(ret)
    }

    /// Nearly's internal_fail: a failure during CreateToken fails the launch, a later one keeps its step.
    fn fail(&mut self, id: u64, step: &str) -> PromiseOrValue<bool> {
        emit("launch_step_failed", json!({ "id": id, "step": step }));
        let l = &mut self.rec_mut(id).launch;
        if l.step == Step::CreateToken {
            l.step = Step::Failed;
            emit(
                "launch_failed",
                json!({ "id": id, "token": l.token, "refundable": l.dev_buy_held }),
            );
        }
        PromiseOrValue::Value(false)
    }

    fn mark_failed(&mut self, id: u64, reason: &str) -> PromiseOrValue<bool> {
        emit("launch_step_failed", json!({ "id": id, "step": reason }));
        let l = &mut self.rec_mut(id).launch;
        l.step = Step::Failed;
        emit(
            "launch_failed",
            json!({ "id": id, "token": l.token, "refundable": l.dev_buy_held }),
        );
        PromiseOrValue::Value(false)
    }

    fn done(&mut self, id: u64) {
        let l = &mut self.rec_mut(id).launch;
        l.step = Step::Done;
        emit(
            "launch",
            json!({ "id": id, "token": l.token, "creator": l.creator, "name": l.name, "symbol": l.symbol,
                    "pool_id": l.pool_id, "lpt_id": l.lpt_id, "init_point": l.init_point,
                    "left_point": l.left_point, "right_point": l.right_point, "dev_buy_near": l.dev_buy_near,
                    "dev_buy_tokens": l.dev_buy_tokens, "ts_ms": env::block_timestamp_ms() }),
        );
    }
}

/// Nearly's first_buy_by_locker: a funded dev buy that has not gone out goes out with the position.
fn first_buy(r: &Record) -> bool {
    let l = &r.launch;
    l.dev_buy_near.0 > 0 && !r.dev_buy_done && l.dev_buy_held.0 == l.dev_buy_near.0 && !r.dev_buy_wrapped
}

fn gas_left() -> Gas {
    env::prepaid_gas().saturating_sub(env::used_gas())
}

/// The U128 a promise result answered, or None when it failed.
fn promise_u128(idx: u64) -> Option<u128> {
    let raw = env::promise_result_checked(idx, 100).ok()?;
    Some(serde_json::from_slice::<U128>(&raw).map_or(0, |u| u.0))
}

fn dcl_call(dcl: &AccountId, method: &str, args: serde_json::Value) -> Promise {
    Promise::new(dcl.clone()).function_call(
        method,
        args.to_string().into_bytes(),
        NearToken::from_yoctonear(0),
        GAS_DCL_CALL,
    )
}

fn relay(nearly: AccountId, method: &str, args: serde_json::Value) -> Promise {
    Promise::new(nearly).function_call(
        method,
        args.to_string().into_bytes(),
        env::attached_deposit(),
        env::prepaid_gas()
            .saturating_sub(env::used_gas())
            .saturating_sub(GAS_RELAY_KEEP),
    )
}

#[cfg(test)]
mod tests;
