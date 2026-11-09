//! The Dragon's decisions as pure functions of its state and one observation: price samples and the 24 h reference,
//! dip confirmation, the day and its breath minute, slice sizing under the price-impact cap, Hunt, hoard
//! reconciliation, the tax and the mood. Nothing here touches the chain, so all of it runs under native tests.
use crate::limits::*;
use crate::math::{mul_div, mul_ge};
use near_sdk::near;
use wyrm_types::{envelope, Cadence, DipLadder, Strategy};

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliceKind {
    Breath,
    Dip,
    Hunt,
}

impl SliceKind {
    pub fn side(self) -> Side {
        match self {
            SliceKind::Breath | SliceKind::Dip => Side::Buy,
            SliceKind::Hunt => Side::Sell,
        }
    }
}

/// `cumulative` is the sum of tokens_per_near times whole seconds held, over every sample before this one. It wraps:
/// only differences over a window are read, and those fit while the window mean is under 3.9e33.
#[near(serializers = [borsh])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub at: u64,
    pub tokens_per_near: u128,
    pub cumulative: u128,
}

fn secs(t: u64) -> u128 {
    (t / SECOND) as u128
}

impl Sample {
    pub fn first(at: u64, tokens_per_near: u128) -> Self {
        Self {
            at,
            tokens_per_near,
            cumulative: 0,
        }
    }

    /// The accumulator at `t`, assuming this sample's price held from its own time until `t`.
    pub fn cumulative_at(&self, t: u64) -> u128 {
        self.cumulative
            .wrapping_add(self.tokens_per_near.wrapping_mul(secs(t) - secs(self.at)))
    }

    pub fn next(&self, at: u64, tokens_per_near: u128) -> Self {
        Self {
            at,
            tokens_per_near,
            cumulative: self.cumulative_at(at),
        }
    }
}

pub fn sample_due(newest: Option<&Sample>, now: u64) -> bool {
    newest.is_none_or(|s| now >= s.at + SAMPLE_GAP)
}

/// Time-weighted mean of tokens_per_near over the REFERENCE_WINDOW before `now`. Each sample counts for the time
/// until the next one, the newest until `now`, so a sample taken at `now` has no weight yet. `get(0)` is the oldest
/// of `len` samples. `None` until some time has passed since the first sample.
pub fn reference(len: u32, get: impl Fn(u32) -> Sample, now: u64) -> Option<u128> {
    if len == 0 {
        return None;
    }
    let start = now.saturating_sub(REFERENCE_WINDOW);
    let oldest = get(0);
    let (base, from) = if oldest.at >= start {
        (oldest, oldest.at)
    } else {
        // The newest sample at or before the window start; get(lo).at <= start holds throughout.
        let (mut lo, mut hi) = (0, len - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if get(mid).at <= start {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        (get(lo), start)
    };
    let width = secs(now) - secs(from);
    if width == 0 {
        return None;
    }
    Some(get(len - 1).cumulative_at(now).wrapping_sub(base.cumulative_at(from)) / width)
}

/// A price `step_bps` below the reference buys at least `reference * 10_000 / (10_000 - step_bps)` tokens per NEAR.
fn beyond(tokens_per_near: u128, reference: u128, step_bps: u16) -> bool {
    mul_ge(tokens_per_near, BPS - step_bps as u128, reference, BPS)
}

/// How many dip steps a price is at or beyond: 0 for none, 3 for the deepest.
pub fn dip_depth(tokens_per_near: u128, reference: u128, steps_bps: &[u16; 3]) -> u8 {
    steps_bps
        .iter()
        .take_while(|s| beyond(tokens_per_near, reference, **s))
        .count() as u8
}

/// A confirmed dip: the deepest step confirmed, and the run of samples that confirmed it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dip {
    pub depth: u8,
    pub confirmations: u8,
    pub since: u64,
}

/// The deepest step the newest samples confirm: at least DIP_SAMPLES samples in a row, newest first, all beyond the
/// step, the oldest of them at least DIP_SPAN before the newest. The run reported is the shortest that confirms.
pub fn confirmed_dip(newest_first: &[Sample], reference: u128, steps_bps: &[u16; 3]) -> Dip {
    let Some(newest) = newest_first.first() else {
        return Dip::default();
    };
    let confirming_run = |step: u16| {
        newest_first
            .iter()
            .take_while(|s| beyond(s.tokens_per_near, reference, step))
            .enumerate()
            .find(|(i, s)| i + 1 >= DIP_SAMPLES && newest.at - s.at >= DIP_SPAN)
            .map(|(i, s)| (i as u8 + 1, s.at))
    };
    let mut dip = Dip::default();
    for (k, step) in steps_bps.iter().enumerate() {
        let Some((confirmations, since)) = confirming_run(*step) else {
            break;
        };
        dip = Dip {
            depth: k as u8 + 1,
            confirmations,
            since,
        };
    }
    dip
}

/// The run the newest samples are in, for the Stirring mood: the deepest step the newest sample is beyond, how many
/// samples in a row (within the scan) are beyond it, and when the oldest of them was taken. `None` when no dip.
pub fn dip_run(newest_first: &[Sample], reference: u128, steps_bps: &[u16; 3]) -> Option<Dip> {
    let newest = newest_first.first()?;
    let depth = dip_depth(newest.tokens_per_near, reference, steps_bps);
    if depth == 0 {
        return None;
    }
    let step = steps_bps[depth as usize - 1];
    let run: Vec<&Sample> = newest_first
        .iter()
        .take_while(|s| beyond(s.tokens_per_near, reference, step))
        .collect();
    Some(Dip {
        depth,
        confirmations: run.len() as u8,
        since: run.last().map_or(newest.at, |s| s.at),
    })
}

/// Spot is more than SPIKE_BPS above the reference price, so a NEAR buys that much fewer tokens.
pub fn spiking(tokens_per_near: u128, reference: u128) -> bool {
    !mul_ge(tokens_per_near, BPS + SPIKE_BPS, reference, BPS)
}

/// Budgets are fractions of `opening_reserve`, the reserve at the day's first observation, so money arriving during
/// the day waits for tomorrow. `budget` is what the latest observation allowed, kept for the views and events.
#[near(serializers = [borsh])]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Day {
    pub index: u64,
    pub breath_at: u64,
    pub opening_reserve: u128,
    pub budget: u128,
    pub spent: u128,
    pub sold_wyrm: u128,
}

impl Day {
    /// The breath minute is drawn from the block's random seed at the day's first observation, so that a spread
    /// starting there ends inside the UTC day (the Gym's breathMinute).
    pub fn starting(now: u64, seed: [u8; 32], opening_reserve: u128, cadence: &Cadence) -> Self {
        let index = now / DAY;
        let draw = u64::from_le_bytes(seed[..8].try_into().expect("8 bytes"));
        let minutes = (DAY / MINUTE).saturating_sub(cadence.spread_minutes as u64).max(1);
        Self {
            index,
            breath_at: index * DAY + (draw % minutes) * MINUTE,
            opening_reserve,
            ..Self::default()
        }
    }

    /// A day not observed yet: nothing is unlocked and nothing spent.
    pub fn unobserved(now: u64) -> Self {
        Self {
            index: now / DAY,
            breath_at: u64::MAX,
            ..Self::default()
        }
    }

    pub fn breath_open(&self, now: u64) -> bool {
        now >= self.breath_at
    }
}

pub fn roll(day: &Day, now: u64, seed: [u8; 32], reserve: u128, cadence: &Cadence) -> Option<Day> {
    (now / DAY != day.index).then(|| Day::starting(now, seed, reserve, cadence))
}

/// The Council-moved strategy with the preset's fixed ladder and cadence, as one borrow for the rules.
#[derive(Clone, Copy)]
pub struct Rules<'a> {
    pub strategy: &'a Strategy,
    pub ladder: &'a DipLadder,
    pub cadence: &'a Cadence,
}

/// What the day may spend in total, in bps of its opening reserve: the breath budget once the breath minute has
/// come, plus the extras of every step `dip` confirms in this observation. A dip that lapses takes its extras back.
/// With a spread, the breath part opens linearly: one minute's share at the breath minute, all of it once
/// `spread_minutes` have passed (the Gym's allowance).
pub fn day_budget(r: Rules, day: &Day, now: u64, dip: u8) -> u128 {
    let extras: u16 = r.ladder.extra_bps[..dip as usize].iter().sum();
    if !day.breath_open(now) {
        return mul_div(extras as u128, day.opening_reserve, BPS);
    }
    let budget = r.strategy.budget_bps;
    let spread = r.cadence.spread_minutes as u64 * MINUTE;
    if spread == 0 {
        return mul_div((budget + extras) as u128, day.opening_reserve, BPS);
    }
    let opened = (now - day.breath_at + MINUTE).min(spread);
    let breath = mul_div(
        mul_div(budget as u128, day.opening_reserve, BPS),
        opened as u128,
        spread as u128,
    );
    breath + mul_div(extras as u128, day.opening_reserve, BPS)
}

pub fn buy_room(r: Rules, day: &Day, now: u64, dip: u8) -> u128 {
    day_budget(r, day, now, dip).saturating_sub(day.spent)
}

/// `wyrm_bought` counts only swap output, so donations never lower the average cost `near_spent / wyrm_bought`.
/// `wyrm` is what the Dragon owns, its token balance less `ash_due`, so no cap and no sale ever counts WYRM owed to
/// ash.
#[near(serializers = [borsh])]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hoard {
    pub wyrm: u128,
    pub near_spent: u128,
    pub wyrm_bought: u128,
    pub donated: u128,
    pub granted: u128,
    pub realized_near: u128,
    pub burned: u128,
    pub auto_burned: u128,
    pub ash_due: u128,
}

/// What swaps since the last reconcile should deliver: the net of a buy, the quoted wNEAR of a sell.
#[near(serializers = [borsh])]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unsettled {
    pub wyrm_out: u128,
    pub wnear_out: u128,
}

/// Never called while an outflow is in flight, because its transfer can land before its callback. Sets the hoard
/// to the balance less `ash_due`. A rise is credited to `wyrm_bought` up to the expected net of the swaps since the
/// last reconcile and to `donated` beyond it. `auto_burn_bps` of the bought part, never of a donation, moves from
/// the hoard to `ash_due`.
pub fn reconcile_wyrm(hoard: &mut Hoard, unsettled: &mut Unsettled, balance: u128, auto_burn_bps: u16) {
    let seen = balance.saturating_sub(hoard.ash_due);
    let mut burn = 0;
    if seen > hoard.wyrm {
        let gain = seen - hoard.wyrm;
        let bought = gain.min(unsettled.wyrm_out);
        hoard.wyrm_bought += bought;
        hoard.donated += gain - bought;
        burn = mul_div(bought, auto_burn_bps as u128, BPS);
        if bought > 0 {
            // DCL delivers a swap's output in one transfer, so the remainder of a quote never arrives.
            unsettled.wyrm_out = 0;
        }
    }
    hoard.ash_due += burn;
    hoard.wyrm = seen - burn;
}

/// A buy: DCL checks `min_output_amount` before the token takes the buy tax, so `min_out` stays on the gross quote,
/// and what the Dragon expects to receive is the net.
pub fn expected_net(quoted_out: u128) -> u128 {
    mul_div(quoted_out, BPS - envelope::TAX.buy_bps as u128, BPS)
}

/// A sell: the token takes the sell tax from the Dragon's transfer into the pool, so DCL swaps only the net.
pub fn sell_quote_input(gross_in: u128) -> u128 {
    mul_div(gross_in, BPS - envelope::TAX.sell_bps as u128, BPS)
}

/// The wNEAR counterpart: a rise up to the quoted proceeds of Hunt sales is realized NEAR.
pub fn reconcile_wnear(known: &mut u128, hoard: &mut Hoard, unsettled: &mut Unsettled, balance: u128) {
    if balance > *known {
        let realized = (balance - *known).min(unsettled.wnear_out);
        hoard.realized_near += realized;
        if realized > 0 {
            unsettled.wnear_out = 0;
        }
    }
    *known = balance;
}

/// Spot is at least `sell_multiple_bps` of the average cost: `ONE_NEAR / tpn >= multiple / BPS * spent / bought`.
pub fn above_sell_line(hoard: &Hoard, tokens_per_near: u128, sell_multiple_bps: u32) -> bool {
    if hoard.wyrm_bought == 0 || tokens_per_near == 0 {
        return false;
    }
    let Some(cost) = (sell_multiple_bps as u128).checked_mul(hoard.near_spent) else {
        return false;
    };
    mul_ge(ONE_NEAR * BPS, hoard.wyrm_bought, cost, tokens_per_near)
}

pub fn sell_room(max_sell_bps: u16, day: &Day, hoard: &Hoard) -> u128 {
    mul_div(max_sell_bps as u128, hoard.wyrm + day.sold_wyrm, BPS)
        .saturating_sub(day.sold_wyrm)
        .min(hoard.wyrm)
}

/// The NEAR value of an amount on `side`'s input at the observed price.
pub fn near_value(side: Side, amount_in: u128, tokens_per_near: u128) -> u128 {
    match side {
        Side::Buy => amount_in,
        Side::Sell => mul_div(amount_in, ONE_NEAR, tokens_per_near.max(1)),
    }
}

pub struct Observation {
    pub tokens_per_near: u128,
    pub reference: Option<u128>,
    /// The deepest step confirmed by the newest samples.
    pub dip: u8,
}

pub struct Inputs<'a> {
    pub now: u64,
    pub rules: Rules<'a>,
    pub day: &'a Day,
    pub hoard: &'a Hoard,
    pub reserve: u128,
    pub last_slice_at: u64,
    pub market: Option<Observation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rest {
    NoPrice,
    Gap,
    NoReference,
    Spike,
    BeforeBreath,
    Spent,
    Dust,
}

impl Rest {
    pub fn as_str(self) -> &'static str {
        match self {
            Rest::NoPrice => "no price",
            Rest::Gap => "slice gap",
            Rest::NoReference => "no reference yet",
            Rest::Spike => "price spike",
            Rest::BeforeBreath => "before the breath",
            Rest::Spent => "budget spent",
            Rest::Dust => "too little to trade",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Rest(Rest),
    Trade { kind: SliceKind, amount_in: u128 },
}

/// Hunt sell first, then a dip buy, then a breath buy. The amount is the most the rules allow; the price-impact cap
/// then cuts it with `candidates` and `pick_slice`.
pub fn decide(i: &Inputs) -> Decision {
    let Some(m) = &i.market else {
        return Decision::Rest(Rest::NoPrice);
    };
    if i.now < i.last_slice_at + i.rules.cadence.gap_seconds as u64 * SECOND {
        return Decision::Rest(Rest::Gap);
    }
    if let Some(h) = i.rules.strategy.hunt {
        if above_sell_line(i.hoard, m.tokens_per_near, h.sell_multiple_bps) {
            let room = sell_room(h.max_sell_bps, i.day, i.hoard);
            if near_value(Side::Sell, room, m.tokens_per_near) >= MIN_SLICE_NEAR {
                return Decision::Trade {
                    kind: SliceKind::Hunt,
                    amount_in: room,
                };
            }
        }
    }
    let Some(reference) = m.reference else {
        return Decision::Rest(Rest::NoReference);
    };
    if spiking(m.tokens_per_near, reference) {
        return Decision::Rest(Rest::Spike);
    }
    if !i.day.breath_open(i.now) && m.dip == 0 {
        return Decision::Rest(Rest::BeforeBreath);
    }
    let room = buy_room(i.rules, i.day, i.now, m.dip);
    if room == 0 {
        return Decision::Rest(Rest::Spent);
    }
    let amount_in = room.min(i.reserve);
    if amount_in < MIN_SLICE_NEAR {
        return Decision::Rest(Rest::Dust);
    }
    let kind = if m.dip > 0 { SliceKind::Dip } else { SliceKind::Breath };
    Decision::Trade { kind, amount_in }
}

/// The amounts the slice cap tries, largest first: the decided amount halved up to MAX_HALVINGS times, none under
/// the minimum slice.
pub fn candidates(side: Side, amount_in: u128, tokens_per_near: u128) -> Vec<u128> {
    (0..=MAX_HALVINGS)
        .map(|h| amount_in >> h)
        .filter(|a| near_value(side, *a, tokens_per_near) >= MIN_SLICE_NEAR)
        .collect()
}

/// A small trade's quote that prices the slice cap: `unit_in` in quoted `unit_out` out. A buy measures against the
/// observed 1 NEAR quote; a sell against the quote for selling 1 NEAR worth of WYRM (`tokens_per_near` of it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unit {
    pub unit_in: u128,
    pub unit_out: u128,
}

impl Unit {
    pub fn buy(tokens_per_near: u128) -> Self {
        Self {
            unit_in: ONE_NEAR,
            unit_out: tokens_per_near,
        }
    }
}

/// The slice cap as price impact: the trade must get at least `10_000 - cap_bps` bps of what the unit quote prices
/// it at. Unlike pool depth, this cannot be inflated with liquidity far from the price.
pub fn within_impact(amount_in: u128, quoted_out: u128, unit: Unit, cap_bps: u16) -> bool {
    let at_unit_price = mul_div(amount_in, unit.unit_out, unit.unit_in.max(1));
    quoted_out > 0 && mul_ge(quoted_out, BPS, at_unit_price, BPS - cap_bps as u128)
}

/// The first candidate whose quote passes the slice cap, with its quote.
pub fn pick_slice(candidates: &[u128], quotes: &[Option<u128>], unit: Unit, cap_bps: u16) -> Option<(u128, u128)> {
    candidates.iter().zip(quotes).find_map(|(a, q)| {
        let q = (*q)?;
        within_impact(*a, q, unit, cap_bps).then_some((*a, q))
    })
}

pub fn min_out(quoted_out: u128) -> u128 {
    mul_div(quoted_out, BPS - SLIPPAGE_BPS, BPS)
}

#[near(serializers = [json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Sleeping,
    Stirring,
    Breathing,
    Gorged,
    Hunting,
}

/// `swapping` is the side of a swap in flight; `spot_depth` the steps the newest price is beyond, confirmed or not.
pub fn mood(i: &Inputs, swapping: Option<Side>, spot_depth: u8) -> Mood {
    match swapping {
        Some(Side::Sell) => return Mood::Hunting,
        Some(Side::Buy) => return Mood::Breathing,
        None => {}
    }
    let Some(m) = &i.market else { return Mood::Sleeping };
    if let Some(h) = i.rules.strategy.hunt {
        if above_sell_line(i.hoard, m.tokens_per_near, h.sell_multiple_bps) {
            return Mood::Hunting;
        }
    }
    let room = buy_room(i.rules, i.day, i.now, m.dip).min(i.reserve);
    if i.day.breath_open(i.now) && i.day.spent > 0 && room < MIN_SLICE_NEAR {
        return Mood::Gorged;
    }
    let open = i.day.breath_open(i.now) || m.dip > 0;
    let calm = m.reference.is_some_and(|r| !spiking(m.tokens_per_near, r));
    if open && calm && room >= MIN_SLICE_NEAR {
        return Mood::Breathing;
    }
    if spot_depth > m.dip {
        return Mood::Stirring;
    }
    Mood::Sleeping
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod tests;
