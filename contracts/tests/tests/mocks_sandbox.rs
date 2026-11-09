//! The mocks that stand in for Nearly, its taxed token and DCL: mock-nearly's launch, park, failure and panic paths,
//! the mock-ft tax on buys, sells and refunds, and mock-dcl's missing pool and reserve cap. View shapes are checked
//! against the mainnet fixtures in contracts/mock-nearly/fixtures.
mod dragon_world;

use dragon_world::{sandbox, u, wasm, DAY, NEAR, SEC};
use near_workspaces::network::Sandbox;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, AccountId, Contract, Worker};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const POOL_NEAR: u128 = 1_000 * NEAR;
const SUPPLY: u128 = 1_000_000_000 * 10u128.pow(18);
const POOL_CREATE: u128 = 14 * 10u128.pow(21);
const NO_TAX: Value = Value::Null;

struct Mocks {
    worker: Worker<Sandbox>,
    _node: near_sandbox::Sandbox,
    wrap: Contract,
    dcl: Contract,
    nearly: Contract,
    alice: Account,
    bob: Account,
}

fn tax_args() -> Value {
    json!({ "buy_bps": 400, "sell_bps": 400, "creator_bps": 10000, "burn_bps": 0, "holders_bps": 0 })
}

fn coin(symbol: &str, tax: Value, dev_buy: u128) -> Value {
    json!({ "name": "Wyrm", "symbol": symbol, "fee_to": "tithe.test.near", "tax": tax, "icon": "https://x.io/w.png",
            "dev_buy": (dev_buy > 0).then(|| dev_buy.to_string()) })
}

fn tgas(g: near_workspaces::types::Gas) -> f64 {
    g.as_gas() as f64 / 1e12
}

/// Every log line of one event standard, as (event, data).
fn events(res: &ExecutionFinalResult, standard: &str) -> Vec<(String, Value)> {
    res.logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str::<Value>(j).unwrap())
        .filter(|v| v["standard"] == standard)
        .map(|v| (v["event"].as_str().unwrap().to_string(), v["data"][0].clone()))
        .collect()
}

fn named(res: &ExecutionFinalResult, name: &str) -> Vec<Value> {
    events(res, "nearpad")
        .into_iter()
        .filter(|(n, _)| n == name)
        .map(|(_, d)| d)
        .collect()
}

fn transfers_with_memo(res: &ExecutionFinalResult, memo: &str) -> Vec<Value> {
    events(res, "nep141")
        .into_iter()
        .filter(|(n, d)| n == "ft_transfer" && d["memo"] == memo)
        .map(|(_, d)| d)
        .collect()
}

fn ret(res: &ExecutionFinalResult) -> Value {
    res.clone().json().unwrap()
}

/// One line per receipt: who ran it and what it burnt.
fn receipts(res: &ExecutionFinalResult) {
    for o in res.receipt_outcomes() {
        println!("  receipt {} {:.2} Tgas", o.executor_id, tgas(o.gas_burnt));
    }
}

fn failures(res: &ExecutionFinalResult) -> String {
    format!("{:?}", res.receipt_failures())
}

fn key_set(v: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    if let Value::Object(m) = v {
        for (k, child) in m {
            out.insert(format!("{prefix}{k}"));
            key_set(child, &format!("{prefix}{k}."), out);
        }
    }
}

fn keys(v: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    key_set(v, "", &mut out);
    out
}

fn fixture(view: &str) -> Value {
    let p = format!("{}/../mock-nearly/fixtures/{view}.json", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"));
    serde_json::from_str::<Value>(&raw).unwrap()["result"].clone()
}

impl Mocks {
    async fn new() -> anyhow::Result<Self> {
        let (worker, node) = sandbox().await?;
        let root = worker.root_account()?;
        let deploy = |name: &'static str, krate: &'static str| {
            let root = root.clone();
            async move {
                let acc = root
                    .create_subaccount(name)
                    .initial_balance(NearToken::from_near(60))
                    .transact()
                    .await?
                    .into_result()?;
                anyhow::Ok(acc.deploy(&wasm(krate)).await?.into_result()?)
            }
        };
        let wrap = deploy("wrap", "mock_ft").await?;
        let dcl = deploy("dcl", "mock_dcl").await?;
        let nearly = deploy("nearly", "mock_nearly").await?;
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
        wrap.call("mint")
            .args_json(json!({ "account_id": dcl.id(), "amount": (10 * POOL_NEAR).to_string() }))
            .transact()
            .await?
            .into_result()?;
        let user = |name: &'static str| {
            let root = root.clone();
            async move {
                anyhow::Ok(
                    root.create_subaccount(name)
                        .initial_balance(NearToken::from_near(100))
                        .transact()
                        .await?
                        .into_result()?,
                )
            }
        };
        let alice = user("alice").await?;
        let bob = user("bob").await?;
        Ok(Self {
            worker,
            _node: node,
            wrap,
            dcl,
            nearly,
            alice,
            bob,
        })
    }

    async fn view(&self, on: &AccountId, method: &str, args: Value) -> anyhow::Result<Value> {
        Ok(self.worker.view(on, method).args_json(args).await?.json()?)
    }

    async fn nearly_view(&self, method: &str, args: Value) -> Value {
        self.view(self.nearly.id(), method, args).await.unwrap()
    }

    async fn quote_total(&self, dev_buy: u128) -> u128 {
        let q = self
            .nearly_view(
                "quote_launch",
                json!({ "icon_bytes": 0, "dev_buy": dev_buy.to_string(), "tax": true }),
            )
            .await;
        u(&q["total"])
    }

    async fn launch(&self, by: &Account, coin: Value) -> ExecutionFinalResult {
        self.launch_with(by, coin, 300).await
    }

    /// `by` launches with the quote plus 0.1 NEAR attached. Nearly returns the unused part.
    async fn launch_with(&self, by: &Account, coin: Value, tgas: u64) -> ExecutionFinalResult {
        let dev_buy = coin["dev_buy"].as_str().map_or(0, |d| d.parse().unwrap());
        let deposit = self.quote_total(dev_buy).await + NEAR / 10;
        by.call(self.nearly.id(), "launch")
            .args_json(json!({ "args": coin }))
            .deposit(NearToken::from_yoctonear(deposit))
            .gas(Gas::from_tgas(tgas))
            .transact()
            .await
            .unwrap()
    }

    async fn resume(&self, by: &Account, id: u64, tgas: u64) -> ExecutionFinalResult {
        by.call(self.nearly.id(), "resume")
            .args_json(json!({ "launch_id": id.to_string() }))
            .gas(Gas::from_tgas(tgas))
            .transact()
            .await
            .unwrap()
    }

    async fn set_mode(&self, mode: &str) {
        let res = self
            .nearly
            .call("set_mode")
            .args_json(json!({ "mode": mode }))
            .transact()
            .await
            .unwrap();
        assert!(res.is_success(), "{}", failures(&res));
    }

    /// A second mock-nearly that only relays launch and resume, so the creator is a contract, as the Dragon is.
    async fn contract_caller(&self) -> Contract {
        let caller = self
            .worker
            .root_account()
            .unwrap()
            .create_subaccount("caller")
            .initial_balance(NearToken::from_near(10))
            .transact()
            .await
            .unwrap()
            .into_result()
            .unwrap()
            .deploy(&wasm("mock_nearly"))
            .await
            .unwrap()
            .into_result()
            .unwrap();
        let init = caller
            .call("new")
            .args_json(json!({ "dcl": self.dcl.id(), "wrap": self.wrap.id(), "pool_near": "0" }))
            .transact()
            .await
            .unwrap();
        assert!(init.is_success(), "{}", failures(&init));
        caller
    }

    async fn record(&self, id: u64) -> Value {
        self.nearly_view("get_launch", json!({ "launch_id": id.to_string() }))
            .await
    }

    /// Read a few blocks after the last transaction, so its gas refund has landed.
    async fn near(&self, of: &AccountId) -> u128 {
        self.worker.fast_forward(3).await.unwrap();
        self.worker.view_account(of).await.unwrap().balance.as_yoctonear()
    }

    async fn ft(&self, token: &AccountId, of: &AccountId) -> u128 {
        u(&self
            .view(token, "ft_balance_of", json!({ "account_id": of }))
            .await
            .unwrap())
    }

    async fn call(
        &self,
        by: &Account,
        on: &AccountId,
        method: &str,
        args: Value,
        deposit: u128,
    ) -> ExecutionFinalResult {
        by.call(on, method)
            .args_json(args)
            .deposit(NearToken::from_yoctonear(deposit))
            .gas(Gas::from_tgas(300))
            .transact()
            .await
            .unwrap()
    }

    async fn register(&self, token: &AccountId, who: &Account) {
        let res = self.call(who, token, "storage_deposit", json!({}), NEAR / 100).await;
        assert!(res.is_success(), "{}", failures(&res));
    }

    async fn quote(&self, pool_id: &str, input: &AccountId, output: &AccountId, amount: u128) -> anyhow::Result<u128> {
        let q = self
            .view(
                self.dcl.id(),
                "quote",
                json!({ "pool_ids": [pool_id], "input_token": input, "output_token": output,
                        "input_amount": amount.to_string() }),
            )
            .await?;
        Ok(u(&q["amount"]))
    }

    /// Swaps by ft_transfer_call into mock-dcl, as the Dragon and every mainnet caller do.
    async fn swap(
        &self,
        by: &Account,
        token: &AccountId,
        pool_id: &str,
        out: &AccountId,
        amount: u128,
        min_out: u128,
    ) -> ExecutionFinalResult {
        let msg =
            json!({ "Swap": { "pool_ids": [pool_id], "output_token": out, "min_output_amount": min_out.to_string() } });
        self.call(
            by,
            token,
            "ft_transfer_call",
            json!({ "receiver_id": self.dcl.id(), "amount": amount.to_string(), "msg": msg.to_string() }),
            1,
        )
        .await
    }

    /// Moves chain time past `ns` from now. A fast-forwarded block is about 1.3 s.
    async fn skip(&self, ns: u64) {
        let target = self.worker.view_block().await.unwrap().timestamp() + ns;
        while self.worker.view_block().await.unwrap().timestamp() < target {
            let left = target - self.worker.view_block().await.unwrap().timestamp();
            self.worker.fast_forward((left / SEC).max(1)).await.unwrap();
        }
    }

    /// A taxed coin, launched by alice and live. Returns (token, pool id).
    async fn live_coin(&self) -> (AccountId, String) {
        let res = self.launch(&self.alice, coin("WYRM", tax_args(), 0)).await;
        assert!(res.is_success(), "{}", failures(&res));
        let l = self.record(0).await;
        assert_eq!(l["step"], "Done");
        (
            l["token"].as_str().unwrap().parse().unwrap(),
            l["pool_id"].as_str().unwrap().into(),
        )
    }
}

#[tokio::test]
async fn a_launch_creates_the_coin_and_the_pool_and_answers_in_mainnet_shapes() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    let before = m.near(m.alice.id()).await;
    let res = m.launch(&m.alice, coin("WYRM", tax_args(), 0)).await;
    assert!(res.is_success(), "{}", failures(&res));
    println!(
        "GAS mock-nearly launch (taxed, no dev buy): {:.2} Tgas burnt",
        tgas(res.total_gas_burnt)
    );
    receipts(&res);
    assert_eq!(ret(&res), true, "the step chain reached Done in the launch transaction");
    assert_eq!(named(&res, "launch").len(), 1, "Done emits Nearly's launch event");
    let started = named(&res, "launch_started");
    assert_eq!(started.len(), 1);
    assert_eq!(started[0]["creator"], m.alice.id().as_str());
    assert_eq!(started[0]["token"], "wyrm.nearly.test.near");

    let l = m.record(0).await;
    assert_eq!(l["creator"], m.alice.id().as_str());
    assert_eq!(l["step"], "Done");
    assert_eq!(l["pool_id"], "wrap.test.near|wyrm.nearly.test.near|10000");
    assert!(l["lpt_id"].is_string());
    assert!(
        l["icon"].is_null(),
        "Nearly drops the icon from the record at CreateToken"
    );
    let token: AccountId = "wyrm.nearly.test.near".parse()?;
    let reg = m
        .view(&token, "storage_balance_of", json!({ "account_id": m.alice.id() }))
        .await?;
    assert!(!reg.is_null(), "Nearly registered the creator on its coin");
    assert_eq!(
        m.ft(&token, m.dcl.id()).await,
        SUPPLY,
        "the whole supply is in the pool"
    );
    let tax = m.view(&token, "get_tax", json!({})).await?;
    assert_eq!(tax["pairs"], json!([m.dcl.id()]));
    assert_eq!(
        (tax["buy_bps"].as_u64(), tax["sell_bps"].as_u64()),
        (Some(400), Some(400))
    );
    let pool = m
        .view(m.dcl.id(), "get_pool", json!({ "pool_id": l["pool_id"] }))
        .await?;
    assert_eq!(
        (pool["token_x"].as_str(), pool["state"].as_str()),
        (Some("wrap.test.near"), Some("Running"))
    );
    let real_pool = fixture("get_pool");
    println!(
        "view get_pool: {} keys, fixture {} keys",
        keys(&pool).len(),
        keys(&real_pool).len()
    );
    assert_eq!(keys(&pool), keys(&real_pool), "get_pool keys");
    for (k, real) in real_pool.as_object().unwrap() {
        assert_eq!(
            std::mem::discriminant(&pool[k]),
            std::mem::discriminant(real),
            "get_pool {k} JSON type"
        );
    }

    let spent = before - m.near(m.alice.id()).await;
    let icon_bytes = "https://x.io/w.png".len();
    let exact = json!({ "icon_bytes": icon_bytes, "dev_buy": null, "tax": true, "text_bytes": "WyrmWYRM".len() });
    let quoted = u(&m.nearly_view("quote_launch", exact).await["total"]);
    assert!(
        spent + NEAR / 100 > quoted && spent < quoted + NEAR / 100,
        "the creator paid the quote and got the extra back: spent {spent}, quote {quoted}"
    );

    let id = json!({ "launch_id": "0" });
    let views = [
        ("get_launch", id.clone()),
        ("get_fee_to", id.clone()),
        ("get_fee_opts", id.clone()),
        ("get_tax", id.clone()),
        ("get_slug_count", json!({ "slug": "wyrm" })),
        (
            "quote_launch",
            json!({ "icon_bytes": 0, "dev_buy": null, "tax": true, "text_bytes": null }),
        ),
        ("get_dev_buy_cap", json!({})),
    ];
    for (view, args) in views {
        let mine = m.nearly_view(view, args).await;
        let real = fixture(view);
        println!(
            "view {view}: {} keys, fixture {} keys",
            keys(&mine).len(),
            keys(&real).len()
        );
        assert_eq!(keys(&mine), keys(&real), "{view} keys");
        assert!(!mine.is_null(), "{view} answered");
    }
    let by_symbol = m.nearly_view("get_launch_by_symbol", json!({ "symbol": "wyrm" })).await;
    assert_eq!(
        keys(&by_symbol),
        keys(&fixture("get_launch")),
        "get_launch_by_symbol keys"
    );
    assert_eq!(by_symbol["id"], 0);
    assert_eq!(m.nearly_view("get_fee_to", id.clone()).await, "tithe.test.near");
    assert_eq!(m.nearly_view("get_slug_count", json!({ "slug": "wyrm" })).await, 1);

    let again = m.launch(&m.bob, coin("WYRM", tax_args(), 0)).await;
    assert!(again.is_success(), "{}", failures(&again));
    assert_eq!(m.record(1).await["token"], "wyrm-2.nearly.test.near");
    Ok(())
}

#[tokio::test]
async fn the_taxed_token_taxes_buys_and_sells_and_refunds_the_gross() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    let (token, pool_id) = m.live_coin().await;
    for who in [&m.alice, &m.bob] {
        m.register(m.wrap.id(), who).await;
        m.register(&token, who).await;
    }
    let res = m
        .call(&m.alice, m.wrap.id(), "near_deposit", json!({}), 10 * NEAR)
        .await;
    assert!(res.is_success(), "{}", failures(&res));

    let quoted = m.quote(&pool_id, m.wrap.id(), &token, NEAR).await?;
    let buy = m.swap(&m.alice, m.wrap.id(), &pool_id, &token, NEAR, quoted).await;
    assert!(
        buy.is_success() && buy.receipt_failures().is_empty(),
        "{}",
        failures(&buy)
    );
    println!("GAS taxed buy of 1 NEAR: {:.2} Tgas burnt", tgas(buy.total_gas_burnt));
    receipts(&buy);
    let got = m.ft(&token, m.alice.id()).await;
    let tax = quoted * 400 / 10_000;
    println!("buy amount_out {quoted}, buyer received {got}, tax {tax}");
    assert_eq!(got, quoted - tax, "the buyer receives 96% of amount_out");
    let taxed = transfers_with_memo(&buy, "tax");
    assert_eq!(taxed.len(), 1);
    assert_eq!(taxed[0]["old_owner_id"], m.dcl.id().as_str());
    assert_eq!(taxed[0]["new_owner_id"], token.as_str());
    assert_eq!(u(&taxed[0]["amount"]), tax);
    assert_eq!(m.ft(&token, &token).await, tax, "the tax stays on the token account");
    let pool = m.view(m.dcl.id(), "get_pool", json!({ "pool_id": pool_id })).await?;
    assert_eq!(
        (u(&pool["volume_x_in"]), u(&pool["volume_y_out"])),
        (NEAR, quoted),
        "the buy shows in the pool's volume"
    );
    assert_eq!(
        u(&pool["total_fee_x_charged"]),
        NEAR / 100 / 5,
        "DCL's protocol part of the 1% fee"
    );

    let fail = m.swap(&m.alice, &token, &pool_id, m.wrap.id(), got, u128::MAX).await;
    assert!(failures(&fail).contains("E204: slippage error"), "{}", failures(&fail));
    assert_eq!(m.ft(&token, m.alice.id()).await, got, "a failed sell returns the gross");
    assert_eq!(transfers_with_memo(&fail, "tax refund").len(), 1);
    assert_eq!(ret(&fail), "0", "the token reports nothing used");

    let half = got / 2;
    let send = m
        .call(
            &m.alice,
            &token,
            "ft_transfer",
            json!({ "receiver_id": m.bob.id(), "amount": half.to_string() }),
            1,
        )
        .await;
    assert!(send.is_success(), "{}", failures(&send));
    assert_eq!(m.ft(&token, m.bob.id()).await, half, "wallet to wallet pays no tax");
    assert!(transfers_with_memo(&send, "tax").is_empty());

    let pool_before = m.ft(&token, m.dcl.id()).await;
    let sell = m.swap(&m.bob, &token, &pool_id, m.wrap.id(), half, 0).await;
    assert!(sell.receipt_failures().is_empty(), "{}", failures(&sell));
    println!("GAS taxed sell: {:.2} Tgas burnt", tgas(sell.total_gas_burnt));
    assert_eq!(
        m.ft(&token, m.dcl.id()).await - pool_before,
        half - half * 400 / 10_000,
        "the pool gets the net"
    );

    let off = m.call(&m.alice, &token, "set_tax", json!({ "tax": null }), 0).await;
    assert!(off.is_success(), "{}", failures(&off));
    let q2 = m.quote(&pool_id, m.wrap.id(), &token, NEAR).await?;
    let before = m.ft(&token, m.alice.id()).await;
    let plain = m.swap(&m.alice, m.wrap.id(), &pool_id, &token, NEAR, q2).await;
    assert!(plain.receipt_failures().is_empty(), "{}", failures(&plain));
    println!(
        "GAS untaxed buy of 1 NEAR: {:.2} Tgas burnt",
        tgas(plain.total_gas_burnt)
    );
    receipts(&plain);
    assert_eq!(
        m.ft(&token, m.alice.id()).await - before,
        q2,
        "with the tax off the buyer gets amount_out"
    );
    Ok(())
}

#[tokio::test]
async fn mock_dcl_refuses_a_missing_pool_and_caps_output_at_reserves() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    let (token, pool_id) = m.live_coin().await;
    let missing = "wrap.test.near|zzz.test.near|10000";
    let err = m.quote(missing, m.wrap.id(), &token, NEAR).await.unwrap_err();
    println!("quote on a missing pool: {err}");
    assert!(format!("{err:?}").contains("E403: pool not exist"));
    assert!(m
        .view(m.dcl.id(), "get_pool", json!({ "pool_id": missing }))
        .await?
        .is_null());

    let huge = m.quote(&pool_id, m.wrap.id(), &token, 1_000_000_000 * NEAR).await?;
    let pool = m.view(m.dcl.id(), "get_pool", json!({ "pool_id": pool_id })).await?;
    let reserve = u(&pool["total_y"]);
    println!("1e9 NEAR quotes {huge} of a {reserve} reserve");
    assert!(huge > 0 && huge <= reserve);
    Ok(())
}

#[tokio::test]
async fn a_parked_launch_resumes_to_done_and_a_precreated_pool_fails_it() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    m.set_mode("park").await;
    let res = m.launch(&m.alice, coin("WYRM", tax_args(), 0)).await;
    assert!(res.is_success(), "{}", failures(&res));
    assert_eq!(ret(&res), false, "a parked launch reports not done");
    let parked = named(&res, "launch_step_parked");
    assert_eq!(parked.len(), 1);
    assert_eq!(parked[0]["step"], "CreatePool");
    assert_eq!(parked[0]["reason"], "create_pool refused and no pool exists");
    assert_eq!(m.record(0).await["step"], "CreatePool");
    m.set_mode("panic").await;
    let paused = m.resume(&m.bob, 0, 300).await;
    assert!(failures(&paused).contains("paused"), "{}", failures(&paused));
    assert_eq!(m.record(0).await["step"], "CreatePool");
    m.set_mode("park").await;
    let resumed = m
        .call(&m.bob, m.nearly.id(), "resume", json!({ "launch_id": "0" }), 0)
        .await;
    assert!(resumed.is_success(), "{}", failures(&resumed));
    assert_eq!(ret(&resumed), true);
    assert_eq!(m.record(0).await["step"], "Done");
    println!("park run: launch parked at CreatePool, resume by a stranger reached Done");

    m.set_mode("normal").await;
    let grief = "wrap.test.near|wyrm-2.nearly.test.near|10000";
    m.dcl
        .call("set_pool")
        .args_json(json!({ "pool_id": grief, "total_x": NEAR.to_string(), "total_y": (2 * NEAR).to_string() }))
        .transact()
        .await?
        .into_result()?;
    let dev_buy = NEAR;
    let before = m.near(m.bob.id()).await;
    let res = m.launch(&m.bob, coin("WYRM", tax_args(), dev_buy)).await;
    assert!(res.is_success(), "{}", failures(&res));
    assert_eq!(ret(&res), false);
    assert_eq!(m.record(1).await["step"], "Failed");
    assert_eq!(named(&res, "launch_failed").len(), 1);
    let refunded = named(&res, "pool_deposit_refunded");
    assert_eq!(refunded.len(), 1);
    assert_eq!(u(&refunded[0]["amount"]), POOL_CREATE);
    let spent = before - m.near(m.bob.id()).await;
    let quoted = m.quote_total(dev_buy).await;
    assert!(
        spent < quoted - POOL_CREATE + NEAR / 20,
        "the pool deposit came back: spent {spent}, quote {quoted}"
    );
    println!("grief run: launch Failed, pool deposit {POOL_CREATE} refunded to the creator");

    let before = m.near(m.bob.id()).await;
    let r = m
        .call(&m.alice, m.nearly.id(), "refund_failed", json!({ "launch_id": "1" }), 0)
        .await;
    assert!(r.is_success(), "{}", failures(&r));
    assert_eq!(
        m.near(m.bob.id()).await - before,
        dev_buy,
        "anyone sends the held dev buy to the creator"
    );
    let again = m
        .call(&m.alice, m.nearly.id(), "refund_failed", json!({ "launch_id": "1" }), 0)
        .await;
    assert!(failures(&again).contains("nothing to refund"));
    Ok(())
}

#[tokio::test]
async fn a_launch_that_panics_refunds_the_whole_deposit_to_a_contract_caller() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    let caller = m.contract_caller().await;
    m.set_mode("panic").await;
    let deposit = m.quote_total(0).await + NEAR / 10;
    let before = m.near(caller.id()).await;
    let res = m
        .call(
            &m.alice,
            caller.id(),
            "call_launch",
            json!({ "nearly": m.nearly.id(), "args": coin("WYRM", tax_args(), 0) }),
            deposit,
        )
        .await;
    assert!(failures(&res).contains("launches paused"), "{}", failures(&res));
    let gained = m.near(caller.id()).await - before;
    println!("caller sent {deposit} and holds {gained} more after the refusal");
    assert!(
        gained >= deposit && gained < deposit + NEAR / 1_000,
        "the runtime refunded the whole deposit to the contract caller"
    );
    assert!(named(&res, "launch_started").is_empty());
    assert!(m.record(0).await.is_null());
    Ok(())
}

#[tokio::test]
async fn a_dev_buy_reaches_the_creator_net_of_tax_and_a_refunded_one_parks_for_the_creator() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    let dev_buy = 2 * NEAR;
    let res = m.launch(&m.alice, coin("WYRM", tax_args(), dev_buy)).await;
    assert!(res.is_success(), "{}", failures(&res));
    println!(
        "GAS mock-nearly launch (taxed, 2 NEAR dev buy): {:.2} Tgas burnt",
        tgas(res.total_gas_burnt)
    );
    receipts(&res);
    assert_eq!(ret(&res), true);
    let l = m.record(0).await;
    assert_eq!((l["step"].as_str(), u(&l["dev_buy_held"])), (Some("Done"), 0));
    let token: AccountId = "wyrm.nearly.test.near".parse()?;
    let got = m.ft(&token, m.alice.id()).await;
    let tax = transfers_with_memo(&res, "tax");
    assert_eq!(tax.len(), 1, "the dev buy is a pool transfer, so it is taxed");
    assert_eq!(
        got * 10_000 / (10_000 - 400) / 1_000,
        (got + u(&tax[0]["amount"])) / 1_000
    );

    m.dcl
        .call("set_fail_swaps")
        .args_json(json!({ "fail": true }))
        .transact()
        .await?
        .into_result()?;
    let res = m.launch(&m.bob, coin("WYRM", tax_args(), dev_buy)).await;
    assert_eq!(ret(&res), false);
    let parked = named(&res, "launch_step_parked");
    assert_eq!(
        (parked[0]["step"].as_str(), parked[0]["reason"].as_str()),
        (Some("DevBuy"), Some("swap refunded"))
    );
    let stranger = m
        .call(&m.alice, m.nearly.id(), "resume", json!({ "launch_id": "1" }), 0)
        .await;
    assert!(failures(&stranger).contains("dev buy: creator, owner or relayer only"));
    m.dcl
        .call("set_fail_swaps")
        .args_json(json!({ "fail": false }))
        .transact()
        .await?
        .into_result()?;
    let short = m.resume(&m.bob, 1, 234).await;
    assert!(
        failures(&short).contains("attach more gas for the dev buy"),
        "{}",
        failures(&short)
    );
    assert_eq!(m.record(1).await["step"], "DevBuy", "the launch stays parked");
    let creator = m.resume(&m.bob, 1, 240).await;
    assert!(creator.is_success(), "{}", failures(&creator));
    assert_eq!(ret(&creator), true);
    assert!(m.ft(&"wyrm-2.nearly.test.near".parse()?, m.bob.id()).await > 0);
    Ok(())
}

#[tokio::test]
async fn a_stuck_launch_cancels_after_a_day_and_fees_push_once() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    m.set_mode("park").await;
    let dev_buy = NEAR;
    let res = m.launch(&m.alice, coin("WYRM", NO_TAX, dev_buy)).await;
    assert_eq!(ret(&res), false);
    let early = m
        .call(
            &m.alice,
            m.nearly.id(),
            "cancel_stuck_launch",
            json!({ "launch_id": "0" }),
            0,
        )
        .await;
    assert!(failures(&early).contains("24h after it was created"));
    m.skip(DAY + 60 * SEC).await;
    let before = m.near(m.alice.id()).await;
    let cancel = m
        .call(
            &m.alice,
            m.nearly.id(),
            "cancel_stuck_launch",
            json!({ "launch_id": "0" }),
            0,
        )
        .await;
    assert!(cancel.is_success(), "{}", failures(&cancel));
    assert_eq!(m.record(0).await["step"], "Failed");
    let back = m.near(m.alice.id()).await + NEAR / 100 - before;
    assert!(
        back >= dev_buy && back < dev_buy + NEAR / 100,
        "the held dev buy came back: {back}"
    );

    let tithe: AccountId = "tithe.test.near".parse()?;
    let credit = m
        .call(
            &m.bob,
            m.nearly.id(),
            "credit_creator_fees",
            json!({ "account_id": m.alice.id() }),
            NEAR,
        )
        .await;
    assert!(credit.is_success(), "{}", failures(&credit));
    let before = m.near(m.alice.id()).await;
    let push = m
        .call(
            &m.bob,
            m.nearly.id(),
            "push_creator_fees",
            json!({ "account_id": m.alice.id() }),
            0,
        )
        .await;
    assert!(push.is_success(), "{}", failures(&push));
    assert_eq!(
        m.near(m.alice.id()).await - before,
        NEAR,
        "anyone pushes the payee's fees as a plain transfer"
    );
    assert_eq!(named(&push, "creator_fees_claimed")[0]["pushed"], true);
    let twice = m
        .call(
            &m.bob,
            m.nearly.id(),
            "push_creator_fees",
            json!({ "account_id": m.alice.id() }),
            0,
        )
        .await;
    assert!(failures(&twice).contains("nothing to push"));
    let none = m.nearly_view("get_creator_fees", json!({ "account_id": tithe })).await;
    assert_eq!(none, "0");
    Ok(())
}

#[tokio::test]
async fn a_short_gassed_launch_parks_reports_true_and_anyone_resumes_it() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    let res = m.launch_with(&m.alice, coin("WYRM", tax_args(), 0), 150).await;
    assert!(res.is_success(), "{}", failures(&res));
    assert_eq!(ret(&res), true, "mainnet returns true for a gas park");
    let parked = named(&res, "launch_step_parked");
    assert_eq!(parked.len(), 1);
    assert_eq!(parked[0], json!({ "id": 0, "step": "AddLiquidity" }));
    let short = m.resume(&m.bob, 0, 221).await;
    assert!(
        failures(&short).contains("attach more gas: adding liquidity needs 300 TGas"),
        "{}",
        failures(&short)
    );
    assert_eq!(m.record(0).await["step"], "AddLiquidity", "the launch stays parked");
    let resumed = m.resume(&m.bob, 0, 226).await;
    assert!(resumed.is_success(), "{}", failures(&resumed));
    assert_eq!(ret(&resumed), true);
    assert_eq!(m.record(0).await["step"], "Done");
    Ok(())
}

#[tokio::test]
async fn a_parked_first_buy_needs_the_creator_and_260_tgas() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    let res = m.launch_with(&m.alice, coin("WYRM", tax_args(), NEAR), 200).await;
    assert!(res.is_success(), "{}", failures(&res));
    assert_eq!(ret(&res), true, "mainnet returns true for a first-buy park");
    let parked = named(&res, "launch_step_parked");
    assert_eq!(
        (parked[0]["step"].as_str(), parked[0]["reason"].as_str()),
        (Some("AddLiquidity"), Some("first buy: resume with at least 260 TGas"))
    );
    let stranger = m.resume(&m.bob, 0, 300).await;
    assert!(failures(&stranger).contains("first buy pending: only the creator, the owner or a relayer"));
    let short = m.resume(&m.alice, 0, 250).await;
    assert!(
        failures(&short).contains("attach at least 260 TGas"),
        "{}",
        failures(&short)
    );
    let creator = m.resume(&m.alice, 0, 300).await;
    assert!(creator.is_success(), "{}", failures(&creator));
    assert_eq!(ret(&creator), true);
    assert_eq!(m.record(0).await["step"], "Done");
    assert!(m.ft(&"wyrm.nearly.test.near".parse()?, m.alice.id()).await > 0);
    Ok(())
}

#[tokio::test]
async fn a_dev_buy_resumed_through_a_contract_parks_again_for_the_signer() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    let caller = m.contract_caller().await;
    m.set_mode("park").await;
    let deposit = m.quote_total(NEAR).await + NEAR / 10;
    let args = json!({ "nearly": m.nearly.id(), "args": coin("WYRM", tax_args(), NEAR) });
    let _ = m.call(&m.alice, caller.id(), "call_launch", args, deposit).await;
    assert_eq!(m.record(0).await["creator"], caller.id().as_str());
    assert_eq!(m.record(0).await["step"], "CreatePool");
    let relay = json!({ "nearly": m.nearly.id(), "launch_id": "0" });
    let first = m.call(&m.alice, caller.id(), "call_resume", relay.clone(), 0).await;
    assert!(first.receipt_failures().is_empty(), "{}", failures(&first));
    assert_eq!(ret(&first), true);
    let parked = named(&first, "launch_step_parked");
    assert_eq!(
        parked[0]["reason"], "first buy: resume by the creator, the owner or a relayer",
        "the signer is not the contract creator"
    );
    let second = m.call(&m.alice, caller.id(), "call_resume", relay, 0).await;
    assert!(second.receipt_failures().is_empty(), "{}", failures(&second));
    assert_eq!(ret(&second), true);
    assert_eq!(m.record(0).await["step"], "Done");
    assert!(
        m.ft(&"wyrm.nearly.test.near".parse()?, caller.id()).await > 0,
        "the creator contract got the buy"
    );
    Ok(())
}

#[tokio::test]
async fn a_failed_token_creation_fails_the_launch_and_refunds_its_storage() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    m.nearly
        .as_account()
        .create_subaccount("wyrm")
        .initial_balance(NearToken::from_near(1))
        .transact()
        .await?
        .into_result()?;
    let res = m.launch(&m.alice, coin("WYRM", tax_args(), NEAR)).await;
    assert!(res.is_success(), "{}", failures(&res));
    assert_eq!(ret(&res), false);
    assert_eq!(named(&res, "token_storage_refunded").len(), 1);
    assert_eq!(named(&res, "launch_failed").len(), 1);
    let l = m.record(0).await;
    assert_eq!(l["step"], "Failed");
    assert!(l["icon"].is_null());
    let before = m.near(m.alice.id()).await;
    let r = m
        .call(&m.bob, m.nearly.id(), "refund_failed", json!({ "launch_id": "0" }), 0)
        .await;
    assert!(r.is_success(), "{}", failures(&r));
    assert_eq!(m.near(m.alice.id()).await - before, NEAR, "the held dev buy came back");
    Ok(())
}

#[tokio::test]
async fn an_empty_pool_at_the_start_point_is_adopted() -> anyhow::Result<()> {
    let m = Mocks::new().await?;
    let pool_id = "wrap.test.near|wyrm.nearly.test.near|10000";
    let made = m
        .call(&m.bob, m.dcl.id(), "create_pool", json!({ "pool_id": pool_id }), 0)
        .await;
    assert!(made.is_success(), "{}", failures(&made));
    let res = m.launch(&m.alice, coin("WYRM", tax_args(), 0)).await;
    assert!(res.is_success(), "{}", failures(&res));
    assert_eq!(ret(&res), true);
    assert_eq!(named(&res, "pool_adopted").len(), 1);
    assert_eq!(m.record(0).await["step"], "Done");
    Ok(())
}
