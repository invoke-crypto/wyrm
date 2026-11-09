use super::*;
use proptest::prelude::*;
use wyrm_types::{HuntLine, Strategy};

const T0: u64 = 20_000 * DAY;
const P: u128 = 1_000 * 1_000_000_000_000_000_000; // 1,000 WYRM per NEAR at 18 decimals
const NO_SPREAD: Cadence = Cadence {
    spread_minutes: 0,
    gap_seconds: 60,
};
const HUNT: HuntLine = HuntLine {
    sell_multiple_bps: 25_000,
    max_sell_bps: 1_000,
};

/// v1's launch settings (a 20% breath, extras of 5, 10 and 15%, a 2.5% cap), so the rules tests keep their numbers.
#[derive(Clone, Copy)]
struct Setup {
    strategy: Strategy,
    ladder: DipLadder,
    cadence: Cadence,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            strategy: Strategy {
                budget_bps: 2_000,
                cap_bps: 250,
                hunt: None,
                auto_burn_bps: 0,
            },
            ladder: DipLadder {
                steps_bps: [2_000, 3_500, 5_000],
                extra_bps: [500, 1_000, 1_500],
            },
            cadence: NO_SPREAD,
        }
    }
}

impl Setup {
    fn rules(&self) -> Rules<'_> {
        Rules {
            strategy: &self.strategy,
            ladder: &self.ladder,
            cadence: &self.cadence,
        }
    }
}

fn series(points: &[(u64, u128)]) -> Vec<Sample> {
    let mut out: Vec<Sample> = vec![];
    for &(at, p) in points {
        let s = match out.last() {
            None => Sample::first(at, p),
            Some(prev) => prev.next(at, p),
        };
        out.push(s);
    }
    out
}

fn reference_of(samples: &[Sample], now: u64) -> Option<u128> {
    reference(samples.len() as u32, |i| samples[i as usize], now)
}

#[test]
fn the_reference_weights_each_price_by_how_long_it_held() {
    // 1,000 for 3 hours, then 2,000 for 1 hour: (3 * 1,000 + 1 * 2,000) / 4.
    let s = series(&[(T0, 1_000), (T0 + 3 * HOUR, 2_000)]);
    assert_eq!(reference_of(&s, T0 + 4 * HOUR), Some(1_250));
    assert_eq!(reference_of(&s[..1], T0), None, "no time has passed");
}

#[test]
fn the_reference_forgets_prices_older_than_a_day() {
    let s = series(&[(T0, 9_000), (T0 + 2 * HOUR, 1_000)]);
    assert_eq!(reference_of(&s, T0 + 2 * HOUR + DAY), Some(1_000));
    // An hour of the old price is still inside the window one hour earlier.
    assert_eq!(reference_of(&s, T0 + HOUR + DAY), Some((9_000 + 23 * 1_000) / 24));
}

proptest! {
    // The poisoning bound: a sample at any price, taken at time t, moves the reference at t + dt by exactly its
    // weight dt / window times its distance from the price it replaced, give or take rounding.
    #[test]
    fn one_late_sample_moves_the_reference_by_no_more_than_its_time_weight(
        gaps in prop::collection::vec(60u64..20_000, 1..60),
        prices in prop::collection::vec(1u128..1_000_000_000_000_000_000_000_000_000_000, 60),
        poison in 0u128..u128::MAX / 4_000_000,
        late in 60u64..200_000,
        dt in 0u64..3_000,
    ) {
        let mut points = vec![];
        let mut t = T0;
        for (i, g) in gaps.iter().enumerate() {
            points.push((t, prices[i]));
            t += g * SECOND;
        }
        let honest = series(&points);
        let last = *honest.last().unwrap();
        let at = last.at + late * SECOND;
        let mut poisoned = honest.clone();
        poisoned.push(last.next(at, poison));
        let now = at + dt * SECOND;
        let with = reference_of(&poisoned, now).unwrap();
        let without = reference_of(&honest, now).unwrap();
        let from = now.saturating_sub(REFERENCE_WINDOW).max(T0);
        let width = secs(now) - secs(from);
        let bound = mul_div(dt as u128, poison.abs_diff(last.tokens_per_near), width) + 1;
        prop_assert!(with.abs_diff(without) <= bound, "moved {} > bound {}", with.abs_diff(without), bound);
        if dt == 0 {
            prop_assert_eq!(with, without);
        }
    }

    #[test]
    fn the_reference_stays_between_the_lowest_and_highest_sample(
        points in prop::collection::vec((60u64..50_000, 1u128..1_000_000_000_000_000_000_000_000_000), 1..80),
        tail in 1u64..100_000,
    ) {
        let mut t = T0;
        let pts: Vec<(u64, u128)> = points.iter().map(|(g, p)| { t += g * SECOND; (t, *p) }).collect();
        let s = series(&pts);
        let r = reference_of(&s, t + tail * SECOND).unwrap();
        let lo = pts.iter().map(|p| p.1).min().unwrap();
        let hi = pts.iter().map(|p| p.1).max().unwrap();
        prop_assert!(lo <= r && r <= hi);
    }
}

fn newest_first(points: &[(u64, u128)]) -> Vec<Sample> {
    let mut s = series(points);
    s.reverse();
    s
}

#[test]
fn a_dip_needs_three_samples_over_ten_minutes() {
    let steps = [1_000, 2_000, 3_000];
    let dip = P * 10_000 / 8_500; // 15% cheaper: beyond step 1 only
    let m = MINUTE;
    assert_eq!(
        confirmed_dip(
            &newest_first(&[(T0, dip), (T0 + 5 * m, dip), (T0 + 10 * m, dip)]),
            P,
            &steps
        ),
        Dip {
            depth: 1,
            confirmations: 3,
            since: T0
        }
    );
    let none = |pts: &[(u64, u128)]| confirmed_dip(&newest_first(pts), P, &steps).depth;
    assert_eq!(none(&[(T0, dip), (T0 + 10 * m, dip)]), 0, "two samples");
    assert_eq!(
        none(&[(T0, dip), (T0 + 4 * m, dip), (T0 + 9 * m, dip)]),
        0,
        "nine minutes"
    );
    assert_eq!(
        none(&[(T0, dip), (T0 + 5 * m, P), (T0 + 10 * m, dip), (T0 + 11 * m, dip)]),
        0,
        "unbroken run"
    );
    let deep = P * 10_000 / 6_900;
    assert_eq!(none(&[(T0, deep), (T0 + 5 * m, deep), (T0 + 10 * m, deep)]), 3);
    let every_minute: Vec<(u64, u128)> = (0..=12).map(|i| (T0 + i * m, dip)).collect();
    assert_eq!(
        confirmed_dip(&newest_first(&every_minute), P, &steps),
        Dip {
            depth: 1,
            confirmations: 11,
            since: T0 + 2 * m
        }
    );
    assert_eq!(dip_depth(deep, P, &steps), 3);
    let ten = (P * 10_000).div_ceil(9_000);
    assert_eq!(dip_depth(ten, P, &steps), 1, "exactly 10% is a dip");
    assert_eq!(dip_depth(ten - 1, P, &steps), 0);
}

#[test]
fn a_spike_is_more_than_fifteen_percent_above_the_reference() {
    let edge = (P * 10_000).div_ceil(11_500);
    assert!(!spiking(edge, P));
    assert!(spiking(edge - 1, P));
    assert!(!spiking(P, P));
}

#[test]
fn the_breath_minute_falls_inside_its_own_utc_day() {
    for b in 0..=255u8 {
        let d = Day::starting(T0 + 123 * SECOND, [b; 32], 7, &NO_SPREAD);
        assert_eq!(d.index, T0 / DAY);
        assert!(d.breath_at >= T0 && d.breath_at < T0 + DAY && d.breath_at.is_multiple_of(MINUTE));
    }
    let d = Day::starting(T0, [7; 32], 7, &NO_SPREAD);
    assert!(roll(&d, T0 + DAY - 1, [0; 32], 9, &NO_SPREAD).is_none());
    let next = roll(&d, T0 + DAY, [0; 32], 9, &NO_SPREAD).unwrap();
    assert_eq!((next.index, next.opening_reserve, next.spent), (d.index + 1, 9, 0));
}

fn day(breath_at: u64) -> Day {
    Day {
        index: T0 / DAY,
        breath_at,
        opening_reserve: 100 * ONE_NEAR,
        ..Day::default()
    }
}

fn inputs<'a>(s: &'a Setup, d: &'a Day, h: &'a Hoard, now: u64, reserve: u128, dip: u8) -> Inputs<'a> {
    Inputs {
        now,
        rules: s.rules(),
        day: d,
        hoard: h,
        reserve,
        last_slice_at: 0,
        market: Some(Observation {
            tokens_per_near: P,
            reference: Some(P),
            dip,
        }),
    }
}

#[test]
fn the_breath_spends_the_budget_only_from_the_breath_minute() {
    let s = Setup::default();
    let h = Hoard::default();
    let d = day(T0 + HOUR);
    assert_eq!(
        decide(&inputs(&s, &d, &h, T0 + HOUR - 1, 100 * ONE_NEAR, 0)),
        Decision::Rest(Rest::BeforeBreath)
    );
    assert_eq!(
        decide(&inputs(&s, &d, &h, T0 + HOUR, 100 * ONE_NEAR, 0)),
        Decision::Trade {
            kind: SliceKind::Breath,
            amount_in: 20 * ONE_NEAR
        }
    );
    // The budget is of the opening reserve: later money does not raise it, and a smaller reserve caps it.
    assert_eq!(
        decide(&inputs(&s, &d, &h, T0 + HOUR, 500 * ONE_NEAR, 0)),
        Decision::Trade {
            kind: SliceKind::Breath,
            amount_in: 20 * ONE_NEAR
        }
    );
    assert_eq!(
        decide(&inputs(&s, &d, &h, T0 + HOUR, 5 * ONE_NEAR, 0)),
        Decision::Trade {
            kind: SliceKind::Breath,
            amount_in: 5 * ONE_NEAR
        }
    );
}

#[test]
fn dip_extras_are_cumulative_and_open_buying_before_the_breath() {
    let s = Setup::default();
    let h = Hoard::default();
    let mut d = day(T0 + 10 * HOUR);
    assert_eq!(
        decide(&inputs(&s, &d, &h, T0, 100 * ONE_NEAR, 3)),
        Decision::Trade {
            kind: SliceKind::Dip,
            amount_in: 30 * ONE_NEAR
        },
        "step 3 adds all three extras: 5 + 10 + 15"
    );
    assert_eq!(
        decide(&inputs(&s, &d, &h, T0, 100 * ONE_NEAR, 1)),
        Decision::Trade {
            kind: SliceKind::Dip,
            amount_in: 5 * ONE_NEAR
        }
    );
    // After the breath minute the base budget adds on top: 20 + 5 + 10 + 15.
    assert_eq!(day_budget(s.rules(), &d, T0 + 10 * HOUR, 3), 50 * ONE_NEAR);
    // Extras count only while the dip is confirmed: once it lapses they are gone, even if spent.
    d.spent = 30 * ONE_NEAR;
    assert_eq!(
        decide(&inputs(&s, &d, &h, T0, 100 * ONE_NEAR, 0)),
        Decision::Rest(Rest::BeforeBreath)
    );
    assert_eq!(
        decide(&inputs(&s, &d, &h, T0 + 10 * HOUR, 70 * ONE_NEAR, 0)),
        Decision::Rest(Rest::Spent)
    );
}

#[test]
fn spikes_gaps_and_a_missing_reference_stop_buys() {
    let s = Setup::default();
    let h = Hoard::default();
    let d = day(T0);
    let mut i = inputs(&s, &d, &h, T0 + HOUR, 100 * ONE_NEAR, 0);
    i.market = Some(Observation {
        tokens_per_near: P * 10_000 / 11_600,
        reference: Some(P),
        dip: 0,
    });
    assert_eq!(decide(&i), Decision::Rest(Rest::Spike));
    i.market = Some(Observation {
        tokens_per_near: P,
        reference: None,
        dip: 0,
    });
    assert_eq!(decide(&i), Decision::Rest(Rest::NoReference));
    let mut i = inputs(&s, &d, &h, T0 + HOUR, 100 * ONE_NEAR, 0);
    i.last_slice_at = T0 + HOUR - MINUTE + 1;
    assert_eq!(decide(&i), Decision::Rest(Rest::Gap));
}

proptest! {
    #[test]
    fn a_buy_never_exceeds_the_day_budget_or_the_reserve(
        budget in 1_000u16..=4_000,
        opening in 0u128..1_000_000 * ONE_NEAR,
        reserve in 0u128..1_000_000 * ONE_NEAR,
        spent in 0u128..1_000_000 * ONE_NEAR,
        dip in 0u8..=3,
        breath_open in any::<bool>(),
    ) {
        let mut s = Setup::default();
        s.strategy.budget_bps = budget;
        let h = Hoard::default();
        let mut d = day(if breath_open { T0 } else { T0 + DAY - 1 });
        d.spent = spent;
        d.opening_reserve = opening;
        let now = T0 + HOUR;
        if let Decision::Trade { amount_in, kind } = decide(&inputs(&s, &d, &h, now, reserve, dip)) {
            prop_assert!(amount_in <= reserve);
            let base = if breath_open { budget as u128 } else { 0 };
            let total_bps = base + s.ladder.extra_bps[..dip as usize].iter().map(|e| *e as u128).sum::<u128>();
            prop_assert!((spent + amount_in) * BPS <= total_bps * opening);
            prop_assert_eq!(kind == SliceKind::Dip, dip > 0);
        }
    }

    #[test]
    fn the_picked_slice_always_passes_the_impact_cap(
        amount in ONE_NEAR / 100..10_000 * ONE_NEAR,
        depth in 10u128..100_000,
        cap in 50u16..=300,
    ) {
        // Constant-product quotes on a pool with `depth` NEAR and 1,000 tokens per NEAR.
        let x = depth * ONE_NEAR;
        let y = x * 1_000;
        let quote = |a: u128| mul_div(y, a, x + a);
        let tpn = quote(ONE_NEAR);
        let c = candidates(Side::Buy, amount, tpn);
        let q: Vec<Option<u128>> = c.iter().map(|a| Some(quote(*a))).collect();
        if let Some((a, out)) = pick_slice(&c, &q, Unit::buy(tpn), cap) {
            prop_assert!(out * BPS >= mul_div(a, tpn, ONE_NEAR) * (BPS - cap as u128));
            prop_assert!(a <= amount && a >= amount >> MAX_HALVINGS);
            // Constant product: impact against the 1 NEAR quote is (a - 1 NEAR) / (x + a).
            prop_assert!(mul_div(a.saturating_sub(ONE_NEAR), BPS, x + a) <= cap as u128 + 1);
        }
    }
}

#[test]
fn halving_stops_at_six_and_skips_a_slice_that_is_still_too_large() {
    let c = candidates(Side::Buy, 64 * ONE_NEAR, P);
    assert_eq!(c.len(), 7);
    assert_eq!(*c.last().unwrap(), ONE_NEAR);
    // Every candidate gets half of what the spot price promises.
    let q: Vec<Option<u128>> = c.iter().map(|a| Some(mul_div(*a, P, ONE_NEAR) / 2)).collect();
    assert_eq!(pick_slice(&c, &q, Unit::buy(P), 250), None);
    let mut q2 = q.clone();
    q2[2] = Some(mul_div(c[2], P, ONE_NEAR));
    assert_eq!(
        pick_slice(&c, &q2, Unit::buy(P), 250),
        Some((16 * ONE_NEAR, mul_div(c[2], P, ONE_NEAR)))
    );
    // A sell is measured against the quote for 1 NEAR worth of WYRM, which already carries the pool fee.
    let unit = Unit {
        unit_in: P,
        unit_out: ONE_NEAR * 99 / 100,
    };
    assert!(within_impact(
        10 * P,
        10 * ONE_NEAR * 99 / 100 * 9_750 / 10_000,
        unit,
        250
    ));
    assert!(!within_impact(
        10 * P,
        10 * ONE_NEAR * 99 / 100 * 9_749 / 10_000,
        unit,
        250
    ));
    assert_eq!(min_out(10_000), 9_850);
}

fn bought(near: u128, tokens: u128) -> Hoard {
    Hoard {
        wyrm: tokens,
        near_spent: near,
        wyrm_bought: tokens,
        ..Hoard::default()
    }
}

#[test]
fn hunt_is_off_by_default_and_sells_only_above_the_multiple() {
    let h = bought(10 * ONE_NEAR, 10 * P); // average cost: 1,000 tokens per NEAR
    let d = day(T0);
    let mut s = Setup::default();
    let three_x = P / 3;
    let mut i = inputs(&s, &d, &h, T0 + HOUR, 0, 0);
    i.market = Some(Observation {
        tokens_per_near: three_x,
        reference: Some(three_x),
        dip: 0,
    });
    assert!(matches!(decide(&i), Decision::Rest(_)), "Hunt off: no sell even at 3x");
    s.strategy.hunt = Some(HUNT);
    let mut i = inputs(&s, &d, &h, T0 + HOUR, 0, 0);
    i.market = Some(Observation {
        tokens_per_near: three_x,
        reference: Some(three_x),
        dip: 0,
    });
    assert_eq!(
        decide(&i),
        Decision::Trade {
            kind: SliceKind::Hunt,
            amount_in: P
        },
        "10% of the hoard"
    );
    i.market = Some(Observation {
        tokens_per_near: P / 2,
        reference: Some(P / 2),
        dip: 0,
    });
    assert!(matches!(decide(&i), Decision::Rest(_)), "2x is under the 2.5x line");
    assert!(above_sell_line(&h, P * 10 / 25, 25_000));
    assert!(!above_sell_line(&h, P * 10 / 25 + 1, 25_000));
}

#[test]
fn donations_never_lower_the_average_cost() {
    let mut h = bought(10 * ONE_NEAR, 10 * P);
    let mut u = Unsettled {
        wyrm_out: 5 * P,
        wnear_out: 0,
    };
    // The swap delivered 4.9 P against a quote of 5 P, and someone donated 100 P in the same interval.
    reconcile_wyrm(&mut h, &mut u, 10 * P + 49 * P / 10 + 100 * P, 0);
    assert_eq!(h.wyrm_bought, 15 * P);
    assert_eq!(h.donated, 99 * P + 9 * P / 10);
    assert_eq!(u.wyrm_out, 0);
    // With nothing quoted in between, a rise is all donation.
    let held = h.wyrm;
    reconcile_wyrm(&mut h, &mut u, held + 7, 0);
    assert_eq!(h.wyrm_bought, 15 * P);
    // WYRM owed to ash is in the balance but not in the hoard, and is neither bought nor donated.
    let held = h.wyrm;
    h.ash_due = 1_000;
    reconcile_wyrm(&mut h, &mut u, held + 1_000, 0);
    assert_eq!((h.wyrm, h.donated), (held, 99 * P + 9 * P / 10 + 7));
    // A taxed buy delivers its expected net: all of it is bought, and the 4% the token kept is never donated.
    u.wyrm_out = expected_net(10 * P);
    reconcile_wyrm(&mut h, &mut u, held + 1_000 + 96 * P / 10, 0);
    assert_eq!(h.wyrm_bought, 15 * P + 96 * P / 10);
    assert_eq!(h.donated, 99 * P + 9 * P / 10 + 7);
}

#[test]
fn expected_net_takes_the_buy_tax_off_the_quote() {
    assert_eq!(expected_net(10_000), 9_600);
    assert_eq!(expected_net(P), P / 100 * 96);
    assert_eq!(expected_net(0), 0);
    assert_eq!(min_out(10_000), 9_850, "min_out stays on the gross quote");
}

#[test]
fn sell_quote_input_takes_the_sell_tax_off_what_is_sent() {
    assert_eq!(sell_quote_input(10_000), 9_600);
    assert_eq!(sell_quote_input(25 * P), 24 * P);
    assert_eq!(min_out(sell_quote_input(10_000)), 9_456);
}

proptest! {
    #[test]
    fn auto_burn_takes_its_share_of_bought_not_donated(
        held in 0u128..1_000_000 * P,
        expected in 0u128..1_000_000 * P,
        arrived in 0u128..2_000_000 * P,
        bps in 0u16..=5_000,
        owed in 0u128..1_000 * P,
    ) {
        let mut h = Hoard { wyrm: held, ash_due: owed, ..Hoard::default() };
        let mut u = Unsettled { wyrm_out: expected, wnear_out: 0 };
        reconcile_wyrm(&mut h, &mut u, held + owed + arrived, bps);
        let bought = arrived.min(expected);
        prop_assert_eq!(h.wyrm_bought, bought);
        prop_assert_eq!(h.donated, arrived - bought);
        let burn = mul_div(bought, bps as u128, BPS);
        prop_assert_eq!(h.ash_due, owed + burn);
        prop_assert_eq!(h.wyrm + h.ash_due, held + owed + arrived, "every token is the hoard's or ash's");
    }
}

#[test]
fn a_spread_opens_the_budget_linearly_inside_the_day() {
    let mut s = Setup::default();
    s.cadence.spread_minutes = 360;
    s.ladder.extra_bps = [0, 0, 0];
    for b in 0..=255u8 {
        let d = Day::starting(T0 + 123 * SECOND, [b; 32], 7, &s.cadence);
        assert!(
            d.breath_at + 360 * MINUTE <= T0 + DAY,
            "the spread ends inside the UTC day"
        );
    }
    let mut seed = [0u8; 32];
    seed[..8].copy_from_slice(&1_079u64.to_le_bytes());
    assert_eq!(
        Day::starting(T0, seed, 7, &s.cadence).breath_at,
        T0 + 1_079 * MINUTE,
        "the last minute that fits"
    );
    seed[..8].copy_from_slice(&1_080u64.to_le_bytes());
    assert_eq!(Day::starting(T0, seed, 7, &s.cadence).breath_at, T0);
    // The Gym's allowance: budget * min(1, (since + 1 minute) / spread).
    let d = day(T0 + HOUR);
    let full = 20 * ONE_NEAR;
    assert_eq!(day_budget(s.rules(), &d, T0 + HOUR - 1, 0), 0);
    assert_eq!(day_budget(s.rules(), &d, T0 + HOUR, 0), full / 360);
    assert_eq!(day_budget(s.rules(), &d, T0 + HOUR + 179 * MINUTE, 0), full / 2);
    assert_eq!(day_budget(s.rules(), &d, T0 + HOUR + 359 * MINUTE, 0), full);
    assert_eq!(day_budget(s.rules(), &d, T0 + HOUR + 20 * HOUR, 0), full);
    assert_eq!(
        decide(&inputs(
            &s,
            &d,
            &Hoard::default(),
            T0 + HOUR + 59 * MINUTE,
            100 * ONE_NEAR,
            0
        )),
        Decision::Trade {
            kind: SliceKind::Breath,
            amount_in: full / 6
        },
        "an hour in, a sixth of the day's breath"
    );
}

#[test]
fn the_gap_comes_from_the_preset() {
    let mut s = Setup::default();
    s.cadence.gap_seconds = 900;
    let h = Hoard::default();
    let d = day(T0);
    let mut i = inputs(&s, &d, &h, T0 + HOUR, 100 * ONE_NEAR, 0);
    i.last_slice_at = T0 + HOUR - 900 * SECOND + 1;
    assert_eq!(decide(&i), Decision::Rest(Rest::Gap));
    i.last_slice_at = T0 + HOUR - 900 * SECOND;
    assert!(matches!(decide(&i), Decision::Trade { .. }));
}

#[test]
fn each_of_the_five_moods() {
    let s = Setup::default();
    let h = Hoard::default();
    let d = day(T0 + HOUR);
    let r = 100 * ONE_NEAR;
    assert_eq!(mood(&inputs(&s, &d, &h, T0, r, 0), None, 0), Mood::Sleeping);
    assert_eq!(mood(&inputs(&s, &d, &h, T0, r, 0), None, 1), Mood::Stirring);
    assert_eq!(
        mood(&inputs(&s, &d, &h, T0, r, 1), None, 1),
        Mood::Breathing,
        "a confirmed dip breathes"
    );
    assert_eq!(mood(&inputs(&s, &d, &h, T0 + HOUR, r, 0), None, 0), Mood::Breathing);
    let mut gorged = d.clone();
    gorged.spent = 20 * ONE_NEAR;
    assert_eq!(
        mood(&inputs(&s, &gorged, &h, T0 + HOUR, 70 * ONE_NEAR, 0), None, 0),
        Mood::Gorged
    );
    assert_eq!(mood(&inputs(&s, &d, &h, T0, r, 0), Some(Side::Sell), 0), Mood::Hunting);
    let mut hunting = Setup::default();
    hunting.strategy.hunt = Some(HUNT);
    let rich = bought(10 * ONE_NEAR, 10 * P);
    let mut i = inputs(&hunting, &d, &rich, T0, r, 0);
    i.market = Some(Observation {
        tokens_per_near: P / 3,
        reference: Some(P / 3),
        dip: 0,
    });
    assert_eq!(mood(&i, None, 0), Mood::Hunting);
}
