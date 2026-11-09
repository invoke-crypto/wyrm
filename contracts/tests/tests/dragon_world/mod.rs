#![allow(dead_code)]
//! A sandbox world for the Dragon, launched and bound as on mainnet: mock-ft as wNEAR, mock-dcl, mock-nearly, the
//! tithe, scales with its 100 vaults on the published global vault code, and the Dragon. The creator calls
//! `dragon.launch`, Nearly (the mock) creates the coin as a taxed mock-ft and its pool, and the creator binds. scales
//! keeps its key here, so a test can make the Dragon `obey` as scales does without a vote.
use near_workspaces::network::{Sandbox, ValidatorKey};
use near_workspaces::operations::Function;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, AccountId, Contract, ContractState, Worker};
use serde_json::{json, Value};
use std::path::PathBuf;

pub const NEAR: u128 = 1_000_000_000_000_000_000_000_000;
pub const WYRM: u128 = 1_000_000_000_000_000_000;
pub const SEC: u64 = 1_000_000_000;
pub const MIN: u64 = 60 * SEC;
pub const HOUR: u64 = 60 * MIN;
pub const DAY: u64 = 24 * HOUR;
/// The pool's NEAR side; its WYRM side sets the price.
pub const POOL_NEAR: u128 = 1_000 * NEAR;
/// 1,000 WYRM per NEAR before the 1% pool fee.
pub const POOL_WYRM: u128 = 1_000_000 * WYRM;
pub const POKE_GAS: Gas = Gas::from_tgas(1_000);
pub const LAUNCH_GAS: Gas = Gas::from_tgas(500);
pub const BIND_GAS: Gas = Gas::from_tgas(150);
pub const LAUNCH_DEPOSIT: u128 = 2 * NEAR;
const GLOBAL_CODE_BYTE_COST: u128 = 10u128.pow(20);
pub const TITHE_RESERVE: u128 = 3 * NEAR;
pub const YOCTO: NearToken = NearToken::from_yoctonear(1);

pub struct World {
    pub worker: Worker<Sandbox>,
    /// The node behind `worker`; dropping it stops the node.
    pub node: near_sandbox::Sandbox,
    pub wrap: Contract,
    pub dcl: Contract,
    pub nearly: Contract,
    pub dragon: Contract,
    /// The scales contract, on an account that keeps its key.
    pub scales: Account,
    pub tithe: Account,
    pub ash: Account,
    pub creator: Account,
    pub keeper: Account,
    /// The bound coin, a mock-ft Nearly created.
    pub wyrm: AccountId,
    pub pool_id: String,
    pub launch_id: u64,
    pub launch: Option<ExecutionFinalResult>,
    pub bind: Option<ExecutionFinalResult>,
}

/// The wasm built by `cargo near build`, under CARGO_TARGET_DIR when set, else contracts/target.
pub fn wasm(crate_name: &str) -> Vec<u8> {
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"));
    let p = target.join(format!("near/{0}/{0}.wasm", crate_name));
    std::fs::read(&p).unwrap_or_else(|e| panic!("build the contracts first ({p:?}): {e}"))
}

/// A sandbox node whose first epoch outlasts every test. fast_forward crosses epochs one at a time, about 25 blocks
/// a second with the default 60-block epochs, so a day took most of an hour; inside one epoch it jumps days in a second.
pub async fn sandbox() -> anyhow::Result<(Worker<Sandbox>, near_sandbox::Sandbox)> {
    if std::env::var_os("NEAR_SANDBOX_LOG").is_none() {
        std::env::set_var("NEAR_SANDBOX_LOG", "near=error,stats=error,network=error");
    }
    let node = near_sandbox::Sandbox::start_sandbox_with_config(near_sandbox::SandboxConfig {
        additional_genesis: Some(json!({ "epoch_length": 1_000_000 })),
        additional_accounts: vec![near_sandbox::GenesisAccount::default_with_name("registrar".parse()?)],
        ..Default::default()
    })
    .await?;
    let worker = near_workspaces::sandbox()
        .rpc_addr(&node.rpc_addr)
        .validator_key(ValidatorKey::HomeDir(node.home_dir.path().to_path_buf()))
        .await?;
    Ok((worker, node))
}

pub struct Options {
    pub preset: &'static str,
    pub dragon_near: u128,
    /// Leave the coin's 4% tax on, as every launch asks Nearly for it. Off turns the token's tax off after bind,
    /// so the same suite runs on an untaxed coin.
    pub taxed: bool,
    /// Stop after deploying everything, before `launch`.
    pub unlaunched: bool,
    /// Calls of `create_vaults`, 25 vaults each.
    pub vault_batches: u8,
    /// A parent account, under the sandbox root, for the Dragon, scales, the tithe, ash and the creator, as
    /// `thewyrm.near` holds them on mainnet. None puts them under the root.
    pub root: Option<&'static str>,
}

impl Options {
    pub fn new(dragon_near: u128) -> Self {
        Self {
            preset: "hoarder",
            dragon_near,
            taxed: true,
            unlaunched: false,
            vault_batches: 4,
            root: None,
        }
    }
}

pub async fn setup(dragon_near: u128) -> anyhow::Result<World> {
    setup_with(Options::new(dragon_near)).await
}

/// The coin text every world launches.
pub fn coin(symbol: &str) -> Value {
    json!({ "name": "Wyrm", "symbol": symbol, "icon": "https://wyrm.example/icon.png", "description": "sandbox" })
}

pub async fn setup_with(o: Options) -> anyhow::Result<World> {
    let (worker, node) = sandbox().await?;
    let root = worker.root_account()?;
    let sub = |of: &Account, name: &'static str, near: u128| {
        let of = of.clone();
        async move {
            of.create_subaccount(name)
                .initial_balance(NearToken::from_yoctonear(near))
                .transact()
                .await
                .map(|r| r.into_result().expect("subaccount"))
        }
    };
    let contract = |of: &Account, name: &'static str, near: u128, krate: &'static str| {
        let sub = sub(of, name, near);
        async move { anyhow::Ok(sub.await?.deploy(&wasm(krate)).await?.into_result()?) }
    };
    let parent = match o.root {
        Some(name) => sub(&root, name, 1_000 * NEAR).await?,
        None => root.clone(),
    };
    let wrap = contract(&root, "wrap", 50 * NEAR, "mock_ft").await?;
    let dcl = contract(&root, "dcl", 50 * NEAR, "mock_dcl").await?;
    let nearly = contract(&root, "nearly", 60 * NEAR, "mock_nearly").await?;
    let dragon = contract(&parent, "dragon", o.dragon_near, "dragon").await?;
    let scales = sub(&parent, "scales", 30 * NEAR).await?;
    let tithe = sub(&parent, "tithe", TITHE_RESERVE).await?;
    let ash = sub(&parent, "ash", NEAR / 100).await?;
    let creator = sub(&parent, "creator", 100 * NEAR).await?;
    let keeper = sub(&root, "keeper", 50 * NEAR).await?;
    wrap.call("new")
        .args_json(json!({ "symbol": "wNEAR", "decimals": 24 }))
        .transact()
        .await?
        .into_result()?;
    dcl.call("new").transact().await?.into_result()?;
    nearly
        .call("new")
        .args_json(json!({ "dcl": dcl.id(), "wrap": wrap.id(), "pool_near": POOL_NEAR.to_string() }))
        .transact()
        .await?
        .into_result()?;
    nearly
        .call("set_token_code")
        .args(wasm("mock_ft"))
        .gas(Gas::from_tgas(300))
        .transact()
        .await?
        .into_result()?;
    // The pool's wNEAR inventory, so sells out of it can be paid.
    wrap.call("mint")
        .args_json(json!({ "account_id": dcl.id(), "amount": (1_000_000 * NEAR).to_string() }))
        .transact()
        .await?
        .into_result()?;

    dragon
        .call("new")
        .args_json(json!({
            "preset": o.preset,
            "peers": { "wrap": wrap.id(), "dcl": dcl.id(), "nearly": nearly.id(), "tithe": tithe.id(),
                       "scales": scales.id(), "ash": ash.id(), "creator": creator.id() },
        }))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?
        .into_result()?;
    tithe
        .batch(tithe.id())
        .deploy(&wasm("tithe"))
        .call(
            Function::new("new")
                .args_json(json!({
                    "dragon": dragon.id(), "scales": scales.id(), "wrap": wrap.id(),
                    "min_round": NEAR.to_string(), "reserve": TITHE_RESERVE.to_string(),
                }))
                .gas(Gas::from_tgas(50)),
        )
        .transact()
        .await?
        .into_result()?;
    let vault_code = wasm("scale_vault");
    let probe = worker.dev_deploy(&vault_code).await?;
    let ContractState::LocalHash(vault_code_hash) = probe.view_account().await?.contract_state else {
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
                    "preset": o.preset, "dragon": dragon.id(), "tithe": tithe.id(), "creator": creator.id(),
                    "vault_code_hash": vault_code_hash.to_string(),
                    "collection": { "name": "WYRM Scales", "symbol": "SCALE", "icon": null, "tier_media": tier_media },
                }))
                .gas(Gas::from_tgas(300)),
        )
        .transact()
        .await?
        .into_result()?;
    creator
        .call(scales.id(), "publish_vault_code")
        .args(vault_code.clone())
        .deposit(NearToken::from_yoctonear(
            vault_code.len() as u128 * GLOBAL_CODE_BYTE_COST,
        ))
        .max_gas()
        .transact()
        .await?
        .into_result()?;
    let mut w = World {
        worker,
        node,
        wrap,
        dcl,
        nearly,
        dragon,
        scales,
        tithe,
        ash,
        creator,
        keeper,
        wyrm: "unbound.test.near".parse()?,
        pool_id: String::new(),
        launch_id: 0,
        launch: None,
        bind: None,
    };
    for _ in 0..o.vault_batches {
        let res = w
            .keeper
            .call(w.scales.id(), "create_vaults")
            .max_gas()
            .transact()
            .await?;
        assert!(res.is_success(), "create_vaults: {:?}", res.failures());
    }
    if o.unlaunched {
        return Ok(w);
    }
    let launch = w.launch_coin(&coin("WYRM"), LAUNCH_DEPOSIT, LAUNCH_GAS).await;
    assert!(launch.is_success(), "launch: {:?}", launch.failures());
    w.launch_id = launch_started(&launch).expect("Nearly started a launch");
    let bind = w.bind_launch(w.launch_id, BIND_GAS).await;
    assert!(bind.is_success(), "bind: {:?}", bind.failures());
    let c: Value = w.dragon.view("get_coin").await?.json()?;
    assert!(!c.is_null(), "bound: {:?}", events(&bind));
    (w.launch, w.bind) = (Some(launch), Some(bind));
    w.wyrm = c["token"].as_str().unwrap().parse()?;
    w.pool_id = c["pool_id"].as_str().unwrap().into();
    if !o.taxed {
        w.keeper
            .call(&w.wyrm, "set_tax")
            .args_json(json!({ "tax": null }))
            .transact()
            .await?
            .into_result()?;
    }
    w.set_pool(POOL_NEAR, POOL_WYRM).await?;
    Ok(w)
}

/// The id in Nearly's `launch_started` event, the only place a launch id appears in its transaction.
pub fn launch_started(res: &ExecutionFinalResult) -> Option<u64> {
    res.logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .filter_map(|j| serde_json::from_str::<Value>(j).ok())
        .find(|v| v["standard"] == "nearpad" && v["event"] == "launch_started")
        .and_then(|v| v["data"][0]["id"].as_u64())
}

pub fn u(v: &Value) -> u128 {
    v.as_str()
        .unwrap_or_else(|| panic!("not a string amount: {v}"))
        .parse()
        .unwrap()
}

/// Every `wyrm` event a transaction emitted, as (name, data). The tokens' NEP-141 and the Scales' NEP-171 events in
/// the same transaction are skipped.
pub fn events(res: &ExecutionFinalResult) -> Vec<(String, Value)> {
    res.logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str::<Value>(j).unwrap())
        .filter(|v| v["standard"] == "wyrm")
        .map(|v| (v["event"].as_str().unwrap().to_string(), v["data"][0].clone()))
        .collect()
}

pub fn named(evs: &[(String, Value)], name: &str) -> Vec<Value> {
    evs.iter().filter(|(n, _)| n == name).map(|(_, d)| d.clone()).collect()
}

pub fn failure_text(res: &ExecutionFinalResult) -> String {
    format!("{:?}", res.receipt_failures())
}

pub fn tgas(g: Gas) -> f64 {
    g.as_gas() as f64 / 1e12
}

/// What each receipt of a transaction burnt, in execution order.
pub fn receipts(res: &ExecutionFinalResult) -> String {
    res.receipt_outcomes()
        .iter()
        .map(|o| format!("{} {:.2}", o.executor_id, tgas(o.gas_burnt)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn gas_line(name: &str, res: &ExecutionFinalResult) {
    println!(
        "GAS {name}: {:.2} Tgas burnt in all; the called method's receipt {:.2} Tgas; receipts [{}]",
        tgas(res.total_gas_burnt),
        tgas(res.receipt_outcomes()[0].gas_burnt),
        receipts(res)
    );
}

impl World {
    pub async fn now(&self) -> u64 {
        self.worker.view_block().await.unwrap().timestamp()
    }

    /// Moves chain time to at least `t`, and at most a few seconds past it. The node picks the time step of a
    /// fast-forwarded block (about 1.3 s), so the first jump assumes a longer step and falls short, and each later jump
    /// is sized from the step the previous one showed.
    pub async fn skip_to(&self, t: u64) {
        let mut step = 2 * SEC;
        loop {
            let now = self.now().await;
            if now >= t {
                return;
            }
            let blocks = ((t - now) / step).max(1);
            self.worker.fast_forward(blocks).await.unwrap();
            step = ((self.now().await - now) / blocks).max(1);
        }
    }

    /// A transaction counts as executed before its gas refund lands, so its signer's balance is read after a few blocks.
    pub async fn settle(&self) {
        self.worker.fast_forward(3).await.unwrap();
    }

    pub async fn skip(&self, ns: u64) {
        let now = self.now().await;
        self.skip_to(now + ns).await;
    }

    pub async fn launch_coin(&self, coin: &Value, deposit: u128, gas: Gas) -> ExecutionFinalResult {
        self.creator
            .call(self.dragon.id(), "launch")
            .args_json(json!({ "coin": coin }))
            .deposit(NearToken::from_yoctonear(deposit))
            .gas(gas)
            .transact()
            .await
            .unwrap()
    }

    pub async fn bind_launch(&self, launch_id: u64, gas: Gas) -> ExecutionFinalResult {
        self.creator
            .call(self.dragon.id(), "bind")
            .args_json(json!({ "launch_id": launch_id.to_string() }))
            .gas(gas)
            .transact()
            .await
            .unwrap()
    }

    pub async fn mint(&self, token: &AccountId, to: &str, amount: u128) -> anyhow::Result<()> {
        self.keeper
            .call(token, "mint")
            .args_json(json!({ "account_id": to, "amount": amount.to_string() }))
            .transact()
            .await?
            .into_result()?;
        Ok(())
    }

    pub async fn set_pool(&self, near: u128, wyrm: u128) -> anyhow::Result<()> {
        let (x, y) = if self.wrap.id().as_str() < self.wyrm.as_str() {
            (near, wyrm)
        } else {
            (wyrm, near)
        };
        self.dcl
            .call("set_pool")
            .args_json(json!({ "pool_id": self.pool_id, "total_x": x.to_string(), "total_y": y.to_string() }))
            .transact()
            .await?
            .into_result()?;
        Ok(())
    }

    pub async fn dcl_switch(&self, method: &str, on: bool) -> anyhow::Result<()> {
        let arg = if method == "set_fail_swaps" { "fail" } else { "hold" };
        self.dcl
            .call(method)
            .args_json(json!({ arg: on }))
            .transact()
            .await?
            .into_result()?;
        Ok(())
    }

    pub async fn poke(&self) -> anyhow::Result<ExecutionFinalResult> {
        Ok(self
            .keeper
            .call(self.dragon.id(), "poke")
            .gas(POKE_GAS)
            .transact()
            .await?)
    }

    /// A poke that must succeed; returns its events.
    pub async fn poke_ok(&self) -> Vec<(String, Value)> {
        let res = self.poke().await.unwrap();
        assert!(res.is_success(), "poke failed: {:?}", res.failures());
        events(&res)
    }

    pub async fn state(&self) -> Value {
        self.dragon.view("get_state").await.unwrap().json().unwrap()
    }

    pub async fn log(&self) -> Vec<Value> {
        self.dragon
            .view("get_log")
            .args_json(json!({ "limit": 256 }))
            .await
            .unwrap()
            .json()
            .unwrap()
    }

    pub async fn balance(&self, token: &AccountId, of: &str) -> u128 {
        let v: Value = self
            .worker
            .view(token, "ft_balance_of")
            .args_json(json!({ "account_id": of }))
            .await
            .unwrap()
            .json()
            .unwrap();
        u(&v)
    }

    /// Makes the Dragon obey `order` as scales does, with the 100 Tgas scales attaches.
    pub async fn obey(&self, order: Value) -> ExecutionFinalResult {
        self.scales
            .call(self.dragon.id(), "obey")
            .args_json(json!({ "order": order }))
            .gas(Gas::from_tgas(100))
            .transact()
            .await
            .unwrap()
    }

    /// Obeys and returns the Verdict the Dragon answered.
    pub async fn verdict(&self, order: Value) -> Value {
        let res = self.obey(order.clone()).await;
        assert!(res.is_success(), "obey {order}: {:?}", res.failures());
        res.json().unwrap()
    }

    /// The donation path: `amount` of the coin minted to a fresh account and sent with ft_transfer_call.
    pub async fn donate(&self, amount: u128) -> anyhow::Result<()> {
        let dev = self.worker.dev_create_account().await?;
        self.mint(&self.wyrm, dev.id().as_str(), amount).await?;
        dev.call(&self.wyrm, "ft_transfer_call")
            .args_json(json!({ "receiver_id": self.dragon.id(), "amount": amount.to_string(), "msg": "" }))
            .deposit(YOCTO)
            .max_gas()
            .transact()
            .await?
            .into_result()?;
        Ok(())
    }

    /// Moves to just after the start of a UTC day whose breath minute falls between `from` and `until` after midnight,
    /// observing once there so the day is rolled. Returns (day start, breath_at).
    pub async fn day_with_breath(&self, from: u64, until: u64) -> (u64, u64) {
        loop {
            let start = (self.now().await / DAY + 1) * DAY;
            self.skip_to(start + MIN).await;
            self.poke_ok().await;
            let breath_at = u(&self.state().await["day"]["breath_at"]) as u64;
            if breath_at >= start + from && breath_at <= start + until {
                return (start, breath_at);
            }
        }
    }
}
