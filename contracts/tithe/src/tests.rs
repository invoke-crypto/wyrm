use super::*;
use near_sdk::mock::MockAction;
use near_sdk::serde_json::json;
use near_sdk::test_utils::{get_created_receipts, get_logs, VMContextBuilder};
use near_sdk::testing_env;

const NEAR: u128 = 10u128.pow(24);
const RESERVE: u128 = 3 * NEAR;
// 1.5 NEAR of storage stake at 10^19 yocto per byte, under the reserve.
const STORAGE: u64 = 150_000;

fn setup(reserve: u128, balance: u128, storage: u64) -> Tithe {
    testing_env!(VMContextBuilder::new()
        .current_account_id("tithe.near".parse().unwrap())
        .account_balance(NearToken::from_yoctonear(balance))
        .storage_usage(storage)
        .build());
    let t = Tithe::new(
        "dragon.near".parse().unwrap(),
        "scales.near".parse().unwrap(),
        "wrap.near".parse().unwrap(),
        NearToken::from_near(1),
        NearToken::from_yoctonear(reserve),
    );
    // Start every test from a clean receipt list and the full balance, as a fresh distribute() call would.
    testing_env!(VMContextBuilder::new()
        .current_account_id("tithe.near".parse().unwrap())
        .account_balance(NearToken::from_yoctonear(balance))
        .storage_usage(storage)
        .build());
    t
}

fn retire_as(t: &mut Tithe, caller: &str, first_unsold: u8) {
    let balance = env::account_balance();
    testing_env!(VMContextBuilder::new()
        .current_account_id("tithe.near".parse().unwrap())
        .predecessor_account_id(caller.parse().unwrap())
        .account_balance(balance)
        .storage_usage(STORAGE)
        .build());
    t.retire(first_unsold);
}

fn retire(t: &mut Tithe, first_unsold: u8) {
    retire_as(t, "scales.near", first_unsold);
}

fn panic_message(f: impl FnOnce()) -> String {
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).expect_err("expected a panic");
    *err.downcast::<String>().expect("near-sdk panics with a String")
}

fn transfers() -> Vec<(String, u128)> {
    get_created_receipts()
        .into_iter()
        .map(|r| match r.actions.as_slice() {
            [MockAction::Transfer { deposit, .. }] => (r.receiver_id.to_string(), deposit.as_yoctonear()),
            other => panic!("expected one transfer, got {other:?}"),
        })
        .collect()
}

#[test]
#[should_panic(expected = "below min_round")]
fn refuses_with_nothing_above_the_reserve() {
    setup(RESERVE, RESERVE, STORAGE).distribute();
}

#[test]
#[should_panic(expected = "below min_round")]
fn refuses_below_min_round() {
    setup(RESERVE, RESERVE + NEAR - 1, STORAGE).distribute();
}

#[test]
fn round_splits_half_and_hundredths_and_keeps_dust() {
    let free = 10 * NEAR + 12_345;
    let mut t = setup(RESERVE, RESERVE + free, STORAGE);
    t.distribute();
    let gas = env::used_gas();

    let sent = transfers();
    assert_eq!(sent.len(), 101);
    assert_eq!(sent[0], ("dragon.near".to_string(), free / 2));
    let per_scale = (free - free / 2) / 100;
    for (i, (to, amount)) in sent[1..].iter().enumerate() {
        assert_eq!(to, &format!("{:03}.scales.near", i + 1));
        assert_eq!(*amount, per_scale);
    }
    let dust = free - free / 2 - 100 * per_scale;
    assert_eq!(dust, 73, "the odd yocto amount leaves dust");
    assert_eq!(
        env::account_balance().as_yoctonear(),
        RESERVE + dust,
        "only reserve and dust stay"
    );

    let stats = t.get_stats();
    assert_eq!(stats.rounds, 1);
    assert_eq!(stats.paid_dragon.0, free / 2);
    assert_eq!(stats.paid_scales.0, 100 * per_scale);
    assert_eq!(stats.distributable.0, dust);

    let event: near_sdk::serde_json::Value =
        near_sdk::serde_json::from_str(get_logs()[0].strip_prefix("EVENT_JSON:").unwrap()).unwrap();
    assert_eq!(
        event,
        json!({
            "standard": "wyrm",
            "version": "1.0.0",
            "event": "tithe_round",
            "data": [{ "round": 1, "to_dragon": (free / 2).to_string(), "per_scale": per_scale.to_string(), "dust": dust.to_string() }],
        })
    );

    // Host-function gas of the call, including the send and execution fees of all 101 receipts; wasm compute is not
    // counted in the mock.
    println!("distribute() used {} Tgas in the mocked VM", gas.as_gas() as f64 / 1e12);
    assert!(gas < Gas::from_tgas(250));
}

#[test]
fn dust_joins_the_next_round() {
    let mut t = setup(RESERVE, RESERVE + 10 * NEAR + 12_345, STORAGE);
    t.distribute();
    let left = env::account_balance().as_yoctonear();
    testing_env!(VMContextBuilder::new()
        .current_account_id("tithe.near".parse().unwrap())
        .account_balance(NearToken::from_yoctonear(left + NEAR))
        .storage_usage(STORAGE)
        .build());
    t.distribute();
    let sent: u128 = transfers().iter().map(|(_, a)| a).sum();
    let free = left + NEAR - RESERVE;
    assert_eq!(sent + (free - free / 2) % 100, free);
    assert_eq!(t.get_stats().rounds, 2);
}

#[test]
#[should_panic(expected = "below min_round")]
fn a_second_call_in_the_same_block_finds_nothing_to_pay() {
    let mut t = setup(RESERVE, RESERVE + 10 * NEAR, STORAGE);
    t.distribute();
    t.distribute();
}

#[test]
fn keeps_the_storage_stake_when_the_reserve_is_lower() {
    // 3 NEAR of storage stake and a reserve of 0.
    let mut t = setup(0, 5 * NEAR, 300_000);
    t.distribute();
    let sent: u128 = transfers().iter().map(|(_, a)| a).sum();
    assert_eq!(sent, 2 * NEAR);
    assert_eq!(env::account_balance().as_yoctonear(), 3 * NEAR);
}

#[test]
#[should_panic(expected = "wNEAR is unwrapped, not forwarded")]
fn forward_ft_refuses_wnear() {
    let _ = setup(RESERVE, RESERVE, STORAGE).forward_ft("wrap.near".parse().unwrap());
}

#[test]
#[should_panic(expected = "bad vault account")]
fn new_refuses_a_scales_id_too_long_for_its_vaults() {
    testing_env!(VMContextBuilder::new().build());
    Tithe::new(
        "dragon.near".parse().unwrap(),
        format!("{}.near", "a".repeat(58)).parse().unwrap(),
        "wrap.near".parse().unwrap(),
        NearToken::from_near(1),
        NearToken::from_near(3),
    );
}

fn callback_with(balance: u128) -> Tithe {
    let t = setup(RESERVE, RESERVE, STORAGE);
    testing_env!(
        VMContextBuilder::new()
            .current_account_id("tithe.near".parse().unwrap())
            .predecessor_account_id("tithe.near".parse().unwrap())
            .account_balance(NearToken::from_yoctonear(RESERVE))
            .storage_usage(STORAGE)
            .build(),
        near_sdk::test_vm_config(),
        near_sdk::RuntimeFeesConfig::test(),
        Default::default(),
        vec![near_sdk::PromiseResult::Successful(
            format!("\"{balance}\"").into_bytes()
        )],
    );
    t
}

fn single_call() -> (String, String, String, u128) {
    let receipts = get_created_receipts();
    assert_eq!(receipts.len(), 1);
    match receipts[0].actions.as_slice() {
        [MockAction::FunctionCallWeight {
            method_name,
            args,
            attached_deposit,
            ..
        }] => (
            receipts[0].receiver_id.to_string(),
            String::from_utf8(method_name.clone()).unwrap(),
            String::from_utf8(args.clone()).unwrap(),
            attached_deposit.as_yoctonear(),
        ),
        other => panic!("expected one function call, got {other:?}"),
    }
}

#[test]
fn unwrap_withdraws_the_whole_wnear_balance() {
    callback_with(5 * NEAR).on_wrap_balance(U128(5 * NEAR));
    let (to, method, args, deposit) = single_call();
    assert_eq!(
        (to.as_str(), method.as_str(), deposit),
        ("wrap.near", "near_withdraw", 1)
    );
    assert_eq!(args, format!(r#"{{"amount":"{}"}}"#, 5 * NEAR));
}

#[test]
#[should_panic(expected = "nothing to unwrap")]
fn unwrap_refuses_an_empty_balance() {
    callback_with(0).on_wrap_balance(U128(0));
}

#[test]
fn forward_ft_sends_the_whole_balance_to_the_dragon() {
    callback_with(7).on_ft_balance("wyrm.near".parse().unwrap(), U128(7));
    let (to, method, args, deposit) = single_call();
    assert_eq!((to.as_str(), method.as_str(), deposit), ("wyrm.near", "ft_transfer", 1));
    assert_eq!(args, r#"{"receiver_id":"dragon.near","amount":"7","memo":null}"#);
}

#[test]
#[should_panic(expected = "nothing to forward")]
fn forward_ft_refuses_an_empty_balance() {
    callback_with(0).on_ft_balance("wyrm.near".parse().unwrap(), U128(0));
}

#[test]
fn retire_takes_one_suffix_once_from_scales_only() {
    let mut t = setup(RESERVE, RESERVE, STORAGE);
    assert!(panic_message(|| retire_as(&mut t, "stranger.near", 46)).contains("only scales can retire"));
    for outside in [0, 91, 255] {
        assert!(panic_message(|| retire(&mut t, outside)).contains("first_unsold must be 1 to 90"));
    }
    assert_eq!(t.get_stats().retired_from, None);

    retire(&mut t, 46);
    assert_eq!(t.get_stats().retired_from, Some(46));
    assert!(panic_message(|| retire(&mut t, 47)).contains("already retired from another id"));
    assert_eq!(t.get_stats().retired_from, Some(46));
}

#[test]
fn a_repeat_of_the_same_value_is_a_no_op() {
    let mut t = setup(RESERVE, RESERVE + 10 * NEAR, STORAGE);
    retire(&mut t, 46);
    retire(&mut t, 46);
    assert_eq!(t.get_stats().retired_from, Some(46));
    t.distribute();
    assert_eq!(transfers().len(), 56);
}

#[test]
fn a_round_after_retirement_pays_the_retired_shares_to_the_dragon() {
    let free = 10 * NEAR + 12_345;
    let half = free / 2;
    let per_scale = (free - half) / 100;
    let dust = free - half - 100 * per_scale;
    for first in [1u8, 46, 90] {
        let mut t = setup(RESERVE, RESERVE + free, STORAGE);
        retire(&mut t, first);
        t.distribute();

        let retired = 91 - u128::from(first);
        let sent = transfers();
        assert_eq!(sent.len() as u128, 101 - retired, "first unsold {first}");
        assert_eq!(sent[0], ("dragon.near".to_string(), half + retired * per_scale));
        let live: Vec<String> = (1..first as u32)
            .chain(91..=100)
            .map(|i| format!("{i:03}.scales.near"))
            .collect();
        let paid: Vec<String> = sent[1..].iter().map(|(to, _)| to.clone()).collect();
        assert_eq!(paid, live, "only live vaults are paid");
        assert!(
            sent[1..].iter().all(|(_, amount)| *amount == per_scale),
            "live vaults get what they got before"
        );
        assert_eq!(
            env::account_balance().as_yoctonear(),
            RESERVE + dust,
            "only reserve and dust stay"
        );

        let stats = t.get_stats();
        assert_eq!(stats.paid_dragon.0, half + retired * per_scale);
        assert_eq!(stats.paid_scales.0, (100 - retired) * per_scale);
        assert_eq!(stats.distributable.0, dust);
    }
}

#[test]
fn get_config_returns_the_new_arguments() {
    let t = setup(RESERVE, RESERVE, STORAGE);
    assert_eq!(
        near_sdk::serde_json::to_value(t.get_config()).unwrap(),
        json!({
            "dragon": "dragon.near",
            "scales": "scales.near",
            "wrap": "wrap.near",
            "min_round": NEAR.to_string(),
            "reserve": RESERVE.to_string(),
        })
    );
}
