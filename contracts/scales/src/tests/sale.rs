use super::*;
use crate::sale::{split, SaleState};
use near_sdk::PromiseOrValue;

fn ids(range: std::ops::RangeInclusive<u8>) -> Vec<String> {
    range.map(|i| i.to_string()).collect()
}

/// The keeper's `close_sale`: its answer, or None when it sent `retire` to the tithe.
fn close(w: &mut W) -> Option<String> {
    match w.as_("keeper", ZERO).close_sale() {
        PromiseOrValue::Value(answer) => Some(answer),
        PromiseOrValue::Promise(_) => None,
    }
}

#[test]
fn creator_scales_91_to_100_belong_to_the_creator_and_never_vote() {
    let mut w = W::new();
    let v = w.view();
    assert_eq!(v.nft_total_supply(), U128(10));
    assert_eq!(v.nft_supply_for_owner(a("creator")), U128(10));
    for id in 91..=100 {
        let s = v.get_scale(id.to_string()).unwrap();
        assert_eq!(s.owner, a("creator"));
        assert!(!s.voting);
    }
    assert!(v.get_scale("1".into()).is_none());
    assert!(v.get_scale("90".into()).is_none());
}

#[test]
fn sale_splits_every_price_creator_and_dragon() {
    let mut w = W::open();
    w.as_("bob", milli(2_300)).buy_scale();
    assert_eq!(
        transfers(),
        vec![
            (a("bob"), milli(300)),
            (a("creator"), milli(200)),
            (a("dragon"), milli(1_800))
        ]
    );
    let s = w.view().get_scale("1".into()).unwrap();
    assert_eq!((s.owner, s.voting), (a("bob"), true));
    assert_eq!(
        w.c.get_history("1".into()),
        vec![ScaleEvent::Minted {
            to: a("bob"),
            price: PRICE,
            at: U64(T0)
        }]
    );
    for i in 2..=90u8 {
        assert_eq!(w.buy_scale(&format!("b{}", i % 7)), i.to_string());
        assert_eq!(
            transfers(),
            vec![(a("creator"), milli(200)), (a("dragon"), milli(1_800))]
        );
    }
    assert_eq!(w.view().nft_total_supply(), U128(100));
    refuses("sold out", || w.as_("late", PRICE).buy_scale());
    for price in [
        milli(100),
        PRICE,
        NearToken::from_yoctonear(4_999_999_999_999_999_999_999_997),
    ] {
        for bps in [0, 1, 1_000, 2_000] {
            let (to_creator, to_dragon) = split(price, bps);
            assert_eq!(to_creator.as_yoctonear(), price.as_yoctonear() * bps as u128 / 10_000);
            assert_eq!(to_creator.saturating_add(to_dragon), price);
        }
    }
}

#[test]
fn underpaying_the_price_is_refused() {
    let mut w = W::open();
    refuses("below the price", || w.as_("alice", milli(1_999)).buy_scale());
    assert_eq!(w.view().get_sale().sold, 0);
}

#[test]
fn the_sale_closes_at_ends_at_and_unsold_ids_are_never_minted() {
    let mut w = W::new();
    w.advance(DAY);
    w.as_("dragon", ZERO).open_sale();
    let ends_at = w.view().get_sale().ends_at.unwrap().0;
    assert_eq!(ends_at, T0 + DAY + SALE_LENGTH, "14 days from the opening at bind");
    w.now = ends_at - 1;
    w.buy_scale("alice");
    w.now = ends_at;
    refuses("the sale has closed", || w.as_("bob", PRICE).buy_scale());
    let v = w.view();
    assert_eq!(v.nft_total_supply(), U128(11));
    assert!(v.get_scale("2".into()).is_none());
    assert!(v.nft_token("2".into()).is_none());
}

#[test]
fn scale_minted_carries_token_id_to_and_price() {
    let mut w = W::open();
    w.buy_scale("alice");
    let minted = &events("scale_minted")[0];
    assert_eq!(minted["token_id"], "1");
    assert_eq!(minted["to"], "alice.near");
    assert_eq!(minted["price"], PRICE.as_yoctonear().to_string());
}

#[test]
fn new_runs_the_preset_and_collection_checks() {
    refuses("runs only on .testnet accounts", || {
        deploy(PresetName::Drill, "scales.near")
    });
    let mut bad = collection();
    bad.symbol = "SC ALE".into();
    fresh_chain();
    refuses("symbol must be ASCII letters and digits", || {
        Scales::new(
            PresetName::Hoarder,
            a("dragon"),
            a("tithe"),
            a("creator"),
            VAULT_HASH.into(),
            bad,
        )
    });
    let mut w = W::new();
    let config = near_sdk::serde_json::to_value(w.view().get_config()).unwrap();
    assert_eq!(config["preset"], "hoarder");
    assert_eq!(config["engine_version"], wyrm_types::ENGINE_VERSION);
    let hash = Base58CryptoHash::from(wyrm_types::preset(PresetName::Hoarder).hash());
    assert_eq!(config["preset_hash"], near_sdk::serde_json::to_value(hash).unwrap());
    assert_eq!(
        (config["tithe"].as_str(), config["royalty_receiver"].as_str()),
        (Some("tithe.near"), Some("creator.near"))
    );
    assert_eq!(config["sale"]["state"], "Waiting");
    assert_eq!(w.view().nft_metadata().symbol, "SCALE");
}

#[test]
fn the_sale_opens_on_the_dragons_call_and_never_reopens() {
    let mut w = W::new();
    let sale = w.view().get_sale();
    assert_eq!(
        (sale.state, sale.ends_at, sale.price, sale.creator_bps),
        (SaleState::Waiting, None, PRICE, 1_000)
    );
    refuses("the sale is not open", || w.as_("alice", PRICE).buy_scale());
    w.advance(DAY);
    w.as_("dragon", ZERO).open_sale();
    assert_eq!(events("sale_opened").len(), 1);
    let opened = Some(U64(T0 + DAY));
    let ends_at = Some(U64(T0 + DAY + SALE_LENGTH));
    let sale = w.view().get_sale();
    assert_eq!(
        (sale.state, sale.opened_at, sale.ends_at),
        (SaleState::Open, opened, ends_at)
    );
    w.buy_scale("alice");
    w.advance(DAY);
    w.as_("dragon", ZERO).open_sale();
    assert!(events("sale_opened").is_empty());
    let sale = w.view().get_sale();
    assert_eq!((sale.opened_at, sale.ends_at, sale.sold), (opened, ends_at, 1));
    w.now = ends_at.unwrap().0;
    close(&mut w);
    w.as_("dragon", ZERO).open_sale();
    assert_eq!(w.view().get_sale().state, SaleState::Closed);
}

#[test]
fn open_sale_refuses_anyone_but_the_dragon() {
    let mut w = W::new();
    for who in ["creator", "tithe", "mallory"] {
        refuses("only the dragon opens the sale", || w.as_(who, ZERO).open_sale());
    }
    assert_eq!(w.view().get_sale().state, SaleState::Waiting);
}

#[test]
fn a_mint_sweeps_pre_sale_income_to_the_dragon() {
    let mut w = W::open();
    w.make_vaults();
    assert_eq!(w.buy_scale("alice"), "1");
    let c = calls();
    assert_eq!((c[0].0.as_str(), c[0].1.as_str()), ("001.scales.near", "release"));
    assert_eq!(c[0].2, json!({ "to": "dragon.near" }));
    assert_eq!((c[1].0.clone(), c[1].1.as_str()), (me(), "on_claimed"));
    assert_eq!(c[1].2["by"], "dragon.near");
    assert_eq!(
        events("scale_swept"),
        vec![json!({ "token_ids": ["1"], "to": "dragon.near" })]
    );
    let started = U64(w.now);
    assert_eq!(w.view().get_scale("1".into()).unwrap().claiming_since, Some(started));
    refuses("a claim is in flight", || {
        w.as_("alice", YOCTO).list("1".into(), near(6))
    });
    refuses("a claim is in flight", || w.as_("alice", YOCTO).claim("1".into()));
    refuses("a claim is in flight", || {
        w.as_("alice", YOCTO).nft_transfer(a("bob"), "1".into(), None, None)
    });

    w.advance(NANOS_PER_SEC);
    let one = U128(near(1).as_yoctonear());
    assert_eq!(
        w.callback(vec![]).on_claimed("1".into(), a("dragon"), started, Ok(one)),
        one
    );
    let s = w.view().get_scale("1".into()).unwrap();
    assert_eq!(
        (s.claiming_since, s.claimed_total),
        (None, U128(0)),
        "a sweep is not the owner's claim"
    );
    assert_eq!(
        w.c.get_history("1".into()).last(),
        Some(&ScaleEvent::Swept {
            amount: near(1),
            at: U64(w.now)
        })
    );
    w.as_("alice", YOCTO).list("1".into(), near(6));

    w.buy_scale("bob");
    let started = U64(w.now);
    w.callback(vec![])
        .on_claimed("2".into(), a("dragon"), started, Err(near_sdk::PromiseError::Failed));
    assert_eq!(w.view().get_scale("2".into()).unwrap().claiming_since, None);
    assert_eq!(
        w.c.get_history("2".into()).len(),
        1,
        "a failed release records no sweep"
    );
}

#[test]
fn sweep_pays_only_the_dragon_and_only_for_unminted() {
    let mut w = W::open();
    w.make_vaults();
    w.buy_scale("alice");
    refuses("sweep 1 to 20 Scales", || w.as_("anyone", ZERO).sweep(ids(50..=70)));
    refuses("sweep 1 to 20 Scales", || w.as_("anyone", ZERO).sweep(vec![]));
    refuses("Scale 1 is minted", || w.as_("anyone", ZERO).sweep(ids(1..=2)));
    refuses("Scale 95 is minted", || w.as_("anyone", ZERO).sweep(vec!["95".into()]));
    w.as_("anyone", ZERO).sweep(ids(50..=69));
    let c = calls();
    assert_eq!(c.len(), 20);
    for ((to, method, args, _), i) in c.iter().zip(50..=69) {
        assert_eq!(
            (to.as_str(), method.as_str()),
            (format!("{i:03}.scales.near").as_str(), "release")
        );
        assert_eq!(args, &json!({ "to": "dragon.near" }));
    }
    assert!(transfers().is_empty());
    assert_eq!(events("scale_swept")[0]["token_ids"], json!(ids(50..=69)));

    let mut w = W::open();
    refuses("the vault of Scale 5 does not exist yet", || {
        w.as_("anyone", ZERO).sweep(vec!["5".into()])
    });
}

#[test]
fn close_sale_retires_the_unsold_suffix_and_retries_the_tithe() {
    let mut w = W::new();
    assert_eq!(close(&mut w).as_deref(), Some("not due"));
    w.as_("dragon", ZERO).open_sale();
    for i in 1..=40 {
        w.buy_scale(&format!("b{i}"));
    }
    assert_eq!(close(&mut w).as_deref(), Some("not due"));
    w.advance(SALE_LENGTH);
    assert_eq!(close(&mut w), None);
    let c = calls();
    assert_eq!((c[0].0.clone(), c[0].1.as_str()), (a("tithe"), "retire"));
    assert_eq!(c[0].2, json!({ "first_unsold": 41 }));
    assert_eq!((c[1].0.clone(), c[1].1.as_str()), (me(), "on_retired"));
    let sale = w.view().get_sale();
    assert_eq!(
        (sale.state, sale.sold, sale.retired_from, sale.tithe_told),
        (SaleState::Closed, 40, Some(41), false)
    );

    assert!(w
        .callback(vec![PromiseResult::Failed])
        .on_retired()
        .contains("call close_sale again"));
    assert!(!w.view().get_sale().tithe_told);
    assert_eq!(close(&mut w), None);
    assert_eq!(
        calls()[0].2,
        json!({ "first_unsold": 41 }),
        "the next close_sale tells the tithe again"
    );
    assert_eq!(
        w.callback(vec![PromiseResult::Successful(vec![])]).on_retired(),
        "closed"
    );
    assert!(w.view().get_sale().tithe_told);
    assert_eq!(close(&mut w).as_deref(), Some("closed"));
    assert!(calls().is_empty());

    let mut w = W::open();
    for i in 1..=90 {
        w.buy_scale(&format!("b{}", i % 7));
    }
    assert_eq!(
        close(&mut w).as_deref(),
        Some("closed"),
        "a sold-out sale closes before ends_at"
    );
    assert!(calls().is_empty(), "nothing retired, so the tithe is not told");
    let sale = w.view().get_sale();
    assert_eq!(
        (sale.state, sale.retired_from, sale.tithe_told),
        (SaleState::Closed, None, true)
    );
}

#[test]
fn retired_ids_are_never_minted_and_stay_sweepable() {
    let mut w = W::open();
    w.make_vaults();
    for i in 1..=40 {
        w.buy_scale(&format!("b{i}"));
    }
    w.advance(SALE_LENGTH);
    close(&mut w);
    refuses("the sale has closed", || w.as_("late", PRICE).buy_scale());
    let v = w.view();
    assert!(v.nft_token("41".into()).is_none());
    assert_eq!(v.nft_total_supply(), U128(50));
    assert_eq!(v.get_sweepable(), ids(41..=90));
    w.as_("keeper", ZERO).sweep(ids(71..=90));
    assert_eq!(calls().len(), 20);
}

#[test]
fn get_sweepable_lists_unminted_ids_with_a_vault() {
    let mut w = W::new();
    assert!(w.view().get_sweepable().is_empty(), "no vault exists yet");
    w.make_vaults();
    assert_eq!(w.view().get_sweepable(), ids(1..=90));
    w.as_("dragon", ZERO).open_sale();
    w.buy_scale("alice");
    w.buy_scale("bob");
    assert_eq!(w.view().get_sweepable(), ids(3..=90));
}
