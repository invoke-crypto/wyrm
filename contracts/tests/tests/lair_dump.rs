//! Records every view the v2 Lair can read from the real contracts in the sandbox and writes them to
//! /tmp/lair-sandbox.json for `lair/fixtures/sandbox.json`. The system is the hold world's, so the answers cover
//! an unbound Dragon, a bound coin, trades, a closed sale, a listing, an offer and a proposal of each status.
mod dragon_world;
mod v6_world;

use dragon_world::*;
use near_workspaces::AccountId;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use v6_world::system::{self, Marks};
use v6_world::*;

#[derive(Default)]
struct Dump(BTreeMap<String, Vec<Value>>);

impl Dump {
    async fn view(&mut self, w: &World, on: &AccountId, method: &str, args: Value) {
        let answer: Value = w
            .worker
            .view(on, method)
            .args_json(args.clone())
            .await
            .unwrap_or_else(|e| panic!("{on}.{method}({args}): {e}"))
            .json()
            .unwrap();
        let label = on.as_str().split('.').next().unwrap();
        self.0.entry(format!("{label}.{method}")).or_default().push(answer);
    }

    async fn dragon(&mut self, w: &World) {
        for method in ["get_coin", "get_config", "get_state"] {
            self.view(w, w.dragon.id(), method, json!({})).await;
        }
        self.view(w, w.dragon.id(), "get_log", json!({ "limit": 256 })).await;
        self.view(w, w.dragon.id(), "get_samples", json!({ "limit": 100 }))
            .await;
    }

    async fn sale(&mut self, w: &World) {
        self.view(w, w.scales.id(), "get_sale", json!({})).await;
        self.view(w, w.scales.id(), "get_sweepable", json!({})).await;
    }

    fn into_json(self) -> Value {
        let mut out = Map::new();
        out.insert(
            "_source".into(),
            json!("Recorded from the real v2 contracts in near-sandbox by contracts/tests/tests/lair_dump.rs: an unbound Dragon, a launch and bind, trades, a sold-out and closed sale, a listing, an offer, and proposals Executed, Rejected, Carried, LeashHeld and Open. Each key is <account's first label>.<method>."),
        );
        let multi: Vec<&String> = self.0.iter().filter(|(_, v)| v.len() > 1).map(|(k, _)| k).collect();
        out.insert("_multi".into(), json!(multi));
        for (k, mut v) in self.0 {
            out.insert(k, if v.len() == 1 { v.remove(0) } else { Value::Array(v) });
        }
        Value::Object(out)
    }
}

#[tokio::test]
async fn lair_dump() -> anyhow::Result<()> {
    let mut w = system::deploy_unlaunched().await?;
    let mut marks = Marks::default();
    let mut d = Dump::default();
    d.dragon(&w).await;
    d.sale(&w).await;

    system::launch_and_bind(&mut w, &mut marks).await?;
    d.dragon(&w).await;
    d.sale(&w).await;
    system::trade(&w).await?;
    d.dragon(&w).await;

    let council = system::sell_out_and_close(&w).await?;
    system::proposals(&w, &council, &mut marks).await?;
    d.dragon(&w).await;
    d.sale(&w).await;

    let s = w.scales.id().clone();
    let token = council.held[0][1].clone();
    for (method, args) in [
        ("get_config", json!({})),
        ("get_council", json!({})),
        ("get_due", json!({})),
        ("get_proposals", json!({})),
        ("get_scales", json!({ "limit": 100 })),
        ("get_scale", json!({ "token_id": token })),
        ("get_history", json!({ "token_id": "1" })),
        ("get_vault", json!({ "token_id": "1" })),
        ("get_listings", json!({})),
        ("get_offers", json!({})),
        ("get_offers_for", json!({ "token_id": council.held[2][1] })),
        ("eligible_now", json!({})),
        ("nft_metadata", json!({})),
        ("nft_total_supply", json!({})),
        ("nft_token", json!({ "token_id": token })),
        ("nft_tokens", json!({ "limit": 10 })),
        ("nft_supply_for_owner", json!({ "account_id": council.holders[0].id() })),
    ] {
        d.view(&w, &s, method, args).await;
    }
    for (_, id) in &marks.proposals {
        d.view(&w, &s, "get_proposal", json!({ "id": id })).await;
        d.view(&w, &s, "get_votes", json!({ "id": id })).await;
    }
    d.view(&w, w.tithe.id(), "get_stats", json!({})).await;
    d.view(&w, w.tithe.id(), "get_config", json!({})).await;
    let vault: AccountId = format!("001.{s}").parse()?;
    d.view(&w, &vault, "get_claimable", json!({})).await;
    w.skip(VOTING + MIN).await;
    d.view(&w, &s, "get_due", json!({})).await;

    let json = d.into_json();
    let keys: Vec<&String> = json
        .as_object()
        .unwrap()
        .keys()
        .filter(|k| !k.starts_with('_'))
        .collect();
    for want in [
        "dragon.get_coin",
        "scales.get_sale",
        "scales.get_council",
        "scales.get_due",
    ] {
        assert!(keys.iter().any(|k| *k == want), "{want} recorded");
    }
    std::fs::write("/tmp/lair-sandbox.json", serde_json::to_string_pretty(&json)?)?;
    println!("wrote /tmp/lair-sandbox.json with {} keys: {keys:?}", keys.len());
    Ok(())
}
