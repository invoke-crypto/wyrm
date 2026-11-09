//! JSON shapes of the views. State is stored as plain integers; amounts and times leave as decimal strings.
use crate::council::Pending;
use crate::nearly::Coin;
use crate::rules::{Day, Hoard, Mood, Sample, Side};
use crate::Peers;
use near_sdk::json_types::{Base58CryptoHash, U128, U64};
use near_sdk::{near, AccountId};
use wyrm_types::{Preset, PresetName, Strategy, TaxPolicy};

/// The engine's fixed rules beside the preset's: the same in every Dragon of this build.
#[near(serializers = [json])]
pub struct EngineView {
    pub tax: TaxPolicy,
    pub spike_bps: u16,
    pub slippage_bps: u16,
    pub dip_samples: u8,
    pub dip_span_ns: U64,
    pub outflow_bps: u16,
    pub outflow_window_bps: u16,
    pub outflow_window_ns: U64,
    pub stale_ns: U64,
    pub launch_margin: U128,
}

#[near(serializers = [json])]
pub struct ConfigView {
    pub preset: PresetName,
    pub preset_value: Preset,
    pub preset_hash: Base58CryptoHash,
    pub engine_version: u32,
    pub peers: Peers,
    pub strategy: Strategy,
    pub coin: Option<Coin>,
    pub engine: EngineView,
}

#[near(serializers = [json])]
pub enum PhaseView {
    Idle,
    Observing {
        step: U64,
        since: U64,
    },
    Quoting {
        step: U64,
        since: U64,
        side: Side,
        amount_in: U128,
    },
    Swapping {
        step: U64,
        since: U64,
        side: Side,
        amount_in: U128,
        quoted_out: U128,
        min_out: U128,
    },
}

#[near(serializers = [json])]
pub struct DayView {
    pub index: U64,
    pub breath_at: U64,
    pub opening_reserve: U128,
    pub budget: U128,
    pub spent: U128,
    pub sold_wyrm: U128,
}

impl From<&Day> for DayView {
    fn from(d: &Day) -> Self {
        Self {
            index: U64(d.index),
            breath_at: U64(d.breath_at),
            opening_reserve: U128(d.opening_reserve),
            budget: U128(d.budget),
            spent: U128(d.spent),
            sold_wyrm: U128(d.sold_wyrm),
        }
    }
}

#[near(serializers = [json])]
pub struct HoardView {
    pub wyrm: U128,
    pub near_spent: U128,
    pub wyrm_bought: U128,
    pub donated: U128,
    pub granted: U128,
    pub realized_near: U128,
    pub burned: U128,
    pub auto_burned: U128,
    pub ash_due: U128,
}

impl From<&Hoard> for HoardView {
    fn from(h: &Hoard) -> Self {
        Self {
            wyrm: U128(h.wyrm),
            near_spent: U128(h.near_spent),
            wyrm_bought: U128(h.wyrm_bought),
            donated: U128(h.donated),
            granted: U128(h.granted),
            realized_near: U128(h.realized_near),
            burned: U128(h.burned),
            auto_burned: U128(h.auto_burned),
            ash_due: U128(h.ash_due),
        }
    }
}

/// The run of newest samples beyond a dip step: `step` is the deepest step the newest sample is beyond (1 to 3),
/// `samples` how many in a row count toward confirming it, `first_at` when that run began.
#[near(serializers = [json])]
pub struct DipView {
    pub step: u8,
    pub step_bps: u16,
    pub samples: u8,
    pub first_at: U64,
    pub confirmed: bool,
}

#[near(serializers = [json])]
pub struct PendingView {
    pub seq: U64,
    pub kind: String,
    pub to: Option<AccountId>,
    pub amount: U128,
    pub at: U64,
}

impl From<&Pending> for PendingView {
    fn from(p: &Pending) -> Self {
        Self {
            seq: U64(p.seq),
            kind: p.kind.name().into(),
            to: p.kind.grantee().cloned(),
            amount: U128(p.amount),
            at: U64(p.at),
        }
    }
}

#[near(serializers = [json])]
pub struct StateView {
    pub mood: Mood,
    pub phase: PhaseView,
    pub coin: Option<Coin>,
    pub reserve: U128,
    pub reserve_near: U128,
    pub reserve_wnear: U128,
    pub spot: Option<U128>,
    pub reference: Option<U128>,
    pub hoard: HoardView,
    pub day: DayView,
    pub strategy: Strategy,
    pub dip: Option<DipView>,
    pub last_slice_at: U64,
    pub last_launch_at: U64,
    pub pending: Vec<PendingView>,
    pub samples: u32,
    pub log_next: U64,
}

#[near(serializers = [json])]
pub struct SampleView {
    pub at: U64,
    pub tokens_per_near: U128,
}

impl From<&Sample> for SampleView {
    fn from(s: &Sample) -> Self {
        Self {
            at: U64(s.at),
            tokens_per_near: U128(s.tokens_per_near),
        }
    }
}
