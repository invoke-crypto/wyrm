//! The Dragon's NEP-297 events. Every event is emitted as a log; all but `observe` also enter the on-chain log ring,
//! so pokes and price samples can never push a buy, an outflow, a launch or a Council order out of it. The ring
//! keeps each event as the JSON it was emitted with, so it needs no second encoding.
use crate::rules::SliceKind;
use near_sdk::json_types::{U128, U64};
use near_sdk::{env, near, serde_json, AccountId};
use wyrm_types::Strategy;

/// What every slice event carries, so `audit-dragon` can re-derive the decision from the receipt alone.
/// `amount_in` and `quoted_out` are what the swap actually used; `budget` is the day's budget when it was decided.
/// A sell's `amount_in` is the gross the Dragon sent and its `quoted_out` the quote for the net after the sell tax.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub at: U64,
    pub breath_at: U64,
    pub amount_in: U128,
    pub quoted_out: U128,
    pub min_out: U128,
    pub tokens_per_near: U128,
    pub reference: Option<U128>,
    pub cap_bps: u16,
    pub budget: U128,
}

/// The confirmed dip behind a dip slice: the step, and the run of samples that confirmed it.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct DipFacts {
    pub step_bps: u16,
    pub confirmations: u8,
    pub confirmed_since: U64,
}

#[near(serializers = [json])]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Observe {
        at: U64,
        step: U64,
        tokens_per_near: Option<U128>,
        reference: Option<U128>,
        depth_near: Option<U128>,
        dip: u8,
        reserve: U128,
        hoard_wyrm: U128,
        breath_at: U64,
        decision: String,
    },
    BreathSlice(Slice),
    DipSlice {
        #[serde(flatten)]
        slice: Slice,
        #[serde(flatten)]
        dip: DipFacts,
    },
    HuntSlice(Slice),
    SwapFailed {
        at: U64,
        slice: SliceKind,
        amount_in: U128,
        reason: String,
    },
    GrantPaid {
        at: U64,
        to: AccountId,
        amount: U128,
        hoard: U128,
    },
    BurnPaid {
        at: U64,
        amount: U128,
        hoard: U128,
    },
    AutoBurn {
        at: U64,
        amount: U128,
    },
    OutflowFailed {
        at: U64,
        kind: String,
        amount: U128,
    },
    SettingsChanged(Strategy),
    OrderRefused {
        reason: String,
    },
    LaunchRefused {
        reason: String,
    },
    LaunchSent {
        ok: bool,
    },
    BindRefused {
        launch_id: U64,
        reason: String,
    },
    Bound {
        launch_id: U64,
        token: AccountId,
        pool_id: String,
    },
}

impl Event {
    pub fn logged(&self) -> bool {
        !matches!(self, Event::Observe { .. })
    }

    /// Logs the NEP-297 line and returns the event's members, `"event":"<name>","data":<data>`, which is also
    /// what a log entry stores. Serialized adjacently tagged and respliced, so no serde_json Value (and its float
    /// formatting) enters the wasm.
    pub fn emit(&self) -> String {
        let tagged = serde_json::to_string(self).expect("event serializes");
        let members = &tagged[1..tagged.len() - 1];
        let (name, data) = members.split_once(r#","data":"#).expect("an adjacently tagged event");
        env::log_str(&format!(
            r#"EVENT_JSON:{{"standard":"wyrm","version":"1.0.0",{name},"data":[{data}]}}"#
        ));
        members.into()
    }
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub at: u64,
    /// The event's JSON members, as `emit` returned them.
    pub event: String,
}
