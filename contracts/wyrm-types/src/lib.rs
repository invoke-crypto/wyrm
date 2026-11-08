pub mod coin;
pub mod collection;
pub mod order;
pub mod preset;

pub use coin::{CoinArgs, Links};
pub use collection::CollectionArgs;
pub use order::{bond_returns, Feature, Order, Verdict, VoteClass};
pub use preset::{
    envelope, preset, BpsRange, Cadence, CouncilTimes, DipLadder, Features, HuntLine, Network, Preset, PresetName,
    RoyaltyTo, SaleTerms, Strategy, StrategyBounds, TaxPolicy, ALL,
};

/// Bumped on every release of the engine contracts, so engines from different builds cannot pair.
pub const ENGINE_VERSION: u32 = 2_00;
