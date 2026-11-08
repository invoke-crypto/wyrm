use super::*;
use near_sdk::serde_json::Value;
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::testing_env;
use std::collections::BTreeSet;

const NEAR: u128 = 10u128.pow(24);

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn ctx(predecessor: &str, deposit: u128, input: Vec<u8>) {
    ctx_with(predecessor, deposit, input, 1_000 * NEAR, 300);
}

fn ctx_with(predecessor: &str, deposit: u128, input: Vec<u8>, balance: u128, tgas: u64) {
    let mut b = VMContextBuilder::new();
    b.current_account_id(acc("nearly.test.near"))
        .predecessor_account_id(acc(predecessor))
        .signer_account_id(acc(predecessor))
        .attached_deposit(NearToken::from_yoctonear(deposit))
        .account_balance(NearToken::from_yoctonear(balance))
        .prepaid_gas(Gas::from_tgas(tgas));
    b.context.input = input.into();
    testing_env!(b.build());
}

fn pad() -> MockNearly {
    ctx("owner.test.near", 0, vec![0; 1_000]);
    let mut n = MockNearly::new(acc("dcl.test.near"), acc("wrap.test.near"), U128(1_000 * NEAR));
    let _ = n.set_token_code();
    n
}

fn args(symbol: &str) -> LaunchArgs {
    serde_json::from_value(json!({
        "name": "Wyrm", "symbol": symbol, "fee_to": "tithe.test.near", "fee_mode": "creator",
        "tax": { "buy_bps": 400, "sell_bps": 400, "creator_bps": 10000, "burn_bps": 0, "holders_bps": 0 }
    }))
    .unwrap()
}

fn launch(n: &mut MockNearly, creator: &str, a: LaunchArgs) -> u64 {
    ctx(creator, 100 * NEAR, vec![]);
    let id = n.next_id;
    let _ = n.launch(a);
    id
}

/// Every key path in a JSON value: object keys, nested with dots. A scalar or null has none.
fn keys(v: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    if let Value::Object(m) = v {
        for (k, child) in m {
            let path = format!("{prefix}{k}");
            keys(child, &format!("{path}."), out);
            out.insert(path);
        }
    }
}

fn key_set(v: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    keys(v, "", &mut out);
    out
}

fn fixture(name: &str) -> Value {
    let raw = match name {
        "get_launch" => include_str!("../fixtures/get_launch.json"),
        "get_fee_to" => include_str!("../fixtures/get_fee_to.json"),
        "get_fee_opts" => include_str!("../fixtures/get_fee_opts.json"),
        "get_tax" => include_str!("../fixtures/get_tax.json"),
        "get_slug_count" => include_str!("../fixtures/get_slug_count.json"),
        "quote_launch" => include_str!("../fixtures/quote_launch.json"),
        "get_dev_buy_cap" => include_str!("../fixtures/get_dev_buy_cap.json"),
        _ => unreachable!(),
    };
    serde_json::from_str::<Value>(raw).unwrap()["result"].clone()
}

#[test]
fn every_view_has_the_key_set_mainnet_answered() {
    let mut n = pad();
    let id = U64(launch(&mut n, "dragon.test.near", args("WYRM")));
    let views: [(&str, Value); 7] = [
        ("get_launch", serde_json::to_value(n.get_launch(id)).unwrap()),
        ("get_fee_to", serde_json::to_value(n.get_fee_to(id)).unwrap()),
        ("get_fee_opts", serde_json::to_value(n.get_fee_opts(id)).unwrap()),
        ("get_tax", serde_json::to_value(n.get_tax(id)).unwrap()),
        (
            "get_slug_count",
            serde_json::to_value(n.get_slug_count("wyrm".into())).unwrap(),
        ),
        (
            "quote_launch",
            serde_json::to_value(n.quote_launch(0, None, Some(true), None)).unwrap(),
        ),
        ("get_dev_buy_cap", serde_json::to_value(n.get_dev_buy_cap()).unwrap()),
    ];
    for (name, mine) in views {
        let real = fixture(name);
        assert_eq!(key_set(&mine), key_set(&real), "{name} keys");
        assert!(!mine.is_null(), "{name} answered");
        if !real.is_null() {
            assert_eq!(
                std::mem::discriminant(&mine),
                std::mem::discriminant(&real),
                "{name} JSON type"
            );
        }
    }
    assert!(key_set(&fixture("get_launch")).contains("links.twitter"));
}

#[test]
fn launch_records_the_caller_and_the_terms() {
    let mut n = pad();
    let id = U64(launch(&mut n, "dragon.test.near", args("WYRM")));
    let l = n.get_launch(id).unwrap();
    assert_eq!(l.creator, acc("dragon.test.near"));
    assert_eq!(l.token, acc("wyrm.nearly.test.near"));
    assert_eq!(l.pool_id, "wrap.test.near|wyrm.nearly.test.near|10000");
    assert!(!l.token_is_x);
    assert_eq!(l.step, Step::CreateToken);
    assert_eq!(n.get_fee_to(id), Some(acc("tithe.test.near")));
    assert_eq!(n.get_fee_opts(id).mode, "creator");
    let t = n.get_tax(id).unwrap();
    assert_eq!(
        (t.buy_bps, t.sell_bps, t.creator_bps, t.platform_bps),
        (400, 400, 10_000, 4_000)
    );
}

#[test]
fn a_repeated_symbol_gets_the_next_slug() {
    let mut n = pad();
    let a = launch(&mut n, "a.test.near", args("WYRM"));
    let b = launch(&mut n, "b.test.near", args("wyrm"));
    let c = launch(&mut n, "c.test.near", args("LOCK"));
    assert_eq!(n.get_launch(U64(a)).unwrap().token, acc("wyrm.nearly.test.near"));
    assert_eq!(n.get_launch(U64(b)).unwrap().token, acc("wyrm-2.nearly.test.near"));
    assert_eq!(n.get_launch(U64(c)).unwrap().token, acc("lock-2.nearly.test.near"));
    assert_eq!(n.get_slug_count("wyrm".into()), 2);
}

#[test]
fn fee_to_equal_to_the_creator_is_dropped() {
    let mut n = pad();
    let mut a = args("WYRM");
    a.fee_to = Some(acc("dragon.test.near"));
    let id = U64(launch(&mut n, "dragon.test.near", a));
    assert_eq!(n.get_fee_to(id), None);
}

#[test]
fn a_launch_without_tax_has_no_tax_view() {
    let mut n = pad();
    let mut a = args("WYRM");
    a.tax = None;
    let id = U64(launch(&mut n, "dragon.test.near", a));
    assert!(n.get_tax(id).is_none());
}

#[test]
fn the_dev_buy_is_capped() {
    let mut n = pad();
    let cap = n.get_dev_buy_cap().0;
    // 1,000 NEAR of wNEAR buys 4% of supply for 1000 * 0.04 / 0.96 / 0.99 NEAR.
    assert_eq!(cap / 10u128.pow(20), 420_875);
    let mut a = args("WYRM");
    a.dev_buy = Some(U128(cap * 3));
    let id = U64(launch(&mut n, "dragon.test.near", a));
    assert_eq!(n.get_launch(id).unwrap().dev_buy_held.0, cap);
}

#[test]
fn quote_total_is_the_sum_of_its_parts() {
    let n = pad();
    let q = n.quote_launch(0, Some(U128(NEAR)), Some(true), Some(100));
    let parts = q.launch_fee.0 + q.token_storage.0 + q.record_storage.0 + q.pool_create.0 + q.dcl_storage.0;
    assert_eq!(q.total.0, parts + NEAR);
    assert!(q.token_storage.0 > n.quote_launch(0, None, Some(false), None).token_storage.0);
}

#[test]
#[should_panic(expected = "launches paused")]
fn panic_mode_refuses_in_the_first_receipt() {
    let mut n = pad();
    n.set_mode(Mode::Panic);
    launch(&mut n, "dragon.test.near", args("WYRM"));
}

#[test]
#[should_panic(expected = "attached deposit below quote_launch(...).total")]
fn a_short_deposit_is_refused() {
    let mut n = pad();
    ctx("dragon.test.near", NEAR / 100, vec![]);
    let _ = n.launch(args("WYRM"));
}

#[test]
#[should_panic(expected = "tax: max 4% a side")]
fn a_tax_over_400_bps_is_refused() {
    let mut n = pad();
    let mut a = args("WYRM");
    a.tax.as_mut().unwrap().buy_bps = 401;
    launch(&mut n, "dragon.test.near", a);
}

#[test]
#[should_panic(expected = "nothing to push")]
fn push_without_fees_is_refused() {
    let mut n = pad();
    let _ = n.push_creator_fees(acc("tithe.test.near"));
}

#[test]
fn credited_fees_are_pushed_once() {
    let mut n = pad();
    ctx("nearly.test.near", NEAR, vec![]);
    n.credit_creator_fees(acc("tithe.test.near"));
    assert_eq!(n.get_creator_fees(acc("tithe.test.near")).0, NEAR);
    let _ = n.push_creator_fees(acc("tithe.test.near"));
    assert_eq!(n.get_creator_fees(acc("tithe.test.near")).0, 0);
}

#[test]
#[should_panic(expected = "creator only")]
fn only_the_creator_cancels_a_stuck_launch() {
    let mut n = pad();
    let id = launch(&mut n, "dragon.test.near", args("WYRM"));
    ctx("stranger.test.near", 0, vec![]);
    n.cancel_stuck_launch(U64(id));
}

#[test]
#[should_panic(expected = "owner only")]
fn only_the_owner_sets_the_mode() {
    let mut n = pad();
    ctx("stranger.test.near", 0, vec![]);
    n.set_mode(Mode::Park);
}

#[test]
#[should_panic(expected = "owner only")]
fn only_the_owner_publishes_the_token_code() {
    let mut n = pad();
    ctx("stranger.test.near", 0, vec![1; 10]);
    let _ = n.set_token_code();
}

#[test]
fn a_symbol_finds_its_first_launch() {
    let mut n = pad();
    launch(&mut n, "a.test.near", args("WYRM"));
    launch(&mut n, "b.test.near", args("wyrm"));
    let l = n.get_launch_by_symbol("Wyrm".into()).unwrap();
    assert_eq!((l.id, l.creator), (0, acc("a.test.near")));
    assert!(n.get_launch_by_symbol("NONE".into()).is_none());
}

#[test]
#[should_panic(expected = "attached deposit below quote_launch(...).total")]
fn launch_charges_the_escaped_icon_that_a_raw_byte_quote_misses() {
    let mut n = pad();
    let mut a = args("WYRM");
    let icon = format!("https://x.io/{}", "\"".repeat(200));
    let text = ("Wyrm".len() + "WYRM".len()) as u32;
    let q = n.quote_launch(icon.len() as u32, None, Some(true), Some(text));
    a.icon = Some(icon);
    ctx("dragon.test.near", q.total.0, vec![]);
    let _ = n.launch(a);
}

#[test]
fn an_escaped_icon_quote_covers_the_launch() {
    let mut n = pad();
    let mut a = args("WYRM");
    let icon = format!("https://x.io/{}", "\"".repeat(200));
    let text = ("Wyrm".len() + "WYRM".len()) as u32;
    let q = n.quote_launch(json_len(&icon) as u32, None, Some(true), Some(text));
    a.icon = Some(icon);
    ctx("dragon.test.near", q.total.0, vec![]);
    let _ = n.launch(a);
    assert_eq!(n.next_id, 1);
}

#[test]
fn an_unfronted_pool_costs_the_full_pool_creation() {
    let n = pad();
    assert_eq!(n.quote_launch(0, None, None, None).pool_create.0, 14 * NEAR / 1_000);
    ctx_with("dragon.test.near", 0, vec![], NEAR, 300);
    assert_eq!(n.quote_launch(0, None, None, None).pool_create.0, NEAR / 10);
}

#[test]
fn the_quote_matches_mainnet_for_the_same_inputs() {
    let n = pad();
    let mine = serde_json::to_value(n.quote_launch(0, None, Some(true), None)).unwrap();
    let real = fixture("quote_launch");
    for k in ["token_storage", "record_storage", "pool_create", "dcl_storage", "total"] {
        assert_eq!(mine[k], real[k], "{k}");
    }
}

#[test]
#[should_panic(expected = "attach more gas: this step needs 300 TGas")]
fn a_launch_below_the_gas_floor_is_refused() {
    let mut n = pad();
    ctx_with("dragon.test.near", 100 * NEAR, vec![], 1_000 * NEAR, 86);
    let _ = n.launch(args("WYRM"));
}

fn panics(f: impl FnOnce()) -> String {
    let e = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).expect_err("expected a panic");
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

/// A launch by dragon, parked at `step`, with a funded `dev_buy`.
fn parked(step: Step, dev_buy: u128) -> MockNearly {
    let mut n = pad();
    let mut a = args("WYRM");
    a.dev_buy = (dev_buy > 0).then_some(U128(dev_buy));
    let id = launch(&mut n, "dragon.test.near", a);
    let r = n.launches.get_mut(&id).unwrap();
    r.launch.step = step;
    r.launch.inflight = false;
    n
}

fn resume_with(n: &mut MockNearly, by: &str, tgas: u64) {
    ctx_with(by, 0, vec![], 1_000 * NEAR, tgas);
    let _ = n.resume(U64(0));
}

#[test]
fn a_resume_at_add_liquidity_needs_222_tgas() {
    let mut n = parked(Step::AddLiquidity, 0);
    let msg = panics(|| resume_with(&mut n, "stranger.test.near", 221));
    assert!(
        msg.contains("attach more gas: adding liquidity needs 300 TGas"),
        "{msg}"
    );
    let l = n.get_launch(U64(0)).unwrap();
    assert_eq!((l.step, l.inflight), (Step::AddLiquidity, false), "still parked");
    resume_with(&mut n, "stranger.test.near", 223);
    assert!(n.get_launch(U64(0)).unwrap().inflight);
}

#[test]
fn a_resume_at_dev_buy_needs_235_tgas_and_a_funded_buy() {
    let mut n = parked(Step::DevBuy, NEAR);
    let msg = panics(|| resume_with(&mut n, "dragon.test.near", 234));
    assert!(msg.contains("attach more gas for the dev buy"), "{msg}");
    let l = n.get_launch(U64(0)).unwrap();
    assert_eq!((l.step, l.inflight), (Step::DevBuy, false), "still parked");
    resume_with(&mut n, "dragon.test.near", 236);
    assert!(n.get_launch(U64(0)).unwrap().inflight);

    let mut n = parked(Step::DevBuy, NEAR);
    n.launches.get_mut(&0).unwrap().launch.dev_buy_held = U128(0);
    let msg = panics(|| resume_with(&mut n, "dragon.test.near", 300));
    assert!(msg.contains("dev buy not funded"), "{msg}");
}

#[test]
fn a_pause_refuses_resume_and_parks_the_steps_in_flight() {
    let mut n = parked(Step::CreatePool, 0);
    ctx("owner.test.near", 0, vec![]);
    n.set_mode(Mode::Panic);
    let msg = panics(|| resume_with(&mut n, "dragon.test.near", 300));
    assert!(msg.contains("paused"), "{msg}");

    for step in [Step::AddLiquidity, Step::DevBuy] {
        let mut n = parked(step, NEAR);
        ctx("owner.test.near", 0, vec![]);
        n.set_mode(Mode::Panic);
        ctx_with("nearly.test.near", 0, vec![], 1_000 * NEAR, 300);
        let ret = n.advance(0, true);
        assert!(matches!(ret, PromiseOrValue::Value(true)), "{step:?} parks with true");
        let logs = near_sdk::test_utils::get_logs();
        assert!(
            logs.iter()
                .any(|l| l.contains("launch_step_parked") && l.contains(r#""reason":"paused""#)),
            "{step:?}: {logs:?}"
        );
    }
}

#[test]
fn an_icon_over_16_kb_is_refused() {
    let mut n = pad();
    let icon = |len: usize| format!("https://x.io/{}", "a".repeat(len - "https://x.io/".len()));
    let mut a = args("WYRM");
    a.icon = Some(icon(16 * 1024));
    launch(&mut n, "dragon.test.near", a);
    let mut a = args("WYRN");
    a.icon = Some(icon(16 * 1024 + 1));
    let msg = panics(|| {
        launch(&mut n, "dragon.test.near", a);
    });
    assert!(msg.contains("icon too large"), "{msg}");
}

#[test]
fn the_dev_buy_min_out_is_97_percent_of_the_fresh_pool_output() {
    let mut n = pad();
    let mut a = args("WYRM");
    a.dev_buy = Some(U128(10 * NEAR));
    let id = launch(&mut n, "dragon.test.near", a);
    let got = n.launches.get(&id).unwrap().dev_out as f64;
    let net = 10.0 * 0.99;
    let out = 1e27 * net / (1_000.0 + net);
    assert!((got / out - 0.97).abs() < 1e-6, "min out {got} of {out}");
}
