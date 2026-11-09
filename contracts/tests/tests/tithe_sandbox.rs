use near_workspaces::network::Sandbox;
use near_workspaces::operations::Function;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, AccountId, Contract, Worker};
use serde_json::{json, Value};
use std::ops::RangeInclusive;
use std::path::PathBuf;

const NEAR: u128 = 10u128.pow(24);
const RESERVE: u128 = 3 * NEAR;
const MIN_ROUND: u128 = NEAR;
const VAULT_START: u128 = NEAR / 100;

struct World {
    worker: Worker<Sandbox>,
    wrap: Contract,
    tithe: Account,
    dragon: Account,
    scales: Account,
    alice: Account,
    bob: Account,
}

fn wasm(krate: &str) -> Vec<u8> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../target"));
    let p = target.join(format!("near/{krate}/{krate}.wasm"));
    std::fs::read(&p).unwrap_or_else(|e| panic!("build the contracts first with contracts/build.sh ({p:?}): {e}"))
}

/// A NEP-141 token from contracts/mock-ft. Its near_deposit and near_withdraw behave like wrap.near's, so one stands in
/// for wNEAR and another for WYRM.
async fn token(worker: &Worker<Sandbox>, name: &str, symbol: &str, decimals: u8) -> anyhow::Result<Contract> {
    let token = worker
        .root_account()?
        .create_subaccount(name)
        .initial_balance(NearToken::from_near(20))
        .transact()
        .await?
        .into_result()?
        .deploy(&wasm("mock_ft"))
        .await?
        .into_result()?;
    token
        .call("new")
        .args_json(json!({ "symbol": symbol, "decimals": decimals }))
        .transact()
        .await?
        .into_result()?;
    Ok(token)
}

/// One account signs every creation, so they go one at a time: parallel transactions from one key race for its nonce.
async fn create_vaults(scales: &Account, ids: RangeInclusive<u32>) -> anyhow::Result<()> {
    for i in ids {
        scales
            .create_subaccount(&format!("{i:03}"))
            .initial_balance(NearToken::from_yoctonear(VAULT_START))
            .transact()
            .await?
            .into_result()?;
    }
    Ok(())
}

async fn world(reserve: u128, start: u128, vaults: u32) -> anyhow::Result<World> {
    let worker = near_workspaces::sandbox().await?;
    let root = worker.root_account()?;
    let wrap = token(&worker, "wrap", "wNEAR", 24).await?;
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
    let scales = make("scales", 10).await?;
    let alice = make("alice", 100).await?;
    let bob = make("bob", 10).await?;
    let tithe = root
        .create_subaccount("tithe")
        .initial_balance(NearToken::from_yoctonear(start))
        .transact()
        .await?
        .into_result()?;
    // Deploy and init in one batch, as on mainnet.
    tithe
        .batch(tithe.id())
        .deploy(&wasm("tithe"))
        .call(
            Function::new("new")
                .args_json(json!({
                    "dragon": dragon.id(),
                    "scales": scales.id(),
                    "wrap": wrap.id(),
                    "min_round": MIN_ROUND.to_string(),
                    "reserve": reserve.to_string(),
                }))
                .gas(Gas::from_tgas(50)),
        )
        .transact()
        .await?
        .into_result()?;
    create_vaults(&scales, 1..=vaults).await?;
    Ok(World {
        worker,
        wrap,
        tithe,
        dragon,
        scales,
        alice,
        bob,
    })
}

impl World {
    async fn balance(&self, id: &AccountId) -> anyhow::Result<u128> {
        Ok(self.worker.view_account(id).await?.balance.as_yoctonear())
    }

    async fn vault_balances(&self) -> anyhow::Result<Vec<Option<u128>>> {
        let mut out = Vec::new();
        for i in 1..=100u32 {
            let id: AccountId = format!("{i:03}.{}", self.scales.id()).parse()?;
            out.push(
                self.worker
                    .view_account(&id)
                    .await
                    .ok()
                    .map(|a| a.balance.as_yoctonear()),
            );
        }
        Ok(out)
    }

    async fn fund(&self, amount: u128) -> anyhow::Result<()> {
        self.alice
            .transfer_near(self.tithe.id(), NearToken::from_yoctonear(amount))
            .await?
            .into_result()?;
        Ok(())
    }

    async fn distribute(&self, caller: &Account) -> anyhow::Result<ExecutionFinalResult> {
        Ok(caller
            .call(self.tithe.id(), "distribute")
            .gas(Gas::from_tgas(300))
            .transact()
            .await?)
    }

    async fn retire(&self, caller: &Account, first_unsold: u8) -> anyhow::Result<ExecutionFinalResult> {
        Ok(caller
            .call(self.tithe.id(), "retire")
            .args_json(json!({ "first_unsold": first_unsold }))
            .transact()
            .await?)
    }

    async fn stats(&self) -> anyhow::Result<Value> {
        Ok(self.tithe.view(self.tithe.id(), "get_stats").await?.json()?)
    }
}

struct Round {
    round: u64,
    to_dragon: u128,
    per_scale: u128,
    dust: u128,
}

fn round_event(res: &ExecutionFinalResult) -> Round {
    let logs = res.logs();
    let line = logs
        .iter()
        .find_map(|l| l.strip_prefix("EVENT_JSON:"))
        .expect("tithe_round event");
    let v: Value = serde_json::from_str(line).unwrap();
    assert_eq!(v["standard"], "wyrm");
    assert_eq!(v["event"], "tithe_round");
    let d = &v["data"][0];
    let n = |k: &str| d[k].as_str().unwrap().parse::<u128>().unwrap();
    Round {
        round: d["round"].as_u64().unwrap(),
        to_dragon: n("to_dragon"),
        per_scale: n("per_scale"),
        dust: n("dust"),
    }
}

fn assert_split(free: u128, r: &Round) {
    assert_eq!(r.to_dragon, free / 2, "dragon gets half");
    assert_eq!(
        r.per_scale,
        (free - free / 2) / 100,
        "each vault gets a hundredth of the other half"
    );
    assert_eq!(r.dust, free - r.to_dragon - 100 * r.per_scale, "dust is the remainder");
}

fn assert_failed_with(res: &ExecutionFinalResult, msg: &str) {
    assert!(res.is_failure(), "expected failure");
    let err = format!("{:?}", res.clone().into_result().unwrap_err());
    assert!(err.contains(msg), "expected {msg:?} in {err}");
}

#[tokio::test]
async fn tithe_refuses_zero_and_below_min_round() -> anyhow::Result<()> {
    let w = world(RESERVE, RESERVE, 0).await?;
    assert_failed_with(&w.distribute(&w.bob).await?, "below min_round");
    w.fund(MIN_ROUND / 2).await?;
    assert_failed_with(&w.distribute(&w.bob).await?, "below min_round");
    assert_eq!(w.stats().await?["rounds"], 0);
    Ok(())
}

#[tokio::test]
async fn tithe_round_splits_exactly_and_carries_dust() -> anyhow::Result<()> {
    let w = world(RESERVE, RESERVE, 100).await?;
    w.fund(10 * NEAR + 12_345).await?;

    let before = w.balance(w.tithe.id()).await?;
    let dragon_before = w.balance(w.dragon.id()).await?;
    let res = w.distribute(&w.bob).await?;
    assert!(res.is_success(), "{:?}", res.failures());
    let r = round_event(&res);
    assert_eq!(r.round, 1);
    assert_split(before - RESERVE, &r);
    assert!(r.dust > 0, "the odd yocto amount leaves dust");

    assert_eq!(w.balance(w.dragon.id()).await? - dragon_before, r.to_dragon);
    for (i, b) in w.vault_balances().await?.into_iter().enumerate() {
        assert_eq!(b.unwrap() - VAULT_START, r.per_scale, "vault {}", i + 1);
    }

    let call = &res.receipt_outcomes()[0];
    println!(
        "GAS distribute() with 101 transfers: tithe receipt burnt {} Tgas, whole tx {} Tgas",
        call.gas_burnt.as_gas() as f64 / 1e12,
        res.total_gas_burnt.as_gas() as f64 / 1e12
    );
    assert!(call.gas_burnt < Gas::from_tgas(250));

    let after = w.balance(w.tithe.id()).await?;
    assert!(after >= RESERVE + r.dust, "reserve and dust stay");
    let stats = w.stats().await?;
    assert_eq!(stats["rounds"], 1);
    assert_eq!(stats["paid_dragon"], r.to_dragon.to_string());
    assert_eq!(stats["paid_scales"], (100 * r.per_scale).to_string());
    assert_eq!(stats["distributable"], (after - RESERVE).to_string());

    w.fund(MIN_ROUND).await?;
    let before2 = w.balance(w.tithe.id()).await?;
    let res = w.distribute(&w.bob).await?;
    let r2 = round_event(&res);
    assert_eq!(r2.round, 2);
    assert_split(before2 - RESERVE, &r2);
    assert!(
        before2 - RESERVE >= MIN_ROUND + r.dust,
        "the next round spends the carried dust"
    );
    assert!(w.balance(w.tithe.id()).await? >= RESERVE);
    Ok(())
}

#[tokio::test]
async fn tithe_missing_vaults_refund_to_next_round() -> anyhow::Result<()> {
    let w = world(RESERVE, RESERVE, 50).await?;
    w.fund(10 * NEAR).await?;
    let before = w.balance(w.tithe.id()).await?;
    let res = w.distribute(&w.bob).await?;
    assert!(res.is_success());
    let r = round_event(&res);
    assert_split(before - RESERVE, &r);
    assert_eq!(
        res.receipt_failures().len(),
        50,
        "a transfer to each missing vault fails"
    );

    let balances = w.vault_balances().await?;
    assert!(balances[..50].iter().all(|b| b.unwrap() - VAULT_START == r.per_scale));
    assert!(balances[50..].iter().all(Option::is_none));
    let refunded = 50 * r.per_scale;
    let after = w.balance(w.tithe.id()).await?;
    assert!(
        after >= RESERVE + r.dust + refunded,
        "the 50 missing shares came back to the tithe"
    );

    create_vaults(&w.scales, 51..=100).await?;
    let before2 = w.balance(w.tithe.id()).await?;
    let res = w.distribute(&w.bob).await?;
    assert!(res.is_success());
    assert!(res.receipt_failures().is_empty());
    let r2 = round_event(&res);
    assert_split(before2 - RESERVE, &r2);
    let balances = w.vault_balances().await?;
    assert!(balances[50..].iter().all(|b| b.unwrap() - VAULT_START == r2.per_scale));
    Ok(())
}

#[tokio::test]
async fn tithe_two_calls_together_pay_once() -> anyhow::Result<()> {
    let w = world(RESERVE, RESERVE, 100).await?;
    w.fund(4 * NEAR).await?;
    let dragon_before = w.balance(w.dragon.id()).await?;
    let (a, b) = tokio::join!(
        w.alice
            .call(w.tithe.id(), "distribute")
            .gas(Gas::from_tgas(300))
            .transact_async(),
        w.bob
            .call(w.tithe.id(), "distribute")
            .gas(Gas::from_tgas(300))
            .transact_async(),
    );
    let (a, b) = (a?.await?, b?.await?);
    let blocks = (a.receipt_outcomes()[0].block_hash, b.receipt_outcomes()[0].block_hash);
    println!("two distribute receipts in the same block: {}", blocks.0 == blocks.1);

    let (ok, failed) = if a.is_success() { (a, b) } else { (b, a) };
    assert!(ok.is_success());
    assert_failed_with(&failed, "below min_round");
    let r = round_event(&ok);
    assert_eq!(
        w.balance(w.dragon.id()).await? - dragon_before,
        r.to_dragon,
        "the dragon was paid once"
    );
    assert_eq!(w.stats().await?["rounds"], 1);
    Ok(())
}

#[tokio::test]
async fn tithe_never_spends_the_storage_stake() -> anyhow::Result<()> {
    // A reserve of zero still keeps the account's storage stake, so a round cannot fail for lack of state balance.
    let w = world(0, 2 * NEAR, 100).await?;
    w.fund(2 * NEAR).await?;
    let res = w.distribute(&w.bob).await?;
    assert!(res.is_success(), "{:?}", res.failures());
    let acct = w.worker.view_account(w.tithe.id()).await?;
    let stake = acct.storage_usage as u128 * 10u128.pow(19);
    assert!(acct.balance.as_yoctonear() >= stake);
    assert_failed_with(&w.distribute(&w.bob).await?, "below min_round");
    Ok(())
}

#[tokio::test]
async fn tithe_unwrap_turns_wnear_into_near() -> anyhow::Result<()> {
    let w = world(RESERVE, RESERVE, 0).await?;
    let wnear = 5 * NEAR;
    w.alice
        .call(w.wrap.id(), "storage_deposit")
        .args_json(json!({}))
        .deposit(NearToken::from_millinear(10))
        .transact()
        .await?
        .into_result()?;
    w.alice
        .call(w.wrap.id(), "near_deposit")
        .deposit(NearToken::from_yoctonear(wnear))
        .transact()
        .await?
        .into_result()?;
    w.alice
        .call(w.wrap.id(), "ft_transfer")
        .args_json(json!({ "receiver_id": w.tithe.id(), "amount": wnear.to_string() }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let ft_balance = || async {
        let v: String = w
            .tithe
            .view(w.wrap.id(), "ft_balance_of")
            .args_json(json!({ "account_id": w.tithe.id() }))
            .await?
            .json()?;
        anyhow::Ok(v.parse::<u128>()?)
    };
    assert_eq!(ft_balance().await?, wnear);
    let before = w.balance(w.tithe.id()).await?;
    let res = w
        .bob
        .call(w.tithe.id(), "unwrap")
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(res.is_success(), "{:?}", res.failures());
    assert!(res.receipt_failures().is_empty(), "{:?}", res.receipt_failures());
    assert_eq!(ft_balance().await?, 0);
    assert!(
        w.balance(w.tithe.id()).await? >= before + wnear,
        "the wNEAR arrived as NEAR"
    );

    let res = w
        .bob
        .call(w.tithe.id(), "unwrap")
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert_failed_with(&res, "nothing to unwrap");
    Ok(())
}

#[tokio::test]
async fn tithe_forward_ft_sends_a_token_to_the_dragon() -> anyhow::Result<()> {
    let w = world(RESERVE, RESERVE, 0).await?;
    let token = token(&w.worker, "wyrm", "WYRM", 18).await?;
    for id in [w.alice.id(), w.tithe.id(), w.dragon.id()] {
        w.alice
            .call(token.id(), "storage_deposit")
            .args_json(json!({ "account_id": id }))
            .deposit(NearToken::from_millinear(10))
            .transact()
            .await?
            .into_result()?;
    }
    let amount = 7 * NEAR + 3;
    w.alice
        .call(token.id(), "near_deposit")
        .deposit(NearToken::from_yoctonear(amount))
        .transact()
        .await?
        .into_result()?;
    w.alice
        .call(token.id(), "ft_transfer")
        .args_json(json!({ "receiver_id": w.tithe.id(), "amount": amount.to_string() }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    let ft_balance = |id: AccountId| {
        let token = token.clone();
        async move {
            let v: String = token
                .view("ft_balance_of")
                .args_json(json!({ "account_id": id }))
                .await?
                .json()?;
            anyhow::Ok(v.parse::<u128>()?)
        }
    };
    let near_before = w.balance(w.tithe.id()).await?;
    let res = w
        .bob
        .call(w.tithe.id(), "forward_ft")
        .args_json(json!({ "token": token.id() }))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        res.is_success() && res.receipt_failures().is_empty(),
        "{:?}",
        res.receipt_failures()
    );
    assert_eq!(ft_balance(w.tithe.id().clone()).await?, 0);
    assert_eq!(
        ft_balance(w.dragon.id().clone()).await?,
        amount,
        "the dragon got all of it"
    );
    assert!(
        w.balance(w.tithe.id()).await? + 1 >= near_before,
        "forwarding spends no more of the tithe's NEAR than the 1 yocto deposit"
    );

    let res = w
        .bob
        .call(w.tithe.id(), "forward_ft")
        .args_json(json!({ "token": token.id() }))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert_failed_with(&res, "nothing to forward");

    let res = w
        .bob
        .call(w.tithe.id(), "forward_ft")
        .args_json(json!({ "token": w.wrap.id() }))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert_failed_with(&res, "wNEAR is unwrapped, not forwarded");
    Ok(())
}

async fn retired_round(first_unsold: u8) -> anyhow::Result<()> {
    let w = world(RESERVE, RESERVE, 100).await?;
    assert_failed_with(&w.retire(&w.bob, first_unsold).await?, "only scales can retire");
    assert_eq!(w.stats().await?["retired_from"], Value::Null);
    let res = w.retire(&w.scales, first_unsold).await?;
    assert!(res.is_success(), "{:?}", res.failures());
    assert_eq!(w.stats().await?["retired_from"], first_unsold);

    w.fund(10 * NEAR + 12_345).await?;
    let free = w.balance(w.tithe.id()).await? - RESERVE;
    let dragon_before = w.balance(w.dragon.id()).await?;
    let res = w.distribute(&w.bob).await?;
    assert!(res.is_success(), "{:?}", res.failures());
    assert!(res.receipt_failures().is_empty(), "{:?}", res.receipt_failures());
    let r = round_event(&res);
    let half = free / 2;
    let retired = 91 - u128::from(first_unsold);
    assert_eq!(r.per_scale, (free - half) / 100, "live vaults get what they got before");
    assert_eq!(
        r.to_dragon,
        half + retired * r.per_scale,
        "the dragon gets the retired hundredths"
    );
    assert_eq!(r.dust, free - half - 100 * r.per_scale);

    assert_eq!(w.balance(w.dragon.id()).await? - dragon_before, r.to_dragon);
    for (i, b) in w.vault_balances().await?.into_iter().enumerate() {
        let id = i as u128 + 1;
        let expected = if (u128::from(first_unsold)..=90).contains(&id) {
            0
        } else {
            r.per_scale
        };
        assert_eq!(b.unwrap() - VAULT_START, expected, "vault {id}");
    }
    let stats = w.stats().await?;
    assert_eq!(stats["paid_dragon"], r.to_dragon.to_string());
    assert_eq!(stats["paid_scales"], ((100 - retired) * r.per_scale).to_string());
    assert!(
        w.balance(w.tithe.id()).await? >= RESERVE + r.dust,
        "reserve and dust stay"
    );

    let call = &res.receipt_outcomes()[0];
    println!(
        "GAS distribute() after retire({first_unsold}) with {} transfers: tithe receipt burnt {} Tgas, whole tx {} Tgas",
        101 - retired,
        call.gas_burnt.as_gas() as f64 / 1e12,
        res.total_gas_burnt.as_gas() as f64 / 1e12
    );
    Ok(())
}

#[tokio::test]
async fn tithe_retired_round_pays_the_dragon_and_skips_retired_vaults() -> anyhow::Result<()> {
    retired_round(46).await
}

#[tokio::test]
async fn tithe_round_after_retire_1_pays_only_the_dragon_and_the_creator_vaults() -> anyhow::Result<()> {
    retired_round(1).await
}

#[tokio::test]
async fn tithe_retire_succeeds_on_a_tithe_left_with_only_its_storage_stake() -> anyhow::Result<()> {
    let w = world(0, 2 * NEAR, 100).await?;
    let acct = w.worker.view_account(w.tithe.id()).await?;
    let free = acct.balance.as_yoctonear() - acct.storage_usage as u128 * 10u128.pow(19);
    let even = (free + MIN_ROUND).div_ceil(200) * 200;
    w.fund(even - free).await?;
    let res = w.distribute(&w.bob).await?;
    assert!(res.is_success(), "{:?}", res.failures());
    assert_eq!(round_event(&res).dust, 0);
    let before = w.worker.view_account(w.tithe.id()).await?;
    let above_stake = before.balance.as_yoctonear() - before.storage_usage as u128 * 10u128.pow(19);
    println!("after a dustless round the tithe holds {above_stake} yocto above its storage stake, its gas reward");
    assert!(
        above_stake < NEAR / 1000,
        "the round left almost nothing above the storage stake"
    );

    let res = w
        .scales
        .call(w.tithe.id(), "retire")
        .args(br#"{ "first_unsold": 46 }"#.to_vec())
        .transact()
        .await?;
    assert!(res.is_success(), "{:?}", res.failures());
    assert_eq!(w.stats().await?["retired_from"], 46);
    let after = w.worker.view_account(w.tithe.id()).await?;
    assert_eq!(
        after.storage_usage, before.storage_usage,
        "retire does not grow the state"
    );
    Ok(())
}
