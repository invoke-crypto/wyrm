use near_sdk::json_types::U128;
use near_sdk::{env, ext_contract, near, require, AccountId, Gas, NearToken, Promise};

pub const VAULTS: u128 = 100;
const FIRST_CREATOR_ID: u8 = 91;
// wrap.near's storage_balance_bounds().min.
const WRAP_STORAGE: NearToken = NearToken::from_micronear(1_250);
const FT_GAS: Gas = Gas::from_tgas(10);
const ONE_YOCTO: NearToken = NearToken::from_yoctonear(1);
const CALLBACK_GAS: Gas = Gas::from_tgas(20);

#[ext_contract(ext_ft)]
#[allow(dead_code)]
trait Ft {
    fn storage_deposit(&mut self, account_id: Option<AccountId>, registration_only: Option<bool>);
    fn ft_balance_of(&self, account_id: AccountId) -> U128;
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>);
    fn near_withdraw(&mut self, amount: U128);
}

#[near(contract_state)]
pub struct Tithe {
    dragon: AccountId,
    scales: AccountId,
    wrap: AccountId,
    min_round: NearToken,
    reserve: NearToken,
    rounds: u64,
    paid_dragon: u128,
    paid_scales: u128,
    /// Ids from here to 90 are retired, none while it is 91. A plain u8 so `retire` never grows the state: a tithe
    /// whose round left exactly its storage stake could not pay for one more byte.
    first_retired: u8,
}

impl Default for Tithe {
    fn default() -> Self {
        env::panic_str("not initialized")
    }
}

#[near(serializers = [json])]
pub struct Stats {
    pub rounds: u64,
    pub paid_dragon: U128,
    pub paid_scales: U128,
    pub min_round: NearToken,
    pub distributable: U128,
    pub retired_from: Option<u8>,
}

#[near(serializers = [json])]
pub struct Config {
    pub dragon: AccountId,
    pub scales: AccountId,
    pub wrap: AccountId,
    pub min_round: NearToken,
    pub reserve: NearToken,
}

pub fn vault(scales: &AccountId, i: u128) -> AccountId {
    format!("{i:03}.{scales}")
        .parse()
        .unwrap_or_else(|_| env::panic_str("bad vault account"))
}

#[near]
impl Tithe {
    #[init]
    pub fn new(
        dragon: AccountId,
        scales: AccountId,
        wrap: AccountId,
        min_round: NearToken,
        reserve: NearToken,
    ) -> Self {
        vault(&scales, VAULTS);
        // Below this a round's 100 shares would round to nothing.
        require!(min_round.as_yoctonear() >= 2 * VAULTS, "min_round too small");
        ext_ft::ext(wrap.clone())
            .with_attached_deposit(WRAP_STORAGE)
            .with_static_gas(FT_GAS)
            .storage_deposit(None, Some(true))
            .detach();
        Self {
            dragon,
            scales,
            wrap,
            min_round,
            reserve,
            rounds: 0,
            paid_dragon: 0,
            paid_scales: 0,
            first_retired: FIRST_CREATOR_ID,
        }
    }

    pub fn retire(&mut self, first_unsold: u8) {
        require!(env::predecessor_account_id() == self.scales, "only scales can retire");
        require!(
            (1..FIRST_CREATOR_ID).contains(&first_unsold),
            "first_unsold must be 1 to 90"
        );
        require!(
            self.retired_from().is_none_or(|first| first == first_unsold),
            "already retired from another id"
        );
        self.first_retired = first_unsold;
    }

    /// Outgoing transfers leave the balance when the receipts are created, so a second call in the same block sees
    /// only what is left and a refund from a missing vault is simply part of a later round.
    pub fn distribute(&mut self) {
        let free = self.distributable();
        require!(free >= self.min_round.as_yoctonear(), "below min_round");
        let half = free / 2;
        let per_scale = (free - half) / VAULTS;
        let dust = free - half - per_scale * VAULTS;
        let retired = u128::from(FIRST_CREATOR_ID - self.first_retired);
        let to_dragon = half + per_scale * retired;

        self.rounds += 1;
        self.paid_dragon += to_dragon;
        self.paid_scales += per_scale * (VAULTS - retired);

        Promise::new(self.dragon.clone())
            .transfer(NearToken::from_yoctonear(to_dragon))
            .detach();
        for i in (1..u128::from(self.first_retired)).chain(u128::from(FIRST_CREATOR_ID)..=VAULTS) {
            Promise::new(vault(&self.scales, i))
                .transfer(NearToken::from_yoctonear(per_scale))
                .detach();
        }
        env::log_str(&format!(
            r#"EVENT_JSON:{{"standard":"wyrm","version":"1.0.0","event":"tithe_round","data":[{{"round":{},"to_dragon":"{to_dragon}","per_scale":"{per_scale}","dust":"{dust}"}}]}}"#,
            self.rounds
        ));
    }

    pub fn unwrap(&mut self) -> Promise {
        ext_ft::ext(self.wrap.clone())
            .with_static_gas(FT_GAS)
            .ft_balance_of(env::current_account_id())
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(CALLBACK_GAS)
                    .on_wrap_balance(),
            )
    }

    #[private]
    pub fn on_wrap_balance(&mut self, #[callback_unwrap] balance: U128) {
        require!(balance.0 > 0, "nothing to unwrap");
        // near_withdraw requires one yoctoNEAR and pays it back with the withdrawal.
        ext_ft::ext(self.wrap.clone())
            .with_attached_deposit(ONE_YOCTO)
            .with_static_gas(FT_GAS)
            .near_withdraw(balance)
            .detach();
    }

    pub fn forward_ft(&mut self, token: AccountId) -> Promise {
        // wNEAR belongs to both halves; sending it to the Dragon would skip the vaults.
        require!(token != self.wrap, "wNEAR is unwrapped, not forwarded");
        ext_ft::ext(token.clone())
            .with_static_gas(FT_GAS)
            .ft_balance_of(env::current_account_id())
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(CALLBACK_GAS)
                    .on_ft_balance(token),
            )
    }

    #[private]
    pub fn on_ft_balance(&mut self, token: AccountId, #[callback_unwrap] balance: U128) {
        require!(balance.0 > 0, "nothing to forward");
        ext_ft::ext(token)
            .with_attached_deposit(ONE_YOCTO)
            .with_static_gas(FT_GAS)
            .ft_transfer(self.dragon.clone(), balance, None)
            .detach();
    }

    /// `paid_scales` counts transfers sent. A transfer to a vault that does not exist yet bounces back to the tithe
    /// and is counted again when a later round sends it.
    pub fn get_stats(&self) -> Stats {
        Stats {
            rounds: self.rounds,
            paid_dragon: U128(self.paid_dragon),
            paid_scales: U128(self.paid_scales),
            min_round: self.min_round,
            distributable: U128(self.distributable()),
            retired_from: self.retired_from(),
        }
    }

    pub fn get_config(&self) -> Config {
        Config {
            dragon: self.dragon.clone(),
            scales: self.scales.clone(),
            wrap: self.wrap.clone(),
            min_round: self.min_round,
            reserve: self.reserve,
        }
    }
}

impl Tithe {
    fn retired_from(&self) -> Option<u8> {
        (self.first_retired < FIRST_CREATOR_ID).then_some(self.first_retired)
    }

    fn distributable(&self) -> u128 {
        // The account's storage stake can outgrow a reserve set too low at init; spending into it would fail every round.
        let storage = env::storage_byte_cost().as_yoctonear() * env::storage_usage() as u128;
        let keep = self.reserve.as_yoctonear().max(storage);
        env::account_balance()
            .as_yoctonear()
            .saturating_sub(env::account_locked_balance().as_yoctonear())
            .saturating_sub(keep)
    }
}

#[cfg(test)]
mod tests;
