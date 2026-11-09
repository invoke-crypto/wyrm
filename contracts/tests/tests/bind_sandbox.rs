//! Launch and bind on real wasm against mock-nearly and mock-dcl (design.md sections 5a and 5b). Each of bind's
//! twelve checks refuses with its own reason and leaves the Dragon unbound. Checks 9 and 11 need answers mock-nearly
//! and mock-dcl never give on their own, so the test patches one string in the mock's stored record.
mod dragon_world;
mod v6_world;

use dragon_world::*;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::Contract;
use serde_json::{json, Value};
use v6_world::*;

const SIGNER_GAS_SLACK: u128 = NEAR / 50;
const ICON_BYTES: u32 = "https://wyrm.example/icon.png".len() as u32;
const TEXT_BYTES: u32 = ("Wyrm".len() + "WYRM".len() + "sandbox".len()) as u32;

async fn unlaunched(dragon_near: u128) -> anyhow::Result<World> {
    setup_with(Options {
        unlaunched: true,
        ..Options::new(dragon_near)
    })
    .await
}

async fn bind_on(w: &World, dragon: &Contract, launch_id: u64) -> ExecutionFinalResult {
    w.creator
        .call(dragon.id(), "bind")
        .args_json(json!({ "launch_id": launch_id.to_string() }))
        .gas(BIND_GAS)
        .transact()
        .await
        .unwrap()
}

async fn refusal(w: &World, dragon: &Contract, launch_id: u64) -> String {
    let res = bind_on(w, dragon, launch_id).await;
    assert!(res.is_success(), "a refused bind is an answer: {}", failure_text(&res));
    let evs = events(&res);
    assert!(named(&evs, "bound").is_empty(), "bound on a refused launch: {evs:?}");
    let refused = named(&evs, "bind_refused");
    assert_eq!(refused.len(), 1, "{evs:?}");
    assert_eq!(refused[0]["launch_id"], launch_id.to_string());
    let coin: Value = dragon.view("get_coin").await.unwrap().json().unwrap();
    assert!(coin.is_null(), "the Dragon stays unbound");
    refused[0]["reason"].as_str().unwrap().to_string()
}

async fn set_mode(w: &World, mode: &str) {
    w.nearly
        .call("set_mode")
        .args_json(json!({ "mode": mode }))
        .transact()
        .await
        .unwrap()
        .into_result()
        .unwrap();
}

async fn quote_total(w: &World) -> u128 {
    let q: Value = w
        .nearly
        .view("quote_launch")
        .args_json(json!({ "icon_bytes": ICON_BYTES, "dev_buy": null, "tax": true, "text_bytes": TEXT_BYTES }))
        .await
        .unwrap()
        .json()
        .unwrap();
    u(&q["total"])
}

async fn launch_margin(w: &World) -> u128 {
    let c: Value = w.dragon.view("get_config").await.unwrap().json().unwrap();
    u(&c["engine"]["launch_margin"])
}

fn one_event(res: &ExecutionFinalResult, name: &str) -> Value {
    let found = named(&events(res), name);
    assert_eq!(found.len(), 1, "one {name} in {:?}", events(res));
    found[0].clone()
}

#[tokio::test]
async fn each_check_on_nearlys_record_refuses_with_its_own_reason() -> anyhow::Result<()> {
    let w = unlaunched(50 * NEAR).await?;
    let me = w.dragon.as_account();

    let mute = alt_dragon(&w, "mute", json!({ "nearly": w.ash.id() })).await?;
    assert_eq!(refusal(&w, &mute, 0).await, "a view failed", "check 1");
    assert_eq!(refusal(&w, &w.dragon, 999).await, "no such launch", "check 2");

    set_mode(&w, "park").await;
    let parked = launch_as_creator(&w, me, dragon_launch_args(&w, "PARK")).await?;
    set_mode(&w, "normal").await;
    assert_ne!(record(&w, parked).await["step"], "Done");
    assert_eq!(refusal(&w, &w.dragon, parked).await, "not done", "check 3");

    let theirs = launch_as_creator(&w, &w.keeper, dragon_launch_args(&w, "THEIRS")).await?;
    assert_eq!(
        refusal(&w, &w.dragon, theirs).await,
        "the launch's creator is not this Dragon",
        "check 4"
    );

    let other_wrap = alt_dragon(&w, "otherwrap", json!({ "wrap": "wrap2.test.near" })).await?;
    let id = launch_as_creator(&w, other_wrap.as_account(), dragon_launch_args(&w, "PAIR")).await?;
    assert_eq!(
        refusal(&w, &other_wrap, id).await,
        "the launch is not on the wNEAR pair",
        "check 5"
    );

    let mut args = dragon_launch_args(&w, "FEETO");
    args.as_object_mut().unwrap().remove("fee_to");
    let id = launch_as_creator(&w, me, args).await?;
    assert_eq!(
        refusal(&w, &w.dragon, id).await,
        "the launch's fee_to is not the tithe",
        "check 6"
    );

    let mut args = dragon_launch_args(&w, "BURNS");
    args["fee_mode"] = json!("burn");
    let id = launch_as_creator(&w, me, args).await?;
    assert_eq!(
        refusal(&w, &w.dragon, id).await,
        "the launch's fee mode is not creator",
        "check 7"
    );

    let mut args = dragon_launch_args(&w, "TAX");
    args["tax"]["buy_bps"] = json!(300);
    let id = launch_as_creator(&w, me, args).await?;
    assert_eq!(
        refusal(&w, &w.dragon, id).await,
        "the launch's tax is not the engine tax",
        "check 8"
    );

    let id = launch_as_creator(&w, me, dragon_launch_args(&w, "POOLID")).await?;
    let pool_id = record(&w, id).await["pool_id"].as_str().unwrap().to_string();
    patch_string(&w, w.nearly.id(), "", &pool_id, "not-a-pool-id").await?;
    assert_eq!(
        refusal(&w, &w.dragon, id).await,
        r#"bad pool id: pool_id must be "<x>|<y>|<fee>""#,
        "check 9"
    );
    Ok(())
}

#[tokio::test]
async fn a_pool_on_another_dcl_with_other_tokens_or_paused_is_refused() -> anyhow::Result<()> {
    let w = unlaunched(50 * NEAR).await?;
    let me = w.dragon.as_account();

    let dcl2 = w.worker.dev_deploy(&wasm("mock_dcl")).await?;
    dcl2.call("new").transact().await?.into_result()?;
    let other_dcl = alt_dragon(&w, "otherdcl", json!({ "dcl": dcl2.id() })).await?;
    let id = launch_as_creator(&w, other_dcl.as_account(), dragon_launch_args(&w, "ELSEWHERE")).await?;
    assert_eq!(refusal(&w, &other_dcl, id).await, "no pool on our DCL", "check 10");

    let id = launch_as_creator(&w, me, dragon_launch_args(&w, "TOKENS")).await?;
    let pool_id = record(&w, id).await["pool_id"].as_str().unwrap().to_string();
    patch_string(&w, w.dcl.id(), &pool_id, w.wrap.id().as_str(), "wrap2.test.near").await?;
    assert_eq!(
        refusal(&w, &w.dragon, id).await,
        "the pool does not pair wrap with the coin",
        "check 11"
    );

    let id = launch_as_creator(&w, me, dragon_launch_args(&w, "PAUSED")).await?;
    let pool_id = record(&w, id).await["pool_id"].as_str().unwrap().to_string();
    let set_state = |state: &'static str| {
        let (dcl, pool_id) = (w.dcl.clone(), pool_id.clone());
        async move {
            dcl.call("set_pool")
                .args_json(
                    json!({ "pool_id": pool_id, "total_x": NEAR.to_string(), "total_y": NEAR.to_string(),
                                   "state": state }),
                )
                .transact()
                .await
                .unwrap()
                .into_result()
                .unwrap();
        }
    };
    set_state("Paused").await;
    assert_eq!(refusal(&w, &w.dragon, id).await, "the pool is not running", "check 12");

    set_state("Running").await;
    let res = bind_on(&w, &w.dragon, id).await;
    assert_eq!(one_event(&res, "bound")["launch_id"], id.to_string());
    assert_eq!(
        coin_of(&w).await["pool_id"],
        pool_id,
        "the same launch binds once its pool runs"
    );
    Ok(())
}

#[tokio::test]
async fn a_launch_refused_on_its_wiring_or_deposit_returns_the_whole_deposit() -> anyhow::Result<()> {
    let w = setup_with(Options {
        unlaunched: true,
        vault_batches: 3,
        ..Options::new(50 * NEAR)
    })
    .await?;
    let before = near_of(&w, w.creator.id()).await;
    let res = w.launch_coin(&coin("WYRM"), 10 * NEAR, LAUNCH_GAS).await;
    assert!(res.is_success(), "{}", failure_text(&res));
    assert_eq!(res.clone().json::<bool>()?, false);
    assert_eq!(
        one_event(&res, "launch_refused")["reason"],
        "scales has 75 of 100 vaults"
    );
    assert!(launch_started(&res).is_none(), "nothing reached Nearly");
    w.settle().await;
    assert!(
        before - near_of(&w, w.creator.id()).await < SIGNER_GAS_SLACK,
        "the whole deposit came back"
    );

    let res = w
        .keeper
        .call(w.scales.id(), "create_vaults")
        .max_gas()
        .transact()
        .await?;
    assert!(res.is_success(), "{}", failure_text(&res));
    w.skip(10 * MIN).await;
    let short = quote_total(&w).await + launch_margin(&w).await - 1;
    let before = near_of(&w, w.creator.id()).await;
    let res = w.launch_coin(&coin("WYRM"), short, LAUNCH_GAS).await;
    assert_eq!(
        one_event(&res, "launch_refused")["reason"],
        format!(
            "attach at least {} yoctoNEAR, Nearly's quote plus the margin",
            short + 1
        )
    );
    w.settle().await;
    assert!(before - near_of(&w, w.creator.id()).await < SIGNER_GAS_SLACK);

    let res = w.launch_coin(&coin("WYRM"), 10 * NEAR, LAUNCH_GAS).await;
    refused_with(&res, "launches are at least 10 minutes apart");
    w.skip(10 * MIN).await;
    let res = w.launch_coin(&coin("WYRM"), short + 1, LAUNCH_GAS).await;
    assert!(
        launch_started(&res).is_some(),
        "exactly the quote plus the margin launches"
    );
    Ok(())
}

#[tokio::test]
async fn a_pool_at_the_predicted_id_is_refused_before_any_near_leaves() -> anyhow::Result<()> {
    let w = unlaunched(50 * NEAR).await?;
    let token = format!("wyrm.{}", w.nearly.id());
    let predicted = format!("{}|{token}|10000", w.wrap.id());
    w.dcl
        .call("create_pool")
        .args_json(json!({ "pool_id": predicted }))
        .transact()
        .await?
        .into_result()?;
    let (creator, nearly) = (near_of(&w, w.creator.id()).await, near_of(&w, w.nearly.id()).await);
    let res = w.launch_coin(&coin("WYRM"), 10 * NEAR, LAUNCH_GAS).await;
    assert_eq!(one_event(&res, "launch_refused")["reason"], "pool exists");
    assert!(launch_started(&res).is_none());
    let on_nearly = res
        .receipt_outcomes()
        .iter()
        .filter(|o| &o.executor_id == w.nearly.id())
        .count();
    assert_eq!(on_nearly, 2, "only quote_launch and get_slug_count ran on Nearly");
    w.settle().await;
    assert!(
        creator - near_of(&w, w.creator.id()).await < SIGNER_GAS_SLACK,
        "the whole deposit came back"
    );
    let gained = near_of(&w, w.nearly.id()).await - nearly;
    assert!(
        gained < SIGNER_GAS_SLACK / 10,
        "Nearly got only gas rewards for two views: {gained}"
    );
    let slugs: u32 = w
        .nearly
        .view("get_slug_count")
        .args_json(json!({ "slug": "wyrm" }))
        .await?
        .json()?;
    assert_eq!(slugs, 0);
    Ok(())
}

#[tokio::test]
async fn nearly_gets_the_quote_and_margin_and_the_creator_the_rest() -> anyhow::Result<()> {
    let w = unlaunched(50 * NEAR).await?;
    let cost = quote_total(&w).await + launch_margin(&w).await;
    let (creator, dragon) = (near_of(&w, w.creator.id()).await, near_of(&w, w.dragon.id()).await);
    let res = w.launch_coin(&coin("WYRM"), 10 * NEAR, LAUNCH_GAS).await;
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{}",
        failure_text(&res)
    );
    let id = launch_started(&res).expect("launched");
    assert_eq!(record(&w, id).await["step"], "Done");
    w.settle().await;
    let paid = creator - near_of(&w, w.creator.id()).await;
    assert!(
        paid >= cost && paid - cost < SIGNER_GAS_SLACK,
        "the creator paid the quote plus the margin and gas: {paid} against {cost}"
    );
    let back = near_of(&w, w.dragon.id()).await as i128 - dragon as i128;
    let margin = launch_margin(&w).await as i128;
    assert!(
        back > 0 && back <= margin + (SIGNER_GAS_SLACK / 10) as i128,
        "Nearly's unused margin returns to its creator, the Dragon: {back}"
    );
    Ok(())
}

#[tokio::test]
async fn a_synchronous_nearly_refusal_leaves_the_deposit_in_the_dragon() -> anyhow::Result<()> {
    let w = unlaunched(50 * NEAR).await?;
    let cost = quote_total(&w).await + launch_margin(&w).await;
    set_mode(&w, "panic").await;
    let (creator, dragon) = (near_of(&w, w.creator.id()).await, near_of(&w, w.dragon.id()).await);
    let res = w.launch_coin(&coin("WYRM"), 10 * NEAR, LAUNCH_GAS).await;
    assert!(res.is_success(), "{}", failure_text(&res));
    assert!(failure_text(&res).contains("launches paused"));
    assert!(launch_started(&res).is_none());
    assert_eq!(one_event(&res, "launch_sent")["ok"], false);
    w.settle().await;
    let paid = creator - near_of(&w, w.creator.id()).await;
    assert!(paid >= cost && paid - cost < SIGNER_GAS_SLACK, "{paid} against {cost}");
    let kept = near_of(&w, w.dragon.id()).await - dragon;
    assert!(
        kept >= cost && kept - cost < SIGNER_GAS_SLACK / 10,
        "Nearly's refund stays in the Dragon: {kept} against {cost}"
    );
    Ok(())
}

#[tokio::test]
async fn a_parked_launch_resumes_through_nearly_resume_then_binds() -> anyhow::Result<()> {
    let w = unlaunched(50 * NEAR).await?;
    set_mode(&w, "park").await;
    let res = w.launch_coin(&coin("WYRM"), LAUNCH_DEPOSIT, LAUNCH_GAS).await;
    let id = launch_started(&res).expect("launched");
    assert_ne!(record(&w, id).await["step"], "Done", "the launch parked");
    assert_eq!(refusal(&w, &w.dragon, id).await, "not done");
    set_mode(&w, "normal").await;

    let resume = |gas: u64| {
        w.keeper
            .call(w.dragon.id(), "nearly_resume")
            .args_json(json!({ "launch_id": id.to_string() }))
            .gas(Gas::from_tgas(gas))
            .transact()
    };
    refused_with(&resume(299).await?, "nearly_resume needs at least 300 Tgas");
    let mut rounds = 0;
    while record(&w, id).await["step"] != "Done" {
        rounds += 1;
        assert!(rounds <= 3, "Done within three resumes");
        let res = resume(300).await?;
        assert!(res.is_success(), "{}", failure_text(&res));
        gas_line("nearly_resume with 300 Tgas attached", &res);
    }
    println!("resumed to Done in {rounds} call(s), read from get_launch(id).step");
    let res = bind_on(&w, &w.dragon, id).await;
    assert_eq!(one_event(&res, "bound")["launch_id"], id.to_string());
    refused_with(&resume(300).await?, "already bound");
    Ok(())
}

#[tokio::test]
async fn two_concurrent_binds_bind_once() -> anyhow::Result<()> {
    let w = unlaunched(50 * NEAR).await?;
    let res = w.launch_coin(&coin("WYRM"), LAUNCH_DEPOSIT, LAUNCH_GAS).await;
    let id = launch_started(&res).expect("launched");
    let (a, b) = tokio::join!(bind_on(&w, &w.dragon, id), bind_on(&w, &w.dragon, id));
    for r in [&a, &b] {
        assert!(
            r.is_success(),
            "both binds passed the unbound check before either bound: {}",
            failure_text(r)
        );
    }
    let bound: Vec<Value> = [&a, &b].iter().flat_map(|r| named(&events(r), "bound")).collect();
    assert_eq!(bound.len(), 1, "exactly one bind binds");
    let answers = [a.clone().json::<bool>()?, b.clone().json::<bool>()?];
    assert_eq!(answers.iter().filter(|x| **x).count(), 1, "{answers:?}");
    let refused: usize = [&a, &b].iter().map(|r| named(&events(r), "bind_refused").len()).sum();
    assert_eq!(refused, 0, "the losing bind changes nothing and says nothing");
    let opened = [&a, &b]
        .iter()
        .flat_map(|r| r.receipt_outcomes())
        .filter(|o| &o.executor_id == w.scales.id())
        .count();
    assert_eq!(opened, 1, "open_sale was called once");
    assert_eq!(scales_view(&w, "get_sale", json!({})).await["state"], "Open");
    Ok(())
}

#[tokio::test]
async fn bind_opens_the_sale_and_the_first_buy_reaches_the_dragon_nearly_registered() -> anyhow::Result<()> {
    let mut w = unlaunched(300 * NEAR).await?;
    assert_eq!(scales_view(&w, "get_sale", json!({})).await["state"], "Waiting");
    let stranger = w
        .keeper
        .call(w.dragon.id(), "launch")
        .args_json(json!({ "coin": coin("WYRM") }));
    refused_with(
        &stranger
            .deposit(NearToken::from_near(2))
            .gas(LAUNCH_GAS)
            .transact()
            .await?,
        "only the creator",
    );
    let res = w.launch_coin(&coin("WYRM"), LAUNCH_DEPOSIT, LAUNCH_GAS).await;
    let id = launch_started(&res).expect("launched");
    let token: near_workspaces::AccountId = record(&w, id).await["token"].as_str().unwrap().parse()?;
    let registered: Value = w
        .worker
        .view(&token, "storage_balance_of")
        .args_json(json!({ "account_id": w.dragon.id() }))
        .await?
        .json()?;
    assert!(
        !registered.is_null(),
        "Nearly registered its creator, the Dragon, at CreateToken"
    );
    refused_with(
        &w.keeper
            .call(w.dragon.id(), "bind")
            .args_json(json!({ "launch_id": id.to_string() }))
            .gas(BIND_GAS)
            .transact()
            .await?,
        "only the creator",
    );

    let bind = bind_on(&w, &w.dragon, id).await;
    assert!(bind.receipt_failures().is_empty(), "{}", failure_text(&bind));
    one_event(&bind, "bound");
    let kick = bind
        .receipt_outcomes()
        .iter()
        .find(|o| &o.executor_id == w.scales.id())
        .expect("bind called scales");
    println!("GAS open_sale kick from bind: {:.2} Tgas burnt", tgas(kick.gas_burnt));
    let sale = scales_view(&w, "get_sale", json!({})).await;
    assert_eq!(sale["state"], "Open");
    let opened = u(&sale["opened_at"]) as u64;
    assert_eq!(u(&sale["ends_at"]) as u64, opened + SALE, "14 days from the bind block");

    w.wyrm = token;
    w.pool_id = coin_of(&w).await["pool_id"].as_str().unwrap().into();
    w.set_pool(POOL_NEAR, POOL_WYRM).await?;
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    w.skip_to(breath_at).await;
    let res = w.poke().await?;
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{}",
        failure_text(&res)
    );
    assert_eq!(named(&events(&res), "breath_slice").len(), 1);
    assert!(
        w.balance(&w.wyrm, w.dragon.id().as_str()).await > 0,
        "the first buy arrived"
    );
    Ok(())
}

#[tokio::test]
async fn launch_at_500_and_bind_at_150_tgas_complete_and_one_less_is_refused() -> anyhow::Result<()> {
    let w = unlaunched(50 * NEAR).await?;
    let short = w.launch_coin(&coin("WYRM"), LAUNCH_DEPOSIT, Gas::from_tgas(499)).await;
    refused_with(&short, "launch needs at least 500 Tgas");
    let launch = w.launch_coin(&coin("WYRM"), LAUNCH_DEPOSIT, Gas::from_tgas(500)).await;
    assert!(launch.receipt_failures().is_empty(), "{}", failure_text(&launch));
    gas_line("launch with exactly 500 Tgas attached", &launch);
    let id = launch_started(&launch).expect("launched");
    assert_eq!(record(&w, id).await["step"], "Done", "Done read from get_launch");

    let short = w
        .creator
        .call(w.dragon.id(), "bind")
        .args_json(json!({ "launch_id": id.to_string() }))
        .gas(Gas::from_tgas(149))
        .transact()
        .await?;
    refused_with(&short, "bind needs at least 150 Tgas");
    let bind = w.bind_launch(id, Gas::from_tgas(150)).await;
    assert!(bind.receipt_failures().is_empty(), "{}", failure_text(&bind));
    gas_line("bind with exactly 150 Tgas attached", &bind);
    one_event(&bind, "bound");
    Ok(())
}
