//! A whole system under one parent account, `thewyrm.test.near`, so keeper, verify and the Lair can derive
//! `dragon.<root>`, `scales.<root>` and `tithe.<root>` as on mainnet. Then the steps that fill it with every state a
//! reader meets: an orphan launch, a bound coin, trades, a sold and closed sale, a listing and an offer, and a
//! proposal of each status.
use super::*;

pub const ROOT: &str = "thewyrm";

#[derive(Default)]
pub struct Marks {
    pub orphan_launch: Option<u64>,
    pub proposals: Vec<(String, u64)>,
}

pub async fn deploy_unlaunched() -> anyhow::Result<World> {
    setup_with(Options {
        root: Some(ROOT),
        unlaunched: true,
        ..Options::new(300 * NEAR)
    })
    .await
}

pub async fn parked_launch(w: &mut World) -> anyhow::Result<()> {
    w.nearly
        .call("set_mode")
        .args_json(json!({ "mode": "park" }))
        .transact()
        .await?
        .into_result()?;
    let res = w.launch_coin(&coin("WYRM"), LAUNCH_DEPOSIT, LAUNCH_GAS).await;
    w.launch_id = launch_started(&res).expect("Nearly started a launch");
    w.nearly
        .call("set_mode")
        .args_json(json!({ "mode": "normal" }))
        .transact()
        .await?
        .into_result()?;
    anyhow::ensure!(record(w, w.launch_id).await["step"] != "Done", "the launch parked");
    Ok(())
}

pub async fn launch_and_bind(w: &mut World, marks: &mut Marks) -> anyhow::Result<()> {
    let orphan = w.launch_coin(&coin("WYRM"), LAUNCH_DEPOSIT, LAUNCH_GAS).await;
    marks.orphan_launch = Some(launch_started(&orphan).expect("orphan launched"));
    w.skip(10 * MIN).await;
    let launch = w.launch_coin(&coin("WYRM"), LAUNCH_DEPOSIT, LAUNCH_GAS).await;
    w.launch_id = launch_started(&launch).expect("launched");
    let bind = w.bind_launch(w.launch_id, BIND_GAS).await;
    anyhow::ensure!(
        named(&events(&bind), "bound").len() == 1,
        "bind: {}",
        failure_text(&bind)
    );
    let c = coin_of(w).await;
    w.wyrm = c["token"].as_str().unwrap().parse()?;
    anyhow::ensure!(
        w.wyrm.as_str().starts_with("wyrm-2."),
        "the orphan took the first slug: {}",
        w.wyrm
    );
    w.pool_id = c["pool_id"].as_str().unwrap().into();
    w.set_pool(POOL_NEAR, POOL_WYRM).await?;
    Ok(())
}

pub async fn trade(w: &World) -> anyhow::Result<()> {
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 6 * HOUR).await;
    w.skip_to(breath_at).await;
    w.poke_ok().await;
    w.skip(61 * SEC).await;
    w.dcl_switch("set_fail_swaps", true).await?;
    w.set_pool(POOL_NEAR, POOL_WYRM).await?;
    w.poke_ok().await;
    w.dcl_switch("set_fail_swaps", false).await?;
    w.donate(10_000 * WYRM).await?;
    for _ in 0..3 {
        w.skip(5 * MIN).await;
        w.set_pool(POOL_NEAR, POOL_WYRM * 13 / 10).await?;
        w.poke_ok().await;
    }
    Ok(())
}

pub async fn sell_out_and_close(w: &World) -> anyhow::Result<Council> {
    let council = Council::buy(w, &[8, 8, 8, 7, 7, 7]).await?;
    let (seller, bidder) = (&council.holders[0], &council.holders[1]);
    seller
        .call(w.scales.id(), "list")
        .args_json(json!({ "token_id": council.held[0][1], "price": (8 * NEAR).to_string() }))
        .deposit(YOCTO)
        .transact()
        .await?
        .into_result()?;
    bidder
        .call(w.scales.id(), "make_offer")
        .args_json(json!({ "token_id": council.held[2][1] }))
        .deposit(NearToken::from_near(6))
        .max_gas()
        .transact()
        .await?
        .into_result()?;
    w.skip(SALE + MIN).await;
    let _closed = close_sale(w).await?;
    Ok(council)
}

pub async fn proposals(w: &World, council: &Council, marks: &mut Marks) -> anyhow::Result<()> {
    w.donate(1_000_000 * WYRM).await?;
    w.skip(61 * SEC).await;
    w.poke_ok().await;
    let hunt_on =
        json!({ "Order": { "SetPosture": { "hunt": { "sell_multiple_bps": 25_000, "max_sell_bps": 1_000 } } } });
    let nobody = json!({ "Order": { "Grant": { "to": "nobody.test.near", "amount": WYRM.to_string() } } });
    let kinds = [
        (
            "Executed",
            json!({ "Order": { "SetSlices": { "cap_bps": 100 } } }),
            30,
            0,
        ),
        ("Rejected", hunt_on, 20, 20),
        ("Carried", json!("Signal"), 20, 2),
        ("LeashHeld", nobody, 40, 0),
    ];
    let mut ids = vec![];
    for (i, (want, kind, yes, no)) in kinds.into_iter().enumerate() {
        let id = council.propose(w, i, kind).await?;
        council.tally(w, id, yes, no).await?;
        ids.push((want, id));
    }
    w.skip(VOTING + MIN).await;
    for (want, id) in ids {
        let res = finalize(w, id).await?;
        anyhow::ensure!(res.is_success(), "finalize {want}: {}", failure_text(&res));
        let status = proposal(w, id).await["status"].clone();
        let got = status
            .as_str()
            .map(String::from)
            .or_else(|| status.as_object().map(|o| o.keys().next().unwrap().clone()));
        anyhow::ensure!(
            got.as_deref() == Some(want),
            "proposal {id} ended {status}, wanted {want}"
        );
        marks.proposals.push((want.into(), id));
    }
    let open = council
        .propose(
            w,
            4,
            json!({ "Order": { "Burn": { "amount": (100 * WYRM).to_string() } } }),
        )
        .await?;
    council.tally(w, open, 12, 3).await?;
    marks.proposals.push(("Open".into(), open));
    Ok(())
}
