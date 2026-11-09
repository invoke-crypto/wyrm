/// A JSON object as an anonymous struct, so events and call arguments are written without a `serde_json::Value`.
/// Each field takes the next type parameter name from the list, which caps an object at eight fields.
macro_rules! fields {
    ($($key:ident: $value:expr),* $(,)?) => {
        fields!(@ [A B C D E F G H] [] $($key: $value,)*)
    };
    (@ [$t:ident $($ts:ident)*] [$($done:tt)*] $key:ident: $value:expr, $($rest:tt)*) => {
        fields!(@ [$($ts)*] [$($done)* ($key $t $value)] $($rest)*)
    };
    (@ [$($ts:ident)*] [$(($key:ident $t:ident $value:expr))*]) => {{
        #[derive(near_sdk::serde::Serialize)]
        #[serde(crate = "near_sdk::serde")]
        struct Fields<$($t),*> {
            $($key: $t),*
        }
        Fields { $($key: $value),* }
    }};
}

mod council;
mod ids;
mod market;
mod nft;
mod sale;
mod vaults;

#[cfg(test)]
mod tests;

pub use council::{Proposal, ProposalKind, Status};
pub use ids::{ScaleId, ScaleSet, CREATOR_IDS, SALE_IDS, SALE_SUPPLY, SUPPLY};
pub use market::Offer;
pub use sale::{Sale, SaleView};

use near_contract_standards::non_fungible_token::events::{NftMint, NftTransfer};
use near_contract_standards::non_fungible_token::metadata::NFTContractMetadata;
use near_contract_standards::non_fungible_token::TokenId;
use near_sdk::json_types::{Base58CryptoHash, U128, U64};
use near_sdk::serde::Serialize;
use near_sdk::serde_json;
use near_sdk::store::{IterableMap, LookupMap, Vector};
use near_sdk::{env, near, require, AccountId, BorshStorageKey, CryptoHash, NearToken, PanicOnDefault, Promise};
use std::collections::BTreeMap;
use wyrm_types::{CollectionArgs, Preset, PresetName, RoyaltyTo, ENGINE_VERSION};

pub const NANOS_PER_SEC: u64 = 1_000_000_000;
pub const DAY: u64 = 24 * 3_600 * NANOS_PER_SEC;
pub const CLAIM_TIMEOUT: u64 = 10 * 60 * NANOS_PER_SEC;
pub const HISTORY_LEN: usize = 16;
pub const ROYALTY_BPS: u16 = 500;
pub const BPS: u128 = 10_000;
pub const STORAGE_FEE: NearToken = NearToken::from_millinear(10);

#[derive(BorshStorageKey)]
#[near]
enum Key {
    Scales,
    History,
    Listings,
    Offers,
    OffersByToken,
    OpenOffers,
    Proposals,
    OpenProposal,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldTier {
    Ash,
    Ember,
    Molten,
    Obsidian,
}

impl HoldTier {
    const FROM_DAYS: [(u64, HoldTier); 4] = [
        (14, HoldTier::Obsidian),
        (7, HoldTier::Molten),
        (3, HoldTier::Ember),
        (0, HoldTier::Ash),
    ];

    pub fn of(held: u64) -> HoldTier {
        Self::FROM_DAYS
            .iter()
            .find(|(days, _)| held >= days * DAY)
            .map(|(_, t)| *t)
            .unwrap_or(HoldTier::Ash)
    }
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct Scale {
    pub owner: AccountId,
    pub hold_start: u64,
    pub claimed_total: u128,
    /// Set by a claim or the mint sweep, and cleared by its answer.
    pub claiming_since: Option<u64>,
    pub approvals: BTreeMap<AccountId, u64>,
    pub next_approval_id: u64,
}

impl Scale {
    pub fn claim_in_flight(&self, now: u64) -> bool {
        self.claiming_since.is_some_and(|t| now < t + CLAIM_TIMEOUT)
    }

    /// A sale Scale votes on a proposal when its current owner held it a full season before the proposal was created.
    pub fn seasoned_at(&self, id: ScaleId, at: u64, season: u64) -> bool {
        id.votes() && self.hold_start + season <= at
    }
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum ScaleEvent {
    Minted {
        to: AccountId,
        price: NearToken,
        at: U64,
    },
    Transferred {
        from: AccountId,
        to: AccountId,
        price: Option<NearToken>,
        at: U64,
    },
    Claimed {
        by: AccountId,
        amount: NearToken,
        at: U64,
    },
    /// The mint sweep paid the Dragon what the vault held before this Scale was sold.
    Swept {
        amount: NearToken,
        at: U64,
    },
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Scales {
    preset: PresetName,
    dragon: AccountId,
    tithe: AccountId,
    creator: AccountId,
    /// The creator or the tithe, resolved from the preset's `royalty_to` in `new`.
    royalty_receiver: AccountId,
    vault_code_hash: CryptoHash,
    /// Vaults 1 to `vaults_created` exist.
    vaults_created: u8,
    vaults_batch_since: Option<u64>,
    metadata: NFTContractMetadata,
    tier_media: [String; 4],
    sale: Sale,
    scales: LookupMap<ScaleId, Scale>,
    history: LookupMap<ScaleId, Vec<ScaleEvent>>,
    listings: LookupMap<ScaleId, NearToken>,
    offers: IterableMap<u64, Offer>,
    offers_by_token: LookupMap<ScaleId, Vec<u64>>,
    open_offers: LookupMap<AccountId, u8>,
    next_offer_id: u64,
    /// Open offers plus the bonds of Open proposals. Never available to pay for storage.
    escrow: NearToken,
    proposals: Vector<Proposal>,
    open_proposal: LookupMap<AccountId, u64>,
}

#[near]
impl Scales {
    #[init]
    pub fn new(
        preset: PresetName,
        dragon: AccountId,
        tithe: AccountId,
        creator: AccountId,
        vault_code_hash: Base58CryptoHash,
        collection: CollectionArgs,
    ) -> Self {
        let p = wyrm_types::preset(preset);
        check(p.check());
        check(p.allowed_on(&env::current_account_id()));
        check(collection.check());
        let mut this = Self {
            preset,
            royalty_receiver: royalty_receiver(p.royalty_to, &creator, &tithe),
            dragon,
            tithe,
            creator: creator.clone(),
            vault_code_hash: vault_code_hash.into(),
            vaults_created: 0,
            vaults_batch_since: None,
            metadata: collection.metadata(),
            tier_media: collection.tier_media,
            sale: Sale::Waiting,
            scales: LookupMap::new(Key::Scales),
            history: LookupMap::new(Key::History),
            listings: LookupMap::new(Key::Listings),
            offers: IterableMap::new(Key::Offers),
            offers_by_token: LookupMap::new(Key::OffersByToken),
            open_offers: LookupMap::new(Key::OpenOffers),
            next_offer_id: 0,
            escrow: NearToken::from_yoctonear(0),
            proposals: Vector::new(Key::Proposals),
            open_proposal: LookupMap::new(Key::OpenProposal),
        };
        let creator_ids: Vec<ScaleId> = CREATOR_IDS.filter_map(ScaleId::new).collect();
        for &id in &creator_ids {
            this.mint(id, &creator, NearToken::from_yoctonear(0));
        }
        let token_ids: Vec<String> = creator_ids.iter().map(|id| id.token_id()).collect();
        let token_refs: Vec<&str> = token_ids.iter().map(String::as_str).collect();
        NftMint {
            owner_id: &creator,
            token_ids: &token_refs,
            memo: None,
        }
        .emit();
        this
    }

    pub fn get_config(&self) -> ConfigView {
        let p = self.preset();
        ConfigView {
            preset: self.preset,
            preset_hash: p.hash().into(),
            engine_version: ENGINE_VERSION,
            dragon: self.dragon.clone(),
            tithe: self.tithe.clone(),
            creator: self.creator.clone(),
            royalty_receiver: self.royalty_receiver.clone(),
            vault_code_hash: self.vault_code_hash.into(),
            vaults_created: self.vaults_created,
            royalty_bps: ROYALTY_BPS,
            season_ns: U64(p.times.season),
            claim_timeout_ns: U64(CLAIM_TIMEOUT),
            storage_fee: STORAGE_FEE,
            escrow: self.escrow,
            sale: self.get_sale(),
        }
    }

    pub fn get_scale(&self, token_id: TokenId) -> Option<ScaleView> {
        let id = ScaleId::parse(&token_id)?;
        self.scales.get(&id).map(|s| self.scale_view(id, s))
    }

    pub fn get_scales(&self, from: Option<u32>, limit: Option<u32>) -> Vec<ScaleView> {
        self.minted()
            .skip(from.unwrap_or(0) as usize)
            .take(limit.unwrap_or(SUPPLY as u32) as usize)
            .map(|id| self.scale_view(id, self.scale(id)))
            .collect()
    }

    pub fn get_history(&self, token_id: TokenId) -> Vec<ScaleEvent> {
        ScaleId::parse(&token_id)
            .and_then(|id| self.history.get(&id).cloned())
            .unwrap_or_default()
    }

    /// The electorate a proposal created now would have: the sale Scales sold so far.
    pub fn eligible_now(&self) -> u32 {
        self.electorate()
    }
}

#[near(serializers = [json])]
pub struct ConfigView {
    pub preset: PresetName,
    pub preset_hash: Base58CryptoHash,
    pub engine_version: u32,
    pub dragon: AccountId,
    pub tithe: AccountId,
    pub creator: AccountId,
    pub royalty_receiver: AccountId,
    pub vault_code_hash: Base58CryptoHash,
    pub vaults_created: u8,
    pub royalty_bps: u16,
    pub season_ns: U64,
    pub claim_timeout_ns: U64,
    pub storage_fee: NearToken,
    pub escrow: NearToken,
    pub sale: SaleView,
}

#[near(serializers = [json])]
pub struct ScaleView {
    pub token_id: TokenId,
    pub owner: AccountId,
    pub voting: bool,
    pub hold_start: U64,
    pub tier: HoldTier,
    pub claimed_total: U128,
    pub claiming_since: Option<U64>,
    pub listed_price: Option<NearToken>,
    pub vault: AccountId,
}

impl Scales {
    fn preset(&self) -> Preset {
        wyrm_types::preset(self.preset)
    }

    /// Only sale Scales vote, and retired ones were never sold, so the electorate is the count sold.
    fn electorate(&self) -> u32 {
        self.sale.sold() as u32
    }

    fn is_minted(&self, id: ScaleId) -> bool {
        !id.votes() || id.get() <= self.sale.sold()
    }

    fn minted(&self) -> impl Iterator<Item = ScaleId> + '_ {
        ScaleId::all().filter(|id| self.is_minted(*id))
    }

    fn scale(&self, id: ScaleId) -> &Scale {
        self.scales
            .get(&id)
            .unwrap_or_else(|| env::panic_str("token not minted"))
    }

    fn owned_by(&self, id: ScaleId, account: &AccountId) -> &Scale {
        let s = self.scale(id);
        require!(&s.owner == account, "not the owner");
        s
    }

    fn scale_view(&self, id: ScaleId, s: &Scale) -> ScaleView {
        ScaleView {
            token_id: id.token_id(),
            owner: s.owner.clone(),
            voting: id.votes(),
            hold_start: U64(s.hold_start),
            tier: HoldTier::of(env::block_timestamp().saturating_sub(s.hold_start)),
            claimed_total: U128(s.claimed_total),
            claiming_since: s
                .claiming_since
                .filter(|_| s.claim_in_flight(env::block_timestamp()))
                .map(U64),
            listed_price: self.listings.get(&id).copied(),
            vault: vault_account(id),
        }
    }

    fn mint(&mut self, id: ScaleId, to: &AccountId, price: NearToken) {
        let at = env::block_timestamp();
        self.scales.insert(
            id,
            Scale {
                owner: to.clone(),
                hold_start: at,
                claimed_total: 0,
                claiming_since: None,
                approvals: BTreeMap::new(),
                next_approval_id: 0,
            },
        );
        self.push_history(
            id,
            ScaleEvent::Minted {
                to: to.clone(),
                price,
                at: U64(at),
            },
        );
        emit(
            "scale_minted",
            fields! { token_id: id.token_id(), to: to, price: price, voting: id.votes() },
        );
    }

    fn push_history(&mut self, id: ScaleId, event: ScaleEvent) {
        let ring = self.history.entry(id).or_default();
        ring.push(event);
        if ring.len() > HISTORY_LEN {
            ring.remove(0);
        }
    }

    /// The one path by which a Scale changes owner: NEP-171 transfers, payouts, market sales and accepted offers all end
    /// here. Returns the record as it was before the move.
    fn move_scale(
        &mut self,
        id: ScaleId,
        to: &AccountId,
        price: Option<NearToken>,
        authorized: Option<&AccountId>,
        memo: Option<&str>,
    ) -> Scale {
        let now = env::block_timestamp();
        let before = self.scale(id).clone();
        require!(!before.claim_in_flight(now), "a claim is in flight");
        require!(&before.owner != to, "already the owner");
        refund_approvals(&before.owner, &before.approvals);
        let mut after = before.clone();
        after.owner = to.clone();
        after.hold_start = now;
        after.approvals.clear();
        self.scales.insert(id, after);
        self.listings.remove(&id);
        self.push_history(
            id,
            ScaleEvent::Transferred {
                from: before.owner.clone(),
                to: to.clone(),
                price,
                at: U64(now),
            },
        );
        let token_id = id.token_id();
        NftTransfer {
            old_owner_id: &before.owner,
            new_owner_id: to,
            token_ids: &[&token_id],
            authorized_id: authorized.map(|a| a.as_ref()),
            memo,
        }
        .emit();
        before
    }

    /// Pays `price` for a Scale that already moved: the royalty to `royalty_receiver`, the rest to the seller.
    fn pay_sale(&self, seller: &AccountId, price: NearToken) -> NearToken {
        let royalty = royalty_of(price);
        Promise::new(seller.clone())
            .transfer(price.saturating_sub(royalty))
            .detach();
        if !royalty.is_zero() {
            Promise::new(self.royalty_receiver.clone()).transfer(royalty).detach();
        }
        royalty
    }

    /// Escrowed NEAR is owed to buyers and proposers, so it may never be what pays for this contract's storage.
    fn assert_storage_paid(&mut self) {
        self.scales.flush();
        self.history.flush();
        self.listings.flush();
        self.offers.flush();
        self.offers_by_token.flush();
        self.open_offers.flush();
        self.proposals.flush();
        self.open_proposal.flush();
        env::state_write(self);
        let needed = env::storage_byte_cost().saturating_mul(env::storage_usage() as u128);
        require!(
            env::account_balance().saturating_sub(self.escrow) >= needed,
            "not enough free balance for storage"
        );
    }
}

/// Holders royalties go to the tithe, which splits them like any other income, because one NEP-199 payout cannot
/// carry 100 vault entries.
pub fn royalty_receiver(to: RoyaltyTo, creator: &AccountId, tithe: &AccountId) -> AccountId {
    match to {
        RoyaltyTo::Creator => creator.clone(),
        RoyaltyTo::Holders => tithe.clone(),
    }
}

pub fn royalty_of(price: NearToken) -> NearToken {
    NearToken::from_yoctonear(price.as_yoctonear() * ROYALTY_BPS as u128 / BPS)
}

pub fn vault_account(id: ScaleId) -> AccountId {
    format!("{:03}.{}", id.get(), env::current_account_id())
        .parse()
        .unwrap()
}

fn check(result: Result<(), String>) {
    if let Err(reason) = result {
        env::panic_str(&reason)
    }
}

fn refund_approvals(owner: &AccountId, approvals: &BTreeMap<AccountId, u64>) {
    let bytes: u64 = approvals.keys().map(nft::approval_bytes).sum();
    if bytes > 0 {
        Promise::new(owner.clone())
            .transfer(env::storage_byte_cost().saturating_mul(bytes as u128))
            .detach();
    }
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct Event<'a, T> {
    standard: &'static str,
    version: &'static str,
    event: &'a str,
    data: [T; 1],
}

/// Writes the event straight to text, so a payload type needs no `serde_json::Value` form.
pub(crate) fn emit_delisted(token_id: &str, owner: &AccountId) {
    emit("scale_delisted", fields! { token_id: token_id, owner: owner });
}

pub(crate) fn emit<T: Serialize>(event: &str, data: T) {
    let event = Event {
        standard: "wyrm",
        version: "1.0.0",
        event,
        data: [data],
    };
    env::log_str(&format!(
        "EVENT_JSON:{}",
        serde_json::to_string(&event).unwrap_or_default()
    ));
}
