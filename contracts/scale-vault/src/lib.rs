//! One vault per Scale. Written against `near_sdk::env` directly, without the `#[near]` macros and serde, because the
//! code is published as a global contract at 0.0001 NEAR per byte: the macro build is about ten times larger.

use near_sdk::{env, AccountId, NearToken};

/// Kept back for the vault's own storage. The `scales` contract funds each new vault with exactly this much, so a fresh
/// vault has nothing to claim.
pub const VAULT_RESERVE: NearToken = NearToken::from_millinear(10);
const PARENT_KEY: &[u8] = b"parent";

pub fn claimable(balance: NearToken) -> NearToken {
    balance.saturating_sub(VAULT_RESERVE)
}

/// The `to` of a `release` call. The parent sends `{"to":"<account>"}` and nothing else is accepted.
pub fn parse_release_args(input: &[u8]) -> Option<AccountId> {
    let to = std::str::from_utf8(input)
        .ok()?
        .strip_prefix("{\"to\":\"")?
        .strip_suffix("\"}")?;
    to.parse().ok()
}

fn parent() -> AccountId {
    let raw = env::storage_read(PARENT_KEY).unwrap_or_else(|| env::panic_str("not initialized"));
    String::from_utf8(raw).ok().and_then(|s| s.parse().ok()).unwrap()
}

fn return_json_string(s: &str) {
    env::value_return(format!("\"{s}\"").as_bytes());
}

pub fn new() {
    if env::storage_has_key(PARENT_KEY) {
        env::panic_str("already initialized");
    }
    env::storage_write(PARENT_KEY, env::predecessor_account_id().as_bytes());
}

/// Pays everything above the reserve to `to`. Only the parent may call it. Returns the amount as a U128 string.
pub fn release() {
    if env::predecessor_account_id() != parent() {
        env::panic_str("only the parent may release");
    }
    let to = env::input()
        .as_deref()
        .and_then(parse_release_args)
        .unwrap_or_else(|| env::panic_str("bad args"));
    let amount = claimable(env::account_balance());
    if !amount.is_zero() {
        let p = env::promise_batch_create(&to);
        env::promise_batch_action_transfer(p, amount);
    }
    return_json_string(&amount.as_yoctonear().to_string());
}

pub fn get_parent() {
    return_json_string(parent().as_str());
}

pub fn get_claimable() {
    return_json_string(&claimable(env::account_balance()).as_yoctonear().to_string());
}

#[cfg(target_arch = "wasm32")]
mod exports {
    #[no_mangle]
    pub extern "C" fn new() {
        super::new()
    }

    #[no_mangle]
    pub extern "C" fn release() {
        super::release()
    }

    #[no_mangle]
    pub extern "C" fn get_parent() {
        super::get_parent()
    }

    #[no_mangle]
    pub extern "C" fn get_claimable() {
        super::get_claimable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::mock::MockAction;
    use near_sdk::test_utils::{get_created_receipts, VMContextBuilder};
    use near_sdk::testing_env;

    fn ctx(predecessor: &str, balance: NearToken, input: &[u8]) {
        let mut context = VMContextBuilder::new()
            .current_account_id("001.scales.near".parse().unwrap())
            .predecessor_account_id(predecessor.parse().unwrap())
            .account_balance(balance)
            .build();
        context.input = input.into();
        testing_env!(context);
    }

    #[test]
    fn release_pays_everything_above_the_reserve_to_the_named_account() {
        ctx("scales.near", VAULT_RESERVE, b"");
        new();
        ctx(
            "scales.near",
            VAULT_RESERVE.saturating_add(NearToken::from_near(3)),
            b"{\"to\":\"alice.near\"}",
        );
        release();
        let receipts = get_created_receipts();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].receiver_id.as_str(), "alice.near");
        assert!(
            matches!(receipts[0].actions[..], [MockAction::Transfer { deposit, .. }] if deposit == NearToken::from_near(3))
        );
    }

    #[test]
    fn release_of_an_empty_vault_sends_nothing() {
        ctx("scales.near", VAULT_RESERVE, b"");
        new();
        ctx("scales.near", VAULT_RESERVE, b"{\"to\":\"alice.near\"}");
        release();
        assert!(get_created_receipts().is_empty());
    }

    #[test]
    #[should_panic(expected = "only the parent may release")]
    fn release_refuses_anyone_but_the_parent() {
        ctx("scales.near", VAULT_RESERVE, b"");
        new();
        ctx("mallory.near", NearToken::from_near(5), b"{\"to\":\"mallory.near\"}");
        release();
    }

    #[test]
    #[should_panic(expected = "already initialized")]
    fn new_runs_once() {
        ctx("scales.near", VAULT_RESERVE, b"");
        new();
        ctx("mallory.near", VAULT_RESERVE, b"");
        new();
    }

    #[test]
    fn release_args_are_exactly_the_parents_shape() {
        assert_eq!(
            parse_release_args(b"{\"to\":\"alice.near\"}").unwrap().as_str(),
            "alice.near"
        );
        for bad in [&b"{\"to\": \"alice.near\"}"[..], b"{\"to\":\"Alice\"}", b"{}", b"\xff"] {
            assert_eq!(parse_release_args(bad), None);
        }
    }
}
