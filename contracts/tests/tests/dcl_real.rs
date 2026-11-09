//! One `poke` buy chain against the real Rhea DCL wasm, imported read-only from mainnet `dclv2.ref-labs.near` into
//! the sandbox (code only, fresh state). Nearly is still the mock: it launches the coin and its pool on mock-dcl, and
//! the test seeds the same pool id on the real DCL with a full-range position. A second Dragon, whose `dcl` peer is
//! the real DCL, launches straight on Nearly so Nearly records it as creator, binds against the real pool and buys.
//! Each receipt's burnt gas is printed for docs/GAS.md.
mod dragon_world;
mod v6_world;

use dragon_world::*;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, AccountId, Contract};
use serde_json::{json, Value};
use v6_world::*;

const MAINNET_DCL: &str = "dclv2.ref-labs.near";
const MAINNET_RPC: &str = "https://rpc.mainnet.fastnear.com";
const FEE: u32 = 10_000;
/// About 1,000 WYRM (18 decimals) per wNEAR (24 decimals): 1.0001^-69000 is 1.008e-3.
const INIT_POINT: i32 = -69_000;
const LEFT_POINT: i32 = -80_000;
const RIGHT_POINT: i32 = -58_000;

async fn call(by: &Account, on: &AccountId, method: &str, args: Value, deposit: u128) -> Value {
    let res = by
        .call(on, method)
        .args_json(args.clone())
        .deposit(NearToken::from_yoctonear(deposit))
        .gas(Gas::from_tgas(300))
        .transact()
        .await
        .unwrap();
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{on}.{method}({args}): {}",
        failure_text(&res)
    );
    res.json().unwrap_or(Value::Null)
}

async fn real_dcl(w: &World) -> anyhow::Result<Contract> {
    let mainnet = near_workspaces::mainnet().rpc_addr(MAINNET_RPC).await?;
    let id: AccountId = MAINNET_DCL.parse()?;
    let dest: AccountId = format!("realdcl.{}", w.worker.root_account()?.id()).parse()?;
    let dcl = w
        .worker
        .import_contract(&id, &mainnet)
        .dest_account_id(&dest)
        .initial_balance(NearToken::from_near(300))
        .transact()
        .await?;
    let code = w.worker.view_code(dcl.id()).await?;
    println!(
        "imported {MAINNET_DCL} code: {} bytes, {:?}",
        code.len(),
        dcl.view_account().await?.contract_state
    );
    let meta = dcl
        .call("new")
        .args_json(json!({ "owner_id": w.keeper.id(), "wnear_id": w.wrap.id(), "farming_contract_id": w.keeper.id() }))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(meta.is_success(), "DCL new: {}", failure_text(&meta));
    let v: Value = dcl.view("get_metadata").await?.json()?;
    println!("DCL get_metadata: version {} state {}", v["version"], v["state"]);
    Ok(dcl)
}

async fn seed_pool(w: &World, dcl: &Contract, coin: &AccountId) -> anyhow::Result<String> {
    let lp = &w.keeper;
    let tiers: Value = dcl.view("get_fee_tiers").await?.json()?;
    if tiers[FEE.to_string()].is_null() {
        call(
            lp,
            dcl.id(),
            "add_fee_tier",
            json!({ "fee": FEE, "point_delta": 200 }),
            1,
        )
        .await;
    }
    call(
        lp,
        dcl.id(),
        "storage_deposit",
        json!({ "account_id": lp.id() }),
        10 * NEAR,
    )
    .await;
    let created = call(
        lp,
        dcl.id(),
        "create_pool",
        json!({ "token_a": w.wrap.id(), "token_b": coin, "fee": FEE, "init_point": INIT_POINT }),
        NEAR,
    )
    .await;
    let pool_id = created.as_str().expect("create_pool answers the pool id").to_string();
    call(
        lp,
        dcl.id(),
        "register_tokens",
        json!({ "token_ids": [w.wrap.id(), coin] }),
        NEAR / 10,
    )
    .await;
    let (near_side, wyrm_side) = (2_000 * NEAR, 2_000_000 * WYRM);
    w.mint(w.wrap.id(), lp.id().as_str(), near_side).await?;
    w.mint(coin, lp.id().as_str(), wyrm_side).await?;
    w.mint(w.wrap.id(), dcl.id().as_str(), 0).await?;
    w.mint(coin, dcl.id().as_str(), 0).await?;
    for (token, amount) in [(w.wrap.id(), near_side), (coin, wyrm_side)] {
        call(
            lp,
            token,
            "ft_transfer_call",
            json!({ "receiver_id": dcl.id(), "amount": amount.to_string(), "msg": "\"Deposit\"" }),
            1,
        )
        .await;
    }
    let added = call(
        lp,
        dcl.id(),
        "add_liquidity",
        json!({ "pool_id": pool_id, "left_point": LEFT_POINT, "right_point": RIGHT_POINT,
                "amount_x": near_side.to_string(), "amount_y": wyrm_side.to_string(),
                "min_amount_x": "0", "min_amount_y": "0" }),
        0,
    )
    .await;
    println!("add_liquidity: {added}");
    Ok(pool_id)
}

#[tokio::test]
async fn a_poke_buy_chain_completes_against_the_real_dcl_wasm() -> anyhow::Result<()> {
    let mut w = setup_with(Options {
        unlaunched: true,
        ..Options::new(300 * NEAR)
    })
    .await?;
    let dcl = real_dcl(&w).await?;
    let dragon = alt_dragon(&w, "realdragon", json!({ "dcl": dcl.id() })).await?;
    w.worker
        .root_account()?
        .transfer_near(dragon.id(), NearToken::from_near(270))
        .await?
        .into_result()?;
    let id = launch_as_creator(&w, dragon.as_account(), dragon_launch_args(&w, "WYRM")).await?;
    let r = record(&w, id).await;
    assert_eq!(r["step"], "Done");
    let coin: AccountId = r["token"].as_str().unwrap().parse()?;
    let pool_id = seed_pool(&w, &dcl, &coin).await?;
    assert_eq!(
        pool_id,
        r["pool_id"].as_str().unwrap(),
        "the real DCL names the pool as Nearly did"
    );
    call(
        &w.keeper,
        &coin,
        "set_tax",
        json!({ "tax": { "pairs": [dcl.id()], "buy_bps": 400, "sell_bps": 400 } }),
        0,
    )
    .await;
    let pool: Value = dcl
        .view("get_pool")
        .args_json(json!({ "pool_id": pool_id }))
        .await?
        .json()?;
    println!("real DCL get_pool: {pool}");

    let bind = w
        .creator
        .call(dragon.id(), "bind")
        .args_json(json!({ "launch_id": id.to_string() }))
        .gas(BIND_GAS)
        .transact()
        .await?;
    assert_eq!(
        named(&events(&bind), "bound").len(),
        1,
        "bind on the real pool: {:?}",
        events(&bind)
    );
    gas_line("bind against the real DCL get_pool", &bind);

    (w.dragon, w.wyrm, w.pool_id) = (dragon, coin, pool_id);
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    w.skip_to(breath_at).await;
    let res = w.poke().await?;
    for o in res.receipt_outcomes() {
        println!(
            "GAS real-DCL poke receipt {} {:.2} Tgas{}",
            o.executor_id,
            tgas(o.gas_burnt),
            if o.is_failure() { " FAILED" } else { "" }
        );
    }
    gas_line("poke buy chain against the real DCL wasm with 1000 Tgas attached", &res);
    let dragon_id = w.dragon.id().clone();
    let steps: Vec<String> = res
        .receipt_outcomes()
        .iter()
        .filter(|o| o.executor_id == dragon_id)
        .map(|o| format!("{:.2}", tgas(o.gas_burnt)))
        .collect();
    println!("GAS real-DCL poke, Dragon receipts poke/on_observe/on_quote/on_swap: {steps:?} Tgas");
    let on_dcl: Vec<String> = res
        .receipt_outcomes()
        .iter()
        .filter(|o| o.executor_id == *dcl.id())
        .map(|o| format!("{:.2}", tgas(o.gas_burnt)))
        .collect();
    println!("GAS real-DCL receipts on DCL (quotes, get_pool, ft_on_transfer, callbacks): {on_dcl:?} Tgas");
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{}",
        failure_text(&res)
    );
    let evs = events(&res);
    assert_eq!(
        named(&evs, "breath_slice").len(),
        1,
        "the chain bought a slice: {evs:?}"
    );
    assert_eq!(w.state().await["phase"], json!("Idle"), "the chain ran to its end");
    assert!(
        w.balance(&w.wyrm, dragon_id.as_str()).await > 0,
        "the real DCL delivered the coin"
    );

    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    w.skip_to(breath_at).await;
    let floor = w
        .keeper
        .call(&dragon_id, "poke")
        .gas(Gas::from_tgas(760))
        .transact()
        .await?;
    let bought = named(&events(&floor), "breath_slice").len();
    gas_line("poke at the 760 Tgas floor against the real DCL wasm", &floor);
    assert!(
        floor.is_success() && floor.receipt_failures().is_empty() && bought == 1,
        "the next day's breath, since one slice spent the first day's budget, buys at the 760 Tgas floor: {}",
        failure_text(&floor)
    );
    Ok(())
}
