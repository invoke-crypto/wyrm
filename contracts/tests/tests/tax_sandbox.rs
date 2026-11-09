//! The coin's 4% tax as the Dragon meets it on real wasm (design.md section 6): a buy delivers the net and met
//! `min_out` on the gross, a sell sends the gross and quotes the net, a failed sell comes back whole, the dev buy is a
//! donation, and grants and burns are wallet transfers that pay no tax.
mod dragon_world;
mod v6_world;

use dragon_world::*;
use near_workspaces::result::ExecutionFinalResult;
use serde_json::{json, Value};
use v6_world::*;

const TAX_BPS: u128 = 400;
const SLIPPAGE_BPS: u128 = 150;
/// About 3.3 times the average cost: over the 2.5x sell line, and a spike that stops buys.
const SPIKE_WYRM: u128 = POOL_WYRM * 10 / 33;

fn net(gross: u128) -> u128 {
    gross - gross * TAX_BPS / 10_000
}

/// `q` less `bps`, split so `q * bps` cannot overflow, as the Dragon computes it.
fn less_bps(q: u128, bps: u128) -> u128 {
    q / 10_000 * (10_000 - bps) + q % 10_000 * (10_000 - bps) / 10_000
}

fn expected_net(q: u128) -> u128 {
    less_bps(q, TAX_BPS)
}

fn less_slippage(q: u128) -> u128 {
    less_bps(q, SLIPPAGE_BPS)
}

fn transfers(res: &ExecutionFinalResult, w: &World) -> Vec<(String, String, u128, Option<String>)> {
    res.receipt_outcomes()
        .iter()
        .filter(|o| o.executor_id == w.wyrm)
        .flat_map(|o| o.logs.iter())
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str::<Value>(j).unwrap())
        .filter(|v| v["standard"] == "nep141" && v["event"] == "ft_transfer")
        .flat_map(|v| v["data"].as_array().unwrap().clone())
        .map(|d| {
            (
                d["old_owner_id"].as_str().unwrap().to_string(),
                d["new_owner_id"].as_str().unwrap().to_string(),
                u(&d["amount"]),
                d["memo"].as_str().map(String::from),
            )
        })
        .collect()
}

fn tax_paid(t: &[(String, String, u128, Option<String>)]) -> u128 {
    t.iter().filter(|x| x.3.as_deref() == Some("tax")).map(|x| x.2).sum()
}

async fn hoard(w: &World) -> Value {
    w.state().await["hoard"].clone()
}

async fn bought(w: &World) -> (Value, ExecutionFinalResult) {
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    w.skip_to(breath_at).await;
    let res = w.poke().await.unwrap();
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{}",
        failure_text(&res)
    );
    let slice = named(&events(&res), "breath_slice");
    assert_eq!(slice.len(), 1, "the breath bought one slice");
    (slice[0].clone(), res)
}

async fn ready_to_sell(w: &World) {
    w.set_pool(POOL_NEAR, SPIKE_WYRM).await.unwrap();
    w.skip(61 * SEC).await;
    let evs = w.poke_ok().await;
    assert!(named(&evs, "hunt_slice").is_empty() && named(&evs, "breath_slice").is_empty());
    let line = json!({ "SetPosture": { "hunt": { "sell_multiple_bps": 25_000, "max_sell_bps": 1_000 } } });
    assert_eq!(w.verdict(line).await, json!("Executed"));
    w.skip(61 * SEC).await;
}

#[tokio::test]
async fn a_taxed_buy_books_the_net_and_met_min_out_on_the_gross() -> anyhow::Result<()> {
    let w = setup(300 * NEAR).await?;
    let (slice, res) = bought(&w).await;
    let quoted = u(&slice["quoted_out"]);
    let min_out = u(&slice["min_out"]);
    assert_eq!(
        min_out,
        less_slippage(quoted),
        "min_out is the gross quote less 150 bps"
    );
    let t = transfers(&res, &w);
    let dragon = w.dragon.id().as_str();
    let delivered: u128 = t.iter().filter(|x| x.1 == dragon).map(|x| x.2).sum();
    let gross = delivered + tax_paid(&t);
    assert!(
        gross >= min_out,
        "DCL's amount_out {gross} met min_out {min_out} before the tax"
    );
    assert_eq!(delivered, net(gross), "the Dragon received 96% of amount_out");
    assert_eq!(w.balance(&w.wyrm, dragon).await, delivered);
    w.skip(61 * SEC).await;
    w.poke_ok().await;
    let h = hoard(&w).await;
    assert_eq!(
        u(&h["wyrm_bought"]),
        expected_net(quoted),
        "booked as bought: the expected net of the quote"
    );
    assert_eq!(u(&h["wyrm"]), delivered);
    assert_eq!(
        u(&h["donated"]),
        delivered - expected_net(quoted),
        "anything above the expected net is a gift"
    );
    Ok(())
}

#[tokio::test]
async fn a_taxed_sell_quotes_the_net_and_clears_min_out() -> anyhow::Result<()> {
    let w = setup(300 * NEAR).await?;
    let _ = bought(&w).await;
    ready_to_sell(&w).await;
    let held = u(&hoard(&w).await["wyrm"]);
    let res = w.poke().await?;
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{}",
        failure_text(&res)
    );
    gas_line("poke taxed Hunt sell chain", &res);
    let sold = named(&events(&res), "hunt_slice");
    assert_eq!(sold.len(), 1);
    let gross = u(&sold[0]["amount_in"]);
    assert_eq!(gross, held / 10, "a tenth of the hoard per day, gross");
    let t = transfers(&res, &w);
    let to_dcl: u128 = t.iter().filter(|x| x.1 == w.dcl.id().as_str()).map(|x| x.2).sum();
    assert_eq!(to_dcl, net(gross), "DCL swapped the net");
    assert_eq!(tax_paid(&t), gross - net(gross));
    let quoted = u(&sold[0]["quoted_out"]);
    w.set_pool(POOL_NEAR, SPIKE_WYRM).await?;
    let quote = |amount: u128| {
        let (dcl, args) = (
            w.dcl.clone(),
            json!({ "pool_ids": [w.pool_id], "input_token": w.wyrm,
            "output_token": w.wrap.id(), "input_amount": amount.to_string() }),
        );
        async move {
            u(&dcl
                .view("quote")
                .args_json(args)
                .await
                .unwrap()
                .json::<Value>()
                .unwrap()["amount"])
        }
    };
    assert_eq!(
        quoted,
        quote(expected_net(gross)).await,
        "the sell was quoted on sell_quote_input, the net rounded down, at the pool it met"
    );
    assert!(quote(gross).await > quoted);
    let min_out = u(&sold[0]["min_out"]);
    assert_eq!(min_out, less_slippage(quoted), "min_out is the net quote less 150 bps");
    let wrapped: Vec<Value> = res
        .receipt_outcomes()
        .iter()
        .filter(|o| o.executor_id == *w.wrap.id())
        .flat_map(|o| o.logs.iter())
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str::<Value>(j).unwrap())
        .filter(|v| v["event"] == "ft_transfer")
        .flat_map(|v| v["data"].as_array().unwrap().clone())
        .filter(|d| d["new_owner_id"] == w.dragon.id().as_str())
        .collect();
    assert_eq!(
        wrapped.len(),
        1,
        "the wNEAR out arrived as an ft_transfer, unwrapping skipped"
    );
    assert!(u(&wrapped[0]["amount"]) >= min_out, "the wNEAR out cleared min_out");
    Ok(())
}

#[tokio::test]
async fn a_failed_taxed_sell_restores_the_gross() -> anyhow::Result<()> {
    let w = setup(300 * NEAR).await?;
    let _ = bought(&w).await;
    ready_to_sell(&w).await;
    let held = w.balance(&w.wyrm, w.dragon.id().as_str()).await;
    w.dcl_switch("set_fail_swaps", true).await?;
    let res = w.poke().await?;
    assert!(res.is_success(), "{}", failure_text(&res));
    let evs = events(&res);
    assert_eq!(named(&evs, "hunt_slice").len(), 0, "{evs:?}");
    let failed = named(&evs, "swap_failed");
    assert_eq!(failed.len(), 1, "{evs:?}");
    assert_eq!(failed[0]["slice"], "Hunt");
    let gross = u(&failed[0]["amount_in"]);
    let t = transfers(&res, &w);
    assert_eq!(tax_paid(&t), gross - net(gross), "the sell paid its tax on the way in");
    let back: u128 = t.iter().filter(|x| x.1 == w.dragon.id().as_str()).map(|x| x.2).sum();
    assert_eq!(back, gross, "the net and the tax both came back");
    assert_eq!(
        w.balance(&w.wyrm, w.dragon.id().as_str()).await,
        held,
        "the gross is restored"
    );
    w.dcl_switch("set_fail_swaps", false).await?;
    w.skip(61 * SEC).await;
    let evs = w.poke_ok().await;
    let sold: u128 = named(&evs, "hunt_slice").iter().map(|s| u(&s["amount_in"])).sum();
    let h = hoard(&w).await;
    assert_eq!(
        u(&h["wyrm"]) + sold,
        held,
        "the hoard still holds every token it did not sell since"
    );
    Ok(())
}

#[tokio::test]
async fn a_dev_buy_arrives_as_donated() -> anyhow::Result<()> {
    let mut w = setup_with(Options {
        unlaunched: true,
        ..Options::new(300 * NEAR)
    })
    .await?;
    let dev_buy = NEAR;
    let mut c = coin("WYRM");
    c["dev_buy"] = json!(dev_buy.to_string());
    let launch = w.launch_coin(&c, LAUNCH_DEPOSIT + dev_buy, LAUNCH_GAS).await;
    assert!(launch.is_success(), "{}", failure_text(&launch));
    gas_line("launch with a 1 NEAR dev buy", &launch);
    let id = launch_started(&launch).expect("launched");
    let mut resumes = 0;
    while record(&w, id).await["step"] != "Done" {
        resumes += 1;
        assert!(resumes <= 3, "Done within three resumes: {}", record(&w, id).await);
        let res = w
            .keeper
            .call(w.dragon.id(), "nearly_resume")
            .args_json(json!({ "launch_id": id.to_string() }))
            .gas(near_workspaces::types::Gas::from_tgas(300))
            .transact()
            .await?;
        assert!(res.is_success(), "{}", failure_text(&res));
    }
    println!("dev-buy launch reached Done after {resumes} resume(s)");
    let r = record(&w, id).await;
    w.wyrm = r["token"].as_str().unwrap().parse()?;
    let arrived = w.balance(&w.wyrm, w.dragon.id().as_str()).await;
    let tax = w.balance(&w.wyrm, w.wyrm.as_str()).await;
    assert!(arrived > 0, "the dev buy reached its creator, the Dragon");
    assert_eq!(
        tax,
        (arrived + tax) * TAX_BPS / 10_000,
        "and paid the 4% buy tax on the way: {arrived} net, {tax} tax"
    );
    let bind = w.bind_launch(id, BIND_GAS).await;
    assert_eq!(named(&events(&bind), "bound").len(), 1, "{}", failure_text(&bind));
    w.pool_id = coin_of(&w).await["pool_id"].as_str().unwrap().into();
    w.set_pool(POOL_NEAR, POOL_WYRM).await?;
    w.skip(61 * SEC).await;
    w.poke_ok().await;
    let h = hoard(&w).await;
    assert_eq!(u(&h["donated"]), arrived, "booked as donated at the first observation");
    assert_eq!((u(&h["wyrm_bought"]), u(&h["near_spent"])), (0, 0));
    assert_eq!(u(&h["wyrm"]), arrived);
    Ok(())
}

#[tokio::test]
async fn a_grant_and_a_burn_arrive_untaxed() -> anyhow::Result<()> {
    let w = setup(50 * NEAR).await?;
    let hoarded = 1_000_000 * WYRM;
    w.donate(hoarded).await?;
    w.poke_ok().await;
    let ally = w.worker.dev_create_account().await?;
    w.mint(&w.wyrm, ally.id().as_str(), 0).await?;
    let amount = hoarded / 400;
    let grant = w
        .obey(json!({ "Grant": { "to": ally.id(), "amount": amount.to_string() } }))
        .await;
    assert_eq!(grant.clone().json::<Value>()?, json!("Executed"));
    assert_eq!(tax_paid(&transfers(&grant, &w)), 0);
    assert_eq!(
        w.balance(&w.wyrm, ally.id().as_str()).await,
        amount,
        "the grant arrived whole"
    );

    let burn = w.obey(json!({ "Burn": { "amount": amount.to_string() } })).await;
    assert_eq!(burn.clone().json::<Value>()?, json!("Executed"));
    assert_eq!(tax_paid(&transfers(&burn, &w)), 0);
    assert_eq!(
        w.balance(&w.wyrm, w.ash.id().as_str()).await,
        amount,
        "the burn arrived whole"
    );
    assert_eq!(w.balance(&w.wyrm, w.wyrm.as_str()).await, 0, "the token kept no tax");
    assert_eq!(w.balance(&w.wyrm, w.dragon.id().as_str()).await, hoarded - 2 * amount);
    Ok(())
}

#[tokio::test]
async fn a_first_burn_registers_ash_in_its_own_batch() -> anyhow::Result<()> {
    let w = setup(50 * NEAR).await?;
    let hoarded = 1_000_000 * WYRM;
    w.donate(hoarded).await?;
    w.poke_ok().await;
    let registered = || async {
        let v: Value = w
            .worker
            .view(&w.wyrm, "storage_balance_of")
            .args_json(json!({ "account_id": w.ash.id() }))
            .await
            .unwrap()
            .json()
            .unwrap();
        !v.is_null()
    };
    assert!(!registered().await, "ash starts unregistered on the coin");
    let amount = hoarded / 1_000;
    let (dragon_near, ash_near) = (near_of(&w, w.dragon.id()).await, near_of(&w, w.ash.id()).await);
    let first = w.obey(json!({ "Burn": { "amount": amount.to_string() } })).await;
    assert_eq!(first.clone().json::<Value>()?, json!("Executed"));
    gas_line("obey first Burn, registering ash", &first);
    assert!(registered().await, "the Burn registered ash");
    assert_eq!(w.balance(&w.wyrm, w.ash.id().as_str()).await, amount);
    let on_coin = first
        .receipt_outcomes()
        .iter()
        .filter(|o| o.executor_id == w.wyrm)
        .count();
    assert_eq!(on_coin, 1, "storage_deposit and ft_transfer in one batch on the coin");
    assert_eq!(near_of(&w, w.ash.id()).await, ash_near, "ash's NEAR is untouched");
    assert!(
        near_of(&w, w.dragon.id()).await < dragon_near,
        "the Dragon paid ash's storage"
    );

    let second = w.obey(json!({ "Burn": { "amount": amount.to_string() } })).await;
    assert_eq!(second.clone().json::<Value>()?, json!("Executed"));
    gas_line("obey second Burn to a registered ash", &second);
    assert!(
        second.receipt_failures().is_empty(),
        "a repeat storage_deposit is refunded, not refused"
    );
    assert_eq!(w.balance(&w.wyrm, w.ash.id().as_str()).await, 2 * amount);
    Ok(())
}
