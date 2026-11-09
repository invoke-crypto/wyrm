use near_workspaces::network::{Sandbox, ValidatorKey};
use near_workspaces::operations::Function;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, AccountId, ContractState, Worker};
use serde_json::{json, Value};
use std::path::PathBuf;

const NEAR: u128 = 10u128.pow(24);
const VAULT_RESERVE: u128 = NEAR / 100;
/// A vault earns 30% of the gas its own calls burn, so its balance drifts above the reserve by about this much at most.
const DUST: u128 = NEAR / 10_000;
const GLOBAL_CODE_BYTE_COST: u128 = 10u128.pow(20);
const YOCTO: NearToken = NearToken::from_yoctonear(1);
const PRICE: u128 = 2 * NEAR;
const CREATOR_CUT: u128 = PRICE / 10;
const SEC: u64 = 1_000_000_000;
const DAY: u64 = 86_400 * SEC;

fn wasm(krate: &str) -> Vec<u8> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../target"));
    let p = target.join(format!("near/{krate}/{krate}.wasm"));
    std::fs::read(&p).unwrap_or_else(|e| panic!("build {krate} first with cargo near build ({p:?}): {e}"))
}

fn gas_line(name: &str, res: &ExecutionFinalResult) {
    let tgas = |g: Gas| g.as_gas() as f64 / 1e12;
    let receipts: Vec<String> = res
        .receipt_outcomes()
        .iter()
        .map(|o| format!("{} {:.2}", o.executor_id, tgas(o.gas_burnt)))
        .collect();
    println!(
        "GAS {name}: {:.2} Tgas burnt in all; the called method's receipt {:.2} Tgas; receipts [{}]",
        tgas(res.total_gas_burnt),
        tgas(res.receipt_outcomes()[0].gas_burnt),
        receipts.join(", ")
    );
}

fn failure(res: ExecutionFinalResult) -> String {
    format!("{:?}", res.into_result().unwrap_err())
}

fn wyrm_events(res: &ExecutionFinalResult, name: &str) -> Vec<Value> {
    res.logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .filter_map(|j| serde_json::from_str::<Value>(j).ok())
        .filter(|e| e["event"] == name)
        .map(|e| e["data"][0].clone())
        .collect()
}

struct World {
    worker: Worker<Sandbox>,
    _node: near_sandbox::Sandbox,
    scales: Account,
    dragon: Account,
    creator: Account,
    alice: Account,
    bob: Account,
    mallory: Account,
}

/// A node whose first epoch outlasts every test, so `fast_forward` jumps days in a second.
async fn sandbox() -> anyhow::Result<(Worker<Sandbox>, near_sandbox::Sandbox)> {
    if std::env::var_os("NEAR_SANDBOX_LOG").is_none() {
        std::env::set_var("NEAR_SANDBOX_LOG", "near=error,stats=error,network=error");
    }
    let node = near_sandbox::Sandbox::start_sandbox_with_config(near_sandbox::SandboxConfig {
        additional_genesis: Some(json!({ "epoch_length": 1_000_000 })),
        ..Default::default()
    })
    .await?;
    let worker = near_workspaces::sandbox()
        .rpc_addr(&node.rpc_addr)
        .validator_key(ValidatorKey::HomeDir(node.home_dir.path().to_path_buf()))
        .await?;
    Ok((worker, node))
}

async fn world() -> anyhow::Result<World> {
    let (worker, node) = sandbox().await?;
    let root = worker.root_account()?;
    let make = |name: &'static str, near: u128| {
        let root = root.clone();
        async move {
            anyhow::Ok(
                root.create_subaccount(name)
                    .initial_balance(NearToken::from_near(near))
                    .transact()
                    .await?
                    .into_result()?,
            )
        }
    };
    let dragon = make("dragon", 1).await?;
    let creator = make("creator", 1).await?;
    let alice = make("alice", 100).await?;
    let bob = make("bob", 100).await?;
    let mallory = make("mallory", 10).await?;
    let deployer = make("deployer", 50).await?;
    let scales = make("scales", 30).await?;

    let vault_code = wasm("scale_vault");
    let probe = worker.dev_deploy(&vault_code).await?;
    let ContractState::LocalHash(hash) = probe.view_account().await?.contract_state else {
        anyhow::bail!("the probe has no local code")
    };
    let tier_media: Vec<String> = ["ash", "ember", "molten", "obsidian"]
        .iter()
        .map(|t| format!("https://wyrm.example/{t}.svg"))
        .collect();

    scales
        .batch(scales.id())
        .deploy(&wasm("scales"))
        .call(
            Function::new("new")
                .args_json(json!({
                    "preset": "hoarder",
                    "dragon": dragon.id(),
                    "tithe": format!("tithe.{}", root.id()),
                    "creator": creator.id(),
                    "vault_code_hash": hash.to_string(),
                    "collection": { "name": "WYRM Scales", "symbol": "SCALE", "icon": null, "tier_media": tier_media },
                }))
                .gas(Gas::from_tgas(300)),
        )
        .transact()
        .await?
        .into_result()?;

    deployer
        .call(scales.id(), "publish_vault_code")
        .args(vault_code.clone())
        .deposit(NearToken::from_yoctonear(
            vault_code.len() as u128 * GLOBAL_CODE_BYTE_COST,
        ))
        .max_gas()
        .transact()
        .await?
        .into_result()?;

    Ok(World {
        worker,
        _node: node,
        scales,
        dragon,
        creator,
        alice,
        bob,
        mallory,
    })
}

impl World {
    async fn balance(&self, id: &AccountId) -> anyhow::Result<u128> {
        Ok(self.worker.view_account(id).await?.balance.as_yoctonear())
    }

    async fn view(&self, method: &str, args: Value) -> anyhow::Result<Value> {
        Ok(self
            .scales
            .view(self.scales.id(), method)
            .args_json(args)
            .await?
            .json()?)
    }

    /// A transaction counts as executed before its gas refund lands, so its signer's balance is read after a few blocks.
    async fn settle(&self) -> anyhow::Result<()> {
        Ok(self.worker.fast_forward(3).await?)
    }

    async fn now(&self) -> anyhow::Result<u64> {
        Ok(self.worker.view_block().await?.timestamp())
    }

    /// Moves chain time to at least `t`. A fast-forwarded block steps about 1.3 s, so each jump is sized from the last.
    async fn skip_to(&self, t: u64) -> anyhow::Result<()> {
        let mut step = 2 * SEC;
        loop {
            let now = self.now().await?;
            if now >= t {
                return Ok(());
            }
            let blocks = ((t - now) / step).max(1);
            self.worker.fast_forward(blocks).await?;
            step = ((self.now().await? - now) / blocks).max(1);
        }
    }

    async fn claimable(&self, vault: &AccountId) -> anyhow::Result<u128> {
        let v: String = self.alice.view(vault, "get_claimable").await?.json()?;
        Ok(v.parse()?)
    }

    fn vault(&self, i: u32) -> AccountId {
        format!("{i:03}.{}", self.scales.id()).parse().unwrap()
    }

    /// The Dragon's kick at bind, with the 10 Tgas the Dragon attaches.
    async fn open_sale(&self) -> anyhow::Result<ExecutionFinalResult> {
        Ok(self
            .dragon
            .call(self.scales.id(), "open_sale")
            .gas(Gas::from_tgas(10))
            .transact()
            .await?)
    }

    async fn buy_scale(&self, who: &Account, attach: u128) -> anyhow::Result<ExecutionFinalResult> {
        Ok(who
            .call(self.scales.id(), "buy_scale")
            .deposit(NearToken::from_yoctonear(attach))
            .max_gas()
            .transact()
            .await?)
    }

    async fn create_all_vaults(&self) -> anyhow::Result<Vec<Gas>> {
        let mut burnt = Vec::new();
        for _ in 0..4 {
            let r = self
                .mallory
                .call(self.scales.id(), "create_vaults")
                .max_gas()
                .transact()
                .await?;
            burnt.push(r.total_gas_burnt);
            assert_eq!(
                r.into_result()?.json::<bool>()?,
                true,
                "the batch's last vault reported success"
            );
        }
        Ok(burnt)
    }

    async fn claim(&self, who: &Account, token: &str) -> anyhow::Result<ExecutionFinalResult> {
        Ok(who
            .call(self.scales.id(), "claim")
            .args_json(json!({ "token_id": token }))
            .deposit(YOCTO)
            .max_gas()
            .transact()
            .await?)
    }

    async fn finalize(&self, proposal_id: u64, tgas: u64) -> anyhow::Result<ExecutionFinalResult> {
        Ok(self
            .mallory
            .call(self.scales.id(), "finalize")
            .args_json(json!({ "proposal_id": proposal_id }))
            .gas(Gas::from_tgas(tgas))
            .transact()
            .await?)
    }
}

#[tokio::test]
async fn scales_sale_opens_on_the_dragons_call_charges_the_flat_price_and_sweeps_each_mint() -> anyhow::Result<()> {
    let w = world().await?;
    let alice_before = w.balance(w.alice.id()).await?;
    assert!(failure(w.buy_scale(&w.alice, PRICE).await?).contains("the sale is not open"));
    w.settle().await?;
    assert!(
        alice_before - w.balance(w.alice.id()).await? < NEAR / 100,
        "the refused deposit came back"
    );
    assert!(failure(w.mallory.call(w.scales.id(), "open_sale").transact().await?).contains("only the dragon"));

    let opened = w.open_sale().await?;
    gas_line("open_sale", &opened);
    assert_eq!(wyrm_events(&opened, "sale_opened").len(), 1);
    let sale = w.view("get_sale", json!({})).await?;
    let ends_at: u64 = sale["ends_at"].as_str().unwrap().parse()?;
    let opened_at: u64 = sale["opened_at"].as_str().unwrap().parse()?;
    assert_eq!((sale["state"].as_str(), ends_at - opened_at), (Some("Open"), 14 * DAY));
    let again = w.open_sale().await?;
    assert!(
        again.is_success() && wyrm_events(&again, "sale_opened").is_empty(),
        "a second kick changes nothing"
    );
    assert_eq!(w.view("get_sale", json!({})).await?["ends_at"], sale["ends_at"]);

    let create_gas = w.create_all_vaults().await?;
    println!(
        "GAS create_vaults(25) per batch: [{}] Tgas",
        create_gas
            .iter()
            .map(|g| format!("{:.2}", g.as_gas() as f64 / 1e12))
            .collect::<Vec<_>>()
            .join(", ")
    );
    w.mallory
        .transfer_near(&w.vault(1), NearToken::from_near(1))
        .await?
        .into_result()?;
    let pre_sale = w.claimable(&w.vault(1)).await?;
    let (dragon_before, creator_before) = (w.balance(w.dragon.id()).await?, w.balance(w.creator.id()).await?);
    let r = w.buy_scale(&w.alice, PRICE).await?;
    gas_line("buy_scale with its mint sweep", &r);
    assert_eq!(wyrm_events(&r, "scale_minted")[0]["token_id"], "1");
    assert_eq!(
        wyrm_events(&r, "scale_swept")[0],
        json!({ "token_ids": ["1"], "to": w.dragon.id() })
    );
    assert_eq!(r.into_result()?.json::<String>()?, "1");
    assert_eq!(w.balance(w.creator.id()).await? - creator_before, CREATOR_CUT);
    let to_dragon = w.balance(w.dragon.id()).await? - dragon_before;
    assert!(
        to_dragon >= PRICE - CREATOR_CUT + pre_sale && to_dragon < PRICE - CREATOR_CUT + pre_sale + DUST,
        "the Dragon got 1.8 NEAR and the vault's pre-sale NEAR, got {to_dragon}"
    );
    assert!(w.claimable(&w.vault(1)).await? < DUST, "the mint emptied the vault");
    let s = w.view("get_scale", json!({ "token_id": "1" })).await?;
    assert_eq!(
        (s["claimed_total"].as_str(), &s["claiming_since"]),
        (Some("0"), &Value::Null)
    );
    let history = w.view("get_history", json!({ "token_id": "1" })).await?;
    let swept: u128 = history[1]["Swept"]["amount"].as_str().unwrap().parse()?;
    assert!(
        swept >= pre_sale && swept < pre_sale + DUST,
        "history records the sweep: {history}"
    );

    let bob_before = w.balance(w.bob.id()).await?;
    let r = w.buy_scale(&w.bob, PRICE + NEAR / 3).await?;
    assert_eq!(r.into_result()?.json::<String>()?, "2", "ids mint from 1 upward");
    w.settle().await?;
    let paid = bob_before - w.balance(w.bob.id()).await?;
    assert!(
        paid >= PRICE && paid < PRICE + NEAR / 100,
        "bob paid the flat price and got the excess back, paid {paid}"
    );
    assert!(failure(w.buy_scale(&w.bob, PRICE - 1).await?).contains("below the price"));
    let s = w.view("get_scale", json!({ "token_id": "2" })).await?;
    assert_eq!(
        (s["owner"].as_str(), s["voting"].as_bool()),
        (Some(w.bob.id().as_str()), Some(true))
    );
    let creator_scale = w.view("get_scale", json!({ "token_id": "95" })).await?;
    assert_eq!(
        (creator_scale["owner"].as_str(), &creator_scale["voting"]),
        (Some(w.creator.id().as_str()), &json!(false))
    );
    Ok(())
}

#[tokio::test]
async fn scales_claim_pays_the_owner_from_a_real_keyless_vault() -> anyhow::Result<()> {
    let w = world().await?;
    w.open_sale().await?.into_result()?;
    let create_gas = w.create_all_vaults().await?;
    println!(
        "GAS create_vaults(25) per batch: [{}] Tgas",
        create_gas
            .iter()
            .map(|g| format!("{:.2}", g.as_gas() as f64 / 1e12))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let vault = w.vault(1);
    assert!(
        w.worker.view_access_keys(&vault).await?.is_empty(),
        "the vault has no keys"
    );
    let state = w.worker.view_account(&vault).await?.contract_state;
    assert!(
        matches!(state, ContractState::GlobalHash(_)),
        "the vault runs the global code: {state:?}"
    );
    assert_eq!(
        w.view("get_scale", json!({ "token_id": "100" })).await?["vault"],
        w.vault(100).as_str()
    );
    assert!(
        w.claimable(&vault).await? < DUST,
        "a new vault holds its reserve and the gas reward of its new()"
    );

    w.buy_scale(&w.alice, PRICE).await?.into_result()?;
    w.mallory
        .transfer_near(&vault, NearToken::from_near(2))
        .await?
        .into_result()?;
    let claimable = w.claimable(&vault).await?;
    assert!(claimable >= 2 * NEAR && claimable < 2 * NEAR + DUST);

    let before = w.balance(w.alice.id()).await?;
    let r = w.claim(&w.alice, "1").await?;
    println!("GAS claim: {} Tgas", r.total_gas_burnt.as_tgas());
    assert_eq!(r.into_result()?.json::<String>()?, claimable.to_string());
    w.settle().await?;
    let gained = w.balance(w.alice.id()).await? - before;
    assert!(
        gained > claimable - NEAR / 100 && gained <= claimable,
        "alice gained {gained}"
    );
    assert!(w.claimable(&vault).await? < DUST, "the vault is back to its reserve");
    let s = w.view("get_scale", json!({ "token_id": "1" })).await?;
    assert_eq!(s["claimed_total"], claimable.to_string());
    assert_eq!(s["claiming_since"], Value::Null);

    let outsider = w.claim(&w.mallory, "1").await?;
    assert!(failure(outsider).contains("not the owner"));
    Ok(())
}

#[tokio::test]
async fn scales_vault_release_refuses_anyone_but_the_parent() -> anyhow::Result<()> {
    let w = world().await?;
    w.create_all_vaults().await?;
    let vault = w.vault(7);
    w.alice
        .transfer_near(&vault, NearToken::from_near(1))
        .await?
        .into_result()?;
    let r = w
        .mallory
        .call(&vault, "release")
        .args_json(json!({ "to": w.mallory.id() }))
        .transact()
        .await?;
    assert!(failure(r).contains("only the parent may release"));
    assert!(
        w.balance(&vault).await? >= VAULT_RESERVE + NEAR,
        "nothing left the vault"
    );
    let parent: String = w.alice.view(&vault, "get_parent").await?.json()?;
    assert_eq!(parent, w.scales.id().as_str());
    Ok(())
}

#[tokio::test]
async fn scales_callbacks_refuse_a_stranger() -> anyhow::Result<()> {
    let w = world().await?;
    w.open_sale().await?.into_result()?;
    w.buy_scale(&w.alice, PRICE).await?.into_result()?;
    let calls = [
        (
            "on_claimed",
            json!({ "token_id": "1", "by": w.mallory.id(), "started_at": "0" }),
        ),
        ("on_vaults_created", json!({ "last": 100 })),
        ("on_executed", json!({ "proposal_id": 0 })),
        ("on_retired", json!({})),
        (
            "nft_resolve_transfer",
            json!({ "previous_owner_id": w.mallory.id(), "receiver_id": w.alice.id(), "token_id": "1", "previous_hold_start": "0" }),
        ),
    ];
    for (method, args) in calls {
        let r = w
            .mallory
            .call(w.scales.id(), method)
            .args_json(args)
            .max_gas()
            .transact()
            .await?;
        let err = failure(r);
        assert!(err.contains("is private"), "{method}: {err}");
    }
    let config = w.view("get_config", json!({})).await?;
    assert_eq!(config["vaults_created"], 0);
    Ok(())
}

#[tokio::test]
async fn scales_on_claimed_fits_its_static_gas_with_a_full_history_ring() -> anyhow::Result<()> {
    let w = world().await?;
    w.open_sale().await?.into_result()?;
    w.create_all_vaults().await?;
    w.buy_scale(&w.alice, PRICE).await?.into_result()?;
    for i in 0..20 {
        let (from, to) = if i % 2 == 0 {
            (&w.alice, &w.bob)
        } else {
            (&w.bob, &w.alice)
        };
        from.call(w.scales.id(), "nft_transfer")
            .args_json(json!({ "receiver_id": to.id(), "token_id": "1" }))
            .deposit(YOCTO)
            .transact()
            .await?
            .into_result()?;
    }
    let history = w.view("get_history", json!({ "token_id": "1" })).await?;
    assert_eq!(history.as_array().unwrap().len(), 16);
    w.mallory
        .transfer_near(&w.vault(1), NearToken::from_near(1))
        .await?
        .into_result()?;
    let claimable = w.claimable(&w.vault(1)).await?;
    let r = w.claim(&w.alice, "1").await?;
    println!("GAS claim with full history: {} Tgas", r.total_gas_burnt.as_tgas());
    assert_eq!(r.into_result()?.json::<String>()?, claimable.to_string());
    let history = w.view("get_history", json!({ "token_id": "1" })).await?;
    let last = history.as_array().unwrap().last().unwrap().clone();
    assert!(last.get("Claimed").is_some(), "the claim was recorded: {last}");
    Ok(())
}

#[tokio::test]
async fn scales_council_finalizes_a_signal_and_holds_the_leash_on_a_codeless_dragon() -> anyhow::Result<()> {
    let w = world().await?;
    w.open_sale().await?.into_result()?;
    for buyer in [&w.alice, &w.bob] {
        for _ in 0..5 {
            w.buy_scale(buyer, PRICE).await?.into_result()?;
        }
    }
    w.skip_to(w.now().await? + 7 * DAY + 60 * SEC).await?;
    let propose = |who: &Account, kind: Value, token: &str| {
        who.call(w.scales.id(), "propose")
            .args_json(json!({ "kind": kind, "description": "v4", "token_id": token }))
            .deposit(NearToken::from_millinear(1_010))
            .max_gas()
            .transact()
    };
    let signal: u64 = propose(&w.alice, json!("Signal"), "1").await?.into_result()?.json()?;
    let order: u64 = propose(&w.bob, json!({ "Order": { "SetSlices": { "cap_bps": 100 } } }), "6")
        .await?
        .into_result()?
        .json()?;
    for (who, tokens) in [
        (&w.alice, ["1", "2", "3", "4", "5"]),
        (&w.bob, ["6", "7", "8", "9", "10"]),
    ] {
        for p in [signal, order] {
            who.call(w.scales.id(), "vote")
                .args_json(json!({ "proposal_id": p, "token_ids": tokens, "approve": true }))
                .deposit(YOCTO)
                .transact()
                .await?
                .into_result()?;
        }
    }
    let ends_at: u64 = w.view("get_proposal", json!({ "id": signal })).await?["ends_at"]
        .as_str()
        .unwrap()
        .parse()?;
    w.skip_to(ends_at).await?;
    assert_eq!(w.view("get_due", json!({})).await?, json!([signal, order]));

    assert!(failure(w.finalize(signal, 149).await?).contains("attach at least 150 Tgas to finalize"));
    assert_eq!(w.view("get_proposal", json!({ "id": signal })).await?["status"], "Open");

    let r = w.finalize(signal, 150).await?;
    gas_line("finalize Signal", &r);
    assert!(
        r.receipt_outcomes().iter().all(|o| o.executor_id != *w.dragon.id()),
        "a Signal calls nothing"
    );
    assert_eq!(r.into_result()?.json::<Value>()?, json!("Carried"));

    let r = w.finalize(order, 150).await?;
    gas_line("finalize Order to a codeless Dragon", &r);
    assert_eq!(
        r.into_result()?.json::<Value>()?,
        json!({ "LeashHeld": { "error": "dragon call failed" } })
    );
    assert_eq!(w.view("get_due", json!({})).await?, json!([]));
    Ok(())
}
