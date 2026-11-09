use super::*;
use wyrm_types::RoyaltyTo;

/// Alice owns Scales 1 and 2, Bob owns 3.
fn world() -> W {
    let mut w = W::open();
    w.buy_scale("alice");
    w.buy_scale("alice");
    w.buy_scale("bob");
    w
}

/// `world()` for each royalty receiver a preset can name, with the account it resolves to.
fn worlds() -> impl Iterator<Item = (W, AccountId)> {
    [RoyaltyTo::Creator, RoyaltyTo::Holders].into_iter().map(|to| {
        let mut w = world();
        w.c.royalty_receiver = royalty_receiver(to, &a("creator"), &a("tithe"));
        let receiver = w.c.royalty_receiver.clone();
        (w, receiver)
    })
}

#[test]
fn royalty_to_holders_pays_the_tithe() {
    assert_eq!(
        royalty_receiver(RoyaltyTo::Creator, &a("creator"), &a("tithe")),
        a("creator")
    );
    assert_eq!(
        royalty_receiver(RoyaltyTo::Holders, &a("creator"), &a("tithe")),
        a("tithe")
    );
}

#[test]
fn list_buy_pays_the_seller_95_percent_and_the_royalty_receiver_5() {
    for (mut w, receiver) in worlds() {
        w.as_("alice", YOCTO).list("1".into(), near(10));
        assert_eq!(w.view().get_listings(None, None)[0].price, near(10));
        w.advance(DAY);
        w.as_("carol", near(10)).buy("1".into());
        assert_eq!(transfers(), vec![(a("alice"), milli(9_500)), (receiver, milli(500))]);
        let sold = &events("scale_sold")[0];
        assert_eq!(
            (sold["royalty"].as_str(), sold["to"].as_str()),
            (Some("500000000000000000000000"), Some("carol.near"))
        );
        assert_eq!(w.owner("1"), a("carol"));
        let s = w.view().get_scale("1".into()).unwrap();
        assert_eq!((s.hold_start, s.listed_price), (U64(w.now), None));
        refuses("not listed", || w.as_("dave", near(10)).buy("1".into()));
    }
}

#[test]
fn buy_at_any_other_price_is_refused() {
    let mut w = world();
    w.as_("alice", YOCTO).list("1".into(), near(10));
    refuses("must equal the listed price", || {
        w.as_("carol", milli(9_999)).buy("1".into())
    });
    refuses("must equal the listed price", || {
        w.as_("carol", milli(10_001)).buy("1".into())
    });
    assert_eq!(w.owner("1"), a("alice"));
}

#[test]
fn listing_rules() {
    let mut w = world();
    refuses("price is below 0.1 NEAR", || {
        w.as_("alice", YOCTO).list("1".into(), milli(99))
    });
    refuses("not the owner", || w.as_("bob", YOCTO).list("1".into(), near(1)));
    refuses("Requires attached deposit of exactly 1 yoctoNEAR", || {
        w.as_("alice", ZERO).list("1".into(), near(1))
    });
    w.as_("alice", YOCTO).list("1".into(), near(1));
    refuses("not the owner", || w.as_("bob", YOCTO).delist("1".into()));
    w.as_("alice", YOCTO).delist("1".into());
    refuses("not listed", || w.as_("carol", near(1)).buy("1".into()));
}

#[test]
fn any_transfer_clears_the_listing() {
    let mut w = world();
    w.as_("alice", YOCTO).list("1".into(), near(1));
    w.as_("alice", YOCTO).nft_transfer(a("dave"), "1".into(), None, None);
    refuses("not listed", || w.as_("carol", near(1)).buy("1".into()));

    w.as_("alice", YOCTO).list("2".into(), near(1));
    let offer = w.as_("carol", milli(510)).make_offer(Some("2".into()));
    w.as_("alice", YOCTO).accept_offer(offer, "2".into());
    assert!(w.view().get_listings(None, None).is_empty());
}

#[test]
fn a_token_offer_escrows_and_is_accepted_minus_royalty() {
    for (mut w, receiver) in worlds() {
        let offer = w.as_("carol", milli(2_010)).make_offer(Some("1".into()));
        assert_eq!(w.c.escrow, near(2));
        let listed = w.view().get_offers_for("1".into());
        assert_eq!((listed.len(), listed[0].amount), (1, near(2)));
        refuses("the offer is for another Scale", || {
            w.as_("alice", YOCTO).accept_offer(offer, "2".into())
        });
        refuses("not the owner", || w.as_("bob", YOCTO).accept_offer(offer, "1".into()));
        w.as_("alice", YOCTO).accept_offer(offer, "1".into());
        assert_eq!(transfers(), vec![(a("alice"), milli(1_900)), (receiver, milli(100))]);
        assert_eq!(w.owner("1"), a("carol"));
        assert_eq!(w.c.escrow, ZERO);
        assert!(w.view().get_offers_for("1".into()).is_empty());
        refuses("no such offer", || {
            w.as_("alice", YOCTO).accept_offer(offer, "2".into())
        });
    }
}

#[test]
fn an_any_scale_offer_is_accepted_for_the_token_the_owner_names() {
    let mut w = world();
    let offer = w.as_("carol", milli(1_010)).make_offer(None);
    w.as_("bob", YOCTO).accept_offer(offer, "3".into());
    assert_eq!(transfers(), vec![(a("bob"), milli(950)), (a("creator"), milli(50))]);
    assert_eq!(w.owner("3"), a("carol"));
}

#[test]
fn cancel_refunds_the_offer_but_keeps_the_storage_fee() {
    let mut w = world();
    let offer = w.as_("carol", milli(1_010)).make_offer(Some("1".into()));
    refuses("not your offer", || w.as_("mallory", YOCTO).cancel_offer(offer));
    refuses("Requires attached deposit of exactly 1 yoctoNEAR", || {
        w.as_("carol", ZERO).cancel_offer(offer)
    });
    w.as_("carol", YOCTO).cancel_offer(offer);
    assert_eq!(transfers(), vec![(a("carol"), near(1))]);
    assert_eq!(w.c.escrow, ZERO);
    assert_eq!(events("offer_cancelled").len(), 1);
}

#[test]
fn offer_limits() {
    let mut w = world();
    refuses("at least 0.1 NEAR plus the storage fee", || {
        w.as_("carol", milli(109)).make_offer(None)
    });
    refuses("cannot offer on your own Scale", || {
        w.as_("alice", milli(110)).make_offer(Some("1".into()))
    });
    refuses("token not minted", || {
        w.as_("carol", milli(110)).make_offer(Some("50".into()))
    });
    for _ in 0..10 {
        w.as_("carol", milli(110)).make_offer(Some("1".into()));
    }
    refuses("at most 10 open offers", || w.as_("carol", milli(110)).make_offer(None));
    for i in 0..10 {
        w.as_(&format!("o{i}"), milli(110)).make_offer(Some("1".into()));
    }
    refuses("already has 20 open offers", || {
        w.as_("dave", milli(110)).make_offer(Some("1".into()))
    });
    w.as_("dave", milli(110)).make_offer(Some("2".into()));
}

#[test]
fn escrowed_near_never_pays_for_storage() {
    // The balance covers storage only when the escrowed 1 NEAR is counted, so the next offer is refused.
    let short_by = |short: NearToken| {
        let mut w = world();
        w.as_("carol", milli(1_010)).make_offer(None);
        let storage = env::storage_byte_cost().saturating_mul(env::storage_usage() as u128 + 1_000);
        w.base = storage.saturating_add(near(1)).saturating_sub(short);
        w.as_("dave", milli(110)).make_offer(None);
    };
    short_by(ZERO);
    refuses("not enough free balance for storage", || short_by(milli(500)));
}
