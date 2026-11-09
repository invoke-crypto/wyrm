use super::*;
use crate::vaults::{GLOBAL_CODE_BYTE_COST, ON_CLAIMED_GAS, VAULT_FUNDING};

fn released(amount: NearToken) -> Result<U128, near_sdk::PromiseError> {
    Ok(U128(amount.as_yoctonear()))
}

/// Alice owns Scale 1 and every vault exists.
fn world() -> W {
    let mut w = W::open();
    w.buy_scale("alice");
    w.make_vaults();
    w
}

#[test]
fn create_vaults_makes_keyless_global_contract_accounts_25_at_a_time() {
    let mut w = W::new();
    w.as_("keeper", ZERO).create_vaults().detach();
    let receipts = get_created_receipts();
    let vaults: Vec<_> = receipts
        .iter()
        .filter(|r| matches!(r.actions.first(), Some(MockAction::CreateAccount { .. })))
        .collect();
    assert_eq!(vaults.len(), 25);
    assert_eq!(vaults[0].receiver_id.as_str(), "001.scales.near");
    assert_eq!(vaults[24].receiver_id.as_str(), "025.scales.near");
    for r in &vaults {
        assert!(matches!(r.actions[0], MockAction::CreateAccount { .. }));
        assert!(matches!(r.actions[1], MockAction::Transfer { deposit, .. } if deposit == VAULT_FUNDING));
        assert!(matches!(r.actions[2], MockAction::UseGlobalContract { .. }));
        assert!(matches!(&r.actions[3], MockAction::FunctionCallWeight { method_name, .. } if method_name == b"new"));
        assert_eq!(r.actions.len(), 4, "no keys are added");
    }
    let c = calls();
    let probe = c.iter().find(|(_, m, _, _)| m == "get_parent").unwrap();
    assert_eq!(probe.0.as_str(), "025.scales.near", "only the last vault is asked");
    let callback = receipts.iter().find(|r| r.receiver_id == me()).unwrap();
    assert_eq!(callback.receipt_indices.len(), 1);
    refuses("the remaining vaults are being created", || {
        w.as_("keeper", ZERO).create_vaults()
    });
}

#[test]
fn a_failed_batch_is_retried_from_its_first_vault() {
    let mut w = W::new();
    w.as_("keeper", ZERO).create_vaults().detach();
    assert!(!w.callback(vec![PromiseResult::Failed]).on_vaults_created(25));
    assert_eq!(w.view().get_vault("1".into()), None);
    w.as_("keeper", ZERO).create_vaults().detach();
    let first = get_created_receipts()
        .into_iter()
        .find(|r| r.receiver_id != me())
        .unwrap();
    assert_eq!(first.receiver_id.as_str(), "001.scales.near");
    let stranger = PromiseResult::Successful(b"\"other.near\"".to_vec());
    assert!(
        !w.callback(vec![stranger]).on_vaults_created(25),
        "a vault of another parent does not count"
    );
    w.as_("keeper", ZERO).create_vaults().detach();
    assert!(w.callback(vec![parent_answer()]).on_vaults_created(25));
    assert_eq!(w.view().get_vault("25".into()).unwrap().as_str(), "025.scales.near");
    assert_eq!(w.view().get_vault("26".into()), None);
}

#[test]
fn a_batch_whose_callback_never_came_can_be_retried_after_ten_minutes() {
    let mut w = W::new();
    w.as_("keeper", ZERO).create_vaults().detach();
    w.advance(crate::vaults::VAULT_BATCH_TIMEOUT - 1);
    refuses("the remaining vaults are being created", || {
        w.as_("keeper", ZERO).create_vaults()
    });
    w.advance(1);
    w.as_("keeper", ZERO).create_vaults().detach();
}

#[test]
fn publish_vault_code_only_publishes_the_code_fixed_at_init() {
    let mut w = W::new();
    let code = b"vault wasm".to_vec();
    w.c.vault_code_hash = env::sha256_array(&code);
    let cost = GLOBAL_CODE_BYTE_COST.saturating_mul(code.len() as u128);
    let input = |code: &[u8], deposit: NearToken, now: u64| {
        let mut context = VMContextBuilder::new()
            .current_account_id(me())
            .predecessor_account_id(a("deployer"))
            .attached_deposit(deposit)
            .block_timestamp(now)
            .account_balance(near(1_000))
            .build();
        context.input = code.into();
        testing_env!(context);
    };
    input(b"other wasm", cost, w.now);
    refuses("does not match", || w.c.publish_vault_code());
    input(&code, cost.saturating_sub(YOCTO), w.now);
    refuses("to publish", || w.c.publish_vault_code());
    input(&code, cost.saturating_add(near(1)), w.now);
    w.c.publish_vault_code();
    let receipts = get_created_receipts();
    assert_eq!(receipts[0].receiver_id, me());
    assert!(matches!(&receipts[0].actions[0], MockAction::DeployGlobalContract { code: c, .. } if *c == code));
    assert_eq!(transfers(), vec![(a("deployer"), near(1))]);
}

#[test]
fn claim_asks_the_vault_to_release_to_the_owner_and_records_the_amount() {
    let mut w = world();
    w.as_("alice", YOCTO).claim("1".into()).detach();
    let c = calls();
    assert_eq!(c[0].0.as_str(), "001.scales.near");
    assert_eq!(c[0].1, "release");
    assert_eq!(c[0].2, near_sdk::serde_json::json!({ "to": "alice.near" }));
    assert_eq!((c[1].0.clone(), c[1].1.as_str()), (me(), "on_claimed"));
    assert_eq!(w.view().get_scale("1".into()).unwrap().claiming_since, Some(U64(w.now)));

    let started = U64(w.now);
    w.advance(NANOS_PER_SEC);
    assert_eq!(
        w.callback(vec![])
            .on_claimed("1".into(), a("alice"), started, released(near(3))),
        U128(near(3).as_yoctonear())
    );
    assert_eq!(events("scale_claimed").len(), 1);
    let s = w.view().get_scale("1".into()).unwrap();
    assert_eq!(
        (s.claiming_since, s.claimed_total),
        (None, U128(near(3).as_yoctonear()))
    );
    assert_eq!(
        w.c.get_history("1".into()).last(),
        Some(&ScaleEvent::Claimed {
            by: a("alice"),
            amount: near(3),
            at: U64(w.now)
        })
    );
}

#[test]
fn a_failed_release_clears_the_flag_and_records_nothing() {
    let mut w = world();
    w.as_("alice", YOCTO).claim("1".into()).detach();
    let started = U64(w.now);
    assert_eq!(
        w.callback(vec![])
            .on_claimed("1".into(), a("alice"), started, Err(near_sdk::PromiseError::Failed)),
        U128(0)
    );
    let s = w.view().get_scale("1".into()).unwrap();
    assert_eq!((s.claiming_since, s.claimed_total), (None, U128(0)));
    assert_eq!(w.c.get_history("1".into()).len(), 1);
}

#[test]
fn a_claim_in_flight_blocks_transfer_listing_sale_and_offer_acceptance() {
    let mut w = world();
    w.as_("alice", YOCTO).list("1".into(), near(5));
    let any = w.as_("bob", near(2)).make_offer(None);
    w.as_("alice", YOCTO).claim("1".into()).detach();
    refuses("a claim is in flight", || {
        w.as_("alice", YOCTO).nft_transfer(a("bob"), "1".into(), None, None)
    });
    refuses("a claim is in flight", || {
        w.as_("alice", YOCTO).list("1".into(), near(5))
    });
    refuses("not listed", || w.as_("carol", near(5)).buy("1".into()));
    refuses("a claim is in flight", || {
        w.as_("alice", YOCTO).accept_offer(any, "1".into())
    });
    refuses("a claim is in flight", || w.as_("alice", YOCTO).claim("1".into()));
    assert_eq!(w.owner("1"), a("alice"));
}

#[test]
fn claim_delists_the_token() {
    let mut w = world();
    w.as_("alice", YOCTO).list("1".into(), near(5));
    w.as_("alice", YOCTO).claim("1".into()).detach();
    assert_eq!(events("scale_delisted").len(), 1);
    assert!(w.view().get_listings(None, None).is_empty());
}

#[test]
fn a_stuck_claim_expires_after_ten_minutes_and_a_late_answer_still_records() {
    let mut w = world();
    w.as_("alice", YOCTO).claim("1".into()).detach();
    let first = U64(w.now);
    w.advance(CLAIM_TIMEOUT - 1);
    refuses("a claim is in flight", || {
        w.as_("alice", YOCTO).nft_transfer(a("bob"), "1".into(), None, None)
    });
    w.advance(1);
    w.as_("alice", YOCTO).claim("1".into()).detach();
    let second = U64(w.now);
    w.callback(vec![])
        .on_claimed("1".into(), a("alice"), first, released(near(2)));
    let s = w.view().get_scale("1".into()).unwrap();
    assert_eq!(s.claimed_total, U128(near(2).as_yoctonear()));
    assert_eq!(
        s.claiming_since,
        Some(second),
        "a late answer leaves the newer claim in flight"
    );
}

#[test]
fn an_expired_claim_lets_the_token_move() {
    let mut w = world();
    w.as_("alice", YOCTO).claim("1".into()).detach();
    w.advance(CLAIM_TIMEOUT);
    w.as_("alice", YOCTO).nft_transfer(a("bob"), "1".into(), None, None);
    assert_eq!(w.owner("1"), a("bob"));
}

#[test]
fn only_the_current_owner_can_claim() {
    let mut w = world();
    w.as_("alice", YOCTO).list("1".into(), near(5));
    w.as_("bob", near(5)).buy("1".into());
    refuses("not the owner", || w.as_("alice", YOCTO).claim("1".into()));
    refuses("not the owner", || w.as_("mallory", YOCTO).claim("1".into()));
    refuses("Requires attached deposit of exactly 1 yoctoNEAR", || {
        w.as_("bob", ZERO).claim("1".into())
    });
    w.as_("bob", YOCTO).claim("1".into()).detach();
    assert_eq!(calls()[0].2, near_sdk::serde_json::json!({ "to": "bob.near" }));
}

#[test]
fn claim_waits_for_the_vault_to_exist() {
    let mut w = W::open();
    w.buy_scale("alice");
    refuses("vault does not exist yet", || w.as_("alice", YOCTO).claim("1".into()));
}

#[test]
fn on_claimed_with_a_full_history_ring_keeps_the_last_16_entries() {
    let mut w = world();
    for i in 0..40 {
        let (from, to) = if i % 2 == 0 { ("alice", "bob") } else { ("bob", "alice") };
        w.as_(from, YOCTO).nft_transfer(a(to), "1".into(), None, None);
    }
    assert_eq!(w.c.get_history("1".into()).len(), 16);
    w.as_("alice", YOCTO).claim("1".into()).detach();
    assert_eq!(calls()[1].3, ON_CLAIMED_GAS);
    let started = U64(w.now);
    w.callback(vec![])
        .on_claimed("1".into(), a("alice"), started, released(near(1)));
    let history = w.c.get_history("1".into());
    assert_eq!(history.len(), 16);
    assert!(matches!(history.last(), Some(ScaleEvent::Claimed { .. })));
}

#[test]
fn create_vaults_stops_once_all_100_exist() {
    let mut w = W::new();
    w.make_vaults();
    assert_eq!(w.view().get_config().vaults_created, 100);
    assert_eq!(
        w.view().get_scales(Some(0), Some(1))[0].vault.as_str(),
        "091.scales.near"
    );
    refuses("all 100 vaults exist", || w.as_("keeper", ZERO).create_vaults());
}
