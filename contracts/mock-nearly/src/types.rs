// Nearly's JSON and borsh shapes, field for field with factory lib.rs at e82bfa0, so the views match fixtures/.
use near_sdk::json_types::U128;
use near_sdk::{near, AccountId};

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    CreateToken,
    CreatePool,
    Deposit,
    AddLiquidity,
    DevBuy,
    ReadDevTokens,
    DeliverDevTokens,
    Done,
    Failed,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Normal,
    Park,
    Panic,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Default)]
pub struct Links {
    pub website: Option<String>,
    pub twitter: Option<String>,
    pub telegram: Option<String>,
}

/// Nearly's `Launch` record, field for field.
#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct Launch {
    pub id: u64,
    pub token: AccountId,
    pub creator: AccountId,
    pub name: String,
    pub symbol: String,
    pub icon: Option<String>,
    pub description: String,
    pub links: Links,
    pub created_at_ms: u64,
    pub total_supply: U128,
    pub pool_id: String,
    pub token_is_x: bool,
    pub init_point: i32,
    pub left_point: i32,
    pub right_point: i32,
    pub lpt_id: Option<String>,
    pub step: Step,
    pub inflight: bool,
    pub dev_buy_near: U128,
    pub dev_buy_exact: bool,
    pub dev_buy_tokens: U128,
    pub dev_buy_held: U128,
    pub claims: u64,
    pub fees_near_total: U128,
    pub fees_token_total: U128,
    pub creator_token_fees: U128,
    pub protocol_token_fees: U128,
    pub quote: AccountId,
    pub fees_quote_total: U128,
    pub creator_quote_fees: U128,
    pub protocol_quote_fees: U128,
}

#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct TaxArgs {
    pub buy_bps: u16,
    pub sell_bps: u16,
    pub creator_bps: u16,
    pub burn_bps: u16,
    pub holders_bps: u16,
}

#[near(serializers = [json])]
pub struct LaunchArgs {
    pub name: String,
    pub symbol: String,
    pub icon: Option<String>,
    pub description: Option<String>,
    pub links: Option<Links>,
    pub dev_buy: Option<U128>,
    pub init_point: Option<i32>,
    pub quote: Option<AccountId>,
    pub fee_mode: Option<String>,
    pub creator_share_bps: Option<u16>,
    pub tax: Option<TaxArgs>,
    pub fee_to: Option<AccountId>,
    pub creator_paid_in: Option<String>,
    pub holders_paid_in: Option<String>,
}

#[near(serializers = [json])]
pub struct TaxInfo {
    pub buy_bps: u16,
    pub sell_bps: u16,
    pub creator_bps: u16,
    pub burn_bps: u16,
    pub holders_bps: u16,
    pub platform_bps: u16,
    pub pending: U128,
    pub holders_bucket: U128,
    pub burned: U128,
    pub paid_creator: U128,
    pub paid_holders: U128,
    pub paid_platform: U128,
    pub selling: U128,
    pub floor: U128,
}

#[near(serializers = [json])]
pub struct FeeOpts {
    pub mode: String,
    pub creator_share_bps: u16,
    pub bucket: U128,
    pub burned: U128,
    pub paid_holders: U128,
}

#[near(serializers = [json])]
pub struct LaunchCost {
    pub launch_fee: U128,
    pub token_storage: U128,
    pub record_storage: U128,
    pub pool_create: U128,
    pub dcl_storage: U128,
    pub dev_buy: U128,
    pub total: U128,
}
