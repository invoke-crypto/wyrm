//! The engine constants every Dragon shares, and the outflow cap. The Council's ranges live in the preset
//! (`wyrm_types`); nothing here can be changed after deploy.
pub use wyrm_types::preset::{DAY, HOUR, MINUTE, SECOND};

pub const BPS: u128 = 10_000;
pub const ONE_NEAR: u128 = 1_000_000_000_000_000_000_000_000;

pub const SPIKE_BPS: u128 = 1_500;
pub const SLIPPAGE_BPS: u128 = 150;
pub const DIP_SAMPLES: usize = 3;
pub const DIP_SPAN: u64 = 10 * MINUTE;
/// Samples are at least a minute apart, so this many newest samples always cover a dip run of DIP_SPAN.
pub const DIP_SCAN: u32 = (DIP_SPAN / SAMPLE_GAP) as u32 + 1;
/// A Grant or a Burn is at most this share of `hoard.wyrm`, and each kind's window at most OUTFLOW_WINDOW_BPS.
pub const OUTFLOW_BPS: u128 = 50;
pub const OUTFLOW_WINDOW_BPS: u128 = 100;
pub const OUTFLOW_WINDOW: u64 = 30 * DAY;
pub const OUTFLOW_SLOTS: u32 = 32;
/// Grants, burns and auto-burns sent and not settled.
pub const MAX_PENDING: usize = 3;
/// A pending outflow whose callback never came is dropped after this long, so the reconcile cannot stop for good.
pub const PENDING_EXPIRY: u64 = 10 * MINUTE;
pub const LAUNCH_COOLDOWN: u64 = 10 * MINUTE;

pub const STALE: u64 = 5 * MINUTE;
pub const SAMPLE_GAP: u64 = MINUTE;
pub const REFERENCE_WINDOW: u64 = DAY;
pub const SAMPLES_CAP: u32 = 1_440;
pub const LOG_CAP: u32 = 256;
/// Halvings of a slice that fails the price-impact cap before the Dragon skips it.
pub const MAX_HALVINGS: u32 = 6;
/// No slice smaller than 0.01 NEAR of value; smaller trades cost more in gas and fees than they move.
pub const MIN_SLICE_NEAR: u128 = ONE_NEAR / 100;

/// `hoard` is `hoard.wyrm` before this outflow; `window_sum` is every outflow of its kind still in the window.
pub fn outflow_cap(what: &str, amount: u128, hoard: u128, window_sum: u128) -> Result<(), String> {
    if amount.saturating_mul(BPS) > hoard.saturating_mul(OUTFLOW_BPS) {
        return Err(format!("a {what} is at most {OUTFLOW_BPS} bps of the hoard ({hoard})"));
    }
    if (window_sum + amount).saturating_mul(BPS) > hoard.saturating_mul(OUTFLOW_WINDOW_BPS) {
        return Err(format!(
            "{what}s in any 30 days are at most {OUTFLOW_WINDOW_BPS} bps of the hoard; {window_sum} already sent"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::json_types::U128;
    use wyrm_types::{envelope, preset, HuntLine, Order, ALL};

    #[test]
    fn every_preset_checks_and_launches_inside_its_own_orders() {
        for name in ALL {
            let p = preset(name);
            p.check().unwrap();
            let s = p.strategy;
            for order in [
                Order::SetBreath {
                    budget_bps: s.budget_bps,
                },
                Order::SetSlices { cap_bps: s.cap_bps },
                Order::SetPosture { hunt: s.hunt },
                Order::SetAutoBurn { bps: s.auto_burn_bps },
            ] {
                assert_eq!(order.check(&p), Ok(()), "{name}: {order:?}");
            }
        }
        assert_eq!(
            preset(wyrm_types::PresetName::Hoarder).strategy.hunt,
            None,
            "Hunt is off at launch"
        );
    }

    #[test]
    fn breath_limits_name_the_broken_rule() {
        let p = preset(wyrm_types::PresetName::Hoarder);
        let breath = |budget_bps| Order::SetBreath { budget_bps }.check(&p);
        assert!(breath(1_000).is_ok() && breath(4_000).is_ok());
        assert!(breath(999).unwrap_err().contains("budget_bps"));
        assert!(breath(4_001).unwrap_err().contains("budget_bps"));
        let mut bad = p;
        bad.ladder.steps_bps = [2_000, 2_000, 3_000];
        assert!(bad.check().unwrap_err().contains("strictly increase"));
        let mut bad = p;
        bad.ladder.extra_bps = [2_501, 0, 0];
        assert!(bad.check().unwrap_err().contains("extra_bps"));
        let mut bad = p;
        bad.ladder.extra_bps = [1_500, 1_500, 1_001];
        assert!(bad.check().unwrap_err().contains("8000"));
    }

    #[test]
    fn slice_and_posture_limits() {
        let p = preset(wyrm_types::PresetName::Hoarder);
        let cap = |cap_bps| Order::SetSlices { cap_bps }.check(&p);
        assert!(cap(envelope::CAP_BPS.lo).is_ok() && cap(envelope::CAP_BPS.hi).is_ok());
        assert!(cap(49).is_err() && cap(201).is_err());
        let hunt = |sell_multiple_bps, max_sell_bps| {
            Order::SetPosture {
                hunt: Some(HuntLine {
                    sell_multiple_bps,
                    max_sell_bps,
                }),
            }
            .check(&p)
        };
        assert!(hunt(20_000, 1_500).is_ok());
        assert!(hunt(19_999, 1_000).unwrap_err().contains("sell_multiple_bps"));
        assert!(hunt(25_000, 1_501).unwrap_err().contains("max_sell_bps"));
        assert!(hunt(25_000, 0).unwrap_err().contains("max_sell_bps"));
        assert!(Order::SetPosture { hunt: None }.check(&p).is_ok());
    }

    #[test]
    fn a_grant_or_burn_is_capped_per_order_and_per_window_on_the_hoard() {
        let hoard = 1_000_000;
        for what in ["grant", "burn"] {
            assert!(outflow_cap(what, 5_000, hoard, 0).is_ok());
            assert!(outflow_cap(what, 5_001, hoard, 0).unwrap_err().contains("50 bps"));
            assert!(outflow_cap(what, 5_000, hoard, 5_000).is_ok());
            assert!(outflow_cap(what, 1, hoard, 10_000).unwrap_err().contains("30 days"));
        }
        let p = preset(wyrm_types::PresetName::Hoarder);
        assert!(Order::Burn { amount: U128(0) }.check(&p).is_err());
    }
}
