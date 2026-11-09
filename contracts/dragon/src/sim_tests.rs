//! The sandbox scenarios of contracts/tests/tests/dragon_*.rs, run natively through `sim`: every receipt the Dragon
//! creates is executed against simulated tokens and a constant-product pool, and every expectation is re-derived
//! from the spec and the events' own fields. Every chain launches and binds its coin first.
use super::*;
use crate::sim::*;
use near_sdk::serde_json::{self, Value};
use wyrm_types::{HuntLine, Order};

const DAY0: u64 = 20_400 * DAY;

/// A random seed whose draw puts the breath at this minute of the day.
fn breath_minute(minute: u64) -> [u8; 32] {
    let mut seed = [0u8; 32];
    seed[..8].copy_from_slice(&minute.to_le_bytes());
    seed
}

fn at_of(v: &Value) -> u64 {
    v["at"].as_str().unwrap().parse().unwrap()
}

/// The slice cap re-derived from the event: the swap got at least 10_000 - cap_bps of the 1 NEAR quote's price.
fn passes_cap(s: &Value) -> bool {
    let (a, q, tpn) = (u(&s["amount_in"]), u(&s["quoted_out"]), u(&s["tokens_per_near"]));
    let cap = s["cap_bps"].as_u64().unwrap() as u128;
    math::mul_ge(q, BPS, math::mul_div(a, tpn, ONE_NEAR), BPS - cap)
}

/// A bound chain whose day has rolled at 00:01 with its breath at `minute`, with samples every 30 minutes until
/// `until`.
fn morning_on(near: u128, preset: PresetName, taxed: bool, minute: u64, until: u64) -> (Chain, u64) {
    let mut c = Chain::with(near, preset, taxed);
    c.seed = breath_minute(minute);
    c.poke();
    let breath_at = u(&c.state()["day"]["breath_at"]) as u64;
    assert_eq!(breath_at, DAY0 + minute * MINUTE);
    while c.now + 30 * MINUTE <= until {
        c.skip(30 * MINUTE);
        assert!(slices(&c.poke()).is_empty());
    }
    (c, breath_at)
}

fn morning(near: u128, minute: u64, until: u64) -> (Chain, u64) {
    morning_on(near, PresetName::Hoarder, false, minute, until)
}

fn hunt_line() -> Order {
    Order::SetPosture {
        hunt: Some(HuntLine {
            sell_multiple_bps: 25_000,
            max_sell_bps: 1_000,
        }),
    }
}

#[test]
fn the_breath_buys_only_from_its_minute_within_budget_cap_and_gap() {
    let (mut c, breath_at) = morning_on(300 * ONE_NEAR, PresetName::Hoarder, true, 600, DAY0 + 595 * MINUTE);
    c.now = breath_at - 2 * MINUTE;
    let evs = c.poke();
    assert!(slices(&evs).is_empty());
    assert_eq!(named(&evs, "observe")[0]["decision"], "rest: before the breath");
    c.now = breath_at;

    // The scripted path: the pool is reset before every poke, so the Dragon's own buys never move the price.
    let path = [100u128, 102, 99, 101, 98, 100, 103, 100, 99, 101, 100, 102];
    let mut seen = vec![];
    for k in path {
        c.set_pool(POOL_NEAR, POOL_WYRM * k / 100);
        c.gas.clear();
        let evs = c.poke();
        if !slices(&evs).is_empty() && seen.is_empty() {
            println!("GAS of a trading poke, (method, Ggas used, Ggas given): {:?}", c.gas);
            for (method, used, given) in &c.gas {
                assert!(used <= given, "{method} used {used} of {given} Ggas");
            }
        }
        seen.extend(slices(&evs));
        c.skip(30 * SECOND);
    }
    let state = c.state();
    let opening = u(&state["day"]["opening_reserve"]);
    let budget = opening * 1_000 / 10_000;
    assert!(seen.len() >= 2, "the budget takes capped slices, got {}", seen.len());
    let mut spent = 0;
    let mut last_at = 0;
    for (name, s) in &seen {
        assert_eq!(name, "breath_slice");
        let at = at_of(s);
        assert!(at >= breath_at);
        assert!(
            last_at == 0 || at - last_at >= MINUTE,
            "slices {} s apart",
            (at - last_at) / SECOND
        );
        last_at = at;
        assert!(passes_cap(s), "over the impact cap: {s}");
        assert_eq!(s["cap_bps"], 200);
        assert_eq!(u(&s["budget"]), budget);
        assert_eq!(u(&s["breath_at"]) as u64, breath_at);
        assert_eq!(
            u(&s["min_out"]),
            rules::min_out(u(&s["quoted_out"])),
            "min_out on the gross quote"
        );
        assert!(s["reference"].is_string());
        spent += u(&s["amount_in"]);
    }
    assert!(spent <= budget, "spent {spent} over the budget {budget}");
    assert!(
        budget - spent < MIN_SLICE_NEAR,
        "the breath spends its budget down to dust"
    );
    assert_eq!(u(&state["day"]["spent"]), spent);
    assert_eq!(state["mood"], "Gorged");
    // Besides the swaps, only the wrap registration at init and 1 yocto per ft_transfer_call left; the launch
    // deposit went to Nearly and back to the creator whole.
    assert_eq!(
        c.near() + c.balance(WRAP, DRAGON) + spent + REGISTRATION + seen.len() as u128,
        300 * ONE_NEAR,
        "every other yocto went into a swap"
    );

    c.skip(MINUTE);
    assert!(slices(&c.poke()).is_empty());
    let state = c.state();
    let held = c.balance(WYRM, DRAGON);
    let h = &state["hoard"];
    assert_eq!(
        u(&h["wyrm"]),
        held,
        "the next observation reconciles the hoard to the balance"
    );
    let expected: u128 = seen.iter().map(|(_, s)| rules::expected_net(u(&s["quoted_out"]))).sum();
    assert_eq!(u(&h["wyrm_bought"]), expected, "the taxed buys book their expected net");
    assert!(
        u(&h["donated"]) <= seen.len() as u128,
        "the token rounds its tax down, a yocto a buy"
    );
    assert_eq!(u(&h["near_spent"]), spent);
}

#[test]
fn liquidity_far_from_the_price_does_not_grow_slices() {
    let mut second = vec![];
    for far in [false, true] {
        let (mut c, breath_at) = morning(600 * ONE_NEAR, 600, DAY0 + 9 * HOUR);
        c.now = breath_at;
        let first = slices(&c.poke());
        assert_eq!(first.len(), 1);
        // At 2% impact a constant-product pool of 1,000 NEAR takes about 20.4 NEAR; the halving lands under it.
        assert!(u(&first[0].1["amount_in"]) <= 21 * ONE_NEAR);
        if far {
            c.add_far_liquidity(100_000 * ONE_NEAR, 100_000_000 * T);
        }
        c.set_pool(POOL_NEAR, POOL_WYRM);
        c.skip(61 * SECOND);
        let evs = c.poke();
        assert_eq!(u(&named(&evs, "observe")[0]["depth_near"]) > 100_000 * ONE_NEAR, far);
        second.push(slices(&evs)[0].1["amount_in"].clone());
    }
    assert_eq!(
        second[0], second[1],
        "a hundredfold depth far from the price left the slice unchanged"
    );
}

#[test]
fn a_confirmed_dip_buys_and_an_unconfirmed_one_does_not() {
    let (mut c, _) = morning_on(300 * ONE_NEAR, PresetName::DipHunter, false, 1_200, DAY0 + 6 * HOUR);
    // 1.15 times the tokens per NEAR: beyond dip-hunter's 1,000 bps step, short of 2,000.
    let dip = POOL_WYRM * 115 / 100;
    c.set_pool(POOL_NEAR, dip);
    for n in 1..=2 {
        c.skip(5 * MINUTE);
        assert!(slices(&c.poke()).is_empty(), "sample {n} of the dip does not buy");
        let state = c.state();
        assert_eq!(state["mood"], "Stirring");
        assert_eq!(state["dip"]["step"], 1);
        assert_eq!(state["dip"]["samples"], n);
        assert_eq!(state["dip"]["confirmed"], false);
    }
    c.set_pool(POOL_NEAR, POOL_WYRM);
    c.skip(5 * MINUTE);
    assert!(
        slices(&c.poke()).is_empty(),
        "two samples over five minutes never confirmed"
    );
    assert!(c.state()["dip"].is_null());

    c.set_pool(POOL_NEAR, dip);
    let mut first_at = 0;
    let mut bought = vec![];
    for n in 1..=3 {
        c.skip(5 * MINUTE);
        let evs = c.poke();
        if n == 1 {
            first_at = at_of(&named(&evs, "observe")[0]);
        }
        bought = slices(&evs);
        assert_eq!(
            bought.is_empty(),
            n < 3,
            "only the third sample, ten minutes in, confirms"
        );
    }
    let (name, s) = &bought[0];
    assert_eq!(name, "dip_slice");
    assert_eq!(s["step_bps"], 1_000);
    assert_eq!(s["confirmations"], 3);
    assert_eq!(u(&s["confirmed_since"]) as u64, first_at);
    let opening = u(&c.state()["day"]["opening_reserve"]);
    assert_eq!(
        u(&s["budget"]),
        opening * 1_000 / 10_000,
        "before the breath only the step's extra is open"
    );
    assert!(passes_cap(s));
    // The dip lapses: its extra is gone with it, so nothing more is bought before the breath.
    c.set_pool(POOL_NEAR, POOL_WYRM);
    c.skip(5 * MINUTE);
    assert_eq!(named(&c.poke(), "observe")[0]["decision"], "rest: before the breath");
}

#[test]
fn no_buy_while_spot_is_over_fifteen_percent_above_the_reference() {
    let (mut c, breath_at) = morning(300 * ONE_NEAR, 600, DAY0 + 9 * HOUR);
    c.now = breath_at;
    // 0.8 times the tokens per NEAR is a price 25% higher.
    c.set_pool(POOL_NEAR, POOL_WYRM * 8 / 10);
    let evs = c.poke();
    assert!(slices(&evs).is_empty());
    assert_eq!(named(&evs, "observe")[0]["decision"], "rest: price spike");
    assert_ne!(c.state()["mood"], "Breathing");
    // 13% higher is inside the guard.
    c.set_pool(POOL_NEAR, POOL_WYRM * 100 / 113);
    c.skip(61 * SECOND);
    assert_eq!(slices(&c.poke()).len(), 1);
}

#[test]
fn a_refund_keeps_the_wnear_and_the_budget_and_the_next_buy_spends_that_wnear_first() {
    let (mut c, breath_at) = morning(300 * ONE_NEAR, 600, DAY0 + 9 * HOUR);
    c.now = breath_at;
    c.fail_swaps = true;
    let evs = c.poke();
    let failed = named(&evs, "swap_failed");
    assert_eq!(failed.len(), 1);
    let amount = u(&failed[0]["amount_in"]);
    assert_eq!(c.balance(WRAP, DRAGON), amount, "the refund left wNEAR in the Dragon");
    let state = c.state();
    assert_eq!(u(&state["day"]["spent"]), 0, "the refund gave the budget back");
    assert_eq!(u(&state["reserve_wnear"]), amount);
    assert_eq!(state["phase"], "Idle");
    c.context_log_has("swap_failed");

    c.fail_swaps = false;
    c.skip(61 * SECOND);
    let near_before = c.near();
    let s = slices(&c.poke())[0].1.clone();
    assert_eq!(u(&s["amount_in"]), amount);
    assert_eq!(c.balance(WRAP, DRAGON), 0, "the slice spent the wNEAR");
    assert_eq!(
        c.near(),
        near_before - 1,
        "no NEAR was wrapped; only the 1 yocto of ft_transfer_call left"
    );
}

#[test]
fn late_output_is_reconciled_and_a_donation_meanwhile_does_not_lower_the_average_cost() {
    let (mut c, breath_at) = morning(300 * ONE_NEAR, 600, DAY0 + 9 * HOUR);
    // A transfer with ft_transfer_call is a donation, not a purchase.
    c.mint(WYRM, DRAGON, 5_000 * T);
    c.transfer_in(WYRM, 5_000 * T);
    let state = c.state();
    assert_eq!(u(&state["hoard"]["donated"]), 5_000 * T);
    assert_eq!(u(&state["hoard"]["wyrm_bought"]), 0);

    c.now = breath_at;
    c.hold_output = true;
    let s = slices(&c.poke())[0].1.clone();
    let quoted = u(&s["quoted_out"]);
    assert_eq!(c.balance(WYRM, DRAGON), 5_000 * T, "the output has not arrived");
    c.hold_output = false;

    let gift = 50_000 * T;
    c.mint(WYRM, DRAGON, gift);
    c.release_held();
    let delivered = c.balance(WYRM, DRAGON) - 5_000 * T - gift;
    assert!(delivered >= u(&s["min_out"]) && delivered <= quoted);
    // Inside the slice gap, so this observation only reconciles.
    c.skip(30 * SECOND);
    assert_eq!(named(&c.poke(), "observe")[0]["decision"], "rest: slice gap");
    let h = c.state()["hoard"].clone();
    assert_eq!(
        u(&h["wyrm"]),
        c.balance(WYRM, DRAGON),
        "the hoard matches ft_balance_of"
    );
    assert_eq!(
        u(&h["wyrm_bought"]),
        rules::expected_net(quoted),
        "bought is credited only up to the expected net"
    );
    assert_eq!(u(&h["donated"]), 5_000 * T + delivered + gift - u(&h["wyrm_bought"]));
    assert_eq!(u(&h["near_spent"]), u(&s["amount_in"]));
}

/// A Hunt sale is held to the cap against the quote for selling 1 NEAR worth of WYRM, both net of the sell tax.
fn passes_sell_cap(c: &Chain, s: &Value) -> bool {
    let (w, q, tpn) = (u(&s["amount_in"]), u(&s["quoted_out"]), u(&s["tokens_per_near"]));
    let unit_in = rules::sell_quote_input(tpn);
    let unit_out = c.quote(WYRM, unit_in);
    q > 0 && unit_out > 0 && rules::within_impact(rules::sell_quote_input(w), q, rules::Unit { unit_in, unit_out }, 200)
}

#[test]
fn hunt_is_off_by_default_and_sells_a_taxed_coin_only_above_the_multiple() {
    let (mut c, breath_at) = morning_on(300 * ONE_NEAR, PresetName::Hoarder, true, 600, DAY0 + 9 * HOUR);
    c.now = breath_at;
    assert_eq!(named(&c.poke(), "breath_slice").len(), 1);
    c.skip(30 * SECOND);
    c.poke();
    let bought = u(&c.state()["hoard"]["wyrm_bought"]);
    assert!(bought > 0);
    let held = u(&c.state()["hoard"]["wyrm"]);

    // About 3.3 times the average cost, and a spike, so no buy either.
    c.set_pool(POOL_NEAR, POOL_WYRM * 10 / 33);
    c.skip(61 * SECOND);
    let evs = c.poke();
    assert!(named(&evs, "hunt_slice").is_empty(), "Hunt is off at launch");
    assert!(slices(&evs).is_empty());

    c.obey(hunt_line());
    c.set_pool(POOL_NEAR, POOL_WYRM / 2);
    c.skip(61 * SECOND);
    assert!(
        named(&c.poke(), "hunt_slice").is_empty(),
        "about 2x cost is under the 2.5x line"
    );
    assert_eq!(c.state()["mood"], "Sleeping");

    c.set_pool(POOL_NEAR, POOL_WYRM * 10 / 33);
    c.skip(61 * SECOND);
    let tax_before = c.balance(WYRM, WYRM);
    let sold = named(&c.poke(), "hunt_slice");
    assert_eq!(sold.len(), 1, "a sale over 2.5x cost, even while a spike stops buys");
    let gross = u(&sold[0]["amount_in"]);
    assert_eq!(gross, held / 10, "a tenth of the hoard per day, gross");
    assert_eq!(
        c.balance(WYRM, WYRM) - tax_before,
        gross * 4 / 100,
        "the token kept the sell tax"
    );
    assert_eq!(u(&sold[0]["min_out"]), rules::min_out(u(&sold[0]["quoted_out"])));
    assert!(passes_sell_cap(&c, &sold[0]));
    c.skip(61 * SECOND);
    let evs = c.poke();
    assert!(named(&evs, "hunt_slice").is_empty(), "the day's 10% is used up");
    let h = c.state()["hoard"].clone();
    assert!(u(&h["realized_near"]) > 0, "the proceeds were seen arriving");
    assert_eq!(u(&h["wyrm"]), held - gross);
}

#[test]
fn a_refunded_taxed_sell_restores_the_gross() {
    let (mut c, breath_at) = morning_on(300 * ONE_NEAR, PresetName::Hoarder, true, 600, DAY0 + 9 * HOUR);
    c.now = breath_at;
    c.poke();
    c.skip(30 * SECOND);
    c.poke();
    let held = c.balance(WYRM, DRAGON);
    let donated = u(&c.state()["hoard"]["donated"]);
    c.obey(hunt_line());
    c.set_pool(POOL_NEAR, POOL_WYRM * 10 / 33);
    c.fail_swaps = true;
    c.skip(61 * SECOND);
    let failed = named(&c.poke(), "swap_failed");
    assert_eq!((failed.len(), &failed[0]["slice"]), (1, &Value::from("Hunt")));
    assert_eq!(
        c.balance(WYRM, DRAGON),
        held,
        "the token refunded the gross, tax included"
    );
    assert_eq!(u(&c.state()["hoard"]["wyrm"]), held);
    c.fail_swaps = false;
    c.skip(30 * SECOND);
    c.poke();
    assert_eq!(
        u(&c.state()["hoard"]["donated"]),
        donated,
        "the refund is not mistaken for a donation"
    );
}

#[test]
fn a_grant_to_an_unregistered_account_is_refused_and_restores_the_hoard() {
    let mut c = Chain::new(50 * ONE_NEAR);
    // A breath at the day's last minute, so no buy moves the hoard meanwhile.
    c.seed = breath_minute(1_439);
    let hoard = 1_000_000 * T;
    c.mint(WYRM, DRAGON, hoard);
    c.transfer_in(WYRM, hoard);
    let half = hoard / 200;
    let grant = |to: &str| Order::Grant {
        to: to.parse().unwrap(),
        amount: U128(half),
    };
    c.obey(grant("nobody.near"));
    let evs = c.take_events();
    assert_eq!(named(&evs, "outflow_failed")[0]["kind"], "grant");
    let state = c.state();
    assert_eq!((u(&state["hoard"]["wyrm"]), u(&state["hoard"]["granted"])), (hoard, 0));
    assert_eq!(c.balance(WYRM, DRAGON), hoard);

    c.mint(WYRM, "ally.near", 0);
    c.obey(grant("ally.near"));
    let paid = named(&c.take_events(), "grant_paid");
    assert_eq!(u(&paid[0]["hoard"]), hoard);
    assert_eq!(c.balance(WYRM, "ally.near"), half);
    c.skip(MINUTE);
    c.poke();
    assert_eq!(
        u(&c.state()["hoard"]["wyrm"]),
        hoard - half,
        "reconciled after the grant"
    );
}

#[test]
fn auto_burn_reaches_ash_in_the_batch_that_registers_it() {
    let (mut c, breath_at) = morning_on(300 * ONE_NEAR, PresetName::Hoarder, true, 600, DAY0 + 9 * HOUR);
    c.obey(Order::SetAutoBurn { bps: 5_000 });
    c.now = breath_at;
    let slice = slices(&c.poke())[0].1.clone();
    let arrived = c.balance(WYRM, DRAGON);
    assert_eq!(c.balance(WYRM, ASH), 0);
    c.skip(30 * SECOND);
    c.gas.clear();
    let evs = c.poke();
    assert_eq!(named(&evs, "observe")[0]["decision"], "rest: slice gap");
    let burnt = named(&evs, "auto_burn");
    assert_eq!(burnt.len(), 1);
    let bought = rules::expected_net(u(&slice["quoted_out"]));
    assert_eq!(u(&burnt[0]["amount"]), bought / 2, "half of what the buy delivered");
    assert_eq!(
        c.balance(WYRM, ASH),
        bought / 2,
        "ash was registered and paid in one batch"
    );
    for (method, used, given) in &c.gas {
        assert!(used <= given, "{method} used {used} of {given} Ggas");
    }
    let h = c.state()["hoard"].clone();
    assert_eq!((u(&h["ash_due"]), u(&h["auto_burned"])), (0, bought / 2));
    assert_eq!(u(&h["wyrm"]), arrived - bought / 2);
    // A Burn order's cap is 50 bps of what the Dragon owns after the auto-burn.
    let cap = u(&h["wyrm"]) * 50 / 10_000;
    c.obey(Order::Burn { amount: U128(cap + 1) });
    assert!(named(&c.take_events(), "order_refused")[0]["reason"]
        .as_str()
        .unwrap()
        .contains(&format!("({})", u(&h["wyrm"]))));
    c.obey(Order::Burn { amount: U128(cap) });
    assert_eq!(c.balance(WYRM, ASH), bought / 2 + cap);
}

#[test]
fn a_failed_auto_burn_returns_to_ash_due() {
    let (mut c, breath_at) = morning(300 * ONE_NEAR, 600, DAY0 + 9 * HOUR);
    c.obey(Order::SetAutoBurn { bps: 2_000 });
    c.now = breath_at;
    c.poke();
    c.refuse = Some(ASH);
    c.skip(30 * SECOND);
    let evs = c.poke();
    assert_eq!(named(&evs, "outflow_failed")[0]["kind"], "auto_burn");
    let h = c.state()["hoard"].clone();
    let due = u(&h["ash_due"]);
    assert!(due > 0);
    assert_eq!(u(&h["auto_burned"]), 0);
    assert_eq!(u(&h["wyrm"]) + due, c.balance(WYRM, DRAGON));
    c.refuse = None;
    c.skip(10 * SECOND);
    assert_eq!(
        u(&named(&c.poke(), "auto_burn")[0]["amount"]),
        due,
        "the next rest sends it again"
    );
    assert_eq!(c.balance(WYRM, ASH), due);
}

#[test]
fn a_poke_between_an_outflow_and_its_callback_books_nothing_twice() {
    let mut c = Chain::new(50 * ONE_NEAR);
    // A breath at the day's last minute, so no buy moves the hoard meanwhile.
    c.seed = breath_minute(1_439);
    let hoard = 1_000_000 * T;
    c.mint(WYRM, DRAGON, hoard);
    c.transfer_in(WYRM, hoard);
    c.mint(WYRM, "ally.near", 0);
    let half = hoard / 200;
    c.hold_outflows = true;
    c.obey(Order::Grant {
        to: "ally.near".parse().unwrap(),
        amount: U128(half),
    });
    assert_eq!(
        c.balance(WYRM, "ally.near"),
        half,
        "the transfer landed before its callback"
    );
    c.skip(MINUTE);
    c.poke();
    let h = c.state()["hoard"].clone();
    assert_eq!((u(&h["wyrm"]), u(&h["donated"])), (hoard - half, hoard));
    c.hold_outflows = false;
    c.release_outflows();
    assert_eq!(named(&c.take_events(), "grant_paid").len(), 1);
    c.skip(MINUTE);
    c.poke();
    let h = c.state()["hoard"].clone();
    assert_eq!(
        (u(&h["wyrm"]), u(&h["donated"]), u(&h["granted"])),
        (hoard - half, hoard, half)
    );
    assert!(c.state()["pending"].as_array().unwrap().is_empty());
}

#[test]
fn a_lost_outflow_callback_expires_after_ten_minutes() {
    let mut c = Chain::new(50 * ONE_NEAR);
    // A breath at the day's last minute, so no buy moves the hoard meanwhile.
    c.seed = breath_minute(1_439);
    let hoard = 1_000_000 * T;
    c.mint(WYRM, DRAGON, hoard);
    c.transfer_in(WYRM, hoard);
    let half = hoard / 200;
    c.hold_outflows = true;
    c.refuse = Some(ASH);
    c.obey(Order::Burn { amount: U128(half) });
    assert_eq!(c.state()["pending"].as_array().unwrap().len(), 1);
    c.refuse = None;
    c.skip(PENDING_EXPIRY - SECOND);
    c.poke();
    assert_eq!(c.state()["pending"].as_array().unwrap().len(), 1, "still in flight");
    c.skip(SECOND);
    c.poke();
    let state = c.state();
    assert!(
        state["pending"].as_array().unwrap().is_empty(),
        "dropped after ten minutes"
    );
    // The burn failed, so the reconcile books its amount as donated: the hoard's amount is right again.
    assert_eq!(u(&state["hoard"]["wyrm"]), hoard);
    assert_eq!(u(&state["hoard"]["donated"]), hoard + half);
    c.hold_outflows = false;
    c.release_outflows();
    assert!(
        named(&c.take_events(), "outflow_failed").is_empty(),
        "the late callback changes nothing"
    );
    assert_eq!(u(&c.state()["hoard"]["wyrm"]), hoard);
}

impl Chain {
    fn context_log_has(&mut self, event: &str) {
        let log = serde_json::to_value({
            self.state();
            serde_json::from_str::<Value>(&self.d.log_json(None, None)).unwrap()
        })
        .unwrap();
        assert!(
            log.as_array().unwrap().iter().any(|e| e["event"] == event),
            "{event} in the log ring"
        );
    }
}
