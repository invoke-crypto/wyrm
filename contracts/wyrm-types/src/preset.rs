use near_sdk::{env, near, AccountId, CryptoHash, NearToken};

pub const SECOND: u64 = 1_000_000_000;
pub const MINUTE: u64 = 60 * SECOND;
pub const HOUR: u64 = 60 * MINUTE;
pub const DAY: u64 = 24 * HOUR;

#[near(serializers = [borsh, json])]
#[serde(rename_all = "kebab-case")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresetName {
    Hoarder,
    Steady,
    DipHunter,
    Swift,
    Harvester,
    Drill,
}

pub const ALL: [PresetName; 6] = [
    PresetName::Hoarder,
    PresetName::Steady,
    PresetName::DipHunter,
    PresetName::Swift,
    PresetName::Harvester,
    PresetName::Drill,
];

impl PresetName {
    pub const fn as_str(self) -> &'static str {
        match self {
            PresetName::Hoarder => "hoarder",
            PresetName::Steady => "steady",
            PresetName::DipHunter => "dip-hunter",
            PresetName::Swift => "swift",
            PresetName::Harvester => "harvester",
            PresetName::Drill => "drill",
        }
    }
}

impl std::fmt::Display for PresetName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    pub name: PresetName,
    pub network: Network,
    pub strategy: Strategy,
    pub bounds: StrategyBounds,
    pub ladder: DipLadder,
    pub cadence: Cadence,
    pub sale: SaleTerms,
    pub royalty_to: RoyaltyTo,
    pub features: Features,
    pub times: CouncilTimes,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Any,
    TestnetOnly,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strategy {
    /// Daily breath budget, bps of the day's opening reserve.
    pub budget_bps: u16,
    /// Slice price-impact cap.
    pub cap_bps: u16,
    pub hunt: Option<HuntLine>,
    /// Share of each buy's WYRM sent to ash.
    pub auto_burn_bps: u16,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HuntLine {
    pub sell_multiple_bps: u32,
    pub max_sell_bps: u16,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrategyBounds {
    pub budget_bps: BpsRange,
    pub cap_bps: BpsRange,
    pub sell_multiple_min_bps: u32,
    pub max_sell_bps: BpsRange,
    pub auto_burn_bps: BpsRange,
}

impl StrategyBounds {
    pub fn check_hunt(&self, hunt: &HuntLine) -> Result<(), String> {
        if hunt.sell_multiple_bps < self.sell_multiple_min_bps {
            return Err(format!(
                "sell_multiple_bps must be at least {}, got {}",
                self.sell_multiple_min_bps, hunt.sell_multiple_bps
            ));
        }
        self.max_sell_bps.check("max_sell_bps", hunt.max_sell_bps)
    }
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DipLadder {
    pub steps_bps: [u16; 3],
    pub extra_bps: [u16; 3],
}

/// spread_minutes 0 opens the whole budget at the breath minute. A positive value opens it linearly over that many
/// minutes, and the breath minute is drawn so the spread ends inside the UTC day.
#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cadence {
    pub spread_minutes: u16,
    pub gap_seconds: u16,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaleTerms {
    pub price: NearToken,
    pub creator_bps: u16,
    pub length: u64,
}

/// Who receives the 5% resale royalty. Holders goes through the tithe, because one NEP-199 payout cannot carry 100
/// vault entries.
#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoyaltyTo {
    Creator,
    Holders,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    pub grant: bool,
    pub burn: bool,
    pub hunt: bool,
    pub auto_burn: bool,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CouncilTimes {
    pub season: u64,
    pub voting: u64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BpsRange {
    pub lo: u16,
    pub hi: u16,
}

impl BpsRange {
    pub const fn contains(&self, v: u16) -> bool {
        self.lo <= v && v <= self.hi
    }

    pub fn check(&self, what: &str, v: u16) -> Result<(), String> {
        if self.contains(v) {
            Ok(())
        } else {
            Err(format!("{what} must be {} to {}, got {v}", self.lo, self.hi))
        }
    }

    fn check_inside(&self, what: &str, outer: &BpsRange) -> Result<(), String> {
        if self.lo <= self.hi && outer.contains(self.lo) && outer.contains(self.hi) {
            Ok(())
        } else {
            Err(format!(
                "{what} bounds {} to {} must sit inside {} to {}",
                self.lo, self.hi, outer.lo, outer.hi
            ))
        }
    }
}

/// Nearly's tax args.
#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaxPolicy {
    pub buy_bps: u16,
    pub sell_bps: u16,
    pub creator_bps: u16,
    pub burn_bps: u16,
    pub holders_bps: u16,
}

pub mod envelope {
    use super::{BpsRange, CouncilTimes, TaxPolicy, DAY};
    use near_sdk::NearToken;
    use std::ops::RangeInclusive;

    /// 4% a side is Nearly's maximum.
    pub const TAX: TaxPolicy = TaxPolicy {
        buy_bps: 400,
        sell_bps: 400,
        creator_bps: 10_000,
        burn_bps: 0,
        holders_bps: 0,
    };
    pub const BUDGET_BPS: BpsRange = BpsRange { lo: 1_000, hi: 4_000 };
    pub const CAP_BPS: BpsRange = BpsRange { lo: 50, hi: 200 };
    pub const STEP_BPS: BpsRange = BpsRange { lo: 500, hi: 5_000 };
    pub const EXTRA_BPS: BpsRange = BpsRange { lo: 0, hi: 2_500 };
    pub const DAY_TOTAL_MAX_BPS: u16 = 8_000;
    pub const SELL_MULTIPLE_MIN_BPS: u32 = 20_000;
    pub const MAX_SELL_BPS: BpsRange = BpsRange { lo: 1, hi: 1_500 };
    pub const AUTO_BURN_BPS: BpsRange = BpsRange { lo: 0, hi: 5_000 };
    pub const SPREAD_MINUTES_MAX: u16 = 1_439;
    pub const GAP_SECONDS: RangeInclusive<u16> = 60..=3_600;
    pub const CREATOR_SALE_MAX_BPS: u16 = 2_000;
    pub const MIN_SCALE_PRICE: NearToken = NearToken::from_millinear(100);
    pub const MAINNET_TIMES: CouncilTimes = CouncilTimes {
        season: 7 * DAY,
        voting: 2 * DAY,
    };
    pub const MAINNET_SALE_LENGTH: u64 = 14 * DAY;
}

const HOARDER: Preset = Preset {
    name: PresetName::Hoarder,
    network: Network::Any,
    strategy: Strategy {
        budget_bps: 1_000,
        cap_bps: 200,
        hunt: None,
        auto_burn_bps: 0,
    },
    bounds: StrategyBounds {
        budget_bps: envelope::BUDGET_BPS,
        cap_bps: envelope::CAP_BPS,
        sell_multiple_min_bps: envelope::SELL_MULTIPLE_MIN_BPS,
        max_sell_bps: envelope::MAX_SELL_BPS,
        auto_burn_bps: envelope::AUTO_BURN_BPS,
    },
    ladder: DipLadder {
        steps_bps: [2_000, 3_500, 5_000],
        extra_bps: [0, 0, 0],
    },
    cadence: Cadence {
        spread_minutes: 0,
        gap_seconds: 60,
    },
    sale: SaleTerms {
        price: NearToken::from_near(5),
        creator_bps: 1_000,
        length: envelope::MAINNET_SALE_LENGTH,
    },
    royalty_to: RoyaltyTo::Creator,
    features: Features {
        grant: true,
        burn: true,
        hunt: true,
        auto_burn: true,
    },
    times: envelope::MAINNET_TIMES,
};

pub const fn preset(name: PresetName) -> Preset {
    match name {
        PresetName::Hoarder => HOARDER,
        PresetName::Steady => Preset {
            name,
            cadence: Cadence {
                spread_minutes: 360,
                gap_seconds: 900,
            },
            ..HOARDER
        },
        PresetName::DipHunter => Preset {
            name,
            bounds: StrategyBounds {
                budget_bps: BpsRange { lo: 1_000, hi: 3_500 },
                ..HOARDER.bounds
            },
            ladder: DipLadder {
                steps_bps: [1_000, 2_000, 3_000],
                extra_bps: [1_000, 1_500, 2_000],
            },
            ..HOARDER
        },
        PresetName::Swift => Preset {
            name,
            strategy: Strategy {
                budget_bps: 4_000,
                ..HOARDER.strategy
            },
            ..HOARDER
        },
        PresetName::Harvester => Preset {
            name,
            strategy: Strategy {
                hunt: Some(HuntLine {
                    sell_multiple_bps: 20_000,
                    max_sell_bps: 500,
                }),
                ..HOARDER.strategy
            },
            ..HOARDER
        },
        PresetName::Drill => Preset {
            name,
            network: Network::TestnetOnly,
            sale: SaleTerms {
                price: NearToken::from_millinear(100),
                creator_bps: 1_000,
                length: 2 * HOUR,
            },
            times: CouncilTimes {
                season: 10 * MINUTE,
                voting: 30 * MINUTE,
            },
            ..HOARDER
        },
    }
}

impl Preset {
    pub fn check(&self) -> Result<(), String> {
        use envelope::*;
        let (s, b, ladder) = (&self.strategy, &self.bounds, &self.ladder);

        b.budget_bps.check_inside("budget_bps", &BUDGET_BPS)?;
        b.cap_bps.check_inside("cap_bps", &CAP_BPS)?;
        b.max_sell_bps.check_inside("max_sell_bps", &MAX_SELL_BPS)?;
        b.auto_burn_bps.check_inside("auto_burn_bps", &AUTO_BURN_BPS)?;
        if b.sell_multiple_min_bps < SELL_MULTIPLE_MIN_BPS {
            return Err(format!(
                "sell_multiple_min_bps must be at least {SELL_MULTIPLE_MIN_BPS}, got {}",
                b.sell_multiple_min_bps
            ));
        }
        b.budget_bps.check("budget_bps", s.budget_bps)?;
        b.cap_bps.check("cap_bps", s.cap_bps)?;
        b.auto_burn_bps.check("auto_burn_bps", s.auto_burn_bps)?;
        if let Some(hunt) = &s.hunt {
            b.check_hunt(hunt)?;
        }
        for (step, extra) in ladder.steps_bps.iter().zip(&ladder.extra_bps) {
            STEP_BPS.check("steps_bps", *step)?;
            EXTRA_BPS.check("extra_bps", *extra)?;
        }

        let extras: u32 = ladder.extra_bps.iter().map(|e| *e as u32).sum();
        if b.budget_bps.hi as u32 + extras > DAY_TOTAL_MAX_BPS as u32 {
            return Err(format!(
                "budget ceiling {} plus dip extras {extras} passes the {DAY_TOTAL_MAX_BPS} bps day total",
                b.budget_bps.hi
            ));
        }
        if !ladder.steps_bps.windows(2).all(|w| w[0] < w[1]) {
            return Err(format!("steps_bps must strictly increase, got {:?}", ladder.steps_bps));
        }

        if !self.features.auto_burn && s.auto_burn_bps != 0 {
            return Err("auto-burn is disabled, so auto_burn_bps must launch at 0".into());
        }
        if !self.features.hunt && s.hunt.is_some() {
            return Err("Hunt is disabled, so it cannot be on at launch".into());
        }

        for (what, ns) in [
            ("the seasoning time", self.times.season),
            ("the voting time", self.times.voting),
            ("the sale length", self.sale.length),
        ] {
            if ns == 0 {
                return Err(format!("{what} must be above 0"));
            }
        }
        if self.network == Network::Any && (self.times != MAINNET_TIMES || self.sale.length != MAINNET_SALE_LENGTH) {
            return Err("a preset for any network must use the mainnet Council times and sale length".into());
        }

        if self.cadence.spread_minutes > SPREAD_MINUTES_MAX {
            return Err(format!(
                "spread_minutes must be at most {SPREAD_MINUTES_MAX}, got {}",
                self.cadence.spread_minutes
            ));
        }
        if !GAP_SECONDS.contains(&self.cadence.gap_seconds) {
            return Err(format!(
                "gap_seconds must be {} to {}, got {}",
                GAP_SECONDS.start(),
                GAP_SECONDS.end(),
                self.cadence.gap_seconds
            ));
        }

        if self.sale.price < MIN_SCALE_PRICE {
            return Err(format!(
                "the Scale price must be at least {MIN_SCALE_PRICE}, got {}",
                self.sale.price
            ));
        }
        if self.sale.creator_bps > CREATOR_SALE_MAX_BPS {
            return Err(format!(
                "the creator's sale share must be at most {CREATOR_SALE_MAX_BPS} bps, got {}",
                self.sale.creator_bps
            ));
        }
        Ok(())
    }

    pub fn hash(&self) -> CryptoHash {
        env::sha256_array(&near_sdk::borsh::to_vec(self).expect("a preset always encodes"))
    }

    pub fn allowed_on(&self, account: &AccountId) -> Result<(), String> {
        match self.network {
            Network::Any => Ok(()),
            Network::TestnetOnly if account.as_str().ends_with(".testnet") => Ok(()),
            Network::TestnetOnly => Err(format!(
                "the {} preset runs only on .testnet accounts, not {account}",
                self.name
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::serde_json::{self, Value};

    #[test]
    fn every_preset_checks() {
        for name in ALL {
            let p = preset(name);
            assert_eq!(p.name, name);
            assert_eq!(p.check(), Ok(()), "{name}");
        }
    }

    #[test]
    fn each_envelope_rule_refuses() {
        type Break = fn(&mut Preset);
        let cases: [(&str, Break, &str); 19] = [
            (
                "1 bounds outside the envelope",
                |p| p.bounds.cap_bps.hi = 300,
                "cap_bps bounds 50 to 300",
            ),
            (
                "1 strategy outside its bounds",
                |p| p.strategy.budget_bps = 900,
                "budget_bps must be 1000 to 4000",
            ),
            (
                "1 hunt below the multiple floor",
                |p| {
                    p.strategy.hunt = Some(HuntLine {
                        sell_multiple_bps: 19_999,
                        max_sell_bps: 500,
                    })
                },
                "sell_multiple_bps must be at least 20000",
            ),
            (
                "2 day total",
                |p| p.ladder.extra_bps = [1_500, 1_500, 1_001],
                "8000 bps day total",
            ),
            (
                "2 steps not increasing",
                |p| p.ladder.steps_bps = [2_000, 2_000, 5_000],
                "strictly increase",
            ),
            (
                "3 disabled auto-burn launches on",
                |p| {
                    p.features.auto_burn = false;
                    p.strategy.auto_burn_bps = 100;
                },
                "auto-burn is disabled",
            ),
            (
                "4 disabled hunt launches on",
                |p| {
                    p.features.hunt = false;
                    p.strategy.hunt = Some(HuntLine {
                        sell_multiple_bps: 20_000,
                        max_sell_bps: 500,
                    });
                },
                "Hunt is disabled",
            ),
            (
                "5 short times on any network",
                |p| p.times.voting = HOUR,
                "mainnet Council times",
            ),
            (
                "6 spread",
                |p| p.cadence.spread_minutes = 1_440,
                "spread_minutes must be at most 1439",
            ),
            (
                "6 gap",
                |p| p.cadence.gap_seconds = 59,
                "gap_seconds must be 60 to 3600",
            ),
            (
                "7 price",
                |p| p.sale.price = NearToken::from_millinear(99),
                "Scale price must be at least",
            ),
            ("7 creator share", |p| p.sale.creator_bps = 2_001, "at most 2000 bps"),
            (
                "1 bounds upside down",
                |p| p.bounds.budget_bps = BpsRange { lo: 3_000, hi: 2_000 },
                "budget_bps bounds 3000 to 2000",
            ),
            (
                "1 multiple floor below the envelope",
                |p| p.bounds.sell_multiple_min_bps = 19_999,
                "sell_multiple_min_bps must be at least 20000",
            ),
            (
                "1 step rung",
                |p| p.ladder.steps_bps = [400, 3_500, 5_000],
                "steps_bps must be 500 to 5000, got 400",
            ),
            (
                "1 extra rung",
                |p| p.ladder.extra_bps = [2_600, 0, 0],
                "extra_bps must be 0 to 2500, got 2600",
            ),
            (
                "1 auto-burn outside its bounds",
                |p| p.strategy.auto_burn_bps = 5_001,
                "auto_burn_bps must be 0 to 5000, got 5001",
            ),
            (
                "5 short sale on any network",
                |p| p.sale.length = 13 * DAY,
                "mainnet Council times and sale length",
            ),
            (
                "6 gap above",
                |p| p.cadence.gap_seconds = 3_601,
                "gap_seconds must be 60 to 3600, got 3601",
            ),
        ];
        for (rule, break_it, needle) in cases {
            let mut p = preset(PresetName::Hoarder);
            break_it(&mut p);
            let err = p.check().expect_err(rule);
            assert!(err.contains(needle), "rule {rule}: {err}");
        }
    }

    #[test]
    fn a_zero_time_is_refused_on_every_preset() {
        type Break = fn(&mut Preset);
        let cases: [(Break, &str); 3] = [
            (|p| p.times.season = 0, "the seasoning time must be above 0"),
            (|p| p.times.voting = 0, "the voting time must be above 0"),
            (|p| p.sale.length = 0, "the sale length must be above 0"),
        ];
        for name in ALL {
            for (break_it, reason) in cases {
                let mut p = preset(name);
                break_it(&mut p);
                assert_eq!(p.check(), Err(reason.to_string()), "{name}");
            }
        }
    }

    #[test]
    fn only_drill_has_short_times() {
        for name in ALL {
            let p = preset(name);
            let mainnet = p.times == envelope::MAINNET_TIMES && p.sale.length == envelope::MAINNET_SALE_LENGTH;
            assert_eq!(mainnet, name != PresetName::Drill, "{name}");
            assert_eq!(p.network == Network::TestnetOnly, name == PresetName::Drill, "{name}");
        }
    }

    #[test]
    fn drill_is_refused_off_testnet() {
        let drill = preset(PresetName::Drill);
        let on = |a: &str| drill.allowed_on(&a.parse().unwrap());
        assert_eq!(on("drill.thewyrm.testnet"), Ok(()));
        assert!(on("dragon.thewyrm.near").unwrap_err().contains("drill"));
        assert!(on("evil.testnet.near").is_err());
        assert_eq!(
            preset(PresetName::Hoarder).allowed_on(&"dragon.thewyrm.near".parse().unwrap()),
            Ok(())
        );
    }

    #[test]
    fn the_table_matches_the_gym() {
        let gym: Value = serde_json::from_str(include_str!("../../../gym/out/presets.json")).unwrap();
        let gym_presets = gym["presets"].as_array().unwrap();
        let mut mismatches = vec![];
        let mut seen = vec![];
        for g in gym_presets {
            let name = g["name"].as_str().unwrap();
            let p = ALL.iter().map(|n| preset(*n)).find(|p| p.name.as_str() == name);
            let Some(p) = p else {
                mismatches.push(format!("{name}: in the Gym, not in the table"));
                continue;
            };
            seen.push(p.name);
            let s = &g["settings"];
            let hunt = s["hunt"].as_bool().unwrap();
            let change = &g["needs_contract_change"];
            let spread = change["spread_hours"].as_u64().map_or(0, |h| h * 60);
            let mut field = |field: &str, table: u64, gym: Option<u64>| {
                if gym != Some(table) {
                    mismatches.push(format!("{name} {field}: table {table}, gym {gym:?}"));
                }
            };
            field("budget_bps", p.strategy.budget_bps.into(), s["budget_bps"].as_u64());
            field("cap_bps", p.strategy.cap_bps.into(), s["cap_bps"].as_u64());
            for i in 0..3 {
                field(
                    &format!("steps_bps[{i}]"),
                    p.ladder.steps_bps[i].into(),
                    s["steps_bps"][i].as_u64(),
                );
                field(
                    &format!("extra_bps[{i}]"),
                    p.ladder.extra_bps[i].into(),
                    s["extra_bps"][i].as_u64(),
                );
            }
            field("hunt", p.strategy.hunt.is_some().into(), Some(hunt.into()));
            if let (true, Some(h)) = (hunt, p.strategy.hunt) {
                field(
                    "sell_multiple_bps",
                    h.sell_multiple_bps.into(),
                    s["sell_multiple_bps"].as_u64(),
                );
                field("max_sell_bps", h.max_sell_bps.into(), s["max_sell_bps"].as_u64());
            }
            field("spread_minutes", p.cadence.spread_minutes.into(), Some(spread));
            if let Some(gap) = change["slice_gap_seconds"].as_u64() {
                field("gap_seconds", p.cadence.gap_seconds.into(), Some(gap));
            }
        }
        let missing: Vec<_> = ALL
            .iter()
            .filter(|n| **n != PresetName::Drill && !seen.contains(n))
            .collect();
        assert!(missing.is_empty(), "not in the Gym: {missing:?}");
        assert!(
            mismatches.is_empty(),
            "the table drifted from gym/out/presets.json:\n{}",
            mismatches.join("\n")
        );
        let steady = preset(PresetName::Steady).cadence;
        assert_eq!((steady.spread_minutes, steady.gap_seconds), (360, 900));
    }

    #[test]
    fn the_preset_hash_is_stable_across_a_borsh_round_trip() {
        let hashes: Vec<CryptoHash> = ALL.iter().map(|n| preset(*n).hash()).collect();
        for (name, hash) in ALL.iter().zip(&hashes) {
            let bytes = near_sdk::borsh::to_vec(&preset(*name)).unwrap();
            let back: Preset = near_sdk::borsh::from_slice(&bytes).unwrap();
            assert_eq!(back, preset(*name));
            assert_eq!(&back.hash(), hash, "{name}");
            println!("{name} {}", hash.iter().map(|b| format!("{b:02x}")).collect::<String>());
        }
        for (i, a) in hashes.iter().enumerate() {
            assert!(hashes[i + 1..].iter().all(|b| a != b), "{} shares its hash", ALL[i]);
        }
    }

    #[test]
    fn the_preset_hashes_are_pinned() {
        let pinned = [
            (
                PresetName::Hoarder,
                "e9667830e0d67c7bacacc2e33192bee516480716c8e015d6ea3337302b3ebeb1",
            ),
            (
                PresetName::Steady,
                "8841a211861073ace5e6c8a9ccace33b1d5498c2309fa76d082baccd9357f79d",
            ),
            (
                PresetName::DipHunter,
                "cdcfb875acc586e17e2b9edb8a96d598ec200ced101eb6e109b582aa512a6f6d",
            ),
            (
                PresetName::Swift,
                "fefc3adbf909608a42052cbb6f4524c64bbf78be07f6c60f5077e24f2b286f99",
            ),
            (
                PresetName::Harvester,
                "2c6e6426b38f9a46b282141290eaec312f5a2b22ba91d926926532a160ae52ec",
            ),
            (
                PresetName::Drill,
                "ad9bf68ee25554a11d5060c881cc9262acfc5ee06240aa0c81749fe3a7670588",
            ),
        ];
        assert_eq!(pinned.map(|(n, _)| n), ALL);
        for (name, hex) in pinned {
            let got: String = preset(name).hash().iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(got, hex, "{name}: the table or the field order changed");
        }
    }
}
