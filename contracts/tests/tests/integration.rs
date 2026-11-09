//! Every contract together on real wasm: the Dragon world from `dragon_world` (mock-dcl, mock-nearly, the coin it
//! launched, the tithe, scales with 100 keyless vaults on the published global vault code), deployed, launched and
//! bound as on mainnet. Council orders go through propose, vote and finalize. Gas lines start with `GAS` for
//! docs/GAS.md.
mod dragon_world;

use dragon_world::*;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, AccountId, ContractState};
use serde_json::{json, Value};

/// A vault earns 30% of the gas its own calls burn, so what it can release drifts above zero by less than this.
const DUST: u128 = NEAR / 10_000;
const SALE: u64 = 14 * DAY;
const VOTING: u64 = 2 * DAY;
const PRICE: u128 = 2 * NEAR;
/// The holders who buy the Council's Scales, ten each.
const HOLDERS: usize = 5;

struct Full {
    w: World,
    holders: Vec<Account>,
}

fn wyrm_events(res: &ExecutionFinalResult, name: &str) -> Vec<Value> {
    named(&events(res), name)
}

async fn full(dragon_near: u128) -> anyhow::Result<Full> {
    let w = setup(dragon_near).await?;
    let root = w.worker.root_account()?;
    let mut holders = vec![];
    for name in ["alice", "bob", "carol", "dave", "erin"] {
        holders.push(
            root.create_subaccount(name)
                .initial_balance(NearToken::from_near(100))
                .transact()
                .await?
                .into_result()?,
        );
    }
    Ok(Full { w, holders })
}

impl Full {
    fn scales(&self) -> &Account {
        &self.w.scales
    }

    fn vault(&self, i: u32) -> AccountId {
        format!("{i:03}.{}", self.scales().id()).parse().unwrap()
    }

    async fn claimable(&self, i: u32) -> anyhow::Result<u128> {
        let v: String = self.w.keeper.view(&self.vault(i), "get_claimable").await?.json()?;
        Ok(v.parse()?)
    }

    async fn near(&self, id: &AccountId) -> anyhow::Result<u128> {
        Ok(self.w.worker.view_account(id).await?.balance.as_yoctonear())
    }

    async fn view(&self, method: &str, args: Value) -> anyhow::Result<Value> {
        Ok(self
            .scales()
            .view(self.scales().id(), method)
            .args_json(args)
            .await?
            .json()?)
    }

    async fn proposal(&self, id: u64) -> anyhow::Result<Value> {
        self.view("get_proposal", json!({ "id": id })).await
    }

    async fn buy_scale(&self, who: &Account, attach: u128) -> anyhow::Result<ExecutionFinalResult> {
        Ok(who
            .call(self.scales().id(), "buy_scale")
            .deposit(NearToken::from_yoctonear(attach))
            .max_gas()
            .transact()
            .await?)
    }

    /// Sells 50 sale Scales, ten to each holder, then lets the 14-day sale end, which also passes the 7-day seasoning. Returns each
    /// holder's token ids.
    async fn council(&self) -> anyhow::Result<Vec<Vec<String>>> {
        let mut held = vec![vec![]; HOLDERS];
        for _ in 0..10 {
            for (h, who) in self.holders.iter().enumerate() {
                let res = self.buy_scale(who, PRICE).await?.into_result()?;
                held[h].push(res.json::<String>()?);
            }
        }
        self.w.skip(SALE + 10 * MIN).await;
        let closed = self
            .w
            .keeper
            .call(self.scales().id(), "close_sale")
            .max_gas()
            .transact()
            .await?;
        assert!(closed.is_success(), "{}", failure_text(&closed));
        assert_eq!(self.view("get_sale", json!({})).await?["sold"], 50);
        Ok(held)
    }

    async fn propose(&self, who: &Account, order: Value, token_id: &str) -> anyhow::Result<u64> {
        let res = who
            .call(self.scales().id(), "propose")
            .args_json(json!({ "kind": { "Order": order }, "description": "integration", "token_id": token_id }))
            .deposit(NearToken::from_millinear(1_010))
            .max_gas()
            .transact()
            .await?;
        assert!(res.is_success(), "propose {order}: {:?}", res.failures());
        Ok(res.json()?)
    }

    /// Every holder votes all of its Scales yes.
    async fn all_vote_yes(&self, proposal_id: u64, held: &[Vec<String>]) -> anyhow::Result<()> {
        for (who, tokens) in self.holders.iter().zip(held) {
            who.call(self.scales().id(), "vote")
                .args_json(json!({ "proposal_id": proposal_id, "token_ids": tokens, "approve": true }))
                .deposit(YOCTO)
                .max_gas()
                .transact()
                .await?
                .into_result()?;
        }
        Ok(())
    }

    async fn finalize(&self, proposal_id: u64) -> anyhow::Result<ExecutionFinalResult> {
        Ok(self
            .w
            .keeper
            .call(self.scales().id(), "finalize")
            .args_json(json!({ "proposal_id": proposal_id }))
            .gas(Gas::from_tgas(150))
            .transact()
            .await?)
    }

    async fn reserve(&self) -> u128 {
        u(&self.w.state().await["reserve"])
    }

    async fn ally(&self) -> anyhow::Result<Account> {
        let ally = self.w.worker.dev_create_account().await?;
        self.w.mint(&self.w.wyrm, ally.id().as_str(), 0).await?;
        Ok(ally)
    }
}

#[tokio::test]
async fn a_tithe_round_pays_the_dragon_and_100_keyless_vaults_and_an_owner_claims_its_share() -> anyhow::Result<()> {
    let f = full(50 * NEAR).await?;
    for i in [1, 50, 100] {
        let vault = f.vault(i);
        assert!(
            f.w.worker.view_access_keys(&vault).await?.is_empty(),
            "vault {i} has no keys"
        );
        let state = f.w.worker.view_account(&vault).await?.contract_state;
        assert!(
            matches!(state, ContractState::GlobalHash(_)),
            "vault {i} runs the global code: {state:?}"
        );
    }
    let alice = &f.holders[0];
    assert_eq!(f.buy_scale(alice, PRICE).await?.json::<String>()?, "1");

    let root = f.w.worker.root_account()?;
    root.transfer_near(f.w.tithe.id(), NearToken::from_yoctonear(100 * NEAR + 12_345))
        .await?
        .into_result()?;
    let free = f.near(f.w.tithe.id()).await? - TITHE_RESERVE;
    let mut vaults_before = vec![];
    for i in 1..=100 {
        let claimable = f.claimable(i).await?;
        assert!(claimable < DUST, "vault {i} starts with nothing to claim");
        vaults_before.push(f.near(&f.vault(i)).await?);
    }
    let dragon_before = f.near(f.w.dragon.id()).await?;
    let reserve_before = f.reserve().await;
    let res =
        f.w.keeper
            .call(f.w.tithe.id(), "distribute")
            .gas(Gas::from_tgas(300))
            .transact()
            .await?;
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{:?}",
        res.receipt_failures()
    );
    gas_line("tithe distribute with 100 vaults", &res);
    let round = wyrm_events(&res, "tithe_round")[0].clone();
    let (to_dragon, per_scale) = (u(&round["to_dragon"]), u(&round["per_scale"]));
    assert_eq!(to_dragon, free / 2, "the dragon gets half");
    assert_eq!(
        per_scale,
        (free - free / 2) / 100,
        "each vault gets a hundredth of the other half"
    );

    assert_eq!(f.near(f.w.dragon.id()).await? - dragon_before, to_dragon);
    assert_eq!(
        f.reserve().await - reserve_before,
        to_dragon,
        "the Dragon counts the half in its reserve"
    );
    for (i, before) in (1..=100).zip(vaults_before) {
        assert_eq!(f.near(&f.vault(i)).await? - before, per_scale, "vault {i}");
    }
    let claimable = f.claimable(1).await?;
    assert!(claimable >= per_scale && claimable < per_scale + DUST);

    let before = f.near(alice.id()).await?;
    let res = alice
        .call(f.scales().id(), "claim")
        .args_json(json!({ "token_id": "1" }))
        .deposit(YOCTO)
        .max_gas()
        .transact()
        .await?;
    gas_line("claim", &res);
    assert_eq!(res.into_result()?.json::<String>()?, claimable.to_string());
    f.w.settle().await;
    let gained = f.near(alice.id()).await? - before;
    assert!(
        gained <= claimable && gained > claimable - NEAR / 100,
        "alice got her vault's share less gas: {gained}"
    );
    assert!(f.claimable(1).await? < DUST, "the vault is back to its reserve");
    assert!(f.claimable(2).await? >= per_scale, "other vaults keep their share");
    let scale = f.view("get_scale", json!({ "token_id": "1" })).await?;
    assert_eq!(scale["claimed_total"], claimable.to_string());
    assert_eq!(scale["claiming_since"], Value::Null);
    Ok(())
}

#[tokio::test]
async fn sale_proceeds_and_a_market_royalty_reach_the_dragon() -> anyhow::Result<()> {
    let f = full(50 * NEAR).await?;
    let (alice, bob) = (&f.holders[0], &f.holders[1]);
    let creator = f.w.creator.id().clone();
    let (dragon_before, creator_before) = (f.near(f.w.dragon.id()).await?, f.near(&creator).await?);
    let reserve_before = f.reserve().await;
    let res = f.buy_scale(alice, PRICE).await?;
    gas_line("buy_scale", &res);
    assert_eq!(res.into_result()?.json::<String>()?, "1");
    // The mint sweep also hands the Dragon the new Scale's vault, which holds only gas-reward dust before a round.
    let to_dragon = f.near(f.w.dragon.id()).await? - dragon_before;
    assert!(
        to_dragon >= PRICE * 9 / 10 && to_dragon < PRICE * 9 / 10 + DUST,
        "90% of the flat price: {to_dragon}"
    );
    assert_eq!(
        f.near(&creator).await? - creator_before,
        PRICE / 10,
        "the creator's 10% cut"
    );
    assert_eq!(f.reserve().await - reserve_before, to_dragon);

    let price = 10 * NEAR;
    alice
        .call(f.scales().id(), "list")
        .args_json(json!({ "token_id": "1", "price": price.to_string() }))
        .deposit(YOCTO)
        .transact()
        .await?
        .into_result()?;
    f.w.settle().await;
    let (dragon_before, alice_before) = (f.near(f.w.dragon.id()).await?, f.near(alice.id()).await?);
    let creator_before = f.near(&creator).await?;
    let res = bob
        .call(f.scales().id(), "buy")
        .args_json(json!({ "token_id": "1" }))
        .deposit(NearToken::from_yoctonear(price))
        .max_gas()
        .transact()
        .await?;
    gas_line("market buy", &res);
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{:?}",
        res.failures()
    );
    let sold = wyrm_events(&res, "scale_sold")[0].clone();
    assert_eq!(u(&sold["royalty"]), price / 20);
    assert_eq!(
        f.near(&creator).await? - creator_before,
        price / 20,
        "the hoarder preset pays the 5% to the creator"
    );
    assert_eq!(
        f.near(f.w.dragon.id()).await?,
        dragon_before,
        "a resale pays the Dragon nothing"
    );
    assert_eq!(
        f.near(alice.id()).await? - alice_before,
        price - price / 20,
        "the rest to the seller"
    );
    assert_eq!(
        f.view("get_scale", json!({ "token_id": "1" })).await?["owner"],
        bob.id().as_str()
    );
    Ok(())
}

#[tokio::test]
async fn council_proposals_reach_the_dragon_and_a_refused_one_is_leash_held() -> anyhow::Result<()> {
    let f = full(50 * NEAR).await?;
    let hoard = 1_000_000 * WYRM;
    f.w.donate(hoard).await?;
    let ally = f.ally().await?;
    let held = f.council().await?;
    let h = &f.holders;
    // Each Grant is 49 bps of the hoard: 50 bps of a hoard the first Grant already shrank would pass the cap.
    let g = hoard * 49 / 10_000;
    let grant = || json!({ "Grant": { "to": ally.id(), "amount": g.to_string() } });
    let orders = [
        ("SetSlices", json!({ "SetSlices": { "cap_bps": 100 } })),
        ("SetAutoBurn", json!({ "SetAutoBurn": { "bps": 1_000 } })),
        ("Grant 1", grant()),
        ("Grant 2", grant()),
        ("Grant 3", grant()),
    ];
    let mut ids = vec![];
    for (i, (_, order)) in orders.iter().enumerate() {
        ids.push(f.propose(&h[i], order.clone(), &held[i][0]).await?);
    }
    for &id in &ids {
        f.all_vote_yes(id, &held).await?;
        let p = f.proposal(id).await?;
        assert_eq!(
            (p["yes"].as_u64(), p["no"].as_u64(), p["electorate"].as_u64()),
            (Some(50), Some(0), Some(50))
        );
    }
    assert!(failure_text(&f.finalize(ids[0]).await?).contains("voting still open"));
    f.w.skip(VOTING + MIN).await;

    for (&id, (name, _)) in ids.iter().zip(&orders) {
        let res = f.finalize(id).await?;
        assert!(res.is_success(), "{name}: {}", failure_text(&res));
        gas_line(&format!("finalize {name}"), &res);
        let status = f.proposal(id).await?["status"].clone();
        match *name {
            "Grant 3" => {
                let error = status["LeashHeld"]["error"].as_str().unwrap_or_default().to_string();
                assert!(
                    error.contains("grants in any 30 days"),
                    "the reason names the 30-day window: {status}"
                );
                assert_eq!(wyrm_events(&res, "order_refused")[0]["reason"], error.as_str());
            }
            "SetAutoBurn" => {
                assert_eq!(status, json!("Executed"));
                let changed = &wyrm_events(&res, "settings_changed")[0];
                assert_eq!(changed["auto_burn_bps"], 1_000);
            }
            _ => assert_eq!(status, json!("Executed"), "{name}"),
        }
    }
    let s = &f.w.dragon.view("get_config").await?.json::<Value>()?["strategy"];
    assert_eq!(
        (s["cap_bps"].clone(), s["auto_burn_bps"].clone()),
        (json!(100), json!(1_000))
    );
    assert_eq!(f.w.balance(&f.w.wyrm, ally.id().as_str()).await, 2 * g);
    let state = f.w.state().await["hoard"].clone();
    assert_eq!((u(&state["wyrm"]), u(&state["granted"])), (hoard - 2 * g, 2 * g));
    assert!(
        failure_text(&f.finalize(ids[0]).await?).contains("not open"),
        "executed once"
    );
    Ok(())
}

#[tokio::test]
async fn council_grants_pay_a_registered_account_and_a_failed_grant_restores_the_hoard() -> anyhow::Result<()> {
    let f = full(50 * NEAR).await?;
    let hoard = 1_000_000 * WYRM;
    let dragon = f.w.dragon.id().as_str().to_string();
    f.w.donate(hoard).await?;
    assert_eq!(u(&f.w.state().await["hoard"]["wyrm"]), hoard);
    let ally = f.ally().await?;
    let held = f.council().await?;
    let paid = hoard / 200;
    let lost = hoard / 1_000;
    let nobody = format!("nobody.{}", f.w.worker.root_account()?.id());
    let id = f
        .propose(
            &f.holders[0],
            json!({ "Grant": { "to": ally.id(), "amount": paid.to_string() } }),
            &held[0][0],
        )
        .await?;
    let id2 = f
        .propose(
            &f.holders[1],
            json!({ "Grant": { "to": nobody, "amount": lost.to_string() } }),
            &held[1][0],
        )
        .await?;
    f.all_vote_yes(id, &held).await?;
    f.all_vote_yes(id2, &held).await?;
    f.w.skip(VOTING + MIN).await;
    let res = f.finalize(id).await?;
    gas_line("finalize Grant paid", &res);
    assert_eq!(
        f.proposal(id).await?["status"],
        json!("Executed"),
        "{}",
        failure_text(&res)
    );
    assert_eq!(f.w.balance(&f.w.wyrm, ally.id().as_str()).await, paid);
    let h = f.w.state().await["hoard"].clone();
    assert_eq!((u(&h["wyrm"]), u(&h["granted"])), (hoard - paid, paid));

    // Inside the 30-day cap of 1% of what is left, so only the missing storage registration can fail it.
    let res = f.finalize(id2).await?;
    gas_line("finalize Grant refused by the token", &res);
    assert_eq!(
        f.proposal(id2).await?["status"],
        json!({ "LeashHeld": { "error": "transfer failed" } })
    );
    let h = f.w.state().await["hoard"].clone();
    assert_eq!(
        (u(&h["wyrm"]), u(&h["granted"])),
        (hoard - paid, paid),
        "the hoard is restored"
    );
    assert_eq!(f.w.balance(&f.w.wyrm, &dragon).await, hoard - paid);
    Ok(())
}

#[tokio::test]
async fn the_poke_buy_chain_fits_in_1000_tgas_and_poke_refuses_below_760() -> anyhow::Result<()> {
    let f = full(300 * NEAR).await?;
    let w = &f.w;
    gas_line("launch with 500 Tgas attached", w.launch.as_ref().unwrap());
    gas_line("bind with 150 Tgas attached", w.bind.as_ref().unwrap());
    let started = named(&events(w.launch.as_ref().unwrap()), "launch_sent");
    assert_eq!(started[0]["ok"], true, "launch_sent {{ok: true}}");
    let record: Value = w
        .nearly
        .view("get_launch")
        .args_json(json!({ "launch_id": w.launch_id.to_string() }))
        .await?
        .json()?;
    assert_eq!(
        record["step"], "Done",
        "Done is read from get_launch, not from launch_sent"
    );
    assert_eq!(named(&events(w.bind.as_ref().unwrap()), "bound").len(), 1);
    assert_eq!(
        f.view("get_sale", json!({})).await?["state"],
        "Open",
        "bind opened the sale"
    );

    let short = w
        .keeper
        .call(w.dragon.id(), "poke")
        .gas(Gas::from_tgas(759))
        .transact()
        .await?;
    assert!(
        failure_text(&short).contains("poke needs at least 760 Tgas"),
        "{}",
        failure_text(&short)
    );
    let (_, breath_at) = w.day_with_breath(30 * MIN, DAY - 2 * HOUR).await;
    w.skip_to(breath_at).await;
    let res = w.poke().await?;
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{:?}",
        res.receipt_failures()
    );
    let evs = events(&res);
    assert_eq!(
        named(&evs, "breath_slice").len(),
        1,
        "the first poke after the breath minute bought a slice"
    );
    assert_eq!(w.state().await["phase"], json!("Idle"), "the chain ran to its end");
    let dragon = w.dragon.id();
    let steps: Vec<String> = res
        .receipt_outcomes()
        .iter()
        .filter(|o| &o.executor_id == dragon)
        .map(|o| format!("{:.2}", tgas(o.gas_burnt)))
        .collect();
    println!("GAS poke buy chain, Dragon receipts poke/on_observe/on_quote/on_swap: {steps:?} Tgas");
    gas_line("poke buy chain with 1000 Tgas attached", &res);
    let idle = w.poke().await?;
    gas_line("poke with nothing to do", &idle);
    w.skip(61 * SEC).await;
    let floor = w
        .keeper
        .call(w.dragon.id(), "poke")
        .gas(Gas::from_tgas(760))
        .transact()
        .await?;
    let bought = named(&events(&floor), "breath_slice").len();
    println!(
        "GAS poke at the 760 Tgas floor: success {}, receipt failures {}, slices {bought}",
        floor.is_success(),
        floor.receipt_failures().len()
    );
    Ok(())
}
