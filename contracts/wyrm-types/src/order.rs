use crate::preset::{HuntLine, Preset};
use near_sdk::json_types::U128;
use near_sdk::{near, AccountId};

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Order {
    SetBreath { budget_bps: u16 },
    SetSlices { cap_bps: u16 },
    SetPosture { hunt: Option<HuntLine> },
    Grant { to: AccountId, amount: U128 },
    Burn { amount: U128 },
    SetAutoBurn { bps: u16 },
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    Grant,
    Burn,
    Hunt,
    AutoBurn,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoteClass {
    Light,
    Strict,
}

/// The Dragon answers a refused order instead of panicking, because a PromiseError carries no message.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Executed,
    Refused { reason: String },
}

impl Order {
    pub fn feature(&self) -> Option<Feature> {
        match self {
            Order::SetBreath { .. } | Order::SetSlices { .. } => None,
            Order::SetPosture { .. } => Some(Feature::Hunt),
            Order::Grant { .. } => Some(Feature::Grant),
            Order::Burn { .. } => Some(Feature::Burn),
            Order::SetAutoBurn { .. } => Some(Feature::AutoBurn),
        }
    }

    pub fn class(&self) -> VoteClass {
        match self {
            Order::Grant { .. } | Order::SetPosture { hunt: Some(_) } => VoteClass::Strict,
            _ => VoteClass::Light,
        }
    }

    pub fn check(&self, preset: &Preset) -> Result<(), String> {
        if let Some(feature) = self.feature() {
            let f = &preset.features;
            let enabled = match feature {
                Feature::Grant => f.grant,
                Feature::Burn => f.burn,
                Feature::Hunt => f.hunt,
                Feature::AutoBurn => f.auto_burn,
            };
            if !enabled {
                return Err(format!("{feature:?} is disabled in the {} preset", preset.name));
            }
        }
        let b = &preset.bounds;
        match self {
            Order::SetBreath { budget_bps } => b.budget_bps.check("budget_bps", *budget_bps),
            Order::SetSlices { cap_bps } => b.cap_bps.check("cap_bps", *cap_bps),
            Order::SetPosture { hunt: None } => Ok(()),
            Order::SetPosture { hunt: Some(hunt) } => b.check_hunt(hunt),
            Order::Grant { amount, .. } | Order::Burn { amount } if amount.0 == 0 => {
                Err("amount must be above 0".into())
            }
            Order::Grant { .. } | Order::Burn { .. } => Ok(()),
            Order::SetAutoBurn { bps } => b.auto_burn_bps.check("auto_burn_bps", *bps),
        }
    }
}

impl VoteClass {
    pub const fn passes(self, yes: u32, no: u32, electorate: u32) -> bool {
        match self {
            VoteClass::Light => bond_returns(yes, no, electorate) && yes > no,
            VoteClass::Strict => {
                let (yes, no, electorate) = (yes as u64, no as u64, electorate as u64);
                (yes + no) * 3 >= electorate * 2 && yes * 5 >= (yes + no) * 4
            }
        }
    }

    pub const fn min_electorate(self) -> u32 {
        match self {
            VoteClass::Light => 10,
            VoteClass::Strict => 45,
        }
    }

    pub const fn needs_closed_sale(self) -> bool {
        matches!(self, VoteClass::Strict)
    }
}

pub const fn bond_returns(yes: u32, no: u32, electorate: u32) -> bool {
    (yes as u64 + no as u64) * 10 >= electorate as u64 * 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::{preset, PresetName, ALL};

    fn hunt(sell_multiple_bps: u32, max_sell_bps: u16) -> Option<HuntLine> {
        Some(HuntLine {
            sell_multiple_bps,
            max_sell_bps,
        })
    }

    fn grant(amount: u128) -> Order {
        Order::Grant {
            to: "fund.near".parse().unwrap(),
            amount: U128(amount),
        }
    }

    #[test]
    fn each_order_has_its_class() {
        let cases = [
            (Order::SetBreath { budget_bps: 2_000 }, VoteClass::Light),
            (Order::SetSlices { cap_bps: 100 }, VoteClass::Light),
            (
                Order::SetPosture {
                    hunt: hunt(20_000, 500),
                },
                VoteClass::Strict,
            ),
            (Order::SetPosture { hunt: None }, VoteClass::Light),
            (grant(1), VoteClass::Strict),
            (Order::Burn { amount: U128(1) }, VoteClass::Light),
            (Order::SetAutoBurn { bps: 1_000 }, VoteClass::Light),
        ];
        for (order, class) in cases {
            assert_eq!(order.class(), class, "{order:?}");
            assert_eq!(class.needs_closed_sale(), class == VoteClass::Strict);
        }
        assert_eq!(
            (VoteClass::Light.min_electorate(), VoteClass::Strict.min_electorate()),
            (10, 45)
        );
    }

    #[test]
    fn order_check_holds_each_bound() {
        let p = preset(PresetName::DipHunter);
        let inside = [
            Order::SetBreath { budget_bps: 1_000 },
            Order::SetBreath { budget_bps: 3_500 },
            Order::SetSlices { cap_bps: 50 },
            Order::SetSlices { cap_bps: 200 },
            Order::SetPosture { hunt: hunt(20_000, 1) },
            Order::SetPosture {
                hunt: hunt(20_000, 1_500),
            },
            Order::SetAutoBurn { bps: 0 },
            Order::SetAutoBurn { bps: 5_000 },
        ];
        for order in inside {
            assert_eq!(order.check(&p), Ok(()), "{order:?}");
        }
        let outside = [
            (
                Order::SetBreath { budget_bps: 999 },
                "budget_bps must be 1000 to 3500, got 999",
            ),
            (
                Order::SetBreath { budget_bps: 3_501 },
                "budget_bps must be 1000 to 3500, got 3501",
            ),
            (Order::SetSlices { cap_bps: 49 }, "cap_bps must be 50 to 200, got 49"),
            (Order::SetSlices { cap_bps: 201 }, "cap_bps must be 50 to 200, got 201"),
            (
                Order::SetPosture {
                    hunt: hunt(19_999, 500),
                },
                "sell_multiple_bps must be at least 20000, got 19999",
            ),
            (
                Order::SetPosture { hunt: hunt(20_000, 0) },
                "max_sell_bps must be 1 to 1500, got 0",
            ),
            (
                Order::SetPosture {
                    hunt: hunt(20_000, 1_501),
                },
                "max_sell_bps must be 1 to 1500, got 1501",
            ),
            (
                Order::SetAutoBurn { bps: 5_001 },
                "auto_burn_bps must be 0 to 5000, got 5001",
            ),
        ];
        for (order, reason) in outside {
            assert_eq!(order.check(&p), Err(reason.to_string()));
        }
    }

    #[test]
    fn disabled_features_are_refused() {
        let mut p = preset(PresetName::Hoarder);
        p.features.grant = false;
        p.features.burn = false;
        p.features.hunt = false;
        p.features.auto_burn = false;
        for order in [
            grant(1),
            Order::Burn { amount: U128(1) },
            Order::SetPosture { hunt: None },
            Order::SetAutoBurn { bps: 0 },
        ] {
            assert!(
                order
                    .check(&p)
                    .unwrap_err()
                    .contains("is disabled in the hoarder preset"),
                "{order:?}"
            );
        }
        assert_eq!(Order::SetBreath { budget_bps: 2_000 }.check(&p), Ok(()));
    }

    #[test]
    fn a_zero_grant_or_burn_is_refused() {
        for name in ALL {
            let p = preset(name);
            assert_eq!(grant(0).check(&p), Err("amount must be above 0".into()));
            assert_eq!(
                Order::Burn { amount: U128(0) }.check(&p),
                Err("amount must be above 0".into())
            );
            assert_eq!(grant(1).check(&p), Ok(()));
            assert_eq!(Order::Burn { amount: U128(1) }.check(&p), Ok(()));
        }
    }

    #[test]
    fn vote_classes_pass_at_their_boundaries() {
        let light = VoteClass::Light;
        assert!(light.passes(2, 1, 10));
        assert!(!light.passes(1, 1, 10));
        assert!(!light.passes(2, 0, 10));
        assert!(light.passes(14, 13, 90));
        assert!(!light.passes(13, 13, 90));

        let strict = VoteClass::Strict;
        assert!(strict.passes(48, 12, 90));
        assert!(!strict.passes(47, 12, 90));
        assert!(!strict.passes(47, 13, 90));
        assert!(strict.passes(30, 0, 45));
        assert!(!strict.passes(29, 0, 45));
        assert!(strict.passes(24, 6, 45));
        assert!(!strict.passes(23, 7, 45));
    }

    #[test]
    fn the_bond_returns_at_30_percent_turnout() {
        assert!(bond_returns(0, 3, 10));
        assert!(!bond_returns(1, 1, 10));
        assert!(bond_returns(27, 0, 90));
        assert!(!bond_returns(13, 13, 90));
    }
}
