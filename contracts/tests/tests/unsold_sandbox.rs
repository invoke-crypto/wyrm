//! Unsold Scales on real wasm (design.md sections 5e and 7): each mint sweeps the new Scale's vault to the Dragon,
//! `close_sale` tells the tithe the first unsold id once, and every later round pays the retired shares to the Dragon
//! and nothing to the retired vaults.
mod dragon_world;
mod v6_world;

use dragon_world::*;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::AccountId;
use serde_json::{json, Value};
use v6_world::*;

/// A vault earns 30% of the gas its own calls burn, so its balance drifts by less than this.
const DUST: u128 = NEAR / 10_000;

fn vault(w: &World, i: u32) -> AccountId {
    format!("{i:03}.{}", w.scales.id()).parse().unwrap()
}

async fn vault_balances(w: &World) -> Vec<u128> {
    let mut b = vec![];
    for i in 1..=100 {
        b.push(near_of(w, &vault(w, i)).await);
    }
    b
}

async fn round(w: &World, near: u128) -> anyhow::Result<(Value, u128)> {
    w.worker
        .root_account()?
        .transfer_near(w.tithe.id(), NearToken::from_yoctonear(near))
        .await?
        .into_result()?;
    let stats: Value = w.keeper.view(w.tithe.id(), "get_stats").await?.json()?;
    let free = u(&stats["distributable"]);
    let res = w
        .keeper
        .call(w.tithe.id(), "distribute")
        .gas(Gas::from_tgas(300))
        .transact()
        .await?;
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{}",
        failure_text(&res)
    );
    gas_line("tithe distribute", &res);
    let round = res
        .logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str::<Value>(j).unwrap())
        .find(|v| v["event"] == "tithe_round")
        .expect("a tithe_round event");
    Ok((round["data"][0].clone(), free))
}

#[tokio::test]
async fn each_mint_sweeps_income_from_before_the_sale_to_the_dragon() -> anyhow::Result<()> {
    let w = setup(50 * NEAR).await?;
    let (paid, _) = round(&w, 100 * NEAR).await?;
    let per_scale = u(&paid["per_scale"]);
    let held = near_of(&w, &vault(&w, 1)).await;
    let dragon = near_of(&w, w.dragon.id()).await;

    let c = Council::buy(&w, &[1]).await?;
    assert_eq!(c.held[0], vec!["1".to_string()]);
    w.settle().await;
    let gained = near_of(&w, w.dragon.id()).await - dragon;
    let price_share = PRICE * 9 / 10;
    assert!(
        gained >= price_share + per_scale && gained < price_share + per_scale + DUST,
        "the Dragon got 90% of the price and the vault's round: {gained}"
    );
    assert!(
        held - near_of(&w, &vault(&w, 1)).await + DUST >= per_scale,
        "the vault was emptied"
    );
    let claimable: String = w.keeper.view(&vault(&w, 1), "get_claimable").await?.json()?;
    assert!(
        claimable.parse::<u128>()? < DUST,
        "the buyer collects nothing from before the sale"
    );
    let scale = scales_view(&w, "get_scale", json!({ "token_id": "1" })).await;
    assert_eq!(
        scale["claiming_since"],
        Value::Null,
        "the sweep settled and released the lock"
    );
    let sweepable = scales_view(&w, "get_sweepable", json!({})).await;
    let sweepable: Vec<&str> = sweepable
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(sweepable.len(), 89, "every unminted sale vault, 2 to 90");
    assert!(!sweepable.contains(&"1") && sweepable.contains(&"2") && !sweepable.contains(&"91"));
    Ok(())
}

#[tokio::test]
async fn close_sale_retires_the_unsold_suffix_on_the_tithe_once() -> anyhow::Result<()> {
    let w = setup(50 * NEAR).await?;
    Council::buy(&w, &[3]).await?;
    round(&w, 100 * NEAR).await?;
    let early = close_sale(&w).await?;
    assert!(
        named(&events(&early), "sale_closed").is_empty(),
        "the sale runs its 14 days"
    );
    assert_eq!(scales_view(&w, "get_sale", json!({})).await["state"], "Open");

    w.skip(SALE + MIN).await;
    let res = close_sale(&w).await?;
    gas_line("close_sale telling the tithe", &res);
    let closed = res
        .logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str::<Value>(j).unwrap())
        .filter(|v| v["event"] == "sale_closed" || v["event"] == "tithe_told")
        .map(|v| (v["event"].as_str().unwrap().to_string(), v["data"][0].clone()))
        .collect::<Vec<_>>();
    assert_eq!(closed.len(), 2, "{closed:?}");
    assert_eq!(closed[0].1["sold"], 3);
    assert_eq!(closed[0].1["retired_from"], 4);
    assert_eq!(closed[1].1["retired_from"], 4);
    let sale = scales_view(&w, "get_sale", json!({})).await;
    assert_eq!(
        (sale["state"].clone(), sale["tithe_told"].clone()),
        (json!("Closed"), json!(true))
    );
    let stats: Value = w.keeper.view(w.tithe.id(), "get_stats").await?.json()?;
    assert_eq!(stats["retired_from"], 4);
    let again = close_sale(&w).await?;
    assert!(
        again.receipt_outcomes().iter().all(|o| o.executor_id != *w.tithe.id()),
        "told once"
    );

    let retired: Vec<String> = (4..24).map(|i| i.to_string()).collect();
    let before = vault_balances(&w).await;
    let dragon = near_of(&w, w.dragon.id()).await;
    let res = w
        .keeper
        .call(w.scales.id(), "sweep")
        .args_json(json!({ "token_ids": retired }))
        .gas(Gas::from_tgas(300))
        .transact()
        .await?;
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{}",
        failure_text(&res)
    );
    gas_line("sweep of 20 retired vaults", &res);
    let after = vault_balances(&w).await;
    let swept: u128 = (3..23).map(|i| before[i] - after[i]).sum();
    let gained = near_of(&w, w.dragon.id()).await - dragon;
    assert!(
        gained >= swept && gained - swept < 20 * DUST,
        "what the retired vaults held went to the Dragon, less each vault's gas reward: {gained} against {swept}"
    );
    assert!(swept >= 20 * NEAR / 200, "each held its hundredth of the round's half");
    Ok(())
}

#[tokio::test]
async fn a_round_after_retirement_pays_the_dragon_and_skips_retired_vaults() -> anyhow::Result<()> {
    let w = setup(50 * NEAR).await?;
    Council::buy(&w, &[3]).await?;
    w.skip(SALE + MIN).await;
    let _ = close_sale(&w).await?;
    let before = vault_balances(&w).await;
    let dragon = near_of(&w, w.dragon.id()).await;
    let (paid, free) = round(&w, 100 * NEAR + 12_345).await?;
    let (to_dragon, per_scale) = (u(&paid["to_dragon"]), u(&paid["per_scale"]));
    assert_eq!(per_scale, (free - free / 2) / 100);
    assert_eq!(
        to_dragon,
        free / 2 + per_scale * 87,
        "half plus the 87 retired hundredths"
    );
    assert_eq!(near_of(&w, w.dragon.id()).await - dragon, to_dragon);
    let after = vault_balances(&w).await;
    for i in 1..=100usize {
        let got = after[i - 1] - before[i - 1];
        if (4..=90).contains(&i) {
            assert!(got < DUST, "retired vault {i} got nothing: {got}");
        } else {
            assert_eq!(got, per_scale, "vault {i} got its hundredth");
        }
    }
    let stats: Value = w.keeper.view(w.tithe.id(), "get_stats").await?.json()?;
    assert_eq!(u(&stats["paid_scales"]), per_scale * 13);
    Ok(())
}
