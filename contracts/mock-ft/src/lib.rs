// Test-only NEP-141 token. One build serves as WYRM and as the wNEAR stand-in, since it also wraps NEAR with
// near_deposit and near_withdraw like wrap.near. Anyone can mint and set the tax. Never deploy this on mainnet.
// The tax copies Nearly's token-tax (Nearlytrade e82bfa0, contracts/token-tax/src/lib.rs): a transfer out of a pair
// pays buy_bps, a transfer into a pair pays sell_bps, wallet to wallet pays nothing, and the tax stays on the token's
// own account.
use near_contract_standards::fungible_token::core::FungibleTokenCore;
use near_contract_standards::fungible_token::metadata::{
    FungibleTokenMetadata, FungibleTokenMetadataProvider, FT_METADATA_SPEC,
};
use near_contract_standards::fungible_token::receiver::ext_ft_receiver;
use near_contract_standards::fungible_token::resolver::FungibleTokenResolver;
use near_contract_standards::fungible_token::FungibleToken;
use near_contract_standards::storage_management::{StorageBalance, StorageBalanceBounds, StorageManagement};
use near_sdk::borsh::BorshSerialize;
use near_sdk::collections::LazyOption;
use near_sdk::json_types::U128;
use near_sdk::{
    assert_one_yocto, env, near, require, AccountId, BorshStorageKey, Gas, NearToken, PanicOnDefault, Promise,
    PromiseOrValue,
};

const BPS: u128 = 10_000;
const MAX_TAX_BPS: u16 = 400;
const GAS_FOR_RESOLVE_TRANSFER: Gas = Gas::from_tgas(5);
const GAS_FOR_FT_TRANSFER_CALL: Gas = Gas::from_tgas(30);

#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct Tax {
    pub pairs: Vec<AccountId>,
    pub buy_bps: u16,
    pub sell_bps: u16,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MockFt {
    token: FungibleToken,
    metadata: LazyOption<FungibleTokenMetadata>,
    tax: Option<Tax>,
}

#[derive(BorshSerialize, BorshStorageKey)]
#[borsh(crate = "near_sdk::borsh")]
enum StorageKey {
    Token,
    Metadata,
}

#[near]
impl MockFt {
    #[init]
    pub fn new(symbol: String, decimals: u8) -> Self {
        let metadata = FungibleTokenMetadata {
            spec: FT_METADATA_SPEC.to_string(),
            name: symbol.clone(),
            symbol,
            icon: None,
            reference: None,
            reference_hash: None,
            decimals,
        };
        Self {
            token: FungibleToken::new(StorageKey::Token),
            metadata: LazyOption::new(StorageKey::Metadata, Some(&metadata)),
            tax: None,
        }
    }

    pub fn mint(&mut self, account_id: AccountId, amount: U128) {
        self.register(&account_id);
        self.token.internal_deposit(&account_id, amount.0);
    }

    #[payable]
    pub fn near_deposit(&mut self) {
        let who = env::predecessor_account_id();
        // Like wrap.near: the account must be registered first.
        assert!(
            self.token.accounts.contains_key(&who),
            "The account {who} is not registered"
        );
        self.token
            .internal_deposit(&who, env::attached_deposit().as_yoctonear());
    }

    #[payable]
    pub fn near_withdraw(&mut self, amount: U128) -> Promise {
        assert_one_yocto();
        let who = env::predecessor_account_id();
        self.token.internal_withdraw(&who, amount.0);
        Promise::new(who).transfer(NearToken::from_yoctonear(amount.0))
    }

    /// `None` turns the tax off.
    pub fn set_tax(&mut self, tax: Option<Tax>) {
        if let Some(t) = &tax {
            require!(
                t.buy_bps <= MAX_TAX_BPS && t.sell_bps <= MAX_TAX_BPS,
                "tax: max 400 bps a side"
            );
            self.register(&env::current_account_id());
        }
        self.tax = tax;
    }

    pub fn get_tax(&self) -> Option<Tax> {
        self.tax.clone()
    }

    #[private]
    pub fn ft_resolve_taxed_transfer(
        &mut self,
        sender_id: AccountId,
        receiver_id: AccountId,
        amount: U128,
        tax: U128,
    ) -> U128 {
        let (used, _) = self.token.internal_ft_resolve_transfer(&sender_id, receiver_id, amount);
        let refund = amount.0 - used;
        let me = env::current_account_id();
        let tax_back = if refund == 0 || !self.token.accounts.contains_key(&sender_id) {
            0
        } else if refund == amount.0 {
            tax.0
        } else {
            mul_div(tax.0, refund, amount.0)
        };
        if tax_back > 0 {
            self.token
                .internal_transfer(&me, &sender_id, tax_back, Some("tax refund".into()));
        }
        U128(used + tax.0 - tax_back)
    }

    fn register(&mut self, account_id: &AccountId) {
        if !self.token.accounts.contains_key(account_id) {
            self.token.internal_register_account(account_id);
        }
    }

    fn tax_for(&self, from: &AccountId, to: &AccountId, amount: u128) -> u128 {
        let Some(t) = &self.tax else { return 0 };
        let me = env::current_account_id();
        let bps = if t.pairs.contains(from) && *to != me {
            t.buy_bps
        } else if t.pairs.contains(to) && *from != me {
            t.sell_bps
        } else {
            0
        };
        mul_div(amount, bps as u128, BPS)
    }

    /// Moves `amount` out of `from`. The receiver gets the net and the token keeps the tax. Returns the tax.
    fn taxed_transfer(&mut self, from: &AccountId, to: &AccountId, amount: u128, memo: Option<String>) -> u128 {
        let tax = self.tax_for(from, to, amount);
        self.token.internal_transfer(from, to, amount - tax, memo);
        if tax > 0 {
            self.token
                .internal_transfer(from, &env::current_account_id(), tax, Some("tax".into()));
        }
        tax
    }
}

// token-tax's mul_div: exact while a * b fits, else both sides shifted down.
fn mul_div(a: u128, b: u128, c: u128) -> u128 {
    match a.checked_mul(b) {
        Some(x) => x / c,
        None => {
            let s = (256 - a.leading_zeros() - b.leading_zeros()).saturating_sub(128);
            (a * (b >> s)).checked_div(c >> s).unwrap_or(0)
        }
    }
}

#[near]
impl FungibleTokenCore for MockFt {
    #[payable]
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        if self.tax.is_none() {
            return self.token.ft_transfer(receiver_id, amount, memo);
        }
        assert_one_yocto();
        self.taxed_transfer(&env::predecessor_account_id(), &receiver_id, amount.0, memo);
    }

    #[payable]
    fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<U128> {
        if self.tax.is_none() {
            return self.token.ft_transfer_call(receiver_id, amount, memo, msg);
        }
        assert_one_yocto();
        require!(env::prepaid_gas() > GAS_FOR_FT_TRANSFER_CALL, "More gas is required");
        let sender_id = env::predecessor_account_id();
        let tax = self.taxed_transfer(&sender_id, &receiver_id, amount.0, memo);
        let net = U128(amount.0 - tax);
        ext_ft_receiver::ext(receiver_id.clone())
            .with_unused_gas_weight(1)
            .ft_on_transfer(sender_id.clone(), net, msg)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_FOR_RESOLVE_TRANSFER)
                    .with_unused_gas_weight(0)
                    .ft_resolve_taxed_transfer(sender_id, receiver_id, net, U128(tax)),
            )
            .into()
    }

    fn ft_total_supply(&self) -> U128 {
        self.token.ft_total_supply()
    }

    fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        self.token.ft_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenResolver for MockFt {
    #[private]
    fn ft_resolve_transfer(&mut self, sender_id: AccountId, receiver_id: AccountId, amount: U128) -> U128 {
        self.token
            .internal_ft_resolve_transfer(&sender_id, receiver_id, amount)
            .0
            .into()
    }
}

// impl_fungible_token_storage! in near-contract-standards 5.29.1 does not compile against the trait: delegate by hand.
#[near]
impl StorageManagement for MockFt {
    #[payable]
    fn storage_deposit(&mut self, account_id: Option<AccountId>, registration_only: Option<bool>) -> StorageBalance {
        self.token.storage_deposit(account_id, registration_only)
    }

    #[payable]
    fn storage_withdraw(&mut self, amount: Option<NearToken>) -> StorageBalance {
        self.token.storage_withdraw(amount)
    }

    #[payable]
    fn storage_unregister(&mut self, force: Option<bool>) -> bool {
        self.token.storage_unregister(force)
    }

    fn storage_balance_bounds(&self) -> StorageBalanceBounds {
        self.token.storage_balance_bounds()
    }

    fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        self.token.storage_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenMetadataProvider for MockFt {
    fn ft_metadata(&self) -> FungibleTokenMetadata {
        self.metadata.get().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::{testing_env, PromiseResult, RuntimeFeesConfig};

    fn acc(s: &str) -> AccountId {
        s.parse().unwrap()
    }

    /// A taxed sell of 1,000 into the pair: the pair holds the net 960 and the token the tax of 40.
    fn sold() -> MockFt {
        testing_env!(VMContextBuilder::new().current_account_id(acc("coin.near")).build());
        let mut t = MockFt::new("COIN".into(), 18);
        t.mint(acc("seller.near"), U128(1_000));
        t.mint(acc("pool.near"), U128(0));
        t.set_tax(Some(Tax {
            pairs: vec![acc("pool.near")],
            buy_bps: 400,
            sell_bps: 400,
        }));
        let tax = t.taxed_transfer(&acc("seller.near"), &acc("pool.near"), 1_000, None);
        assert_eq!(tax, 40);
        t
    }

    fn resolve(t: &mut MockFt, unused: &str) -> u128 {
        let ctx = VMContextBuilder::new()
            .current_account_id(acc("coin.near"))
            .predecessor_account_id(acc("coin.near"))
            .build();
        let answer = PromiseResult::Successful(format!("\"{unused}\"").into_bytes());
        testing_env!(
            ctx,
            near_sdk::test_vm_config(),
            RuntimeFeesConfig::test(),
            Default::default(),
            vec![answer]
        );
        t.ft_resolve_taxed_transfer(acc("seller.near"), acc("pool.near"), U128(960), U128(40))
            .0
    }

    #[test]
    fn a_partial_refund_returns_the_matching_share_of_the_tax() {
        let mut t = sold();
        let used = resolve(&mut t, "480");
        assert_eq!(
            t.ft_balance_of(acc("seller.near")).0,
            480 + 20,
            "half the net and half the tax came back"
        );
        assert_eq!(t.ft_balance_of(acc("coin.near")).0, 20);
        assert_eq!(used, 500, "the gross used is the sent amount less what came back");
    }

    #[test]
    fn a_full_refund_returns_the_gross() {
        let mut t = sold();
        assert_eq!(resolve(&mut t, "960"), 0);
        assert_eq!(t.ft_balance_of(acc("seller.near")).0, 1_000);
        assert_eq!(t.ft_balance_of(acc("coin.near")).0, 0);
    }

    #[test]
    fn wallet_to_wallet_pays_no_tax() {
        let mut t = sold();
        t.mint(acc("friend.near"), U128(0));
        assert_eq!(
            t.taxed_transfer(&acc("pool.near"), &acc("friend.near"), 100, None),
            4,
            "a buy pays"
        );
        assert_eq!(t.taxed_transfer(&acc("friend.near"), &acc("seller.near"), 96, None), 0);
    }
}
