use near_contract_standards::non_fungible_token::TokenId;
use near_sdk::{env, near};
use std::fmt;
use std::ops::RangeInclusive;

pub const SUPPLY: u8 = 100;
/// Sold in the sale, and vote.
pub const SALE_IDS: RangeInclusive<u8> = 1..=90;
/// Minted to the creator at init, and never vote.
pub const CREATOR_IDS: RangeInclusive<u8> = 91..=100;
pub const SALE_SUPPLY: u8 = *SALE_IDS.end();

/// A token id parsed at the boundary: `"1"` to `"100"`, no other spelling.
#[near(serializers = [borsh])]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScaleId(u8);

impl ScaleId {
    pub fn new(n: u8) -> Option<Self> {
        (1..=SUPPLY).contains(&n).then_some(Self(n))
    }

    pub fn parse(token_id: &str) -> Option<Self> {
        let n: u8 = token_id.parse().ok()?;
        Self::new(n).filter(|id| id.to_string() == token_id)
    }

    pub fn expect(token_id: &str) -> Self {
        Self::parse(token_id).unwrap_or_else(|| env::panic_str("unknown token id"))
    }

    pub fn get(self) -> u8 {
        self.0
    }

    /// Only sale Scales vote. The creator's Scales never do, whoever holds them.
    pub fn votes(self) -> bool {
        SALE_IDS.contains(&self.0)
    }

    pub fn token_id(self) -> TokenId {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = ScaleId> {
        (1..=SUPPLY).map(ScaleId)
    }
}

impl fmt::Display for ScaleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A set of Scales as one bit per id, so a vote record or the vault roll costs 16 bytes however full it is.
#[near(serializers = [borsh])]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScaleSet(u128);

impl ScaleSet {
    pub fn contains(self, id: ScaleId) -> bool {
        self.0 & (1 << id.0) != 0
    }

    pub fn insert(&mut self, id: ScaleId) {
        self.0 |= 1 << id.0;
    }

    pub fn remove(&mut self, id: ScaleId) {
        self.0 &= !(1 << id.0);
    }

    pub fn count(self) -> u32 {
        self.0.count_ones()
    }

    pub fn union(self, other: ScaleSet) -> ScaleSet {
        ScaleSet(self.0 | other.0)
    }

    pub fn iter(self) -> impl Iterator<Item = ScaleId> {
        ScaleId::all().filter(move |id| self.contains(*id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_canonical_ids_one_to_one_hundred_parse() {
        assert_eq!(ScaleId::parse("1").map(ScaleId::get), Some(1));
        assert_eq!(ScaleId::parse("100").map(ScaleId::get), Some(100));
        for bad in ["0", "101", "01", "+5", " 5", "", "abc", "255", "256"] {
            assert_eq!(ScaleId::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn set_holds_every_id() {
        let mut s = ScaleSet::default();
        for id in ScaleId::all() {
            s.insert(id);
        }
        assert_eq!(s.count(), 100);
        s.remove(ScaleId::new(100).unwrap());
        assert!(!s.contains(ScaleId::new(100).unwrap()));
        assert_eq!(s.iter().count(), 99);
    }

    #[test]
    fn votes_is_true_for_1_to_90_only() {
        let voting: Vec<u8> = ScaleId::all().filter(|id| id.votes()).map(ScaleId::get).collect();
        assert_eq!(voting, (1..=90).collect::<Vec<_>>());
        assert!(ScaleId::all()
            .filter(|id| !id.votes())
            .map(ScaleId::get)
            .eq(CREATOR_IDS));
    }
}
