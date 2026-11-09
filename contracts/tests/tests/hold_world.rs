//! Builds a bound system under `thewyrm.test.near` with an orphan launch, trades, a closed sale and one proposal of
//! each status, prints `RPC http://127.0.0.1:<port>`, writes the accounts to /tmp/wyrm-world.json and keeps the node
//! up for `WYRM_HOLD_SECS` (1,800 by default), so keeper, verify and the Lair can read a live system.
//! `WYRM_WORLD=unbound` stops before bind, with the launch parked at CreatePool. `WYRM_WORLD=unsold` binds, sells
//! nothing and closes the sale, so all 90 sale vaults are unsold. Like the real deploy, the world deletes the keys of
//! tithe, dragon, scales and ash before it holds; the keeper keeps its own.
mod dragon_world;
mod v6_world;

use dragon_world::MIN;
use serde_json::{json, Value};
use v6_world::system::{self, Marks};
use v6_world::*;

#[tokio::test]
async fn hold_world() -> anyhow::Result<()> {
    let mode = std::env::var("WYRM_WORLD").unwrap_or_default();
    let unbound = mode == "unbound";
    let hold: u64 = std::env::var("WYRM_HOLD_SECS").map_or(Ok(1_800), |s| s.parse())?;
    let mut w = system::deploy_unlaunched().await?;
    let mut marks = Marks::default();
    if unbound {
        system::parked_launch(&mut w).await?;
    } else if mode == "unsold" {
        system::launch_and_bind(&mut w, &mut marks).await?;
        w.skip(SALE + MIN).await;
        close_sale(&w).await?.into_result()?;
    } else {
        system::launch_and_bind(&mut w, &mut marks).await?;
        system::trade(&w).await?;
        let council = system::sell_out_and_close(&w).await?;
        system::proposals(&w, &council, &mut marks).await?;
    }
    for account in [&w.tithe, w.dragon.as_account(), &w.scales, &w.ash] {
        let key = account.secret_key().public_key();
        account
            .batch(account.id())
            .delete_key(key)
            .transact()
            .await?
            .into_result()?;
    }
    let world = json!({
        "network": "sandbox",
        "rpc": w.node.rpc_addr,
        "root": format!("{}.{}", system::ROOT, w.worker.root_account()?.id()),
        "bound": !unbound,
        "accounts": { "dragon": w.dragon.id(), "scales": w.scales.id(), "tithe": w.tithe.id(), "ash": w.ash.id(),
                      "creator": w.creator.id(), "keeper": w.keeper.id(), "wrap": w.wrap.id(), "dcl": w.dcl.id(),
                      "nearly": w.nearly.id() },
        "coin": if unbound { Value::Null } else { coin_of(&w).await },
        "launch_id": w.launch_id.to_string(),
        "launch_step": record(&w, w.launch_id).await["step"],
        "orphan_launch_id": marks.orphan_launch.map(|id| id.to_string()),
        "proposals": marks.proposals.iter().map(|(s, id)| json!({ "status": s, "id": id })).collect::<Vec<_>>(),
        "keeper_secret_key": w.keeper.secret_key().to_string(),
        "creator_secret_key": w.creator.secret_key().to_string(),
        "hold_secs": hold,
    });
    std::fs::write("/tmp/wyrm-world.json", serde_json::to_string_pretty(&world)?)?;
    println!("wrote /tmp/wyrm-world.json");
    println!("RPC {}", w.node.rpc_addr);
    tokio::time::sleep(std::time::Duration::from_secs(hold)).await;
    let block = w.worker.view_block().await?;
    println!("held {hold} s; last block {}", block.height());
    Ok(())
}
