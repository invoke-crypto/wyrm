use crate::*;

pub const MIN_PRICE: NearToken = NearToken::from_millinear(100);
pub const MAX_OFFERS_PER_ACCOUNT: u8 = 10;
pub const MAX_OFFERS_PER_TOKEN: usize = 20;

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct Offer {
    pub id: u64,
    pub buyer: AccountId,
    /// None is an offer for any Scale.
    pub token_id: Option<ScaleId>,
    pub amount: NearToken,
    pub at: u64,
}

#[near(serializers = [json])]
pub struct OfferView {
    pub id: u64,
    pub buyer: AccountId,
    pub token_id: Option<TokenId>,
    pub amount: NearToken,
    pub at: U64,
}

impl From<&Offer> for OfferView {
    fn from(o: &Offer) -> Self {
        OfferView {
            id: o.id,
            buyer: o.buyer.clone(),
            token_id: o.token_id.map(ScaleId::token_id),
            amount: o.amount,
            at: U64(o.at),
        }
    }
}

#[near(serializers = [json])]
pub struct ListingView {
    pub token_id: TokenId,
    pub price: NearToken,
}

#[near]
impl Scales {
    #[payable]
    pub fn list(&mut self, token_id: TokenId, price: NearToken) {
        near_sdk::assert_one_yocto();
        let id = ScaleId::expect(&token_id);
        let owner = env::predecessor_account_id();
        require!(
            !self.owned_by(id, &owner).claim_in_flight(env::block_timestamp()),
            "a claim is in flight"
        );
        require!(price >= MIN_PRICE, "price is below 0.1 NEAR");
        self.listings.insert(id, price);
        emit(
            "scale_listed",
            fields! { token_id: &token_id, owner: &owner, price: price },
        );
        self.assert_storage_paid();
    }

    #[payable]
    pub fn delist(&mut self, token_id: TokenId) {
        near_sdk::assert_one_yocto();
        let id = ScaleId::expect(&token_id);
        let owner = env::predecessor_account_id();
        self.owned_by(id, &owner);
        require!(self.listings.remove(&id).is_some(), "not listed");
        emit_delisted(&token_id, &owner);
    }

    /// Buys a listed Scale for exactly its price. The token moves before anyone is paid.
    #[payable]
    pub fn buy(&mut self, token_id: TokenId) {
        let id = ScaleId::expect(&token_id);
        let buyer = env::predecessor_account_id();
        let price = *self.listings.get(&id).unwrap_or_else(|| env::panic_str("not listed"));
        require!(
            env::attached_deposit() == price,
            "attached deposit must equal the listed price"
        );
        let seller = self.move_scale(id, &buyer, Some(price), None, None).owner;
        let royalty = self.pay_sale(&seller, price);
        emit_sold(&token_id, &seller, &buyer, price, royalty);
        self.assert_storage_paid();
    }

    /// The attached NEAR minus `STORAGE_FEE` is the offer, held in escrow until it is cancelled or accepted.
    #[payable]
    pub fn make_offer(&mut self, token_id: Option<TokenId>) -> u64 {
        let buyer = env::predecessor_account_id();
        let amount = env::attached_deposit().saturating_sub(STORAGE_FEE);
        require!(
            amount >= MIN_PRICE,
            "an offer is at least 0.1 NEAR plus the storage fee"
        );
        let open = self.open_offers.get(&buyer).copied().unwrap_or(0);
        require!(
            open < MAX_OFFERS_PER_ACCOUNT,
            "an account may hold at most 10 open offers"
        );
        let target = token_id.as_deref().map(ScaleId::expect);
        let id = self.next_offer_id;
        if let Some(t) = target {
            require!(self.scale(t).owner != buyer, "cannot offer on your own Scale");
            let index = self.offers_by_token.entry(t).or_default();
            require!(
                index.len() < MAX_OFFERS_PER_TOKEN,
                "this Scale already has 20 open offers"
            );
            index.push(id);
        }
        self.next_offer_id += 1;
        self.open_offers.insert(buyer.clone(), open + 1);
        self.offers.insert(
            id,
            Offer {
                id,
                buyer: buyer.clone(),
                token_id: target,
                amount,
                at: env::block_timestamp(),
            },
        );
        self.escrow = self.escrow.saturating_add(amount);
        emit(
            "offer_made",
            fields! { offer_id: id, buyer: &buyer, token_id: &token_id, amount: amount },
        );
        self.assert_storage_paid();
        id
    }

    #[payable]
    pub fn cancel_offer(&mut self, offer_id: u64) {
        near_sdk::assert_one_yocto();
        require!(
            self.offer(offer_id).buyer == env::predecessor_account_id(),
            "not your offer"
        );
        let offer = self.take_offer(offer_id);
        Promise::new(offer.buyer.clone()).transfer(offer.amount).detach();
        emit(
            "offer_cancelled",
            fields! { offer_id: offer_id, buyer: &offer.buyer, amount: offer.amount },
        );
    }

    /// The owner sells `token_id` to the offer's buyer. For an any-Scale offer the owner names the token.
    #[payable]
    pub fn accept_offer(&mut self, offer_id: u64, token_id: TokenId) {
        near_sdk::assert_one_yocto();
        let id = ScaleId::expect(&token_id);
        let seller = env::predecessor_account_id();
        self.owned_by(id, &seller);
        require!(
            self.offer(offer_id).token_id.is_none_or(|t| t == id),
            "the offer is for another Scale"
        );
        let offer = self.take_offer(offer_id);
        self.move_scale(id, &offer.buyer, Some(offer.amount), None, None);
        let royalty = self.pay_sale(&seller, offer.amount);
        emit_sold(&token_id, &seller, &offer.buyer, offer.amount, royalty);
        self.assert_storage_paid();
    }

    pub fn get_listings(&self, from: Option<u32>, limit: Option<u32>) -> Vec<ListingView> {
        ScaleId::all()
            .filter_map(|id| {
                self.listings.get(&id).map(|p| ListingView {
                    token_id: id.token_id(),
                    price: *p,
                })
            })
            .skip(from.unwrap_or(0) as usize)
            .take(limit.unwrap_or(SUPPLY as u32) as usize)
            .collect()
    }

    pub fn get_offers(&self, from: Option<u32>, limit: Option<u32>) -> Vec<OfferView> {
        self.offers
            .values()
            .skip(from.unwrap_or(0) as usize)
            .take(limit.unwrap_or(50) as usize)
            .map(OfferView::from)
            .collect()
    }

    /// Offers named for this Scale. Any-Scale offers are in `get_offers`.
    pub fn get_offers_for(&self, token_id: TokenId) -> Vec<OfferView> {
        let Some(id) = ScaleId::parse(&token_id) else {
            return vec![];
        };
        self.offers_by_token
            .get(&id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|o| self.offers.get(o))
                    .map(OfferView::from)
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Scales {
    fn offer(&self, offer_id: u64) -> &Offer {
        self.offers
            .get(&offer_id)
            .unwrap_or_else(|| env::panic_str("no such offer"))
    }

    /// Removes an offer from every index and from escrow. The caller pays it out.
    fn take_offer(&mut self, offer_id: u64) -> Offer {
        let offer = self.offers.remove(&offer_id).unwrap();
        if let Some(t) = offer.token_id {
            let index = self.offers_by_token.get_mut(&t).unwrap();
            index.retain(|o| *o != offer_id);
            if index.is_empty() {
                self.offers_by_token.remove(&t);
            }
        }
        match self.open_offers.get(&offer.buyer).copied() {
            Some(1) | None => {
                self.open_offers.remove(&offer.buyer);
            }
            Some(n) => {
                self.open_offers.insert(offer.buyer.clone(), n - 1);
            }
        }
        self.escrow = self.escrow.saturating_sub(offer.amount);
        offer
    }
}

fn emit_sold(token_id: &str, from: &AccountId, to: &AccountId, price: NearToken, royalty: NearToken) {
    emit(
        "scale_sold",
        fields! { token_id: token_id, from: from, to: to, price: price, royalty: royalty },
    );
}
