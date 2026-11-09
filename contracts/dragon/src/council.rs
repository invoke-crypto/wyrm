//! The Council side of the Dragon. scales reaches it through one method, `obey`, which answers a `Verdict` and
//! panics only on a wrong caller. Grants and burns draw on two rolling 30-day windows of `hoard.wyrm`, and every
//! outflow (grant, burn, auto-burn) is recorded in `pending` until its callback settles it.
use crate::*;
use wyrm_types::{HuntLine, Order, Preset, Strategy, Verdict};

const GAS_FT_TRANSFER: Gas = Gas::from_tgas(10);
const GAS_ASH_DEPOSIT: Gas = Gas::from_tgas(5);
const GAS_ON_OUTFLOW: Gas = Gas::from_tgas(10);
/// Ten times a Nearly token's storage_balance_bounds minimum (0.00125 NEAR, RHEA-DCL.md); NEP-145 refunds the rest,
/// and all of it once ash is registered.
const ASH_DEPOSIT: NearToken = NearToken::from_micronear(12_500);
const ONE_YOCTO: NearToken = NearToken::from_yoctonear(1);

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct Spent {
    pub at: u64,
    pub amount: u128,
}

/// One kind's rolling 30-day window. A failed outflow frees its share by zeroing its slot.
#[near(serializers = [borsh])]
pub struct Window {
    ring: Ring<Spent>,
}

impl Window {
    pub fn new(prefix: &[u8]) -> Self {
        Self {
            ring: Ring::new(prefix.to_vec(), OUTFLOW_SLOTS),
        }
    }

    pub fn sum(&self, now: u64) -> u128 {
        (0..self.ring.len())
            .filter_map(|i| self.ring.nth(i))
            .filter(|s| s.at + OUTFLOW_WINDOW > now)
            .map(|s| s.amount)
            .sum()
    }

    fn has_slot(&self, now: u64) -> bool {
        !self.ring.is_full() || self.ring.nth(0).is_some_and(|s| s.at + OUTFLOW_WINDOW <= now)
    }

    fn push(&mut self, now: u64, amount: u128) -> u64 {
        self.ring.push(Spent { at: now, amount })
    }

    fn free(&mut self, slot: u64) {
        if let Some(s) = self.ring.get_mut(slot) {
            s.amount = 0;
        }
    }

    pub fn free_slots(&self) -> u32 {
        self.ring.free_slots()
    }
}

/// The window slot an outflow holds and the `hoard.wyrm` it was measured on.
#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Draw {
    pub slot: u64,
    pub hoard: u128,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq)]
pub enum Outflow {
    Grant { to: AccountId, draw: Draw },
    Burn { draw: Draw },
    AutoBurn,
}

impl Outflow {
    pub fn name(&self) -> &'static str {
        match self {
            Outflow::Grant { .. } => "grant",
            Outflow::Burn { .. } => "burn",
            Outflow::AutoBurn => "auto_burn",
        }
    }

    pub fn grantee(&self) -> Option<&AccountId> {
        match self {
            Outflow::Grant { to, .. } => Some(to),
            _ => None,
        }
    }
}

/// One outflow in flight. `on_outflow` matches `seq`, so a callback for a dropped entry changes nothing.
#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct Pending {
    pub seq: u64,
    pub kind: Outflow,
    pub amount: u128,
    pub at: u64,
}

/// What an admitted order changes.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Strategy(Strategy),
    Grant { to: AccountId, amount: u128 },
    Burn { amount: u128 },
}

/// Bounds and features through `Order::check`, then the windows on `hoard.wyrm`. `grants` and `burns` are what each
/// window already holds. Pure.
pub fn admit(
    order: &Order,
    preset: &Preset,
    s: &Strategy,
    hoard: &Hoard,
    grants: u128,
    burns: u128,
) -> Result<Change, String> {
    order.check(preset)?;
    Ok(match order {
        Order::SetBreath { budget_bps } => Change::Strategy(Strategy {
            budget_bps: *budget_bps,
            ..*s
        }),
        Order::SetSlices { cap_bps } => Change::Strategy(Strategy {
            cap_bps: *cap_bps,
            ..*s
        }),
        Order::SetPosture { hunt } => Change::Strategy(Strategy { hunt: *hunt, ..*s }),
        Order::SetAutoBurn { bps } => Change::Strategy(Strategy {
            auto_burn_bps: *bps,
            ..*s
        }),
        Order::Grant { to, amount } => {
            outflow_cap("grant", amount.0, hoard.wyrm, grants)?;
            Change::Grant {
                to: to.clone(),
                amount: amount.0,
            }
        }
        Order::Burn { amount } => {
            outflow_cap("burn", amount.0, hoard.wyrm, burns)?;
            Change::Burn { amount: amount.0 }
        }
    })
}

/// `{"order": <Order>}` in serde's externally tagged encoding, read with the boundary's scanner: Order's derived
/// visitor alone was 7 KB of the wasm's size limit.
pub fn parse_order(input: &[u8]) -> Option<Order> {
    let order = dcl::object(dcl::object(input)??.raw("order")?.as_bytes())??;
    let (variant, body) = order.only()?;
    let b = dcl::object(body.as_bytes())??;
    let bps = |key: &str| b.number(key).and_then(|n| u16::try_from(n).ok());
    Some(match variant {
        "SetBreath" => Order::SetBreath {
            budget_bps: bps("budget_bps")?,
        },
        "SetSlices" => Order::SetSlices {
            cap_bps: bps("cap_bps")?,
        },
        "SetPosture" if b.is_null("hunt") => Order::SetPosture { hunt: None },
        "SetPosture" => {
            let h = dcl::object(b.raw("hunt")?.as_bytes())??;
            Order::SetPosture {
                hunt: Some(HuntLine {
                    sell_multiple_bps: u32::try_from(h.number("sell_multiple_bps")?).ok()?,
                    max_sell_bps: u16::try_from(h.number("max_sell_bps")?).ok()?,
                }),
            }
        }
        "Grant" => Order::Grant {
            to: b.account("to")?,
            amount: U128(b.u128("amount")?),
        },
        "Burn" => Order::Burn {
            amount: U128(b.u128("amount")?),
        },
        "SetAutoBurn" => Order::SetAutoBurn { bps: bps("bps")? },
        _ => return None,
    })
}

#[near]
impl Dragon {
    /// The Council's one verb, `obey({"order": <Order>})`. Every refusal is an answer naming the limit; only a
    /// caller other than scales panics.
    pub fn obey(&mut self) -> PromiseOrValue<Verdict> {
        require!(env::predecessor_account_id() == self.peers.scales, "only scales");
        let Some(order) = env::input().as_deref().and_then(parse_order) else {
            let reason = "the order did not parse".to_string();
            self.record(Event::OrderRefused { reason: reason.clone() });
            return PromiseOrValue::Value(Verdict::Refused { reason });
        };
        let now = env::block_timestamp();
        let admitted = self.room_for(&order, now).and_then(|()| {
            admit(
                &order,
                &self.preset(),
                &self.strategy,
                &self.hoard,
                self.grants.sum(now),
                self.burns.sum(now),
            )
        });
        let change = match admitted {
            Ok(c) => c,
            Err(reason) => {
                self.record(Event::OrderRefused { reason: reason.clone() });
                return PromiseOrValue::Value(Verdict::Refused { reason });
            }
        };
        let hoard = self.hoard.wyrm;
        let (kind, amount) = match change {
            Change::Strategy(strategy) => {
                self.strategy = strategy;
                self.record(Event::SettingsChanged(strategy));
                return PromiseOrValue::Value(Verdict::Executed);
            }
            Change::Grant { to, amount } => {
                let slot = self.grants.push(now, amount);
                self.hoard.granted += amount;
                (
                    Outflow::Grant {
                        to,
                        draw: Draw { slot, hoard },
                    },
                    amount,
                )
            }
            Change::Burn { amount } => {
                let slot = self.burns.push(now, amount);
                self.hoard.burned += amount;
                (
                    Outflow::Burn {
                        draw: Draw { slot, hoard },
                    },
                    amount,
                )
            }
        };
        self.hoard.wyrm -= amount;
        PromiseOrValue::Promise(self.send_outflow(kind, amount, now))
    }

    /// Settles an outflow. A failure restores the amount where it came from and frees its window share.
    #[private]
    pub fn on_outflow(&mut self, #[serializer(borsh)] seq: U64) -> Verdict {
        let ok = dcl::succeeded(0);
        let refused = || Verdict::Refused {
            reason: "transfer failed".into(),
        };
        let Some(i) = self.pending.iter().position(|p| p.seq == seq.0) else {
            return if ok { Verdict::Executed } else { refused() };
        };
        let Pending { kind, amount, at, .. } = self.pending.remove(i);
        let at = U64(at);
        if ok {
            self.record(match kind {
                Outflow::Grant { to, draw } => Event::GrantPaid {
                    at,
                    to,
                    amount: U128(amount),
                    hoard: U128(draw.hoard),
                },
                Outflow::Burn { draw } => Event::BurnPaid {
                    at,
                    amount: U128(amount),
                    hoard: U128(draw.hoard),
                },
                Outflow::AutoBurn => Event::AutoBurn {
                    at,
                    amount: U128(amount),
                },
            });
            return Verdict::Executed;
        }
        match &kind {
            Outflow::Grant { draw, .. } => {
                self.hoard.wyrm += amount;
                self.hoard.granted -= amount;
                self.grants.free(draw.slot);
            }
            Outflow::Burn { draw } => {
                self.hoard.wyrm += amount;
                self.hoard.burned -= amount;
                self.burns.free(draw.slot);
            }
            Outflow::AutoBurn => {
                self.hoard.ash_due += amount;
                self.hoard.auto_burned -= amount;
            }
        }
        self.record(Event::OutflowFailed {
            at,
            kind: kind.name().into(),
            amount: U128(amount),
        });
        refused()
    }
}

impl Dragon {
    /// What an outflow needs beyond its order's bounds: a bound coin, room in `pending` and a window slot.
    fn room_for(&self, order: &Order, now: u64) -> Result<(), String> {
        let window = match order {
            Order::Grant { .. } => &self.grants,
            Order::Burn { .. } => &self.burns,
            _ => return Ok(()),
        };
        if self.coin.is_none() {
            return Err("not bound".into());
        }
        if self.pending.len() >= MAX_PENDING {
            return Err(format!("{MAX_PENDING} outflows are already in flight"));
        }
        if !window.has_slot(now) {
            return Err(format!("at most {OUTFLOW_SLOTS} of each outflow in any 30 days"));
        }
        Ok(())
    }

    /// Sends ash what the reconcile set aside, when nothing else is in flight. The caller has a resting observation's
    /// gas to spare.
    pub(crate) fn send_auto_burn(&mut self, now: u64) -> Option<Promise> {
        let amount = self.hoard.ash_due;
        if amount == 0 || !self.pending.is_empty() {
            return None;
        }
        self.hoard.ash_due = 0;
        self.hoard.auto_burned += amount;
        Some(self.send_outflow(Outflow::AutoBurn, amount, now))
    }

    /// Drops entries whose callback never came, so a lost callback cannot stop the reconcile for good.
    pub(crate) fn expire_pending(&mut self, now: u64) {
        self.pending.retain(|p| p.at + PENDING_EXPIRY > now);
    }

    /// One batch on the coin. A burn first registers ash, which refunds the deposit when ash already is.
    fn send_outflow(&mut self, kind: Outflow, amount: u128, now: u64) -> Promise {
        let ash = self.peers.ash.clone();
        let (to, memo) = match &kind {
            Outflow::Grant { to, .. } => (to.clone(), "Council grant"),
            Outflow::Burn { .. } => (ash.clone(), "Council burn"),
            Outflow::AutoBurn => (ash.clone(), "auto-burn"),
        };
        let mut p = Promise::new(self.coin().token.clone());
        if kind.grantee().is_none() {
            p = p.function_call(
                "storage_deposit",
                format!(r#"{{"account_id":"{ash}","registration_only":true}}"#).into_bytes(),
                ASH_DEPOSIT,
                GAS_ASH_DEPOSIT,
            );
        }
        let p = p.function_call(
            "ft_transfer",
            format!(r#"{{"receiver_id":"{to}","amount":"{amount}","memo":"{memo}"}}"#).into_bytes(),
            ONE_YOCTO,
            GAS_FT_TRANSFER,
        );
        let seq = self.outflows;
        self.outflows += 1;
        self.pending.push(Pending {
            seq,
            kind,
            amount,
            at: now,
        });
        p.then(
            Self::ext(env::current_account_id())
                .with_static_gas(GAS_ON_OUTFLOW)
                .on_outflow(U64(seq)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wyrm_types::{preset, HuntLine, PresetName};

    fn hoard(wyrm: u128) -> Hoard {
        Hoard {
            wyrm,
            ..Hoard::default()
        }
    }

    #[test]
    fn parse_order_reads_what_serde_writes_and_refuses_the_rest() {
        let line = HuntLine {
            sell_multiple_bps: 30_000,
            max_sell_bps: 500,
        };
        let orders = [
            Order::SetBreath { budget_bps: 2_500 },
            Order::SetSlices { cap_bps: 100 },
            Order::SetPosture { hunt: Some(line) },
            Order::SetPosture { hunt: None },
            Order::Grant {
                to: "fund.near".parse().unwrap(),
                amount: U128(u128::MAX),
            },
            Order::Burn { amount: U128(7) },
            Order::SetAutoBurn { bps: 5_000 },
        ];
        for order in orders {
            let input = near_sdk::serde_json::json!({ "order": order }).to_string();
            assert_eq!(parse_order(input.as_bytes()), Some(order), "{input}");
        }
        for bad in [
            r#"{"order":{"Pause":{"hours":1}}}"#,
            r#"{"order":{"SetBreath":{"budget_bps":70000}}}"#,
            r#"{"order":{"SetBreath":{"budget_bps":"2000"}}}"#,
            r#"{"order":{"SetSlices":{"cap_bps":100},"Burn":{"amount":"1"}}}"#,
            r#"{"order":{"Grant":{"to":"x.near","amount":7}}}"#,
            r#"{"order":{"Grant":{"to":"NOT AN ID","amount":"7"}}}"#,
            r#"{"order":"SetSlices"}"#,
            r#"{"SetSlices":{"cap_bps":100}}"#,
            "",
        ] {
            assert_eq!(parse_order(bad.as_bytes()), None, "{bad}");
        }
    }

    #[test]
    fn admit_applies_settings_and_caps_outflows_on_hoard_wyrm() {
        let p = preset(PresetName::Hoarder);
        let s = p.strategy;
        let h = Hoard {
            ash_due: 1_000_000_000,
            ..hoard(1_000_000)
        };
        let to: AccountId = "fund.near".parse().unwrap();
        let line = HuntLine {
            sell_multiple_bps: 30_000,
            max_sell_bps: 500,
        };
        let cases = [
            (
                Order::SetBreath { budget_bps: 2_500 },
                Ok(Change::Strategy(Strategy { budget_bps: 2_500, ..s })),
            ),
            (
                Order::SetSlices { cap_bps: 100 },
                Ok(Change::Strategy(Strategy { cap_bps: 100, ..s })),
            ),
            (
                Order::SetPosture { hunt: Some(line) },
                Ok(Change::Strategy(Strategy { hunt: Some(line), ..s })),
            ),
            (
                Order::SetAutoBurn { bps: 5_000 },
                Ok(Change::Strategy(Strategy {
                    auto_burn_bps: 5_000,
                    ..s
                })),
            ),
            (
                Order::Grant {
                    to: to.clone(),
                    amount: U128(5_000),
                },
                Ok(Change::Grant {
                    to: to.clone(),
                    amount: 5_000,
                }),
            ),
            (Order::Burn { amount: U128(5_000) }, Ok(Change::Burn { amount: 5_000 })),
        ];
        for (order, want) in cases {
            assert_eq!(admit(&order, &p, &s, &h, 0, 0), want, "{order:?}");
        }
        let refuse = |order: Order, grants, burns| admit(&order, &p, &s, &h, grants, burns).unwrap_err();
        assert!(refuse(Order::SetAutoBurn { bps: 6_000 }, 0, 0).contains("auto_burn_bps must be 0 to 5000, got 6000"));
        assert!(refuse(Order::SetSlices { cap_bps: 201 }, 0, 0).contains("cap_bps"));
        // The ash_due of a billion is outside every cap: 50 bps of the million the Dragon owns, not of the balance.
        assert!(refuse(Order::Burn { amount: U128(5_001) }, 0, 0).contains("50 bps"));
        assert!(refuse(
            Order::Grant {
                to,
                amount: U128(5_000)
            },
            5_001,
            0
        )
        .contains("30 days"));
        assert!(refuse(Order::Burn { amount: U128(5_000) }, 5_001, 5_001).contains("30 days"));
        assert!(refuse(Order::Burn { amount: U128(0) }, 0, 0).contains("above 0"));
        let mut off = p;
        off.features.burn = false;
        assert!(admit(&Order::Burn { amount: U128(1) }, &off, &s, &h, 0, 0)
            .unwrap_err()
            .contains("disabled"));
    }
}
