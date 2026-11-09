//! Contract-level tests on the mocked blockchain: callbacks are fed the promise results the peers would return.
use super::*;
use near_sdk::mock::MockAction;
use near_sdk::serde_json;
use near_sdk::test_utils::{get_created_receipts, VMContextBuilder};
use near_sdk::{test_vm_config, testing_env, PromiseResult, RuntimeFeesConfig};
use std::panic::{catch_unwind, AssertUnwindSafe};
use wyrm_types::{CoinArgs, Order, Verdict};

const T0: u64 = 20_400 * DAY + HOUR;
const TPN: u128 = 1_000 * 1_000_000_000_000_000_000;
const HOARD: u128 = 1_000_000 * 1_000_000_000_000_000_000;
const TOKEN: &str = "wyrm.nearly.near";
const POOL_ID: &str = "wrap.near|wyrm.nearly.near|10000";

fn id(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn peers() -> Peers {
    Peers {
        wrap: id("wrap.near"),
        dcl: id("dcl.near"),
        nearly: id("nearly.near"),
        tithe: id("tithe.near"),
        scales: id("scales.near"),
        ash: id("ash.near"),
        creator: id("creator.near"),
    }
}

fn context(pred: &str, now: u64, deposit: u128, results: Vec<PromiseResult>) {
    context_with(pred, now, deposit, results, "");
}

/// The call's raw input, for the methods that read it themselves.
fn context_with(pred: &str, now: u64, deposit: u128, results: Vec<PromiseResult>, input: &str) {
    let mut ctx = VMContextBuilder::new()
        .current_account_id(id("dragon.near"))
        .predecessor_account_id(id(pred))
        .block_timestamp(now)
        .prepaid_gas(Gas::from_tgas(1000))
        .attached_deposit(NearToken::from_yoctonear(deposit))
        .account_balance(NearToken::from_near(100))
        .build();
    ctx.input = input.as_bytes().into();
    testing_env!(
        ctx,
        test_vm_config(),
        RuntimeFeesConfig::test(),
        Default::default(),
        results
    );
}

fn at(pred: &str, now: u64, results: Vec<PromiseResult>) {
    context(pred, now, 0, results);
}

fn unbound() -> Dragon {
    at("deployer.near", T0, vec![]);
    Dragon::new(PresetName::Hoarder, peers())
}

fn ok(json: &str) -> PromiseResult {
    PromiseResult::Successful(json.as_bytes().to_vec())
}

fn coin() -> Coin {
    Coin {
        launch_id: U64(7),
        token: id(TOKEN),
        pool_id: POOL_ID.into(),
        wrap_is_x: true,
    }
}

fn record(creator: &str, step: &str) -> Vec<PromiseResult> {
    vec![
        ok(&format!(
            r#"{{"id":7,"token":"{TOKEN}","creator":"{creator}","pool_id":"{POOL_ID}","quote":"wrap.near","step":"{step}","lpt_id":"x#1"}}"#
        )),
        ok(r#""tithe.near""#),
        ok(r#"{"mode":"creator"}"#),
        ok(r#"{"buy_bps":400,"sell_bps":400,"creator_bps":10000,"burn_bps":0,"holders_bps":0}"#),
    ]
}

fn pool_answer() -> PromiseResult {
    ok(&format!(
        r#"{{"token_x":"wrap.near","token_y":"{TOKEN}","total_x":"1000000000000000000000000000","total_y":"1","state":"Running"}}"#
    ))
}

/// Runs bind's two callbacks with a Done record and a running pool. Returns what each answered.
fn run_bind(d: &mut Dragon, now: u64) -> (bool, bool) {
    at("dragon.near", now, record("dragon.near", "Done"));
    let first = matches!(d.on_launch_record(U64(7)), PromiseOrValue::Promise(_));
    at("dragon.near", now, vec![pool_answer()]);
    (first, d.on_pool_seen(coin()))
}

fn dragon() -> Dragon {
    let mut d = unbound();
    run_bind(&mut d, T0);
    assert_eq!(d.coin, Some(coin()));
    d
}

fn market() -> Vec<PromiseResult> {
    vec![
        ok("\"0\""),
        ok("\"0\""),
        pool_answer(),
        ok(&format!(r#"{{"amount":"{TPN}"}}"#)),
    ]
}

fn panics(f: impl FnOnce()) -> String {
    let err = catch_unwind(AssertUnwindSafe(f)).expect_err("expected a panic");
    err.downcast_ref::<String>().cloned().unwrap_or_default()
}

fn obey(d: &mut Dragon, now: u64, order: Order) -> PromiseOrValue<Verdict> {
    obey_as("scales.near", d, now, order)
}

fn obey_as(pred: &str, d: &mut Dragon, now: u64, order: Order) -> PromiseOrValue<Verdict> {
    let input = serde_json::json!({ "order": order }).to_string();
    context_with(pred, now, 0, vec![], &input);
    d.obey()
}

/// ft_on_transfer from `token` of `amount`.
fn transfer_in(d: &mut Dragon, token: &str, amount: u128) {
    let input = format!(r#"{{"sender_id":"x.near","amount":"{amount}","msg":""}}"#);
    context_with(token, T0, 0, vec![], &input);
    d.ft_on_transfer();
}

/// The log ring as get_log answers it.
fn log(d: &Dragon) -> Vec<serde_json::Value> {
    serde_json::from_str(&d.log_json(None, Some(1_000))).unwrap()
}

fn refused(v: PromiseOrValue<Verdict>) -> String {
    match v {
        PromiseOrValue::Value(Verdict::Refused { reason }) => reason,
        _ => panic!("expected a refusal"),
    }
}

/// Obeys an outflow order and settles its callback with `paid`.
fn outflow(d: &mut Dragon, now: u64, order: Order, paid: bool) -> Verdict {
    assert!(matches!(obey(d, now, order), PromiseOrValue::Promise(_)));
    let result = if paid { ok("") } else { PromiseResult::Failed };
    at("dragon.near", now, vec![result]);
    d.on_outflow(U64(d.outflows - 1))
}

fn grant(amount: u128) -> Order {
    Order::Grant {
        to: id("ally.near"),
        amount: U128(amount),
    }
}

fn donate(d: &mut Dragon, amount: u128) {
    transfer_in(d, TOKEN, amount);
}

/// Every NEAR transfer the last call created, as (receiver, amount).
fn transfers() -> Vec<(String, u128)> {
    get_created_receipts()
        .iter()
        .flat_map(|r| {
            r.actions.iter().filter_map(|a| match a {
                MockAction::Transfer { deposit, .. } => Some((r.receiver_id.to_string(), deposit.as_yoctonear())),
                _ => None,
            })
        })
        .collect()
}

/// Two observations a few minutes apart, so the second one has a reference. Returns the second step's status.
fn observe_twice(d: &mut Dragon, start: u64) -> PromiseOrValue<String> {
    at("keeper.near", start, vec![]);
    let _ = d.poke();
    at("dragon.near", start, market());
    let _ = d.on_observe(U64(d.step));
    at("keeper.near", start + 3 * MINUTE, vec![]);
    let _ = d.poke();
    at("dragon.near", start + 3 * MINUTE, market());
    d.on_observe(U64(d.step))
}

#[test]
fn a_stale_step_is_replaced_after_five_minutes_and_its_callbacks_do_nothing() {
    let mut d = dragon();
    at("keeper.near", T0, vec![]);
    let _ = d.poke();
    assert_eq!(d.phase, Phase::Observing { step: 1, since: T0 });
    at("keeper.near", T0 + STALE - 1, vec![]);
    assert_eq!(panics(|| drop(d.poke())), "in flight");
    at("keeper.near", T0 + STALE, vec![]);
    let _ = d.poke();
    assert_eq!(
        d.phase,
        Phase::Observing {
            step: 2,
            since: T0 + STALE
        }
    );
    at("dragon.near", T0 + STALE, market());
    assert!(matches!(d.on_observe(U64(1)), PromiseOrValue::Value(s) if s == "stale step"));
    assert_eq!(
        d.phase,
        Phase::Observing {
            step: 2,
            since: T0 + STALE
        },
        "the old callback changed nothing"
    );
    assert_eq!(d.samples.len(), 0);
    assert!(matches!(d.on_observe(U64(2)), PromiseOrValue::Value(_)));
    assert_eq!(d.phase, Phase::Idle);
    assert_eq!(d.samples.len(), 1);
}

/// Observes twice, then answers the breath's quotes at spot, so `on_quote` sends the whole slice. Returns the step,
/// the plan and the time of the quotes.
fn send_breath_slice(d: &mut Dragon) -> (u64, Plan, u64) {
    // A zero random seed puts the breath minute at 00:00, so the breath is open.
    assert!(matches!(observe_twice(d, T0), PromiseOrValue::Promise(_)));
    let Phase::Quoting { step, amount_in, .. } = d.phase.clone() else {
        panic!("expected Quoting, got {:?}", d.phase)
    };
    let quotes: Vec<PromiseResult> = rules::candidates(Side::Buy, amount_in, TPN)
        .iter()
        .map(|a| ok(&format!(r#"{{"amount":"{}"}}"#, math::mul_div(*a, TPN, ONE_NEAR))))
        .collect();
    let now = T0 + 3 * MINUTE;
    at("dragon.near", now, quotes);
    let plan = Plan {
        kind: SliceKind::Breath,
        amount_in: U128(amount_in),
        tokens_per_near: U128(TPN),
        reference: Some(U128(TPN)),
        breath_at: U64(d.day.breath_at),
        budget: U128(d.day.budget),
        dip: None,
    };
    let _ = d.on_quote(U64(step), plan.clone());
    (step, plan, now)
}

#[test]
fn a_buy_wraps_and_swaps_in_one_receipt_with_180_tgas_for_the_swap() {
    let mut d = dragon();
    send_breath_slice(&mut d);
    let calls: Vec<(String, String, u64)> = get_created_receipts()
        .iter()
        .flat_map(|r| {
            r.actions.iter().filter_map(|a| match a {
                MockAction::FunctionCallWeight {
                    method_name,
                    prepaid_gas,
                    ..
                } => Some((
                    r.receiver_id.to_string(),
                    String::from_utf8(method_name.clone()).unwrap(),
                    prepaid_gas.as_tgas(),
                )),
                _ => None,
            })
        })
        .collect();
    let want = [
        ("wrap.near", "near_deposit", 5),
        ("wrap.near", "ft_transfer_call", 180),
        ("dragon.near", "on_swap", 10),
    ];
    assert_eq!(
        calls,
        want.map(|(to, m, g)| (to.to_string(), m.to_string(), g)),
        "the gas each call of a buy is given"
    );
}

#[test]
fn a_late_refund_returns_the_day_budget_after_its_step_was_replaced() {
    let mut d = dragon();
    let (step, plan, now) = send_breath_slice(&mut d);
    let amount_in = plan.amount_in.0;
    let Phase::Swapping {
        amount_in: sent,
        quoted_out,
        min_out,
        ..
    } = d.phase.clone()
    else {
        panic!("not swapping")
    };
    assert_eq!(sent, amount_in, "the full slice passes a quote at spot");
    assert_eq!(d.day.spent, sent, "spent grows when the swap is sent");

    at("keeper.near", now + STALE, vec![]);
    let _ = d.poke();
    let ticket = Ticket {
        plan,
        at: U64(now),
        amount_in: U128(sent),
        quoted_out: U128(quoted_out),
        min_out: U128(min_out),
        cap_bps: 200,
        day: U64(d.day.index),
    };
    at("dragon.near", now + STALE + SECOND, vec![ok("\"0\"")]);
    assert_eq!(d.on_swap(U64(step), ticket), "swap_failed");
    assert_eq!(d.day.spent, 0, "the refund gave the budget back");
    assert!(
        matches!(d.phase, Phase::Observing { .. }),
        "the new step keeps its phase"
    );
    assert_eq!(d.hoard.near_spent, 0);
    assert_eq!(log(&d).last().unwrap()["event"], "swap_failed");
}

#[test]
fn the_log_keeps_the_newest_256_events_and_pokes_do_not_enter_it() {
    let mut d = dragon();
    let bound = d.log.next_seq();
    let _ = observe_twice(&mut d, T0);
    assert_eq!(d.log.next_seq(), bound, "observations are events but not log entries");
    for i in 0..300u16 {
        let _ = obey(&mut d, T0 + HOUR + i as u64, Order::SetSlices { cap_bps: 50 + i % 150 });
    }
    let log = log(&d);
    assert_eq!(log.len(), 256);
    assert_eq!(log[0]["seq"], (bound + 44).to_string());
    assert_eq!(log[255]["seq"], (bound + 299).to_string());
    assert_eq!(log[255]["event"], "settings_changed");
    assert_eq!(log[255]["data"]["cap_bps"], 50 + 299 % 150);
    assert!(log[255]["at"].is_string());
    let tail: Vec<serde_json::Value> = serde_json::from_str(&d.log_json(Some(U64(bound + 298)), Some(10))).unwrap();
    assert_eq!(tail.len(), 2);
}

#[test]
fn grants_in_any_30_days_stay_within_one_percent_of_the_hoard() {
    let mut d = dragon();
    donate(&mut d, HOARD);
    let mut paid: Vec<(u64, u128)> = vec![];
    for day in 0..95u64 {
        let now = T0 + day * DAY;
        let window: u128 = paid
            .iter()
            .filter(|(t, _)| t + OUTFLOW_WINDOW > now)
            .map(|(_, a)| a)
            .sum();
        let hoard = d.hoard.wyrm;
        let ask = (hoard * 50 / 10_000).min((hoard * 100 / 10_000).saturating_sub(window));
        if ask == 0 {
            assert!(refused(obey(&mut d, now, grant(hoard * 50 / 10_000))).contains("30 days"));
            continue;
        }
        assert!(refused(obey(&mut d, now, grant(ask + 1))).contains("bps of the hoard"));
        assert_eq!(outflow(&mut d, now, grant(ask), true), Verdict::Executed);
        paid.push((now, ask));
    }
    for &(t, _) in &paid {
        let in_window: u128 = paid
            .iter()
            .filter(|(s, _)| *s >= t && *s < t + OUTFLOW_WINDOW)
            .map(|(_, a)| a)
            .sum();
        assert!(
            in_window * 10_000 <= HOARD * 100,
            "{in_window} paid in the 30 days from {t}"
        );
    }
    // The window frees itself: the first two grants come back into reach after 30 days, not before.
    assert!(paid.iter().any(|(t, _)| *t == T0 + 30 * DAY));
    assert!(!paid.iter().any(|(t, _)| *t > T0 + DAY && *t < T0 + 30 * DAY));
}

#[test]
fn burns_in_any_30_days_stay_within_one_percent_and_apart_from_grants() {
    let mut d = dragon();
    donate(&mut d, HOARD);
    let half = HOARD / 200;
    let burn = |amount| Order::Burn { amount: U128(amount) };
    assert_eq!(outflow(&mut d, T0, grant(half), true), Verdict::Executed);
    let h1 = HOARD - half;
    let b1 = h1 / 200;
    assert!(refused(obey(&mut d, T0, burn(b1 + 1))).contains("a burn is at most 50 bps"));
    assert_eq!(outflow(&mut d, T0, burn(b1), true), Verdict::Executed);
    let h2 = h1 - b1;
    let rest = h2 / 100 - b1;
    assert!(refused(obey(&mut d, T0 + 1, burn(rest + 1))).contains("burns in any 30 days"));
    assert_eq!(outflow(&mut d, T0 + 1, burn(rest), true), Verdict::Executed);
    assert!(refused(obey(&mut d, T0 + 29 * DAY, burn(1))).contains("30 days"));
    let h3 = h2 - rest;
    assert_eq!(outflow(&mut d, T0 + 30 * DAY, burn(h3 / 400), true), Verdict::Executed);
    assert_eq!(d.hoard.burned, b1 + rest + h3 / 400);
    assert_eq!(d.hoard.wyrm, h3 - h3 / 400);
    assert_eq!(d.hoard.granted, half, "the grant window is its own");
    assert!(log(&d)
        .iter()
        .any(|e| e["event"] == "burn_paid" && e["data"]["hoard"] == h1.to_string()));
}

#[test]
fn a_failed_grant_restores_the_hoard_and_frees_its_share() {
    let mut d = dragon();
    donate(&mut d, HOARD);
    let half = HOARD / 200;
    assert!(matches!(obey(&mut d, T0, grant(half)), PromiseOrValue::Promise(_)));
    assert_eq!(d.hoard.wyrm, HOARD - half);
    assert_eq!(d.pending.len(), 1);
    at("dragon.near", T0, vec![PromiseResult::Failed]);
    assert_eq!(
        d.on_outflow(U64(0)),
        Verdict::Refused {
            reason: "transfer failed".into()
        }
    );
    assert_eq!((d.hoard.wyrm, d.hoard.granted, d.pending.len()), (HOARD, 0, 0));
    // Its share is free again: after another full grant, the window still has room for the rest of 1%.
    assert_eq!(outflow(&mut d, T0 + 1, grant(half), true), Verdict::Executed);
    let rest = (HOARD - half) / 100 - half;
    assert!(refused(obey(&mut d, T0 + 2, grant(rest + 1))).contains("30 days"));
    assert_eq!(outflow(&mut d, T0 + 2, grant(rest), true), Verdict::Executed);
}

/// A poke whose coin balance view answers `wyrm`.
fn observe_balance(d: &mut Dragon, now: u64, wyrm: u128) {
    at("keeper.near", now, vec![]);
    let _ = d.poke();
    let mut answers = market();
    answers[0] = ok(&format!("\"{wyrm}\""));
    at("dragon.near", now, answers);
    let _ = d.on_observe(U64(d.step));
}

#[test]
fn a_poke_that_reads_the_balance_before_a_grant_lands_books_no_donation() {
    let mut d = dragon();
    donate(&mut d, HOARD);
    let half = HOARD / 200;
    assert!(matches!(obey(&mut d, T0, grant(half)), PromiseOrValue::Promise(_)));
    // The poke's balance view runs before the grant's ft_transfer, so it still sees the whole hoard.
    observe_balance(&mut d, T0 + MINUTE, HOARD);
    assert_eq!(
        (d.hoard.wyrm, d.hoard.donated),
        (HOARD - half, HOARD),
        "the grant in flight is not booked back as a donation"
    );
    at("dragon.near", T0 + MINUTE, vec![ok("")]);
    assert_eq!(d.on_outflow(U64(0)), Verdict::Executed);
    observe_balance(&mut d, T0 + MINUTE + STALE, HOARD - half);
    assert_eq!(
        (d.hoard.wyrm, d.hoard.donated, d.hoard.granted),
        (HOARD - half, HOARD, half)
    );
}

#[test]
fn a_fourth_outflow_is_refused_while_three_are_in_flight() {
    let mut d = dragon();
    donate(&mut d, HOARD);
    let small = HOARD / 1_000;
    for i in 0..3 {
        assert!(matches!(obey(&mut d, T0 + i, grant(small)), PromiseOrValue::Promise(_)));
    }
    let burn = Order::Burn { amount: U128(small) };
    assert_eq!(
        refused(obey(&mut d, T0 + 3, burn.clone())),
        "3 outflows are already in flight"
    );
    assert_eq!((d.pending.len(), d.hoard.burned), (3, 0), "the refusal sent nothing");
    at("dragon.near", T0 + 4, vec![ok("")]);
    assert_eq!(d.on_outflow(U64(1)), Verdict::Executed);
    assert!(
        matches!(obey(&mut d, T0 + 5, burn), PromiseOrValue::Promise(_)),
        "a settled outflow makes room"
    );
    assert_eq!((d.pending.len(), d.hoard.burned), (3, small));
}

#[test]
fn only_scales_makes_the_dragon_obey_and_unknown_tokens_are_refused() {
    let mut d = dragon();
    assert!(
        panics(|| drop(obey_as("stranger.near", &mut d, T0, Order::SetSlices { cap_bps: 100 })))
            .contains("only scales")
    );
    assert!(panics(|| drop(obey_as("creator.near", &mut d, T0, grant(1)))).contains("only scales"));
    context_with("scales.near", T0, 0, vec![], r#"{"order":{"Pause":{"hours":1}}}"#);
    assert_eq!(refused(d.obey()), "the order did not parse");
    let v = obey(&mut d, T0, Order::SetAutoBurn { bps: 6_000 });
    assert_eq!(refused(v), "auto_burn_bps must be 0 to 5000, got 6000");
    assert_eq!(
        d.strategy,
        wyrm_types::preset(PresetName::Hoarder).strategy,
        "a refusal changes nothing"
    );
    let refusals: Vec<_> = log(&d).into_iter().filter(|e| e["event"] == "order_refused").collect();
    assert!(refusals.last().unwrap()["data"]["reason"]
        .as_str()
        .unwrap()
        .contains("auto_burn_bps"));
    assert!(panics(|| transfer_in(&mut d, "fake-wyrm.near", 5)).contains("only its bound coin"));
    transfer_in(&mut d, "wrap.near", 5);
    assert_eq!(d.wnear, 5);
}

#[test]
fn ash_due_is_outside_every_cap() {
    let mut d = dragon();
    donate(&mut d, HOARD);
    d.hoard.ash_due = 9 * HOARD;
    let half = HOARD / 200;
    assert!(refused(obey(&mut d, T0, grant(half + 1))).contains("50 bps of the hoard (1000000"));
    assert!(refused(obey(&mut d, T0, Order::Burn { amount: U128(half + 1) })).contains("50 bps"));
    let h = Hoard {
        wyrm: 10 * TPN,
        wyrm_bought: 10 * TPN,
        near_spent: 10 * ONE_NEAR,
        ash_due: 1_000 * TPN,
        ..Hoard::default()
    };
    let day = Day::default();
    assert_eq!(
        rules::sell_room(1_000, &day, &h),
        TPN,
        "Hunt sells a tenth of what the Dragon owns, not of ash's"
    );
}

#[test]
fn the_mood_starts_sleeping_and_breathes_once_the_breath_opens() {
    let mut d = dragon();
    at("keeper.near", T0, vec![]);
    assert!(matches!(d.get_state().mood, rules::Mood::Sleeping));
    let _ = observe_twice(&mut d, T0);
    at("keeper.near", T0 + 3 * MINUTE, vec![]);
    assert!(
        matches!(d.get_state().mood, rules::Mood::Breathing),
        "the zero seed opens the breath at 00:00"
    );
}

fn wiring(vaults: u8, preset_hash: &str) -> Vec<PromiseResult> {
    vec![
        ok(r#"{"dragon":"dragon.near","scales":"scales.near","wrap":"wrap.near","min_round":"1","reserve":"1"}"#),
        ok(&format!(
            r#"{{"preset":"hoarder","preset_hash":"{preset_hash}","engine_version":200,"dragon":"dragon.near","tithe":"tithe.near","creator":"creator.near","vaults_created":{vaults}}}"#
        )),
        ok(r#"{"total":"95050000000000000000000"}"#),
        ok("0"),
    ]
}

fn hoarder_hash() -> String {
    String::from(&near_sdk::json_types::Base58CryptoHash::from(
        wyrm_types::preset(PresetName::Hoarder).hash(),
    ))
}

fn wyrm_coin() -> CoinArgs {
    CoinArgs {
        name: "WYRM".into(),
        symbol: "WYRM".into(),
        icon: None,
        description: String::new(),
        links: Default::default(),
        dev_buy: None,
    }
}

const DEPOSIT: u128 = 10 * ONE_NEAR;

fn launch_args() -> Vec<u8> {
    nearly::launch_args(&wyrm_coin(), &id("tithe.near"))
}

#[test]
fn launch_refuses_bad_wiring_and_returns_the_whole_deposit() {
    let cost = 95_050_000_000_000_000_000_000 + nearly::LAUNCH_MARGIN.as_yoctonear();
    let cases: [(Vec<PromiseResult>, u128, &str); 5] = [
        (wiring(99, &hoarder_hash()), DEPOSIT, "99 of 100 vaults"),
        (
            wiring(100, "11111111111111111111111111111111"),
            DEPOSIT,
            "another preset",
        ),
        (
            vec![
                PromiseResult::Failed,
                wiring(100, &hoarder_hash()).remove(1),
                ok("{}"),
                ok("0"),
            ],
            DEPOSIT,
            "tithe.get_config did not answer",
        ),
        (wiring(100, &hoarder_hash()), cost - 1, "attach at least"),
        (
            vec![
                ok(r#"{"dragon":"other.near","scales":"scales.near","wrap":"wrap.near"}"#),
                ok("{}"),
                ok("{}"),
                ok("0"),
            ],
            DEPOSIT,
            "the tithe is not wired",
        ),
    ];
    for (results, deposit, why) in cases {
        let mut d = unbound();
        context("dragon.near", T0, 0, results);
        assert!(matches!(
            d.on_wiring("WYRM".into(), launch_args(), U128(deposit)),
            PromiseOrValue::Value(false)
        ));
        assert_eq!(transfers(), vec![("creator.near".into(), deposit)], "{why}");
        let last = log(&d).pop().unwrap();
        assert_eq!(last["event"], "launch_refused");
        assert!(last["data"]["reason"].as_str().unwrap().contains(why), "{why}: {last}");
    }
    let mut d = unbound();
    context("dragon.near", T0, 0, wiring(100, &hoarder_hash()));
    assert!(matches!(
        d.on_wiring("WYRM".into(), launch_args(), U128(DEPOSIT)),
        PromiseOrValue::Promise(_)
    ));
    assert!(transfers().is_empty(), "good wiring sends nothing yet");

    // The pool check: a pool at the predicted id refuses with the whole deposit; none sends the quote and margin.
    at("dragon.near", T0, vec![pool_answer()]);
    assert!(matches!(
        d.on_pool_checked(launch_args(), U128(DEPOSIT), U128(cost)),
        PromiseOrValue::Value(false)
    ));
    assert_eq!(transfers(), vec![("creator.near".into(), DEPOSIT)]);
    at("dragon.near", T0, vec![ok("null")]);
    assert!(matches!(
        d.on_pool_checked(launch_args(), U128(DEPOSIT), U128(cost)),
        PromiseOrValue::Promise(_)
    ));
    let receipts = get_created_receipts();
    let sent: Vec<(String, String, u128)> = receipts
        .iter()
        .flat_map(|r| {
            r.actions.iter().filter_map(|a| match a {
                MockAction::Transfer { deposit, .. } => {
                    Some((r.receiver_id.to_string(), "transfer".into(), deposit.as_yoctonear()))
                }
                MockAction::FunctionCallWeight {
                    method_name,
                    attached_deposit,
                    ..
                } => Some((
                    r.receiver_id.to_string(),
                    String::from_utf8(method_name.clone()).unwrap(),
                    attached_deposit.as_yoctonear(),
                )),
                _ => None,
            })
        })
        .collect();
    assert!(sent.contains(&("creator.near".into(), "transfer".into(), DEPOSIT - cost)));
    assert!(sent.contains(&("nearly.near".into(), "launch".into(), cost)));
    assert_eq!(DEPOSIT - cost + cost, DEPOSIT);
}

#[test]
fn launch_waits_ten_minutes_and_never_runs_once_bound() {
    let mut d = unbound();
    context("creator.near", T0, DEPOSIT, vec![]);
    let _ = d.launch(wyrm_coin());
    assert_eq!(d.last_launch_at, T0);
    context("creator.near", T0 + LAUNCH_COOLDOWN - 1, DEPOSIT, vec![]);
    assert!(panics(|| drop(d.launch(wyrm_coin()))).contains("10 minutes apart"));
    context("creator.near", T0 + LAUNCH_COOLDOWN, DEPOSIT, vec![]);
    let _ = d.launch(wyrm_coin());
    let bad = CoinArgs {
        symbol: "W-Y".into(),
        ..wyrm_coin()
    };
    context("creator.near", T0 + 3 * LAUNCH_COOLDOWN, DEPOSIT, vec![]);
    assert!(panics(|| drop(d.launch(bad))).contains("symbol"));
    run_bind(&mut d, T0 + 3 * LAUNCH_COOLDOWN);
    context("creator.near", T0 + DAY, DEPOSIT, vec![]);
    assert!(panics(|| drop(d.launch(wyrm_coin()))).contains("already bound"));
}

#[test]
fn launch_and_bind_refuse_a_stranger() {
    let mut d = unbound();
    for who in ["stranger.near", "scales.near", "dragon.near"] {
        context(who, T0, DEPOSIT, vec![]);
        assert!(
            panics(|| drop(d.launch(wyrm_coin()))).contains("only the creator"),
            "{who}"
        );
        at(who, T0, vec![]);
        assert!(panics(|| drop(d.bind(U64(7)))).contains("only the creator"), "{who}");
        assert!(
            panics(|| drop(d.nearly_cancel_stuck(U64(7)))).contains("only the creator"),
            "{who}"
        );
    }
    at("creator.near", T0, vec![]);
    let _ = d.bind(U64(7));
    assert_eq!(d.coin, None, "bind writes no state");
}

#[test]
fn poke_refuses_until_bound() {
    let mut d = unbound();
    at("keeper.near", T0, vec![]);
    assert!(panics(|| drop(d.poke())).contains("not bound"));
    assert!(refused(obey(&mut d, T0, grant(1))).contains("not bound"));
    run_bind(&mut d, T0);
    at("keeper.near", T0, vec![]);
    let _ = d.poke();
    assert!(matches!(d.phase, Phase::Observing { .. }));
}

#[test]
fn ft_on_transfer_takes_only_wnear_before_bind() {
    let mut d = unbound();
    assert!(panics(|| transfer_in(&mut d, TOKEN, 5)).contains("only its bound coin"));
    transfer_in(&mut d, "wrap.near", 5);
    run_bind(&mut d, T0);
    donate(&mut d, 7);
    assert_eq!((d.wnear, d.hoard.wyrm, d.hoard.donated), (5, 7, 7));
}

#[test]
fn two_concurrent_binds_bind_once_and_bind_refuses_until_done() {
    let mut d = unbound();
    at("dragon.near", T0, record("dragon.near", "AddLiquidity"));
    assert!(matches!(d.on_launch_record(U64(7)), PromiseOrValue::Value(false)));
    let last = log(&d).pop().unwrap();
    assert_eq!(
        (last["event"].clone(), last["data"]["reason"].clone()),
        ("bind_refused".into(), "not done".into())
    );
    at("dragon.near", T0, record("creator.near", "Done"));
    assert!(matches!(d.on_launch_record(U64(7)), PromiseOrValue::Value(false)));
    assert_eq!(d.coin, None);
    // Both binds' records pass before either pool view answers; only the first pool answer binds.
    at("dragon.near", T0, record("dragon.near", "Done"));
    assert!(matches!(d.on_launch_record(U64(7)), PromiseOrValue::Promise(_)));
    at("dragon.near", T0, record("dragon.near", "Done"));
    assert!(matches!(d.on_launch_record(U64(7)), PromiseOrValue::Promise(_)));
    at("dragon.near", T0, vec![pool_answer()]);
    assert!(d.on_pool_seen(coin()));
    let kicks = get_created_receipts()
        .iter()
        .filter(|r| r.receiver_id.as_str() == "scales.near")
        .count();
    assert_eq!(kicks, 1, "the first bind kicks open_sale");
    at("dragon.near", T0, vec![pool_answer()]);
    assert!(!d.on_pool_seen(coin()));
    assert!(get_created_receipts().is_empty(), "the second does nothing");
    at("dragon.near", T0, record("dragon.near", "Done"));
    assert!(matches!(d.on_launch_record(U64(7)), PromiseOrValue::Value(false)));
    let bound = log(&d).iter().filter(|e| e["event"] == "bound").count();
    assert_eq!(bound, 1);
    assert_eq!(d.get_coin(), Some(coin()));
}

#[test]
fn after_bind_only_an_orphan_can_be_cancelled() {
    let mut d = unbound();
    at("creator.near", T0, vec![]);
    let _ = d.nearly_cancel_stuck(U64(7));
    run_bind(&mut d, T0);
    at("creator.near", T0, vec![]);
    assert!(panics(|| drop(d.nearly_cancel_stuck(U64(7)))).contains("bound launch"));
    let _ = d.nearly_cancel_stuck(U64(8));
    let calls: Vec<String> = get_created_receipts()
        .iter()
        .filter(|r| r.receiver_id.as_str() == "nearly.near")
        .flat_map(|r| {
            r.actions.iter().filter_map(|a| match a {
                MockAction::FunctionCallWeight { method_name, args, .. } => Some(format!(
                    "{} {}",
                    String::from_utf8(method_name.clone()).unwrap(),
                    String::from_utf8(args.clone()).unwrap()
                )),
                _ => None,
            })
        })
        .collect();
    assert_eq!(calls, vec![r#"cancel_stuck_launch {"launch_id":"8"}"#.to_string()]);
}

#[test]
fn nearly_resume_refuses_once_bound_and_below_300_tgas() {
    let mut d = unbound();
    at("anyone.near", T0, vec![]);
    let _ = d.nearly_resume(U64(7));
    let ctx = VMContextBuilder::new()
        .current_account_id(id("dragon.near"))
        .predecessor_account_id(id("anyone.near"))
        .prepaid_gas(Gas::from_tgas(299))
        .build();
    testing_env!(ctx);
    assert!(panics(|| drop(d.nearly_resume(U64(7)))).contains("300 Tgas"));
    run_bind(&mut d, T0);
    at("anyone.near", T0, vec![]);
    assert!(panics(|| drop(d.nearly_resume(U64(7)))).contains("already bound"));
}
