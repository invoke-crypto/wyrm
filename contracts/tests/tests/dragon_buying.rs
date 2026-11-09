//! The Dragon's buying against mock-dcl and the coin mock-nearly launched for it: the breath, the slice cap, dips,
//! spikes, refunds and late delivery. Every scenario runs twice, on the coin with Nearly's 4% tax (as every launch
//! has it) and with the token's tax switched off. Expected values are re-derived from each event's own fields.
mod dragon_world;

use dragon_world::*;
use near_workspaces::result::ExecutionFinalResult;
use serde_json::{json, Value};

const TAX_BPS: u128 = 400;
const SLIPPAGE_BPS: u128 = 150;

fn slices(evs: &[(String, Value)]) -> Vec<(String, Value)> {
    evs.iter().filter(|(n, _)| n.ends_with("_slice")).cloned().collect()
}

/// The slice cap re-derived from the event: the swap got at least 10_000 - cap_bps of the 1 NEAR quote's price.
fn passes_cap(s: &Value) -> bool {
    within_cap(u(&s["amount_in"]), u(&s["quoted_out"]), s)
}

/// Whether `a` NEAR quoted at `q` WYRM is inside the cap of slice event `s`, against its 1 NEAR price.
fn within_cap(a: u128, q: u128, s: &Value) -> bool {
    let (tpn, cap) = (u(&s["tokens_per_near"]), s["cap_bps"].as_u64().unwrap());
    q * 10_000 >= a / 1_000_000 * (tpn / 1_000_000) / 1_000_000_000_000 * (10_000 - cap as u128)
}

/// What a buy quoted at `quoted` is expected to deliver: the net after the buy tax.
fn expected_net(quoted: u128) -> u128 {
    quoted / 10_000 * (10_000 - TAX_BPS) + quoted % 10_000 * (10_000 - TAX_BPS) / 10_000
}

/// Gas burnt by each of the Dragon's receipts in a poke that traded, in execution order.
fn gas_report(res: &ExecutionFinalResult, dragon: &str) -> Vec<(String, u64)> {
    let names = ["poke", "on_observe", "on_quote", "on_swap"];
    let mine: Vec<_> = res
        .receipt_outcomes()
        .iter()
        .filter(|o| o.executor_id.as_str() == dragon)
        .collect();
    assert!(
        mine[1].logs.iter().any(|l| l.contains("\"observe\"")),
        "second Dragon receipt is on_observe"
    );
    assert!(
        mine.last().unwrap().logs.iter().any(|l| l.contains("_slice\"")),
        "last Dragon receipt is on_swap"
    );
    mine.iter()
        .zip(names)
        .map(|(o, n)| (n.to_string(), o.gas_burnt.as_gas() / 1_000_000_000))
        .collect()
}

async fn world(near: u128, taxed: bool) -> anyhow::Result<World> {
    setup_with(Options {
        taxed,
        ..Options::new(near)
    })
    .await
}

async fn the_breath_buys_only_from_its_minute_within_budget_cap_and_gap(taxed: bool) -> anyhow::Result<()> {
    let w = world(300 * NEAR, taxed).await?;
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    w.skip_to(breath_at - 4 * MIN).await;
    for _ in 0..3 {
        let evs = w.poke_ok().await;
        assert!(slices(&evs).is_empty(), "no slice before the breath minute");
        assert!(named(&evs, "observe")[0]["decision"]
            .as_str()
            .unwrap()
            .contains("before the breath"));
        w.skip(70 * SEC).await;
    }
    w.skip_to(breath_at).await;

    // The scripted price path: the test resets the pool before every poke, so the Dragon's own buys never move it.
    let path = [100u128, 102, 99, 101, 98, 100, 103, 100, 99, 101, 100, 102];
    let mut seen = vec![];
    let mut gas = None;
    for k in path {
        w.set_pool(POOL_NEAR, POOL_WYRM * k / 100).await?;
        let res = w.poke().await?;
        assert!(res.is_success(), "{:?}", res.failures());
        let evs = events(&res);
        if !slices(&evs).is_empty() && gas.is_none() {
            gas = Some((
                gas_report(&res, w.dragon.id().as_str()),
                res.total_gas_burnt.as_gas() / 1_000_000_000,
            ));
        }
        seen.extend(slices(&evs));
        w.skip(30 * SEC).await;
    }
    let state = w.state().await;
    let opening = u(&state["day"]["opening_reserve"]);
    let budget = opening * 1_000 / 10_000;
    assert!(seen.len() >= 2, "the budget takes capped slices, got {}", seen.len());
    let mut spent = 0;
    let mut last_at = 0u64;
    let mut expected = 0;
    for (name, s) in &seen {
        assert_eq!(name, "breath_slice");
        let at: u64 = s["at"].as_str().unwrap().parse()?;
        assert!(at >= breath_at, "slice at {at} before the breath minute {breath_at}");
        assert!(
            last_at == 0 || at - last_at >= MIN,
            "slices {}s apart",
            (at - last_at) / SEC
        );
        last_at = at;
        assert!(passes_cap(s), "slice over the impact cap: {s}");
        assert_eq!(u(&s["budget"]), budget);
        assert_eq!(s["breath_at"].as_str().unwrap(), breath_at.to_string());
        assert!(s["reference"].is_string());
        let quoted = u(&s["quoted_out"]);
        assert_eq!(
            u(&s["min_out"]),
            quoted / 10_000 * (10_000 - SLIPPAGE_BPS) + quoted % 10_000 * (10_000 - SLIPPAGE_BPS) / 10_000,
            "min_out is the gross quote less 150 bps, taxed or not"
        );
        expected += expected_net(quoted);
        spent += u(&s["amount_in"]);
    }
    assert!(spent <= budget, "spent {spent} over the day's budget {budget}");
    assert!(
        budget - spent < 10_000_000_000_000_000_000_000,
        "the breath spends its budget down to dust"
    );
    assert_eq!(u(&state["day"]["spent"]), spent);
    assert_eq!(state["mood"], "Gorged");
    let held = w.balance(&w.wyrm, w.dragon.id().as_str()).await;
    let next = w.poke_ok().await;
    assert!(slices(&next).is_empty());
    let h = w.state().await["hoard"].clone();
    assert_eq!(
        u(&h["wyrm"]),
        held,
        "the next observation reconciles the hoard to the balance"
    );
    assert_eq!(u(&h["near_spent"]), spent);
    assert_eq!(
        u(&h["wyrm_bought"]),
        expected,
        "bought is the expected net of each quote"
    );
    if taxed {
        assert!(
            held >= expected && held - expected <= seen.len() as u128,
            "the taxed buys delivered 96% of their quotes: {held} against {expected}"
        );
        println!(
            "TAXED bought {expected} of quoted {} (96%)",
            expected * 10_000 / (10_000 - TAX_BPS)
        );
    } else {
        assert_eq!(
            u(&h["donated"]),
            held - expected,
            "untaxed, the 4% the Dragon expected to lose is a gift"
        );
    }
    let (steps, total) = gas.unwrap();
    println!("GAS per step of a trading poke (Ggas burnt): {steps:?}; whole transaction {total} Ggas");
    Ok(())
}

async fn liquidity_far_from_the_price_does_not_grow_slices(taxed: bool) -> anyhow::Result<()> {
    let w = world(600 * NEAR, taxed).await?;
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    w.skip_to(breath_at).await;
    let first = slices(&w.poke_ok().await);
    w.dcl
        .call("add_far_liquidity")
        .args_json(
            json!({ "pool_id": w.pool_id, "x": (100_000 * NEAR).to_string(), "y": (100_000 * NEAR).to_string() }),
        )
        .transact()
        .await?
        .into_result()?;
    w.set_pool(POOL_NEAR, POOL_WYRM).await?;
    w.skip(61 * SEC).await;
    let evs = w.poke_ok().await;
    let depth = u(&named(&evs, "observe")[0]["depth_near"]);
    assert!(depth > 100_000 * NEAR, "get_pool reports the far liquidity as depth");
    let second = slices(&evs);
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    // Both slices are sized by price impact. The second starts from a smaller remaining budget, so its halving ladder
    // lands on a different step; what must hold is that the next step up would breach the cap at today's price.
    let s = &second[0].1;
    let a = u(&s["amount_in"]);
    assert!(passes_cap(s));
    assert!(
        a < u(&s["budget"]) - u(&first[0].1["amount_in"]),
        "the cap, not the budget, bounded the slice"
    );
    w.set_pool(POOL_NEAR, POOL_WYRM).await?;
    let (wrap, wyrm) = (w.wrap.id().to_string(), w.wyrm.to_string());
    let doubled: Value = w
        .dcl
        .view("quote")
        .args_json(json!({ "pool_ids": [w.pool_id], "input_token": wrap, "output_token": wyrm, "input_amount": (2 * a).to_string() }))
        .await?
        .json()?;
    assert!(
        !within_cap(2 * a, u(&doubled["amount"]), s),
        "with a hundredfold depth, twice the slice still breaches the impact cap"
    );
    assert!(a < POOL_NEAR * 200 / 10_000, "the slice is held to the impact cap");
    Ok(())
}

async fn a_confirmed_dip_buys_and_an_unconfirmed_one_does_not(taxed: bool) -> anyhow::Result<()> {
    let w = setup_with(Options {
        preset: "dip-hunter",
        taxed,
        ..Options::new(300 * NEAR)
    })
    .await?;
    w.day_with_breath(8 * HOUR, DAY - HOUR).await;
    for _ in 0..6 {
        w.skip(10 * MIN).await;
        assert!(slices(&w.poke_ok().await).is_empty());
    }
    // 1.15 times the tokens per NEAR: beyond dip-hunter's 1,000 bps step, short of 2,000.
    let dip = POOL_WYRM * 115 / 100;
    w.set_pool(POOL_NEAR, dip).await?;
    for n in 1..=2 {
        w.skip(5 * MIN).await;
        assert!(
            slices(&w.poke_ok().await).is_empty(),
            "sample {n} of the dip does not buy"
        );
        let state = w.state().await;
        assert_eq!(state["mood"], "Stirring");
        assert_eq!(state["dip"]["step"], 1);
        assert_eq!(state["dip"]["samples"], n);
        assert_eq!(state["dip"]["confirmed"], false);
    }
    w.set_pool(POOL_NEAR, POOL_WYRM).await?;
    w.skip(5 * MIN).await;
    assert!(
        slices(&w.poke_ok().await).is_empty(),
        "the dip ended after two samples over five minutes"
    );
    assert!(w.state().await["dip"].is_null());

    w.set_pool(POOL_NEAR, dip).await?;
    let mut first_at = 0u64;
    let mut bought = vec![];
    for n in 1..=3 {
        w.skip(5 * MIN).await;
        let evs = w.poke_ok().await;
        if n == 1 {
            first_at = named(&evs, "observe")[0]["at"].as_str().unwrap().parse()?;
        }
        bought = slices(&evs);
        assert_eq!(
            bought.is_empty(),
            n < 3,
            "only the third sample, ten minutes in, confirms"
        );
    }
    let (name, s) = &bought[0];
    assert_eq!(name, "dip_slice");
    assert_eq!(s["step_bps"], 1_000);
    assert_eq!(s["confirmations"], 3);
    assert_eq!(s["confirmed_since"].as_str().unwrap(), first_at.to_string());
    let opening = u(&w.state().await["day"]["opening_reserve"]);
    assert_eq!(
        u(&s["budget"]),
        opening * 1_000 / 10_000,
        "before the breath only the step's extra is open"
    );
    assert!(passes_cap(s));
    Ok(())
}

async fn no_buy_while_spot_is_over_fifteen_percent_above_the_reference(taxed: bool) -> anyhow::Result<()> {
    let w = world(300 * NEAR, taxed).await?;
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    w.skip_to(breath_at).await;
    // 0.8 times the tokens per NEAR is a price 25% higher.
    w.set_pool(POOL_NEAR, POOL_WYRM * 8 / 10).await?;
    let evs = w.poke_ok().await;
    assert!(slices(&evs).is_empty());
    assert_eq!(named(&evs, "observe")[0]["decision"], "rest: price spike");
    assert_ne!(w.state().await["mood"], "Breathing");
    w.set_pool(POOL_NEAR, POOL_WYRM).await?;
    w.skip(61 * SEC).await;
    assert_eq!(
        slices(&w.poke_ok().await).len(),
        1,
        "back at the reference, the breath resumes"
    );
    Ok(())
}

async fn a_refund_keeps_the_wnear_and_the_budget_and_the_next_buy_spends_that_wnear_first(
    taxed: bool,
) -> anyhow::Result<()> {
    let w = world(300 * NEAR, taxed).await?;
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    w.skip_to(breath_at).await;
    w.dcl_switch("set_fail_swaps", true).await?;
    let evs = w.poke_ok().await;
    let failed = named(&evs, "swap_failed");
    assert_eq!(failed.len(), 1);
    let amount = u(&failed[0]["amount_in"]);
    let dragon = w.dragon.id().as_str();
    assert_eq!(
        w.balance(w.wrap.id(), dragon).await,
        amount,
        "the refund left wNEAR in the Dragon"
    );
    let state = w.state().await;
    assert_eq!(u(&state["day"]["spent"]), 0, "the refund gave the budget back");
    assert_eq!(u(&state["reserve_wnear"]), amount);
    assert!(w.log().await.iter().any(|e| e["event"] == "swap_failed"));

    w.dcl_switch("set_fail_swaps", false).await?;
    w.skip(61 * SEC).await;
    let near_before = w.dragon.view_account().await?.balance.as_yoctonear();
    let evs = w.poke_ok().await;
    let s = &slices(&evs)[0].1;
    assert_eq!(u(&s["amount_in"]), amount);
    assert_eq!(w.balance(w.wrap.id(), dragon).await, 0, "the slice spent the wNEAR");
    let near_after = w.dragon.view_account().await?.balance.as_yoctonear();
    assert!(
        near_after + 1 >= near_before,
        "no NEAR was wrapped: {near_before} -> {near_after}"
    );
    Ok(())
}

async fn late_output_is_reconciled_and_a_donation_meanwhile_does_not_lower_the_average_cost(
    taxed: bool,
) -> anyhow::Result<()> {
    let w = world(300 * NEAR, taxed).await?;
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    let dragon = w.dragon.id().as_str().to_string();
    let before = w.balance(&w.wyrm, &dragon).await;
    // A transfer with ft_transfer_call is a donation, not a purchase. Wallet to wallet, it pays no tax.
    w.donate(5_000 * WYRM).await?;
    let state = w.state().await;
    assert_eq!(u(&state["hoard"]["donated"]), 5_000 * WYRM);
    assert_eq!(u(&state["hoard"]["wyrm_bought"]), 0);

    w.skip_to(breath_at).await;
    w.dcl_switch("set_hold_output", true).await?;
    let s = slices(&w.poke_ok().await)[0].1.clone();
    let quoted = u(&s["quoted_out"]);
    assert_eq!(
        w.balance(&w.wyrm, &dragon).await,
        before + 5_000 * WYRM,
        "the output has not arrived"
    );
    w.dcl_switch("set_hold_output", false).await?;

    let gift = 50_000 * WYRM;
    w.mint(&w.wyrm, &dragon, gift).await?;
    w.dcl.call("release_held").max_gas().transact().await?.into_result()?;
    let delivered = w.balance(&w.wyrm, &dragon).await - before - 5_000 * WYRM - gift;
    let net = if taxed { expected_net(quoted) } else { quoted };
    assert!(delivered >= net.min(u(&s["min_out"])) && delivered <= quoted);
    // A spike stops the next buy, so this observation only reconciles.
    w.set_pool(POOL_NEAR, POOL_WYRM * 8 / 10).await?;
    w.skip(61 * SEC).await;
    w.poke_ok().await;
    let h = w.state().await["hoard"].clone();
    assert_eq!(
        u(&h["wyrm"]),
        w.balance(&w.wyrm, &dragon).await,
        "the hoard matches ft_balance_of"
    );
    assert_eq!(
        u(&h["wyrm_bought"]),
        expected_net(quoted),
        "credited as bought up to the expected net"
    );
    assert_eq!(
        u(&h["donated"]),
        before + 5_000 * WYRM + delivered + gift - u(&h["wyrm_bought"])
    );
    assert_eq!(u(&h["near_spent"]), u(&s["amount_in"]));
    Ok(())
}

macro_rules! taxed_and_untaxed {
    ($($scenario:ident => $taxed:ident, $untaxed:ident;)*) => {
        $(
            #[tokio::test]
            async fn $taxed() -> anyhow::Result<()> {
                $scenario(true).await
            }
            #[tokio::test]
            async fn $untaxed() -> anyhow::Result<()> {
                $scenario(false).await
            }
        )*
    };
}

taxed_and_untaxed! {
    the_breath_buys_only_from_its_minute_within_budget_cap_and_gap => breath_taxed, breath_untaxed;
    liquidity_far_from_the_price_does_not_grow_slices => far_liquidity_taxed, far_liquidity_untaxed;
    a_confirmed_dip_buys_and_an_unconfirmed_one_does_not => dip_taxed, dip_untaxed;
    no_buy_while_spot_is_over_fifteen_percent_above_the_reference => spike_taxed, spike_untaxed;
    a_refund_keeps_the_wnear_and_the_budget_and_the_next_buy_spends_that_wnear_first => refund_taxed, refund_untaxed;
    late_output_is_reconciled_and_a_donation_meanwhile_does_not_lower_the_average_cost => late_output_taxed, late_output_untaxed;
}
