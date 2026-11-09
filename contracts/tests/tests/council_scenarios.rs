//! The six Council scenarios of design.md section 12, on the hoarder preset's mainnet timings with fast-forward:
//! 45 Scales sold, a 14-day sale, 7 days of seasoning and 2-day votes. Every finalize attaches exactly 150 Tgas.
mod dragon_world;
mod v6_world;

use dragon_world::*;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::Account;
use serde_json::{json, Value};
use v6_world::*;

const HOARD: u128 = 1_000_000 * WYRM;

fn status(res: &ExecutionFinalResult) -> Value {
    let finalized: Vec<Value> = res
        .logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str::<Value>(j).unwrap())
        .filter(|v| v["event"] == "proposal_finalized")
        .collect();
    assert_eq!(finalized.len(), 1, "one proposal_finalized");
    finalized[0]["data"][0].clone()
}

async fn finalized(w: &World, id: u64, name: &str) -> anyhow::Result<(Value, ExecutionFinalResult)> {
    let res = finalize(w, id).await?;
    assert!(res.is_success(), "finalize {name}: {}", failure_text(&res));
    gas_line(&format!("finalize {name} with exactly 150 Tgas attached"), &res);
    Ok((proposal(w, id).await["status"].clone(), res))
}

fn untouched(w: &World, res: &ExecutionFinalResult) -> bool {
    res.receipt_outcomes().iter().all(|o| o.executor_id != *w.dragon.id())
}

async fn ally(w: &World) -> anyhow::Result<Account> {
    let ally = w.worker.dev_create_account().await?;
    w.mint(&w.wyrm, ally.id().as_str(), 0).await?;
    Ok(ally)
}

#[tokio::test]
async fn the_six_drill_scenarios_on_mainnet_timings() -> anyhow::Result<()> {
    let w = setup(300 * NEAR).await?;
    let council = Council::buy(&w, &[8, 8, 8, 7, 7, 7]).await?;
    let bought_at = w.now().await;
    w.skip(SALE + MIN).await;
    let _ = close_sale(&w).await?;
    assert_eq!(scales_view(&w, "get_sale", json!({})).await["sold"], 45);
    assert!(w.now().await >= bought_at + SEASON, "every Scale is seasoned");
    w.donate(HOARD).await?;
    w.poke_ok().await;
    assert_eq!(u(&w.state().await["hoard"]["wyrm"]), HOARD);

    let (fund_a, fund_b, fund_c) = (ally(&w).await?, ally(&w).await?, ally(&w).await?);
    let grant =
        |to: &Account, amount: u128| json!({ "Order": { "Grant": { "to": to.id(), "amount": amount.to_string() } } });
    let (grant_a, burn, grant_b) = (HOARD * 40 / 10_000, HOARD * 10 / 10_000, HOARD * 40 / 10_000);

    let slices = council
        .propose(&w, 0, json!({ "Order": { "SetSlices": { "cap_bps": 100 } } }))
        .await?;
    let strict_grant = council.propose(&w, 1, grant(&fund_a, grant_a)).await?;
    let hunt_on =
        json!({ "Order": { "SetPosture": { "hunt": { "sell_multiple_bps": 25_000, "max_sell_bps": 1_000 } } } });
    let hunt = council.propose(&w, 2, hunt_on).await?;
    let burned = council
        .propose(&w, 3, json!({ "Order": { "Burn": { "amount": burn.to_string() } } }))
        .await?;
    let signal = council.propose(&w, 4, json!("Signal")).await?;
    let second_grant = council.propose(&w, 5, grant(&fund_b, grant_b)).await?;
    for (id, class, electorate) in [
        (slices, "Light", 45),
        (strict_grant, "Strict", 45),
        (hunt, "Strict", 45),
    ] {
        let p = proposal(&w, id).await;
        assert_eq!(
            (p["class"].clone(), p["electorate"].clone()),
            (json!(class), json!(electorate))
        );
    }

    council.tally(&w, slices, 20, 5).await?;
    council.tally(&w, strict_grant, 24, 6).await?;
    council.tally(&w, hunt, 34, 9).await?;
    council.tally(&w, burned, 20, 0).await?;
    council.tally(&w, signal, 14, 0).await?;
    council.tally(&w, second_grant, 40, 0).await?;
    let due: Vec<u64> = scales_view(&w, "get_due", json!({}))
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert!(due.is_empty(), "nothing is due before the vote ends");
    refused_with(&finalize(&w, slices).await?, "voting still open");
    w.skip(VOTING + MIN).await;
    let due = scales_view(&w, "get_due", json!({})).await;
    assert_eq!(due.as_array().unwrap().len(), 6);

    let (s, res) = finalized(&w, slices, "SetSlices (Light, 25 of 45, 20 yes)").await?;
    assert_eq!(s, "Executed");
    assert_eq!(named(&events(&res), "settings_changed").len(), 1);
    let config: Value = w.dragon.view("get_config").await?.json()?;
    assert_eq!(config["strategy"]["cap_bps"], 100, "SetSlices reached the Dragon");

    let (s, res) = finalized(&w, strict_grant, "Grant (Strict, 30 of 45 turnout, 24 yes = 80%)").await?;
    assert_eq!(s, "Executed", "two thirds turnout and 80% yes pass the Strict class");
    assert_eq!(status(&res)["status"], "Executing");
    assert_eq!(w.balance(&w.wyrm, fund_a.id().as_str()).await, grant_a);

    let (s, res) = finalized(&w, hunt, "Hunt on rejected (Strict, 34 of 43 yes = 79%)").await?;
    assert_eq!(s, "Rejected", "79% yes is under the Strict 80%");
    assert!(untouched(&w, &res), "a rejected order never reaches the Dragon");
    assert_eq!(
        w.dragon.view("get_config").await?.json::<Value>()?["strategy"]["hunt"],
        Value::Null
    );

    let (s, res) = finalized(&w, burned, "Burn to ash (Light)").await?;
    assert_eq!(s, "Executed");
    assert!(res.receipt_failures().is_empty(), "{}", failure_text(&res));
    assert_eq!(u(&named(&events(&res), "burn_paid")[0]["amount"]), burn);
    assert_eq!(
        w.balance(&w.wyrm, w.ash.id().as_str()).await,
        burn,
        "the Burn reached ash inside 150 Tgas"
    );

    let (s, res) = finalized(&w, signal, "Signal").await?;
    assert_eq!(s, "Carried");
    assert!(untouched(&w, &res), "a Signal calls nothing");

    let (s, _) = finalized(&w, second_grant, "second Grant (Strict, 40 yes)").await?;
    assert_eq!(s, "Executed");
    assert_eq!(w.balance(&w.wyrm, fund_b.id().as_str()).await, grant_b);

    let third = council.propose(&w, 1, grant(&fund_c, grant_a)).await?;
    council.tally(&w, third, 40, 0).await?;
    w.skip(VOTING + MIN).await;
    let (s, res) = finalized(&w, third, "third Grant held by the 30-day window").await?;
    let error = s["LeashHeld"]["error"]
        .as_str()
        .unwrap_or_else(|| panic!("LeashHeld, got {s}"));
    assert!(
        error.contains("grants in any 30 days are at most 100 bps of the hoard")
            && error.contains(&format!("{} already sent", grant_a + grant_b)),
        "the Dragon's reason names the window: {error}"
    );
    assert_eq!(named(&events(&res), "order_refused").len(), 1);
    assert_eq!(w.balance(&w.wyrm, fund_c.id().as_str()).await, 0);
    let h = w.state().await["hoard"].clone();
    assert_eq!(u(&h["wyrm"]), HOARD - grant_a - grant_b - burn);
    assert_eq!(u(&h["granted"]), grant_a + grant_b);
    Ok(())
}
