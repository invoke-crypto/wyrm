use crate::*;
use near_contract_standards::non_fungible_token::approval::ext_nft_approval_receiver;
use near_contract_standards::non_fungible_token::core::ext_nft_receiver;
use near_contract_standards::non_fungible_token::metadata::TokenMetadata;
use near_contract_standards::non_fungible_token::Token;
use near_sdk::{Gas, PromiseOrValue};
use std::collections::HashMap;

pub const MAX_APPROVALS: usize = 3;
const NFT_ON_TRANSFER_GAS: Gas = Gas::from_tgas(25);
const NFT_RESOLVE_GAS: Gas = Gas::from_tgas(15);
const NFT_ON_APPROVE_GAS: Gas = Gas::from_tgas(25);

#[near(serializers = [json])]
pub struct Payout {
    pub payout: HashMap<AccountId, U128>,
}

/// Bytes one approval adds to a token record, as near-contract-standards counts them.
pub fn approval_bytes(account: &AccountId) -> u64 {
    account.as_str().len() as u64 + 4 + 8
}

#[near]
impl Scales {
    #[payable]
    pub fn nft_transfer(
        &mut self,
        receiver_id: AccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        memo: Option<String>,
    ) {
        near_sdk::assert_one_yocto();
        self.authorized_transfer(&receiver_id, &token_id, approval_id, memo.as_deref());
        self.assert_storage_paid();
    }

    #[payable]
    pub fn nft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<bool> {
        near_sdk::assert_one_yocto();
        let before = self.authorized_transfer(&receiver_id, &token_id, approval_id, memo.as_deref());
        self.assert_storage_paid();
        ext_nft_receiver::ext(receiver_id.clone())
            .with_static_gas(NFT_ON_TRANSFER_GAS)
            .nft_on_transfer(
                env::predecessor_account_id(),
                before.owner.clone(),
                token_id.clone(),
                msg,
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(NFT_RESOLVE_GAS)
                    .nft_resolve_transfer(before.owner, receiver_id, token_id, U64(before.hold_start)),
            )
            .into()
    }

    /// Returns true when the token stays with the receiver. A returned token gets back its sender's hold start, since
    /// it never really left.
    #[private]
    pub fn nft_resolve_transfer(
        &mut self,
        previous_owner_id: AccountId,
        receiver_id: AccountId,
        token_id: TokenId,
        previous_hold_start: U64,
    ) -> bool {
        let wants_return = match env::promise_result_checked(0, 16) {
            Ok(value) => near_sdk::serde_json::from_slice::<bool>(&value).unwrap_or(true),
            Err(_) => true,
        };
        if !wants_return {
            return true;
        }
        let id = ScaleId::expect(&token_id);
        let s = self.scale(id);
        if s.owner != receiver_id || s.claim_in_flight(env::block_timestamp()) {
            return true;
        }
        self.move_scale(id, &previous_owner_id, None, None, None);
        let mut restored = self.scale(id).clone();
        restored.hold_start = previous_hold_start.0;
        self.scales.insert(id, restored);
        false
    }

    pub fn nft_token(&self, token_id: TokenId) -> Option<Token> {
        let id = ScaleId::parse(&token_id)?;
        self.scales.get(&id).map(|s| self.token(id, s))
    }

    pub fn nft_metadata(&self) -> NFTContractMetadata {
        self.metadata.clone()
    }

    pub fn nft_total_supply(&self) -> U128 {
        U128(self.minted().count() as u128)
    }

    pub fn nft_tokens(&self, from_index: Option<U128>, limit: Option<u64>) -> Vec<Token> {
        self.minted()
            .skip(from_index.map(|i| i.0 as usize).unwrap_or(0))
            .take(limit.unwrap_or(SUPPLY as u64) as usize)
            .map(|id| self.token(id, self.scale(id)))
            .collect()
    }

    pub fn nft_supply_for_owner(&self, account_id: AccountId) -> U128 {
        U128(self.minted().filter(|id| self.scale(*id).owner == account_id).count() as u128)
    }

    pub fn nft_tokens_for_owner(
        &self,
        account_id: AccountId,
        from_index: Option<U128>,
        limit: Option<u64>,
    ) -> Vec<Token> {
        self.minted()
            .filter(|id| self.scale(*id).owner == account_id)
            .skip(from_index.map(|i| i.0 as usize).unwrap_or(0))
            .take(limit.unwrap_or(SUPPLY as u64) as usize)
            .map(|id| self.token(id, self.scale(id)))
            .collect()
    }

    /// NEP-178. The deposit pays for the approval's storage and the excess is refunded.
    #[payable]
    pub fn nft_approve(&mut self, token_id: TokenId, account_id: AccountId, msg: Option<String>) -> Option<Promise> {
        let id = ScaleId::expect(&token_id);
        let owner = env::predecessor_account_id();
        let mut s = self.owned_by(id, &owner).clone();
        let fresh = !s.approvals.contains_key(&account_id);
        require!(
            !fresh || s.approvals.len() < MAX_APPROVALS,
            "a Scale holds at most 3 approvals"
        );
        let cost = if fresh {
            env::storage_byte_cost().saturating_mul(approval_bytes(&account_id) as u128)
        } else {
            NearToken::from_yoctonear(1)
        };
        let paid = env::attached_deposit();
        require!(
            paid >= cost,
            format!("attach at least {} yoctoNEAR for storage", cost.as_yoctonear())
        );
        let approval_id = s.next_approval_id;
        s.next_approval_id += 1;
        s.approvals.insert(account_id.clone(), approval_id);
        self.scales.insert(id, s);
        let excess = paid.saturating_sub(if fresh { cost } else { NearToken::from_yoctonear(0) });
        if !excess.is_zero() {
            Promise::new(owner.clone()).transfer(excess).detach();
        }
        self.assert_storage_paid();
        msg.map(|msg| {
            ext_nft_approval_receiver::ext(account_id)
                .with_static_gas(NFT_ON_APPROVE_GAS)
                .nft_on_approve(token_id, owner, approval_id, msg)
        })
    }

    #[payable]
    pub fn nft_revoke(&mut self, token_id: TokenId, account_id: AccountId) {
        near_sdk::assert_one_yocto();
        let id = ScaleId::expect(&token_id);
        let owner = env::predecessor_account_id();
        let mut s = self.owned_by(id, &owner).clone();
        if s.approvals.remove(&account_id).is_some() {
            refund_approvals(&owner, &BTreeMap::from([(account_id, 0)]));
            self.scales.insert(id, s);
        }
    }

    #[payable]
    pub fn nft_revoke_all(&mut self, token_id: TokenId) {
        near_sdk::assert_one_yocto();
        let id = ScaleId::expect(&token_id);
        let owner = env::predecessor_account_id();
        let mut s = self.owned_by(id, &owner).clone();
        refund_approvals(&owner, &s.approvals);
        s.approvals.clear();
        self.scales.insert(id, s);
    }

    pub fn nft_is_approved(&self, token_id: TokenId, approved_account_id: AccountId, approval_id: Option<u64>) -> bool {
        let Some(s) = ScaleId::parse(&token_id).and_then(|id| self.scales.get(&id)) else {
            return false;
        };
        match (s.approvals.get(&approved_account_id), approval_id) {
            (Some(actual), Some(expected)) => *actual == expected,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// NEP-199: `ROYALTY_BPS` of the balance to `royalty_receiver`, the rest to the owner.
    pub fn nft_payout(&self, token_id: TokenId, balance: U128, max_len_payout: Option<u32>) -> Payout {
        let s = self.scale(ScaleId::expect(&token_id));
        self.payout(&s.owner, NearToken::from_yoctonear(balance.0), max_len_payout)
    }

    #[payable]
    #[allow(clippy::too_many_arguments)]
    pub fn nft_transfer_payout(
        &mut self,
        receiver_id: AccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        memo: Option<String>,
        balance: U128,
        max_len_payout: Option<u32>,
    ) -> Payout {
        near_sdk::assert_one_yocto();
        let before = self.authorized_transfer(&receiver_id, &token_id, approval_id, memo.as_deref());
        self.assert_storage_paid();
        self.payout(&before.owner, NearToken::from_yoctonear(balance.0), max_len_payout)
    }
}

impl Scales {
    /// The owner, or an approved account with a matching approval id, moves the token through the one owner hook.
    fn authorized_transfer(
        &mut self,
        receiver_id: &AccountId,
        token_id: &str,
        approval_id: Option<u64>,
        memo: Option<&str>,
    ) -> Scale {
        let id = ScaleId::expect(token_id);
        let sender = env::predecessor_account_id();
        let s = self.scale(id);
        let authorized = if s.owner == sender {
            None
        } else {
            let actual = s
                .approvals
                .get(&sender)
                .unwrap_or_else(|| env::panic_str("not the owner or approved"));
            require!(approval_id.is_none_or(|a| a == *actual), "approval id does not match");
            Some(sender)
        };
        self.move_scale(id, receiver_id, None, authorized.as_ref(), memo)
    }

    fn payout(&self, owner: &AccountId, balance: NearToken, max_len_payout: Option<u32>) -> Payout {
        let royalty = royalty_of(balance);
        let mut payout = HashMap::new();
        payout.insert(owner.clone(), U128(balance.saturating_sub(royalty).as_yoctonear()));
        if !royalty.is_zero() {
            let to_receiver = payout.entry(self.royalty_receiver.clone()).or_insert(U128(0));
            to_receiver.0 += royalty.as_yoctonear();
        }
        require!(
            max_len_payout.is_none_or(|max| payout.len() <= max as usize),
            "max_len_payout is below the number of payees"
        );
        Payout { payout }
    }

    fn token(&self, id: ScaleId, s: &Scale) -> Token {
        let tier = HoldTier::of(env::block_timestamp().saturating_sub(s.hold_start));
        Token {
            token_id: id.token_id(),
            owner_id: s.owner.clone(),
            metadata: Some(TokenMetadata {
                title: Some(format!("Scale #{id}")),
                media: Some(self.tier_media[tier as usize].clone()),
                copies: Some(1),
                extra: Some(
                    serde_json::to_string(&fields! { tier: tier, hold_start: U64(s.hold_start), voting: id.votes() })
                        .unwrap_or_default(),
                ),
                ..Default::default()
            }),
            approved_account_ids: Some(s.approvals.iter().map(|(a, i)| (a.clone(), *i)).collect()),
        }
    }
}
