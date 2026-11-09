mod claims;
mod council;
mod market;
mod nft;
mod sale;

use crate::*;
use near_sdk::mock::MockAction;
use near_sdk::serde_json::{json, Value};
use near_sdk::test_utils::{get_created_receipts, get_logs, VMContextBuilder};
use near_sdk::{testing_env, Gas, PromiseResult, RuntimeFeesConfig};
use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use wyrm_types::{CollectionArgs, PresetName};

pub const T0: u64 = 1_800_000_000 * NANOS_PER_SEC;
pub const YOCTO: NearToken = NearToken::from_yoctonear(1);
pub const ZERO: NearToken = NearToken::from_yoctonear(0);
pub const VAULT_HASH: [u8; 32] = [7; 32];
pub const SEASON: u64 = 7 * DAY;
pub const SALE_LENGTH: u64 = 14 * DAY;
pub const PRICE: NearToken = wyrm_types::preset(PresetName::Hoarder).sale.price;

pub fn near(n: u128) -> NearToken {
    NearToken::from_near(n)
}

pub fn milli(n: u128) -> NearToken {
    NearToken::from_millinear(n)
}

pub fn a(name: &str) -> AccountId {
    format!("{name}.near").parse().unwrap()
}

thread_local! {
    static ME: RefCell<AccountId> = RefCell::new("scales.near".parse().unwrap());
}

/// The account scales runs on. Each test runs on its own thread, so a world on a `.testnet` account stays in its test.
pub fn me() -> AccountId {
    ME.with(|me| me.borrow().clone())
}

pub fn media(tier: &str) -> String {
    format!("https://wyrm.example/{tier}.svg")
}

pub fn collection() -> CollectionArgs {
    CollectionArgs {
        name: "WYRM Scales".into(),
        symbol: "SCALE".into(),
        icon: None,
        tier_media: ["ash", "ember", "molten", "obsidian"].map(media),
    }
}

/// Runs scales `new` as the deployer on `account`.
pub fn deploy(name: PresetName, account: &str) -> Scales {
    ME.with(|me| *me.borrow_mut() = account.parse().unwrap());
    fresh_chain();
    Scales::new(
        name,
        a("dragon"),
        a("tithe"),
        a("creator"),
        VAULT_HASH.into(),
        collection(),
    )
}

pub struct W {
    pub c: Scales,
    pub now: u64,
    /// The contract's balance before any attached deposit.
    pub base: NearToken,
}

impl W {
    /// WYRM's preset, with the sale still Waiting for the Dragon.
    pub fn new() -> Self {
        W {
            c: deploy(PresetName::Hoarder, "scales.near"),
            now: T0,
            base: near(1_000),
        }
    }

    /// WYRM's preset after the Dragon opened the sale at T0.
    pub fn open() -> Self {
        let mut w = W::new();
        w.as_("dragon", ZERO).open_sale();
        w
    }

    /// The drill preset on a testnet account, with the sale open.
    pub fn drill() -> Self {
        let mut w = W {
            c: deploy(PresetName::Drill, "scales.testnet"),
            now: T0,
            base: near(1_000),
        };
        w.as_("dragon", ZERO).open_sale();
        w
    }

    /// A world with `n` sale Scales, Scale i owned by `v{i}`, all seasoned. With `closed` the sale has ended.
    pub fn council(n: u8, closed: bool) -> Self {
        let mut w = W::open();
        for i in 1..=n {
            w.buy_scale(&format!("v{i}"));
        }
        w.advance(if closed { SALE_LENGTH } else { SEASON } + 1);
        w
    }

    pub fn as_(&mut self, who: &str, deposit: NearToken) -> &mut Scales {
        ctx(a(who), deposit, self.now, self.base.saturating_add(deposit), vec![]);
        &mut self.c
    }

    /// The contract calling itself, as a callback does, with these promise results.
    pub fn callback(&mut self, results: Vec<PromiseResult>) -> &mut Scales {
        ctx(me(), ZERO, self.now, self.base, results);
        &mut self.c
    }

    pub fn view(&mut self) -> &Scales {
        ctx(a("viewer"), ZERO, self.now, self.base, vec![]);
        &self.c
    }

    pub fn advance(&mut self, ns: u64) {
        self.now += ns;
    }

    pub fn buy_scale(&mut self, who: &str) -> String {
        let price = self.view().get_sale().price;
        self.as_(who, price).buy_scale()
    }

    pub fn owner(&mut self, token: &str) -> AccountId {
        self.view().get_scale(token.into()).unwrap().owner
    }

    pub fn make_vaults(&mut self) {
        for last in [25, 50, 75, 100] {
            self.as_("keeper", ZERO).create_vaults().detach();
            assert!(self.callback(vec![parent_answer()]).on_vaults_created(last));
        }
    }
}

/// The mock carries storage from one context to the next, so a second world in one test would read the first.
pub fn fresh_chain() {
    ctx(a("deployer"), ZERO, T0, near(1_000), vec![]);
    near_sdk::mock::with_mocked_blockchain(|b| {
        b.take_storage();
    });
}

pub fn ctx(predecessor: AccountId, deposit: NearToken, now: u64, balance: NearToken, results: Vec<PromiseResult>) {
    ctx_with_gas(predecessor, deposit, now, balance, results, Gas::from_tgas(300));
}

pub fn ctx_with_gas(
    predecessor: AccountId,
    deposit: NearToken,
    now: u64,
    balance: NearToken,
    results: Vec<PromiseResult>,
    gas: Gas,
) {
    let context = VMContextBuilder::new()
        .current_account_id(me())
        .signer_account_id(predecessor.clone())
        .predecessor_account_id(predecessor)
        .attached_deposit(deposit)
        .block_timestamp(now)
        .account_balance(balance)
        .prepaid_gas(gas)
        .build();
    // Gas is not what these tests check, and the mock's test fee table is far above mainnet's, so fees are free while
    // storage keeps its real price.
    let mut fees = RuntimeFeesConfig::free();
    fees.storage_usage_config = RuntimeFeesConfig::test().storage_usage_config;
    testing_env!(context, near_sdk::test_vm_config(), fees, Default::default(), results);
}

/// Runs `f`, which must panic with a message containing `msg`.
pub fn refuses<R>(msg: &str, f: impl FnOnce() -> R) {
    let err = catch_unwind(AssertUnwindSafe(f))
        .err()
        .unwrap_or_else(|| panic!("expected a refusal: {msg}"));
    let text = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(text.contains(msg), "expected {msg:?}, got {text:?}");
}

pub fn transfers() -> Vec<(AccountId, NearToken)> {
    get_created_receipts()
        .into_iter()
        .flat_map(|r| {
            let to = r.receiver_id.clone();
            r.actions.into_iter().filter_map(move |a| match a {
                MockAction::Transfer { deposit, .. } => Some((to.clone(), deposit)),
                _ => None,
            })
        })
        .collect()
}

pub fn calls() -> Vec<(AccountId, String, Value, Gas)> {
    get_created_receipts()
        .into_iter()
        .flat_map(|r| {
            let to = r.receiver_id.clone();
            r.actions.into_iter().filter_map(move |a| match a {
                MockAction::FunctionCallWeight {
                    method_name,
                    args,
                    prepaid_gas,
                    ..
                } => Some((
                    to.clone(),
                    String::from_utf8(method_name).unwrap(),
                    near_sdk::serde_json::from_slice(&args).unwrap_or(Value::Null),
                    prepaid_gas,
                )),
                _ => None,
            })
        })
        .collect()
}

pub fn events(name: &str) -> Vec<Value> {
    get_logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| near_sdk::serde_json::from_str::<Value>(j).unwrap())
        .filter(|e| e["event"] == name)
        .map(|e| e["data"][0].clone())
        .collect()
}

/// What a vault created by this contract answers to `get_parent`.
pub fn parent_answer() -> PromiseResult {
    PromiseResult::Successful(b"\"scales.near\"".to_vec())
}
