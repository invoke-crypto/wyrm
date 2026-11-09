#![allow(dead_code)]
//! What the V6 suites share on top of `dragon_world`: a second Dragon on other peers, a launch sent straight to
//! Nearly from any account with the Dragon's own arguments, a patch of one borsh string in a mock's state, and a
//! Council of holders who buy Scales, propose, vote and finalize.
use crate::dragon_world::*;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, AccountId, Contract};
use serde_json::{json, Value};

pub mod system;

pub const PRICE: u128 = 2 * NEAR;
pub const SALE: u64 = 14 * DAY;
pub const SEASON: u64 = 7 * DAY;
pub const VOTING: u64 = 2 * DAY;
pub const FINALIZE_GAS: Gas = Gas::from_tgas(150);

pub fn peers(w: &World) -> Value {
    json!({ "wrap": w.wrap.id(), "dcl": w.dcl.id(), "nearly": w.nearly.id(), "tithe": w.tithe.id(),
            "scales": w.scales.id(), "ash": w.ash.id(), "creator": w.creator.id() })
}

pub async fn alt_dragon(w: &World, name: &str, changes: Value) -> anyhow::Result<Contract> {
    let mut p = peers(w);
    for (k, v) in changes.as_object().expect("peer changes are an object") {
        p[k] = v.clone();
    }
    let dragon = w
        .worker
        .root_account()?
        .create_subaccount(name)
        .initial_balance(NearToken::from_near(30))
        .transact()
        .await?
        .into_result()?
        .deploy(&wasm("dragon"))
        .await?
        .into_result()?;
    dragon
        .call("new")
        .args_json(json!({ "preset": "hoarder", "peers": p }))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?
        .into_result()?;
    Ok(dragon)
}

pub fn dragon_launch_args(w: &World, symbol: &str) -> Value {
    json!({
        "name": "Wyrm", "symbol": symbol, "icon": "https://wyrm.example/icon.png", "description": "sandbox",
        "links": {}, "fee_to": w.tithe.id(), "fee_mode": "creator",
        "tax": { "buy_bps": 400, "sell_bps": 400, "creator_bps": 10_000, "burn_bps": 0, "holders_bps": 0 },
    })
}

pub async fn launch_as_creator(w: &World, by: &Account, args: Value) -> anyhow::Result<u64> {
    let res = by
        .call(w.nearly.id(), "launch")
        .args_json(json!({ "args": args }))
        .deposit(NearToken::from_yoctonear(LAUNCH_DEPOSIT))
        .gas(Gas::from_tgas(300))
        .transact()
        .await?;
    assert!(res.is_success(), "direct launch: {:?}", res.failures());
    Ok(launch_started(&res).expect("Nearly started a launch"))
}

pub async fn record(w: &World, launch_id: u64) -> Value {
    w.nearly
        .view("get_launch")
        .args_json(json!({ "launch_id": launch_id.to_string() }))
        .await
        .unwrap()
        .json()
        .unwrap()
}

fn borsh_str(s: &str) -> Vec<u8> {
    let mut b = (s.len() as u32).to_le_bytes().to_vec();
    b.extend_from_slice(s.as_bytes());
    b
}

/// Borsh has no outer length or checksum, so the splice leaves a valid record, as if the contract had stored `new`.
pub async fn patch_string(w: &World, contract: &AccountId, key_has: &str, old: &str, new: &str) -> anyhow::Result<()> {
    let (old_b, new_b) = (borsh_str(old), borsh_str(new));
    let state = w.worker.view_state(contract).await?;
    let mut hits = state
        .iter()
        .filter(|(k, v)| contains(k, key_has.as_bytes()) && contains(v, &old_b));
    let (key, value) = hits
        .next()
        .ok_or_else(|| anyhow::anyhow!("no state value of {contract} holds {old}"))?;
    anyhow::ensure!(
        hits.next().is_none(),
        "more than one state value of {contract} holds {old}"
    );
    let at = value.windows(old_b.len()).position(|x| x == old_b.as_slice()).unwrap();
    let mut patched = value[..at].to_vec();
    patched.extend_from_slice(&new_b);
    patched.extend_from_slice(&value[at + old_b.len()..]);
    w.worker.patch_state(contract, key, &patched).await?;
    Ok(())
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|x| x == needle)
}

pub async fn near_of(w: &World, id: &AccountId) -> u128 {
    w.worker.view_account(id).await.unwrap().balance.as_yoctonear()
}

pub async fn coin_of(w: &World) -> Value {
    w.dragon.view("get_coin").await.unwrap().json().unwrap()
}

pub async fn scales_view(w: &World, method: &str, args: Value) -> Value {
    w.worker
        .view(w.scales.id(), method)
        .args_json(args)
        .await
        .unwrap()
        .json()
        .unwrap()
}

pub fn refused_with(res: &ExecutionFinalResult, why: &str) {
    let text = failure_text(res);
    assert!(
        res.is_failure() && text.contains(why),
        "expected a refusal naming {why:?}: {text}"
    );
}

pub struct Council {
    pub holders: Vec<Account>,
    pub held: Vec<Vec<String>>,
}

impl Council {
    pub async fn buy(w: &World, counts: &[usize]) -> anyhow::Result<Self> {
        let root = w.worker.root_account()?;
        let mut holders = vec![];
        for i in 0..counts.len() {
            let cost = (counts[i] as u128 * PRICE + 20 * NEAR) / NEAR;
            holders.push(
                root.create_subaccount(&format!("holder{i}"))
                    .initial_balance(NearToken::from_near(cost))
                    .transact()
                    .await?
                    .into_result()?,
            );
        }
        let mut held = vec![vec![]; counts.len()];
        for round in 0..counts.iter().copied().max().unwrap_or(0) {
            for (i, who) in holders.iter().enumerate() {
                if round < counts[i] {
                    let res = who
                        .call(w.scales.id(), "buy_scale")
                        .deposit(NearToken::from_yoctonear(PRICE))
                        .max_gas()
                        .transact()
                        .await?;
                    assert!(res.is_success(), "buy_scale: {:?}", res.failures());
                    held[i].push(res.json::<String>()?);
                }
            }
        }
        Ok(Self { holders, held })
    }

    pub async fn propose(&self, w: &World, who: usize, kind: Value) -> anyhow::Result<u64> {
        let res = self.holders[who]
            .call(w.scales.id(), "propose")
            .args_json(json!({ "kind": kind, "description": "v6", "token_id": self.held[who][0] }))
            .deposit(NearToken::from_millinear(1_010))
            .max_gas()
            .transact()
            .await?;
        assert!(res.is_success(), "propose {kind}: {:?}", res.failures());
        Ok(res.json()?)
    }

    pub async fn tally(&self, w: &World, proposal_id: u64, yes: usize, no: usize) -> anyhow::Result<()> {
        let mut tokens: Vec<(usize, &String)> = vec![];
        for (i, held) in self.held.iter().enumerate() {
            tokens.extend(held.iter().map(|t| (i, t)));
        }
        assert!(yes + no <= tokens.len(), "only {} Scales can vote", tokens.len());
        for (approve, slice) in [(true, &tokens[..yes]), (false, &tokens[yes..yes + no])] {
            for (i, holder) in self.holders.iter().enumerate() {
                let mine: Vec<&String> = slice.iter().filter(|(h, _)| *h == i).map(|(_, t)| *t).collect();
                for chunk in mine.chunks(25) {
                    holder
                        .call(w.scales.id(), "vote")
                        .args_json(json!({ "proposal_id": proposal_id, "token_ids": chunk, "approve": approve }))
                        .deposit(YOCTO)
                        .max_gas()
                        .transact()
                        .await?
                        .into_result()?;
                }
            }
        }
        Ok(())
    }
}

pub async fn finalize(w: &World, proposal_id: u64) -> anyhow::Result<ExecutionFinalResult> {
    Ok(w.keeper
        .call(w.scales.id(), "finalize")
        .args_json(json!({ "proposal_id": proposal_id }))
        .gas(FINALIZE_GAS)
        .transact()
        .await?)
}

pub async fn proposal(w: &World, id: u64) -> Value {
    scales_view(w, "get_proposal", json!({ "id": id })).await
}

pub async fn close_sale(w: &World) -> anyhow::Result<ExecutionFinalResult> {
    let res = w.keeper.call(w.scales.id(), "close_sale").max_gas().transact().await?;
    assert!(res.is_success(), "close_sale: {}", failure_text(&res));
    Ok(res)
}
