use crate::*;
use near_sdk::{Gas, PromiseError};

pub const VAULT_BATCH: u8 = 25;
/// Equal to the vault's `VAULT_RESERVE`, so a new vault starts with nothing to claim.
pub const VAULT_FUNDING: NearToken = NearToken::from_millinear(10);
/// Protocol 77 burns 0.0001 NEAR per byte of published global code.
pub const GLOBAL_CODE_BYTE_COST: NearToken = NearToken::from_yoctonear(100_000_000_000_000_000_000);
/// A batch whose callback never ran stops blocking the next attempt after this long.
pub const VAULT_BATCH_TIMEOUT: u64 = 10 * 60 * NANOS_PER_SEC;
const VAULT_INIT_GAS: Gas = Gas::from_tgas(2);
const VAULT_PROBE_GAS: Gas = Gas::from_tgas(2);
const ON_VAULTS_CREATED_GAS: Gas = Gas::from_tgas(5);
const RELEASE_GAS: Gas = Gas::from_tgas(10);
pub const ON_CLAIMED_GAS: Gas = Gas::from_tgas(10);

#[near]
impl Scales {
    /// Publishes the vault code as a global contract from this account. The input is the raw wasm, and it must hash to
    /// the `vault_code_hash` fixed in `new`, so anyone may call it and nothing but that code can be published. The caller
    /// pays the burn.
    #[payable]
    pub fn publish_vault_code(&mut self) {
        let code = env::input().unwrap_or_default();
        require!(
            env::sha256_array(&code) == self.vault_code_hash,
            "code does not match vault_code_hash"
        );
        let cost = GLOBAL_CODE_BYTE_COST.saturating_mul(code.len() as u128);
        let paid = env::attached_deposit();
        require!(
            paid >= cost,
            format!("attach {} yoctoNEAR to publish", cost.as_yoctonear())
        );
        Promise::new(env::current_account_id())
            .deploy_global_contract(code)
            .detach();
        let excess = paid.saturating_sub(cost);
        if !excess.is_zero() {
            Promise::new(env::predecessor_account_id()).transfer(excess).detach();
        }
    }

    /// Creates the next 25 vault accounts, keyless, running the global vault code. Only the batch's last vault is
    /// checked, by asking it for its parent once its creation settled: a batch fails as a whole (the code hash is
    /// missing) or not at all, and a retry is safe because creating an existing vault fails and refunds its funding.
    /// Asking instead of reading the create result means a batch that succeeded but lost its callback still counts on
    /// retry. Joining all 25 results would cost 25 data receipts and not fit in 300 Tgas.
    pub fn create_vaults(&mut self) -> Promise {
        require!(self.vaults_created < SUPPLY, "all 100 vaults exist");
        let now = env::block_timestamp();
        require!(
            self.vaults_batch_since.is_none_or(|t| now >= t + VAULT_BATCH_TIMEOUT),
            "the remaining vaults are being created"
        );
        self.vaults_batch_since = Some(now);
        let last = (self.vaults_created + VAULT_BATCH).min(SUPPLY);
        let mut creates: Vec<Promise> = (self.vaults_created + 1..=last)
            .filter_map(ScaleId::new)
            .map(|id| {
                Promise::new(vault_account(id))
                    .create_account()
                    .transfer(VAULT_FUNDING)
                    .use_global_contract(self.vault_code_hash)
                    .function_call("new", b"{}".to_vec(), NearToken::from_yoctonear(0), VAULT_INIT_GAS)
            })
            .collect();
        let reporter = creates.pop().unwrap();
        for p in creates {
            p.detach();
        }
        self.assert_storage_paid();
        let last_vault = vault_account(ScaleId::new(last).unwrap());
        reporter
            .then(Promise::new(last_vault).function_call(
                "get_parent",
                vec![],
                NearToken::from_yoctonear(0),
                VAULT_PROBE_GAS,
            ))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_VAULTS_CREATED_GAS)
                    .on_vaults_created(last),
            )
    }

    #[private]
    pub fn on_vaults_created(&mut self, last: u8) -> bool {
        self.vaults_batch_since = None;
        let parent = format!("\"{}\"", env::current_account_id());
        let created = env::promise_result_checked(0, 128).is_ok_and(|v| v == parent.as_bytes());
        if created {
            self.vaults_created = self.vaults_created.max(last);
        }
        created
    }

    /// Pays the Scale's vault balance to its owner. The token is delisted and frozen until the release answers.
    #[payable]
    pub fn claim(&mut self, token_id: TokenId) -> Promise {
        near_sdk::assert_one_yocto();
        let id = ScaleId::expect(&token_id);
        let owner = env::predecessor_account_id();
        let now = env::block_timestamp();
        let mut s = self.owned_by(id, &owner).clone();
        require!(!s.claim_in_flight(now), "a claim is in flight");
        require!(self.vault_exists(id), "this Scale's vault does not exist yet");
        s.claiming_since = Some(now);
        self.scales.insert(id, s);
        if self.listings.remove(&id).is_some() {
            emit_delisted(&token_id, &owner);
        }
        release_to(id, &owner).then(
            Self::ext(env::current_account_id())
                .with_static_gas(ON_CLAIMED_GAS)
                .on_claimed(token_id, owner, U64(now)),
        )
    }

    /// Records what the vault released to `by`. A release to the Dragon is the mint sweep, because the Dragon never
    /// claims. A sweep records `Swept` and counts nothing as claimed, since the owner was not paid.
    #[private]
    pub fn on_claimed(
        &mut self,
        token_id: TokenId,
        by: AccountId,
        started_at: U64,
        #[callback_result] released: Result<U128, PromiseError>,
    ) -> U128 {
        let id = ScaleId::expect(&token_id);
        let sweep = by == self.dragon;
        let answered = released.is_ok();
        let amount = released.map(|a| a.0).unwrap_or(0);
        self.end_release(id, started_at.0, if sweep { 0 } else { amount });
        let (at, near) = (U64(env::block_timestamp()), NearToken::from_yoctonear(amount));
        if sweep && answered {
            self.push_history(id, ScaleEvent::Swept { amount: near, at });
        } else if !sweep && amount > 0 {
            self.push_history(
                id,
                ScaleEvent::Claimed {
                    by: by.clone(),
                    amount: near,
                    at,
                },
            );
            emit("scale_claimed", fields! { token_id: &token_id, by: &by, amount: near });
        }
        U128(amount)
    }

    pub fn get_vault(&self, token_id: TokenId) -> Option<AccountId> {
        ScaleId::parse(&token_id)
            .filter(|id| self.vault_exists(*id))
            .map(vault_account)
    }
}

impl Scales {
    pub(crate) fn vault_exists(&self, id: ScaleId) -> bool {
        id.get() <= self.vaults_created
    }

    /// Ends the release that started at `started_at`. A late answer still counts its amount, but only clears the flag
    /// of the release it belongs to.
    fn end_release(&mut self, id: ScaleId, started_at: u64, claimed: u128) {
        let mut s = self.scale(id).clone();
        if s.claiming_since == Some(started_at) {
            s.claiming_since = None;
        }
        s.claimed_total += claimed;
        self.scales.insert(id, s);
    }
}

/// Asks the vault to pay everything above its reserve to `payee`. Claims, the mint sweep and `sweep` all go through it.
pub(crate) fn release_to(id: ScaleId, payee: &AccountId) -> Promise {
    Promise::new(vault_account(id)).function_call(
        "release",
        serde_json::to_vec(&fields! { to: payee }).unwrap_or_default(),
        NearToken::from_yoctonear(0),
        RELEASE_GAS,
    )
}
