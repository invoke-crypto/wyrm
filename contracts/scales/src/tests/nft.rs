use super::*;
use wyrm_types::RoyaltyTo;

/// Alice owns Scale 1.
fn world() -> W {
    let mut w = W::open();
    w.buy_scale("alice");
    w
}

fn approval_cost(account: &str) -> NearToken {
    env::storage_byte_cost().saturating_mul(crate::nft::approval_bytes(&a(account)) as u128)
}

#[test]
fn nft_payout_sends_5_percent_to_the_royalty_receiver() {
    for to in [RoyaltyTo::Creator, RoyaltyTo::Holders] {
        let mut w = world();
        let receiver = royalty_receiver(to, &a("creator"), &a("tithe"));
        w.c.royalty_receiver = receiver.clone();
        let p = w.view().nft_payout("1".into(), U128(1_000), Some(10)).payout;
        assert_eq!(p.len(), 2);
        assert_eq!(p[&a("alice")], U128(950));
        assert_eq!(p[&receiver], U128(50));
        refuses("max_len_payout", || {
            w.view().nft_payout("1".into(), U128(1_000), Some(1))
        });
    }
}

#[test]
fn an_approved_market_sells_with_nft_transfer_payout() {
    let mut w = world();
    w.as_("alice", approval_cost("market"))
        .nft_approve("1".into(), a("market"), None);
    let p = w
        .as_("market", YOCTO)
        .nft_transfer_payout(a("carol"), "1".into(), Some(0), None, U128(2_000), Some(10));
    assert_eq!(p.payout[&a("alice")], U128(1_900));
    assert_eq!(p.payout[&a("creator")], U128(100));
    let logs = get_logs();
    assert_eq!(w.owner("1"), a("carol"));
    assert!(logs
        .iter()
        .any(|l| l.contains("\"standard\":\"nep171\"") && l.contains("\"authorized_id\":\"market.near\"")));
    assert!(
        !w.view().nft_is_approved("1".into(), a("market"), None),
        "the transfer revoked every approval"
    );
}

#[test]
fn approvals_need_a_storage_deposit_and_stop_at_three() {
    let mut w = world();
    refuses("for storage", || {
        w.as_("alice", approval_cost("m1").saturating_sub(YOCTO))
            .nft_approve("1".into(), a("m1"), None)
    });
    for m in ["m1", "m2", "m3"] {
        w.as_("alice", approval_cost(m)).nft_approve("1".into(), a(m), None);
    }
    refuses("at most 3 approvals", || {
        w.as_("alice", approval_cost("m4"))
            .nft_approve("1".into(), a("m4"), None)
    });
    refuses("not the owner", || {
        w.as_("bob", approval_cost("m4")).nft_approve("1".into(), a("m4"), None)
    });
    w.as_("alice", YOCTO).nft_revoke("1".into(), a("m1"));
    assert_eq!(transfers(), vec![(a("alice"), approval_cost("m1"))]);
    assert!(!w.view().nft_is_approved("1".into(), a("m1"), None));
    assert!(w.view().nft_is_approved("1".into(), a("m2"), Some(1)));
    assert!(!w.view().nft_is_approved("1".into(), a("m2"), Some(0)));
    w.as_("alice", YOCTO).nft_transfer(a("bob"), "1".into(), None, None);
    assert_eq!(
        transfers(),
        vec![(a("alice"), approval_cost("m2").saturating_add(approval_cost("m3")))]
    );
}

#[test]
fn approved_transfers_check_the_approval_id() {
    let mut w = world();
    w.as_("alice", approval_cost("market"))
        .nft_approve("1".into(), a("market"), None);
    refuses("approval id does not match", || {
        w.as_("market", YOCTO)
            .nft_transfer(a("carol"), "1".into(), Some(5), None)
    });
    refuses("not the owner or approved", || {
        w.as_("mallory", YOCTO).nft_transfer(a("carol"), "1".into(), None, None)
    });
    w.as_("market", YOCTO)
        .nft_transfer(a("carol"), "1".into(), Some(0), None);
    assert_eq!(w.owner("1"), a("carol"));
}

#[test]
fn nft_approve_with_msg_notifies_the_market() {
    let mut w = world();
    w.as_("alice", approval_cost("market"))
        .nft_approve("1".into(), a("market"), Some("{\"price\":\"1\"}".into()));
    let c = calls();
    assert_eq!((c[0].0.clone(), c[0].1.as_str()), (a("market"), "nft_on_approve"));
    assert_eq!(c[0].2["owner_id"], "alice.near");
}

#[test]
fn nft_token_shows_the_hold_tier_art() {
    let mut w = world();
    let media = |w: &mut W| w.view().nft_token("1".into()).unwrap().metadata.unwrap().media.unwrap();
    assert_eq!(media(&mut w), super::media("ash"));
    w.advance(3 * DAY);
    assert_eq!(media(&mut w), super::media("ember"));
    w.advance(4 * DAY);
    assert_eq!(media(&mut w), super::media("molten"));
    w.advance(7 * DAY);
    assert_eq!(media(&mut w), super::media("obsidian"));
    let extra = w.view().nft_token("1".into()).unwrap().metadata.unwrap().extra.unwrap();
    assert!(extra.contains("\"tier\":\"Obsidian\""));
    w.as_("alice", YOCTO).nft_transfer(a("bob"), "1".into(), None, None);
    assert_eq!(media(&mut w), super::media("ash"), "a sale resets the tier");
}

#[test]
fn enumeration_counts_owners() {
    let mut w = world();
    w.buy_scale("alice");
    let v = w.view();
    assert_eq!(v.nft_supply_for_owner(a("alice")), U128(2));
    assert_eq!(v.nft_supply_for_owner(a("creator")), U128(10));
    let ids: Vec<_> = v
        .nft_tokens(Some(U128(1)), Some(2))
        .into_iter()
        .map(|t| t.token_id)
        .collect();
    assert_eq!(ids, vec!["2", "91"]);
}

#[test]
fn a_returned_transfer_call_restores_the_senders_hold_start() {
    let mut w = world();
    w.advance(10 * DAY);
    let _ = w
        .as_("alice", YOCTO)
        .nft_transfer_call(a("receiver"), "1".into(), None, None, "".into());
    let c = calls();
    assert_eq!(w.owner("1"), a("receiver"));
    assert_eq!(c[0].1, "nft_on_transfer");
    assert_eq!(c[1].2["previous_hold_start"], T0.to_string());
    let back = vec![PromiseResult::Successful(b"true".to_vec())];
    assert!(!w
        .callback(back)
        .nft_resolve_transfer(a("alice"), a("receiver"), "1".into(), U64(T0)));
    let s = w.view().get_scale("1".into()).unwrap();
    assert_eq!((s.owner, s.hold_start), (a("alice"), U64(T0)));
}

#[test]
fn a_kept_transfer_call_stays_with_the_receiver() {
    let mut w = world();
    let _ = w
        .as_("alice", YOCTO)
        .nft_transfer_call(a("receiver"), "1".into(), None, None, "".into());
    let kept = vec![PromiseResult::Successful(b"false".to_vec())];
    assert!(w
        .callback(kept)
        .nft_resolve_transfer(a("alice"), a("receiver"), "1".into(), U64(T0)));
    assert_eq!(w.owner("1"), a("receiver"));
}

#[test]
fn transfer_to_self_and_unknown_ids_are_refused() {
    let mut w = world();
    refuses("already the owner", || {
        w.as_("alice", YOCTO).nft_transfer(a("alice"), "1".into(), None, None)
    });
    refuses("unknown token id", || {
        w.as_("alice", YOCTO).nft_transfer(a("bob"), "01".into(), None, None)
    });
    refuses("token not minted", || {
        w.as_("alice", YOCTO).nft_transfer(a("bob"), "9".into(), None, None)
    });
    refuses("Requires attached deposit of exactly 1 yoctoNEAR", || {
        w.as_("alice", ZERO).nft_transfer(a("bob"), "1".into(), None, None)
    });
}

#[test]
fn owner_methods_refuse_a_stranger() {
    let mut w = world();
    w.as_("alice", approval_cost("m1"))
        .nft_approve("1".into(), a("m1"), None);
    refuses("not the owner", || {
        w.as_("mallory", YOCTO).nft_revoke("1".into(), a("m1"))
    });
    refuses("not the owner", || w.as_("mallory", YOCTO).nft_revoke_all("1".into()));
    refuses("not the owner or approved", || {
        w.as_("mallory", YOCTO)
            .nft_transfer_payout(a("mallory"), "1".into(), None, None, U128(1), None)
    });
    refuses("not the owner or approved", || {
        w.as_("mallory", YOCTO)
            .nft_transfer_call(a("mallory"), "1".into(), None, None, "".into())
    });
}
