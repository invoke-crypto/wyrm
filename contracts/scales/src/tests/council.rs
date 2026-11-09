use super::*;
use crate::council::{Proposal, BOND, FINALIZE_GAS, OBEY_GAS};
use near_sdk::PromiseOrValue;
use wyrm_types::{HuntLine, Order, Verdict, VoteClass};

fn tune() -> ProposalKind {
    ProposalKind::Order(Order::SetSlices { cap_bps: 100 })
}

fn grant() -> ProposalKind {
    ProposalKind::Order(Order::Grant {
        to: a("artist"),
        amount: U128(1_000),
    })
}

fn hunt_on() -> ProposalKind {
    ProposalKind::Order(Order::SetPosture {
        hunt: Some(HuntLine {
            sell_multiple_bps: 20_000,
            max_sell_bps: 500,
        }),
    })
}

fn propose(w: &mut W, who: &str, kind: ProposalKind, token: &str) -> u64 {
    w.as_(who, BOND.saturating_add(STORAGE_FEE))
        .propose(kind, "why".into(), token.into())
}

/// Each listed Scale votes through its own owner `v{i}`.
fn votes(w: &mut W, p: u64, ids: impl IntoIterator<Item = u8>, approve: bool) {
    for i in ids {
        w.as_(&format!("v{i}"), YOCTO).vote(p, vec![i.to_string()], approve);
    }
}

fn status(w: &mut W, p: u64) -> Status {
    w.view().get_proposal(p).unwrap().status
}

/// Finalizes as anyone. A passing Order answers with its call to the Dragon, so it is Executing.
fn finalize(w: &mut W, p: u64) -> Status {
    match w.as_("anyone", ZERO).finalize(p) {
        PromiseOrValue::Value(status) => status,
        PromiseOrValue::Promise(_) => Status::Executing,
    }
}

fn ended(w: &mut W, p: u64) {
    let ends = w.view().get_proposal(p).unwrap().ends_at.0;
    w.now = w.now.max(ends);
}

fn verdict(v: Verdict) -> PromiseResult {
    PromiseResult::Successful(near_sdk::serde_json::to_vec(&v).unwrap())
}

fn leash(error: &str) -> Status {
    Status::LeashHeld { error: error.into() }
}

/// A Grant from `v1` that 30 of 45 Scales passed, finalized and calling the Dragon.
fn passed_grant() -> (W, u64) {
    let mut w = W::council(45, true);
    let p = propose(&mut w, "v1", grant(), "1");
    votes(&mut w, p, 1..=30, true);
    ended(&mut w, p);
    finalize(&mut w, p);
    (w, p)
}

#[test]
fn thresholds_follow_the_integer_rules() {
    let cases = [
        (VoteClass::Light, 10, 2, 1, true),
        (VoteClass::Light, 10, 1, 1, false),
        (VoteClass::Light, 10, 2, 2, false),
        (VoteClass::Light, 9, 9, 0, false),
        (VoteClass::Light, 90, 14, 13, true),
        (VoteClass::Light, 90, 14, 12, false),
        (VoteClass::Strict, 45, 24, 6, true),
        (VoteClass::Strict, 45, 23, 7, false),
        (VoteClass::Strict, 45, 29, 0, false),
        (VoteClass::Strict, 44, 44, 0, false),
        (VoteClass::Strict, 90, 48, 12, true),
        (VoteClass::Strict, 90, 47, 12, false),
    ];
    let proposal = |class, yes: u32, no: u32, electorate| {
        let mut p = Proposal {
            id: 0,
            kind: ProposalKind::Signal,
            class,
            proposer: a("v1"),
            description: String::new(),
            bond: ZERO,
            created_at: 0,
            ends_at: 0,
            electorate,
            yes: ScaleSet::default(),
            no: ScaleSet::default(),
            status: Status::Open,
        };
        (1..=yes).for_each(|i| p.yes.insert(ScaleId::new(i as u8).unwrap()));
        (yes + 1..=yes + no).for_each(|i| p.no.insert(ScaleId::new(i as u8).unwrap()));
        p
    };
    for (class, electorate, yes, no, expected) in cases {
        assert_eq!(
            proposal(class, yes, no, electorate).passes(),
            expected,
            "{class:?} {yes} yes {no} no of {electorate}"
        );
    }
    for electorate in 1..=90u32 {
        for yes in 0..=electorate {
            for no in 0..=electorate - yes {
                let (y, n, e) = (yes as u64, no as u64, electorate as u64);
                let light = e >= 10 && (y + n) * 10 >= e * 3 && y > n;
                let strict = e >= 45 && (y + n) * 3 >= e * 2 && y * 5 >= (y + n) * 4;
                assert_eq!(proposal(VoteClass::Light, yes, no, electorate).passes(), light);
                assert_eq!(proposal(VoteClass::Strict, yes, no, electorate).passes(), strict);
            }
        }
    }
}

#[test]
fn a_tune_passes_at_exactly_30_percent_turnout_and_returns_the_bond() {
    let mut w = W::council(10, false);
    let p = propose(&mut w, "v1", tune(), "1");
    votes(&mut w, p, [1, 2], true);
    votes(&mut w, p, [3], false);
    ended(&mut w, p);
    assert_eq!(finalize(&mut w, p), Status::Executing);
    assert_eq!(transfers(), vec![(a("v1"), BOND)]);
    assert_eq!(w.view().get_proposal(p).unwrap().bond, ZERO);
    assert_eq!(w.c.escrow, ZERO);
}

#[test]
fn a_tune_at_20_percent_turnout_fails_and_the_bond_goes_to_the_dragon() {
    let mut w = W::council(10, false);
    let p = propose(&mut w, "v1", tune(), "1");
    votes(&mut w, p, [1, 2], true);
    ended(&mut w, p);
    assert_eq!(finalize(&mut w, p), Status::Rejected);
    assert_eq!(transfers(), vec![(a("dragon"), BOND)]);
}

#[test]
fn a_tune_needs_an_electorate_of_10() {
    let mut w = W::council(9, false);
    refuses("electorate is too small", || propose(&mut w, "v1", tune(), "1"));
}

#[test]
fn a_grant_passes_at_two_thirds_turnout_and_80_percent_yes() {
    let (mut w, p) = passed_grant();
    assert_eq!(status(&mut w, p), Status::Executing);

    let mut w = W::council(45, true);
    let ps: Vec<u64> = ["v1", "v2", "v3"]
        .iter()
        .zip(["1", "2", "3"])
        .map(|(v, t)| propose(&mut w, v, grant(), t))
        .collect();
    votes(&mut w, ps[0], 1..=24, true);
    votes(&mut w, ps[0], 25..=30, false);
    votes(&mut w, ps[1], 1..=23, true);
    votes(&mut w, ps[1], 24..=30, false);
    votes(&mut w, ps[2], 1..=29, true);
    ended(&mut w, ps[2]);
    assert_eq!(finalize(&mut w, ps[0]), Status::Executing, "24 of 30 is 80%");
    assert_eq!(finalize(&mut w, ps[1]), Status::Rejected, "23 of 30 is under 80%");
    assert_eq!(
        finalize(&mut w, ps[2]),
        Status::Rejected,
        "29 of 45 is under two thirds"
    );
}

#[test]
fn a_grant_opens_only_after_the_sale_with_an_electorate_of_45() {
    let mut w = W::council(45, false);
    refuses("strict proposals open after the sale closes", || {
        propose(&mut w, "v1", grant(), "1")
    });
    let mut w = W::council(44, true);
    refuses("electorate is too small", || propose(&mut w, "v1", grant(), "1"));
}

#[test]
fn the_electorate_counts_unseasoned_scales() {
    let mut w = W::council(10, false);
    w.buy_scale("late");
    let p = propose(&mut w, "v1", tune(), "1");
    let view = w.view().get_proposal(p).unwrap();
    assert_eq!(view.electorate, 11);
    assert_eq!(
        view.ends_at.0,
        view.created_at.0 + 2 * DAY,
        "every vote lasts the preset's 2 days"
    );
    assert_eq!(w.view().eligible_now(), 11);
    refuses("not eligible on this proposal", || {
        w.as_("late", YOCTO).vote(p, vec!["11".into()], true)
    });
}

#[test]
fn a_scale_counts_once_however_often_it_changes_hands() {
    let mut w = W::council(10, false);
    let p = propose(&mut w, "v1", tune(), "1");
    votes(&mut w, p, [2], true);
    refuses("already voted", || w.as_("v2", YOCTO).vote(p, vec!["2".into()], false));
    w.as_("v2", YOCTO).nft_transfer(a("v3"), "2".into(), None, None);
    refuses("not eligible on this proposal", || {
        w.as_("v3", YOCTO).vote(p, vec!["2".into()], true)
    });
    w.as_("v3", YOCTO).nft_transfer(a("v2"), "2".into(), None, None);
    refuses("not eligible on this proposal", || {
        w.as_("v2", YOCTO).vote(p, vec!["2".into()], true)
    });
    let view = w.view().get_proposal(p).unwrap();
    assert_eq!((view.yes, view.no), (1, 0));
    assert_eq!(w.view().get_votes(p).len(), 1);
}

#[test]
fn creator_scales_never_vote_or_propose() {
    let mut w = W::council(10, false);
    w.as_("creator", YOCTO).nft_transfer(a("v1"), "91".into(), None, None);
    w.advance(SEASON);
    refuses("not eligible to vote", || propose(&mut w, "v1", tune(), "91"));
    let p = propose(&mut w, "v1", tune(), "1");
    assert_eq!(w.view().get_proposal(p).unwrap().electorate, 10);
    refuses("not eligible on this proposal", || {
        w.as_("v1", YOCTO).vote(p, vec!["91".into()], true)
    });
    refuses("not eligible on this proposal", || {
        w.as_("creator", YOCTO).vote(p, vec!["92".into()], true)
    });
    assert_eq!(w.view().eligible_now(), 10);
}

#[test]
fn nothing_finalizes_early() {
    let mut w = W::council(10, false);
    let p = propose(&mut w, "v1", tune(), "1");
    votes(&mut w, p, [1, 2, 3], true);
    w.now = w.view().get_proposal(p).unwrap().ends_at.0 - 1;
    refuses("voting still open", || w.as_("anyone", ZERO).finalize(p));
    votes(&mut w, p, [4], true);
    w.advance(1);
    refuses("voting is closed", || votes(&mut w, p, [5], true));
    assert_eq!(finalize(&mut w, p), Status::Executing);
}

#[test]
fn a_second_finalize_pays_nothing() {
    let mut w = W::council(10, false);
    let p = propose(&mut w, "v1", tune(), "1");
    votes(&mut w, p, [1, 2, 3], true);
    ended(&mut w, p);
    finalize(&mut w, p);
    assert_eq!(transfers().len(), 1);
    refuses("not open", || w.as_("anyone", ZERO).finalize(p));
}

#[test]
fn finalize_calls_the_dragon_once_and_holds_it_while_executing() {
    let (mut w, p) = passed_grant();
    let c = calls();
    assert_eq!((c[0].0.clone(), c[0].1.as_str()), (a("dragon"), "obey"));
    assert_eq!(
        c[0].2,
        json!({ "order": { "Grant": { "to": "artist.near", "amount": "1000" } } })
    );
    assert_eq!(c[0].3, OBEY_GAS);
    assert_eq!((c[1].0.clone(), c[1].1.as_str()), (me(), "on_executed"));
    assert_eq!(c.len(), 2);
    assert_eq!(status(&mut w, p), Status::Executing);
    refuses("not open", || w.as_("anyone", ZERO).finalize(p));
}

#[test]
fn the_callback_records_executed_or_leash_held() {
    let outcomes = [
        (verdict(Verdict::Executed), Status::Executed),
        (
            verdict(Verdict::Refused {
                reason: "grants in 30 days would pass 100 bps".into(),
            }),
            leash("grants in 30 days would pass 100 bps"),
        ),
        (PromiseResult::Failed, leash("dragon call failed")),
        (PromiseResult::Successful(b"true".to_vec()), leash("dragon call failed")),
    ];
    for (result, expected) in outcomes {
        let (mut w, p) = passed_grant();
        assert_eq!(w.callback(vec![result]).on_executed(p), expected);
        assert_eq!(events("proposal_executed").len(), 1);
        assert_eq!(status(&mut w, p), expected);
        refuses("not open", || w.as_("anyone", ZERO).finalize(p));
    }
}

#[test]
fn a_verdict_decodes_from_its_own_json() {
    let reasons = [
        "",
        "cap_bps must be 50 to 200, got 250",
        r#"a "quoted" \ reason}}"#,
        "ünïcode\n",
    ];
    let verdicts = reasons.map(|r| Verdict::Refused { reason: r.into() });
    for v in [Verdict::Executed].iter().chain(verdicts.iter()) {
        let json = near_sdk::serde_json::to_vec(v).unwrap();
        assert_eq!(
            crate::council::decode_verdict(&json).as_ref(),
            Some(v),
            "{}",
            String::from_utf8_lossy(&json)
        );
    }
    for junk in [
        &b"true"[..],
        b"",
        b"\"Refused\"",
        b"{\"Refused\":{}}",
        b"{\"Executed\":null}",
    ] {
        assert_eq!(crate::council::decode_verdict(junk), None);
    }
}

#[test]
fn every_order_calls_obey_with_itself() {
    let orders = [
        Order::SetBreath { budget_bps: 3_000 },
        Order::SetSlices { cap_bps: 50 },
        Order::SetPosture {
            hunt: Some(HuntLine {
                sell_multiple_bps: 25_000,
                max_sell_bps: 1_000,
            }),
        },
        Order::SetPosture { hunt: None },
        Order::Grant {
            to: a("artist"),
            amount: U128(1_000),
        },
        Order::Burn { amount: U128(7) },
        Order::SetAutoBurn { bps: 5_000 },
    ];
    let mut w = W::council(45, true);
    for (i, order) in orders.iter().enumerate() {
        let who = format!("v{}", i + 1);
        let p = propose(&mut w, &who, ProposalKind::Order(order.clone()), &(i + 1).to_string());
        votes(&mut w, p, 1..=45, true);
        ended(&mut w, p);
        assert_eq!(finalize(&mut w, p), Status::Executing);
        let c = calls();
        assert_eq!((c[0].0.clone(), c[0].1.as_str()), (a("dragon"), "obey"));
        assert_eq!(c[0].2, json!({ "order": order }));
    }
}

#[test]
fn propose_takes_the_bond_and_fee_and_refunds_the_rest() {
    let mut w = W::council(10, false);
    refuses("attach the 1 NEAR bond plus the storage fee", || {
        w.as_("v1", BOND).propose(tune(), "x".into(), "1".into())
    });
    w.as_("v1", near(3)).propose(tune(), "x".into(), "1".into());
    assert_eq!(transfers(), vec![(a("v1"), near(2).saturating_sub(STORAGE_FEE))]);
    assert_eq!(w.c.escrow, BOND);
}

#[test]
fn one_open_proposal_per_proposer() {
    let mut w = W::council(10, false);
    let p = propose(&mut w, "v1", tune(), "1");
    refuses("already have an open proposal", || propose(&mut w, "v1", tune(), "1"));
    ended(&mut w, p);
    finalize(&mut w, p);
    propose(&mut w, "v1", tune(), "1");
}

#[test]
fn a_description_over_1000_bytes_is_refused() {
    let mut w = W::council(10, false);
    let due = BOND.saturating_add(STORAGE_FEE);
    refuses("over 1000 bytes", || {
        w.as_("v1", due).propose(tune(), "é".repeat(501), "1".into())
    });
    w.as_("v1", due).propose(tune(), "x".repeat(1_000), "1".into());
}

#[test]
fn proposing_and_voting_need_your_own_seasoned_scale() {
    let mut w = W::council(10, false);
    refuses("not the owner", || propose(&mut w, "v1", tune(), "2"));
    w.as_("v2", YOCTO).nft_transfer(a("v1"), "2".into(), None, None);
    refuses("not eligible to vote", || propose(&mut w, "v1", tune(), "2"));
    let p = propose(&mut w, "v1", tune(), "1");
    refuses("not the owner", || w.as_("v1", YOCTO).vote(p, vec!["3".into()], true));
    refuses("Requires attached deposit of exactly 1 yoctoNEAR", || {
        w.as_("v1", ZERO).vote(p, vec!["1".into()], true)
    });
    refuses("vote with 1 to 25 Scales", || w.as_("v1", YOCTO).vote(p, vec![], true));
    let many: Vec<String> = (1..=26).map(|i| i.to_string()).collect();
    refuses("vote with 1 to 25 Scales", || w.as_("v1", YOCTO).vote(p, many, true));
}

#[test]
fn a_signal_carries_and_executes_nothing() {
    let mut w = W::council(10, false);
    let carried = propose(&mut w, "v1", ProposalKind::Signal, "1");
    let rejected = propose(&mut w, "v2", ProposalKind::Signal, "2");
    votes(&mut w, carried, [1, 2, 3], true);
    votes(&mut w, rejected, [1, 2], true);
    votes(&mut w, rejected, [3], false);
    votes(&mut w, rejected, [4], false);
    ended(&mut w, rejected);
    assert!(matches!(
        w.as_("anyone", ZERO).finalize(carried),
        PromiseOrValue::Value(Status::Carried)
    ));
    assert!(calls().is_empty(), "a Signal calls nothing");
    assert_eq!(transfers(), vec![(a("v1"), BOND)]);
    assert_eq!(finalize(&mut w, rejected), Status::Rejected);
    assert_eq!(w.view().get_proposal(carried).unwrap().class, VoteClass::Light);
}

#[test]
fn hunt_on_is_strict_and_hunt_off_is_light() {
    let hunt_off = ProposalKind::Order(Order::SetPosture { hunt: None });
    assert_eq!(
        (hunt_on().class(), hunt_off.class()),
        (VoteClass::Strict, VoteClass::Light)
    );
    let mut w = W::council(10, false);
    refuses("electorate is too small", || propose(&mut w, "v1", hunt_on(), "1"));
    let p = propose(&mut w, "v1", hunt_off, "1");
    assert_eq!(w.view().get_proposal(p).unwrap().class, VoteClass::Light);
}

#[test]
fn burn_and_auto_burn_are_light() {
    let burn = ProposalKind::Order(Order::Burn { amount: U128(1) });
    let auto_burn = ProposalKind::Order(Order::SetAutoBurn { bps: 1_000 });
    let mut w = W::council(10, false);
    let p = propose(&mut w, "v1", burn, "1");
    let q = propose(&mut w, "v2", auto_burn, "2");
    assert_eq!(w.view().get_proposal(p).unwrap().class, VoteClass::Light);
    assert_eq!(w.view().get_proposal(q).unwrap().class, VoteClass::Light);
}

#[test]
fn orders_outside_the_preset_are_refused_at_propose() {
    let mut w = W::council(45, true);
    let refused = [
        (Order::SetSlices { cap_bps: 250 }, "cap_bps must be 50 to 200, got 250"),
        (
            Order::SetBreath { budget_bps: 900 },
            "budget_bps must be 1000 to 4000, got 900",
        ),
        (
            Order::SetAutoBurn { bps: 6_000 },
            "auto_burn_bps must be 0 to 5000, got 6000",
        ),
        (
            Order::Grant {
                to: a("artist"),
                amount: U128(0),
            },
            "amount must be above 0",
        ),
        (Order::Burn { amount: U128(0) }, "amount must be above 0"),
        (
            Order::SetPosture {
                hunt: Some(HuntLine {
                    sell_multiple_bps: 19_999,
                    max_sell_bps: 500,
                }),
            },
            "sell_multiple_bps must be at least 20000, got 19999",
        ),
    ];
    for (order, reason) in refused {
        refuses(reason, || propose(&mut w, "v1", ProposalKind::Order(order), "1"));
    }
    assert_eq!(w.c.escrow, ZERO);
}

#[test]
fn a_strict_proposal_waits_for_the_sale_and_45() {
    let mut w = W::council(45, false);
    refuses("strict proposals open after the sale closes", || {
        propose(&mut w, "v1", hunt_on(), "1")
    });
    let mut w = W::council(44, true);
    refuses("electorate is too small", || propose(&mut w, "v1", hunt_on(), "1"));
    let mut w = W::council(45, false);
    w.advance(SALE_LENGTH);
    let p = propose(&mut w, "v1", hunt_on(), "1");
    assert_eq!(w.view().get_proposal(p).unwrap().class, VoteClass::Strict);
}

#[test]
fn finalize_refuses_below_its_gas_floor() {
    let mut w = W::council(10, false);
    let p = propose(&mut w, "v1", tune(), "1");
    votes(&mut w, p, [1, 2, 3], true);
    ended(&mut w, p);
    let short = FINALIZE_GAS.saturating_sub(Gas::from_gas(1));
    ctx_with_gas(a("anyone"), ZERO, w.now, w.base, vec![], short);
    refuses("attach at least 150 Tgas to finalize", || w.c.finalize(p));
    assert_eq!(status(&mut w, p), Status::Open);
    assert_eq!(w.c.escrow, BOND);
    ctx_with_gas(a("anyone"), ZERO, w.now, w.base, vec![], FINALIZE_GAS);
    assert!(matches!(w.c.finalize(p), PromiseOrValue::Promise(_)));
    assert_eq!(status(&mut w, p), Status::Executing);
}

#[test]
fn get_due_lists_open_proposals_past_their_vote() {
    let mut w = W::council(10, false);
    let first = propose(&mut w, "v1", tune(), "1");
    w.advance(DAY);
    let second = propose(&mut w, "v2", ProposalKind::Signal, "2");
    assert!(w.view().get_due().is_empty());
    ended(&mut w, first);
    assert_eq!(w.view().get_due(), vec![first]);
    ended(&mut w, second);
    assert_eq!(w.view().get_due(), vec![first, second]);
    finalize(&mut w, first);
    assert_eq!(w.view().get_due(), vec![second]);
}

#[test]
fn the_drill_timings_come_from_the_preset() {
    let mut w = W::drill();
    assert_eq!(w.view().get_sale().price, milli(100));
    for i in 1..=10 {
        w.buy_scale(&format!("v{i}"));
    }
    w.advance(10 * 60 * NANOS_PER_SEC - 1);
    refuses("not eligible to vote", || propose(&mut w, "v1", tune(), "1"));
    w.advance(1);
    let p = propose(&mut w, "v1", tune(), "1");
    let view = w.view().get_proposal(p).unwrap();
    assert_eq!(view.ends_at.0 - view.created_at.0, 30 * 60 * NANOS_PER_SEC);
    votes(&mut w, p, 1..=3, true);
    ended(&mut w, p);
    assert_eq!(finalize(&mut w, p), Status::Executing);
}

#[test]
fn leash_held_carries_the_dragons_reason() {
    let (mut w, p) = passed_grant();
    let reason = "a grant may move at most 50 bps of hoard.wyrm";
    w.callback(vec![verdict(Verdict::Refused { reason: reason.into() })])
        .on_executed(p);
    assert_eq!(
        events("proposal_executed")[0]["status"],
        json!({ "LeashHeld": { "error": reason } })
    );
    assert_eq!(status(&mut w, p), leash(reason));
}

#[test]
fn retired_scales_never_join_the_electorate() {
    let mut w = W::council(40, true);
    let _ = w.as_("keeper", ZERO).close_sale();
    assert_eq!(w.view().get_sale().retired_from, Some(41));
    assert_eq!(w.view().eligible_now(), 40);
    let p = propose(&mut w, "v1", tune(), "1");
    assert_eq!(w.view().get_proposal(p).unwrap().electorate, 40);
}

#[test]
fn get_council_shows_the_enabled_kinds_and_their_classes() {
    let mut w = W::new();
    let council = near_sdk::serde_json::to_value(w.view().get_council()).unwrap();
    let kinds: Vec<(String, String)> = council["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| (k["kind"].as_str().unwrap().into(), k["class"].as_str().unwrap().into()))
        .collect();
    let expected = [
        ("SetBreath", "Light"),
        ("SetSlices", "Light"),
        ("SetPosture on", "Strict"),
        ("SetPosture off", "Light"),
        ("Grant", "Strict"),
        ("Burn", "Light"),
        ("SetAutoBurn", "Light"),
        ("Signal", "Light"),
    ];
    assert_eq!(kinds, expected.map(|(k, c)| (k.to_string(), c.to_string())));
    assert_eq!(council["classes"][0]["min_electorate"], 10);
    assert_eq!(council["classes"][1]["min_electorate"], 45);
    assert_eq!(council["classes"][1]["needs_closed_sale"], true);
    assert_eq!(council["bounds"]["cap_bps"], json!({ "lo": 50, "hi": 200 }));
    assert_eq!(council["voting_ns"], (2 * DAY).to_string());
    assert_eq!(council["season_ns"], SEASON.to_string());
}
