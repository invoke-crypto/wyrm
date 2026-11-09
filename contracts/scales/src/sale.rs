use crate::vaults::{release_to, ON_CLAIMED_GAS};
use crate::*;
use near_contract_standards::non_fungible_token::events::NftMint;
use near_sdk::{Gas, PromiseOrValue};

pub const MAX_SWEEP: usize = 20;
const RETIRE_GAS: Gas = Gas::from_tgas(10);
const ON_RETIRED_GAS: Gas = Gas::from_tgas(5);

/// Opens once, when the Dragon binds. Closed is terminal. A sale id is minted while `id <= sold`, and retired once the
/// sale is Closed with `id > sold`.
#[near(serializers = [borsh])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sale {
    Waiting,
    Open { opened_at: u64, ends_at: u64, sold: u8 },
    Closed { sold: u8, tithe_told: bool },
}

impl Sale {
    pub fn sold(self) -> u8 {
        match self {
            Sale::Waiting => 0,
            Sale::Open { sold, .. } | Sale::Closed { sold, .. } => sold,
        }
    }

    /// No Scale can be minted any more, whether or not anyone has called `close_sale`.
    pub fn over(self, now: u64) -> bool {
        match self {
            Sale::Waiting => false,
            Sale::Open { ends_at, sold, .. } => now >= ends_at || sold == SALE_SUPPLY,
            Sale::Closed { .. } => true,
        }
    }

    pub fn retired_from(self) -> Option<u8> {
        match self {
            Sale::Closed { sold, .. } if sold < SALE_SUPPLY => Some(sold + 1),
            _ => None,
        }
    }

    fn mint_next(&mut self, now: u64) -> ScaleId {
        let waiting = *self == Sale::Waiting;
        let Sale::Open { ends_at, sold, .. } = self else {
            env::panic_str(if waiting {
                "the sale is not open"
            } else {
                "the sale has closed"
            })
        };
        require!(*sold < SALE_SUPPLY, "sold out");
        require!(now < *ends_at, "the sale has closed");
        *sold += 1;
        ScaleId::new(*sold).unwrap()
    }
}

/// The creator's cut and the Dragon's, which always sum to `price`.
pub fn split(price: NearToken, creator_bps: u16) -> (NearToken, NearToken) {
    let to_creator = NearToken::from_yoctonear(price.as_yoctonear() * creator_bps as u128 / BPS);
    (to_creator, price.saturating_sub(to_creator))
}

#[near(serializers = [json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaleState {
    Waiting,
    Open,
    Closed,
}

#[near(serializers = [json])]
pub struct SaleView {
    pub state: SaleState,
    pub price: NearToken,
    pub creator_bps: u16,
    pub sold: u8,
    pub opened_at: Option<U64>,
    pub ends_at: Option<U64>,
    /// Ids from here to 90 are retired and never minted.
    pub retired_from: Option<u8>,
    pub tithe_told: bool,
}

#[near]
impl Scales {
    /// The Dragon's kick at bind. It opens a Waiting sale and changes nothing otherwise. It never panics after the
    /// caller check, so the kick cannot be lost to a refusal.
    pub fn open_sale(&mut self) {
        require!(
            env::predecessor_account_id() == self.dragon,
            "only the dragon opens the sale"
        );
        if self.sale != Sale::Waiting {
            return;
        }
        let opened_at = env::block_timestamp();
        let ends_at = opened_at + self.preset().sale.length;
        self.sale = Sale::Open {
            opened_at,
            ends_at,
            sold: 0,
        };
        emit(
            "sale_opened",
            fields! { opened_at: U64(opened_at), ends_at: U64(ends_at) },
        );
    }

    /// Mints the next sale Scale to the caller at the flat price, paying the creator's cut and the Dragon the rest.
    /// Then it sweeps the new Scale's vault to the Dragon under the claim lock, so a buyer never collects income from
    /// before the sale.
    #[payable]
    pub fn buy_scale(&mut self) -> TokenId {
        let now = env::block_timestamp();
        let terms = self.preset().sale;
        let buyer = env::predecessor_account_id();
        let paid = env::attached_deposit();
        require!(paid >= terms.price, "attached deposit is below the price");
        let id = self.sale.mint_next(now);
        self.mint(id, &buyer, terms.price);
        let token_id = id.token_id();
        NftMint {
            owner_id: &buyer,
            token_ids: &[&token_id],
            memo: None,
        }
        .emit();
        let excess = paid.saturating_sub(terms.price);
        if !excess.is_zero() {
            Promise::new(buyer).transfer(excess).detach();
        }
        let (to_creator, to_dragon) = split(terms.price, terms.creator_bps);
        if !to_creator.is_zero() {
            Promise::new(self.creator.clone()).transfer(to_creator).detach();
        }
        Promise::new(self.dragon.clone()).transfer(to_dragon).detach();
        if self.vault_exists(id) {
            self.sweep_on_mint(id, now);
        }
        self.assert_storage_paid();
        token_id
    }

    /// Releases the vaults of unminted Scales to the Dragon. Anyone may call it, and the NEAR can only go there.
    pub fn sweep(&mut self, token_ids: Vec<TokenId>) {
        require!((1..=MAX_SWEEP).contains(&token_ids.len()), "sweep 1 to 20 Scales");
        let ids: Vec<ScaleId> = token_ids.iter().map(|t| ScaleId::expect(t)).collect();
        for &id in &ids {
            require!(!self.is_minted(id), format!("Scale {id} is minted"));
            require!(
                self.vault_exists(id),
                format!("the vault of Scale {id} does not exist yet")
            );
        }
        for id in ids {
            release_to(id, &self.dragon).detach();
        }
        emit("scale_swept", fields! { token_ids: &token_ids[..], to: &self.dragon });
    }

    /// Closes the sale once it is past `ends_at` or sold out. Then, on this call and every later one until the tithe
    /// acknowledges, it tells the tithe the first retired id.
    pub fn close_sale(&mut self) -> PromiseOrValue<String> {
        if let Sale::Open { sold, .. } = self.sale {
            if self.sale.over(env::block_timestamp()) {
                self.sale = Sale::Closed {
                    sold,
                    tithe_told: sold == SALE_SUPPLY,
                };
                emit(
                    "sale_closed",
                    fields! { sold: sold, retired_from: self.sale.retired_from() },
                );
            }
        }
        match self.sale {
            Sale::Waiting | Sale::Open { .. } => PromiseOrValue::Value("not due".into()),
            Sale::Closed { tithe_told: true, .. } => PromiseOrValue::Value("closed".into()),
            Sale::Closed {
                sold,
                tithe_told: false,
            } => Promise::new(self.tithe.clone())
                .function_call(
                    "retire",
                    serde_json::to_vec(&fields! { first_unsold: sold + 1 }).unwrap_or_default(),
                    NearToken::from_yoctonear(0),
                    RETIRE_GAS,
                )
                .then(
                    Self::ext(env::current_account_id())
                        .with_static_gas(ON_RETIRED_GAS)
                        .on_retired(),
                )
                .into(),
        }
    }

    /// A failed `retire` changes nothing, so the next `close_sale` sends it again.
    #[private]
    pub fn on_retired(&mut self) -> String {
        let Sale::Closed { sold, .. } = self.sale else {
            env::panic_str("the sale is not closed")
        };
        if env::promise_result_checked(0, 16).is_err() {
            return "the tithe did not take the retirement; call close_sale again".into();
        }
        self.sale = Sale::Closed { sold, tithe_told: true };
        emit("tithe_told", fields! { retired_from: self.sale.retired_from() });
        "closed".into()
    }

    pub fn get_sale(&self) -> SaleView {
        let terms = self.preset().sale;
        let (state, opened_at, ends_at, tithe_told) = match self.sale {
            Sale::Waiting => (SaleState::Waiting, None, None, false),
            Sale::Open { opened_at, ends_at, .. } => (SaleState::Open, Some(U64(opened_at)), Some(U64(ends_at)), false),
            Sale::Closed { tithe_told, .. } => (SaleState::Closed, None, None, tithe_told),
        };
        SaleView {
            state,
            price: terms.price,
            creator_bps: terms.creator_bps,
            sold: self.sale.sold(),
            opened_at,
            ends_at,
            retired_from: self.sale.retired_from(),
            tithe_told,
        }
    }

    /// Unminted sale ids whose vault exists. The keeper reads their balances from RPC and sweeps the full ones.
    pub fn get_sweepable(&self) -> Vec<TokenId> {
        SALE_IDS
            .filter_map(ScaleId::new)
            .filter(|id| !self.is_minted(*id) && self.vault_exists(*id))
            .map(ScaleId::token_id)
            .collect()
    }
}

impl Scales {
    fn sweep_on_mint(&mut self, id: ScaleId, now: u64) {
        let mut s = self.scale(id).clone();
        s.claiming_since = Some(now);
        self.scales.insert(id, s);
        let token_id = id.token_id();
        release_to(id, &self.dragon)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_CLAIMED_GAS)
                    .on_claimed(token_id.clone(), self.dragon.clone(), U64(now)),
            )
            .detach();
        emit(
            "scale_swept",
            fields! { token_ids: std::slice::from_ref(&token_id), to: &self.dragon },
        );
    }
}
