//! The Dragon's leash: every order `obey` takes, inside and outside its bounds, the Verdict scales records as
//! Executed or LeashHeld, grants and their caps, auto-burn and burns to ash, Hunt on the taxed coin, the tokens it
//! accepts, and what enters its log. scales signs `obey` with its own key here, as its `finalize` would.
mod dragon_world;

use dragon_world::*;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use serde_json::{json, Value};

fn refused(res: &ExecutionFinalResult, why: &str) {
    assert!(res.is_failure(), "expected a refusal naming {why:?}");
    let text = format!("{:?}", res.failures());
    assert!(text.contains(why), "refusal should name {why:?}: {text}");
}

fn leash_held(v: &Value) -> String {
    v["Refused"]["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("expected Refused, got {v}"))
        .to_string()
}

#[tokio::test]
async fn every_order_inside_and_outside_its_bounds_and_from_a_stranger() -> anyhow::Result<()> {
    let w = setup(50 * NEAR).await?;
    let hunt = |m: u32, s: u16| json!({ "SetPosture": { "hunt": { "sell_multiple_bps": m, "max_sell_bps": s } } });
    let cases: Vec<(Value, Option<&str>)> = vec![
        (json!({ "SetBreath": { "budget_bps": 1_000 } }), None),
        (json!({ "SetBreath": { "budget_bps": 4_000 } }), None),
        (
            json!({ "SetBreath": { "budget_bps": 999 } }),
            Some("budget_bps must be 1000 to 4000, got 999"),
        ),
        (
            json!({ "SetBreath": { "budget_bps": 4_001 } }),
            Some("budget_bps must be 1000 to 4000, got 4001"),
        ),
        (json!({ "SetSlices": { "cap_bps": 50 } }), None),
        (json!({ "SetSlices": { "cap_bps": 200 } }), None),
        (
            json!({ "SetSlices": { "cap_bps": 49 } }),
            Some("cap_bps must be 50 to 200, got 49"),
        ),
        (
            json!({ "SetSlices": { "cap_bps": 201 } }),
            Some("cap_bps must be 50 to 200, got 201"),
        ),
        (hunt(20_000, 1_500), None),
        (
            hunt(19_999, 1_000),
            Some("sell_multiple_bps must be at least 20000, got 19999"),
        ),
        (hunt(25_000, 1_501), Some("max_sell_bps must be 1 to 1500, got 1501")),
        (hunt(25_000, 0), Some("max_sell_bps must be 1 to 1500, got 0")),
        (json!({ "SetAutoBurn": { "bps": 5_000 } }), None),
        (
            json!({ "SetAutoBurn": { "bps": 5_001 } }),
            Some("auto_burn_bps must be 0 to 5000, got 5001"),
        ),
        (json!({ "SetAutoBurn": { "bps": 0 } }), None),
        (
            json!({ "Grant": { "to": w.keeper.id(), "amount": "0" } }),
            Some("amount must be above 0"),
        ),
        (json!({ "Burn": { "amount": "0" } }), Some("amount must be above 0")),
        (json!({ "SetPosture": { "hunt": null } }), None),
    ];
    for (order, refusal) in &cases {
        let res = w.obey(order.clone()).await;
        assert!(
            res.is_success(),
            "obey answers instead of panicking: {:?}",
            res.failures()
        );
        let verdict: Value = res.clone().json()?;
        let evs = events(&res);
        match refusal {
            None => {
                assert_eq!(verdict, json!("Executed"), "{order}");
                let ev = &named(&evs, "settings_changed")[0];
                assert!(
                    ev["budget_bps"].is_number() && ev["cap_bps"].is_number(),
                    "Strategy field names"
                );
            }
            Some(why) => {
                assert_eq!(leash_held(&verdict), *why, "the LeashHeld reason names the limit");
                assert_eq!(named(&evs, "order_refused")[0]["reason"], *why);
            }
        }
    }
    let config: Value = w.dragon.view("get_config").await?.json()?;
    let s = &config["strategy"];
    assert_eq!(
        (s["budget_bps"].clone(), s["cap_bps"].clone()),
        (json!(4_000), json!(200))
    );
    assert_eq!((s["hunt"].clone(), s["auto_burn_bps"].clone()), (Value::Null, json!(0)));
    assert_eq!(config["coin"]["pool_id"], w.pool_id);
    assert_eq!(config["peers"]["scales"], w.scales.id().as_str());
    assert_eq!(config["preset"], "hoarder");

    for who in [&w.keeper, &w.creator] {
        refused(
            &who.call(w.dragon.id(), "obey")
                .args_json(json!({ "order": { "SetSlices": { "cap_bps": 100 } } }))
                .gas(Gas::from_tgas(100))
                .transact()
                .await?,
            "only scales",
        );
    }
    for method in ["set_breath", "set_slices", "set_posture", "pause_buys", "grant"] {
        let res = w
            .scales
            .call(w.dragon.id(), method)
            .args_json(json!({}))
            .transact()
            .await?;
        refused(&res, "MethodNotFound");
    }
    let paid = w
        .scales
        .call(w.dragon.id(), "obey")
        .args_json(json!({ "order": { "SetSlices": { "cap_bps": 100 } } }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(paid.is_failure(), "obey takes no deposit");
    Ok(())
}

#[tokio::test]
async fn grants_are_capped_and_a_failed_grant_is_refused_and_restores_the_hoard() -> anyhow::Result<()> {
    let w = setup(50 * NEAR).await?;
    let hoard = 1_000_000 * WYRM;
    let dragon = w.dragon.id().as_str().to_string();
    w.donate(hoard).await?;
    let ally = w.worker.dev_create_account().await?;
    w.mint(&w.wyrm, ally.id().as_str(), 0).await?;
    let half = hoard / 200;
    let grant = |to: &str, amount: u128| json!({ "Grant": { "to": to, "amount": amount.to_string() } });

    // An account with no storage on the token cannot be paid.
    let res = w.obey(grant("nobody.test.near", half)).await;
    assert!(res.is_success());
    assert_eq!(named(&events(&res), "outflow_failed")[0]["kind"], "grant");
    assert_eq!(leash_held(&res.clone().json()?), "transfer failed");
    let state = w.state().await;
    assert_eq!(u(&state["hoard"]["wyrm"]), hoard);
    assert_eq!(u(&state["hoard"]["granted"]), 0);

    let over = w.verdict(grant(ally.id().as_str(), half + 1)).await;
    assert!(leash_held(&over).contains("a grant is at most 50 bps of the hoard"));
    let res = w.obey(grant(ally.id().as_str(), half)).await;
    assert_eq!(res.clone().json::<Value>()?, json!("Executed"));
    let paid = &named(&events(&res), "grant_paid")[0].clone();
    assert_eq!(u(&paid["amount"]), half);
    assert_eq!(u(&paid["hoard"]), hoard, "the hoard before this grant");
    assert!(paid["at"].is_string());
    assert_eq!(
        w.balance(&w.wyrm, ally.id().as_str()).await,
        half,
        "a wallet transfer pays no tax"
    );

    // The failed grant freed its share, so the rest of 1% of the hoard now is still open, and no more.
    let rest = (hoard - half) / 100 - half;
    let over = w.verdict(grant(ally.id().as_str(), rest + 1)).await;
    assert!(leash_held(&over).contains("grants in any 30 days"));
    assert_eq!(w.verdict(grant(ally.id().as_str(), rest)).await, json!("Executed"));
    let state = w.state().await;
    assert_eq!(u(&state["hoard"]["wyrm"]), hoard - half - rest);
    assert_eq!(u(&state["hoard"]["granted"]), half + rest);
    assert_eq!(w.balance(&w.wyrm, &dragon).await, hoard - half - rest);
    Ok(())
}

#[tokio::test]
async fn auto_burn_and_a_burn_pay_ash_and_register_it_in_the_same_batch() -> anyhow::Result<()> {
    let w = setup(300 * NEAR).await?;
    let ash = w.ash.id().as_str().to_string();
    let registered = |w: &World| {
        let token = w.wyrm.clone();
        let ash = ash.clone();
        let worker = w.worker.clone();
        async move {
            let v: Value = worker
                .view(&token, "storage_balance_of")
                .args_json(json!({ "account_id": ash }))
                .await
                .unwrap()
                .json()
                .unwrap();
            !v.is_null()
        }
    };
    assert!(!registered(&w).await, "ash starts unregistered on the coin");
    assert_eq!(
        w.verdict(json!({ "SetAutoBurn": { "bps": 5_000 } })).await,
        json!("Executed")
    );
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    w.skip_to(breath_at).await;
    let net = |q: u128| q / 10_000 * 9_600 + q % 10_000 * 9_600 / 10_000;
    let quoted = u(&named(&w.poke_ok().await, "breath_slice")[0]["quoted_out"]);
    // The next observation reconciles the first buy and trades again: ash waits for one that rests.
    w.skip(61 * SEC).await;
    w.set_pool(POOL_NEAR, POOL_WYRM).await?;
    let evs = w.poke_ok().await;
    assert!(
        named(&evs, "auto_burn").is_empty(),
        "no auto-burn inside a trading chain"
    );
    let quoted_2 = u(&named(&evs, "breath_slice")[0]["quoted_out"]);
    assert_eq!(
        u(&w.state().await["hoard"]["ash_due"]),
        net(quoted) / 2,
        "half of the first buy is owed to ash"
    );
    let arrived = w.balance(&w.wyrm, w.dragon.id().as_str()).await;
    // A spike rests the next observation, which reconciles the second buy and sends ash both halves.
    w.set_pool(POOL_NEAR, POOL_WYRM * 8 / 10).await?;
    w.skip(61 * SEC).await;
    let res = w.poke().await?;
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{}",
        failure_text(&res)
    );
    gas_line("poke that rests and auto-burns", &res);
    let half = net(quoted) / 2 + net(quoted_2) / 2;
    let burnt = &named(&events(&res), "auto_burn")[0];
    assert_eq!(u(&burnt["amount"]), half, "half of what each buy delivered");
    assert_eq!(w.balance(&w.wyrm, &ash).await, half);
    let on_coin = res
        .receipt_outcomes()
        .iter()
        .filter(|o| o.executor_id == w.wyrm)
        .count();
    assert_eq!(
        on_coin, 2,
        "the balance view and one batch that registered ash and paid it"
    );
    assert!(registered(&w).await);
    let h = w.state().await["hoard"].clone();
    assert_eq!((u(&h["ash_due"]), u(&h["auto_burned"])), (0, half));
    assert_eq!(u(&h["wyrm"]), arrived - half, "hoard.wyrm excludes what went to ash");

    let cap = u(&h["wyrm"]) * 50 / 10_000;
    let over = w.verdict(json!({ "Burn": { "amount": (cap + 1).to_string() } })).await;
    assert!(
        leash_held(&over).contains(&format!("a burn is at most 50 bps of the hoard ({})", u(&h["wyrm"]))),
        "the Burn cap reads hoard.wyrm without ash's share: {over}"
    );
    let res = w.obey(json!({ "Burn": { "amount": cap.to_string() } })).await;
    assert_eq!(res.clone().json::<Value>()?, json!("Executed"));
    gas_line("obey Burn to a registered ash", &res);
    assert_eq!(u(&named(&events(&res), "burn_paid")[0]["hoard"]), u(&h["wyrm"]));
    assert_eq!(w.balance(&w.wyrm, &ash).await, half + cap);
    Ok(())
}

#[tokio::test]
async fn hunt_is_off_by_default_and_sells_the_taxed_coin_only_above_the_multiple() -> anyhow::Result<()> {
    let w = setup(300 * NEAR).await?;
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    w.skip_to(breath_at).await;
    let buy = w.poke_ok().await;
    assert_eq!(named(&buy, "breath_slice").len(), 1);
    let hunting = |evs: &[(String, Value)]| named(evs, "hunt_slice");
    // About 3.3 times the average cost, which is also a spike, so nothing is bought either.
    w.set_pool(POOL_NEAR, POOL_WYRM * 10 / 33).await?;
    w.skip(61 * SEC).await;
    assert!(hunting(&w.poke_ok().await).is_empty(), "Hunt is off at launch");
    let bought = u(&w.state().await["hoard"]["wyrm_bought"]);
    assert!(bought > 0);

    let line = json!({ "SetPosture": { "hunt": { "sell_multiple_bps": 25_000, "max_sell_bps": 1_000 } } });
    assert_eq!(w.verdict(line).await, json!("Executed"));
    // About 2 times the average cost: under the 2.5x line.
    w.set_pool(POOL_NEAR, POOL_WYRM / 2).await?;
    w.skip(61 * SEC).await;
    assert!(hunting(&w.poke_ok().await).is_empty(), "no sale under the sell line");
    assert_eq!(w.state().await["mood"], "Sleeping");

    w.set_pool(POOL_NEAR, POOL_WYRM * 10 / 33).await?;
    assert_eq!(
        w.state().await["mood"],
        "Sleeping",
        "the mood reads the newest sample, not the pool"
    );
    let held = u(&w.state().await["hoard"]["wyrm"]);
    w.skip(61 * SEC).await;
    let res = w.poke().await?;
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{}",
        failure_text(&res)
    );
    gas_line("poke Hunt sell chain on the taxed coin", &res);
    let sold = hunting(&events(&res));
    assert_eq!(
        sold.len(),
        1,
        "a sale once the price is over 2.5x cost, even while a spike stops buys"
    );
    assert_eq!(
        u(&sold[0]["amount_in"]),
        held / 10,
        "a tenth of the hoard per day, gross"
    );
    w.skip(61 * SEC).await;
    w.poke_ok().await;
    let state = w.state().await;
    assert!(
        u(&state["hoard"]["realized_near"]) > 0,
        "the proceeds were seen arriving as wNEAR"
    );
    assert_eq!(u(&state["hoard"]["wyrm"]), held - held / 10);
    Ok(())
}

#[tokio::test]
async fn unknown_tokens_are_refused_and_pokes_never_enter_the_log() -> anyhow::Result<()> {
    let w = setup(50 * NEAR).await?;
    let fake = w.worker.dev_deploy(&wasm("mock_ft")).await?;
    fake.call("new")
        .args_json(json!({ "symbol": "FAKE", "decimals": 18 }))
        .transact()
        .await?
        .into_result()?;
    let dev = w.worker.dev_create_account().await?;
    w.mint(fake.id(), dev.id().as_str(), 100 * WYRM).await?;
    w.mint(fake.id(), w.dragon.id().as_str(), 0).await?;
    let res = dev
        .call(fake.id(), "ft_transfer_call")
        .args_json(json!({ "receiver_id": w.dragon.id(), "amount": (100 * WYRM).to_string(), "msg": "" }))
        .deposit(YOCTO)
        .max_gas()
        .transact()
        .await?;
    assert!(failure_text(&res).contains("accepts only its bound coin and wNEAR"));
    assert_eq!(
        w.balance(fake.id(), dev.id().as_str()).await,
        100 * WYRM,
        "the transfer was refunded"
    );
    assert_eq!(w.balance(fake.id(), w.dragon.id().as_str()).await, 0);

    w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    let before: Value = w.state().await;
    for _ in 0..4 {
        w.skip(61 * SEC).await;
        let evs = w.poke_ok().await;
        assert_eq!(named(&evs, "observe").len(), 1, "every poke emits observe");
    }
    let after = w.state().await;
    assert_eq!(
        after["log_next"], before["log_next"],
        "pokes and samples do not enter the log"
    );
    assert_eq!(after["samples"].as_u64(), before["samples"].as_u64().map(|n| n + 4));
    assert_eq!(
        w.verdict(json!({ "SetSlices": { "cap_bps": 150 } })).await,
        json!("Executed")
    );
    let log = w.log().await;
    assert_eq!(log.last().unwrap()["event"], "settings_changed");
    assert!(log.iter().all(|e| e["event"] != "observe"));
    let busy = w.keeper.call(w.dragon.id(), "poke").gas(POKE_GAS).transact().await?;
    assert!(busy.is_success(), "an idle Dragon accepts a poke");
    Ok(())
}
