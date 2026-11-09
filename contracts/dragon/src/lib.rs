//! The Dragon: a keyless treasury for one coin. It launches the coin on Nearly as its creator, binds to it, holds a
//! NEAR reserve and the coin's hoard, buys in small slices through the DCL pool on `poke()`, and obeys only bounded
//! orders from its Council. The decisions live in `rules`, launch and bind in `launch` and `nearly`, the Council in
//! `council`; this file is state, the poke chain and views.
pub mod council;
pub mod dcl;
pub mod events;
pub mod launch;
pub mod limits;
pub mod math;
pub mod nearly;
pub mod ring;
pub mod rules;
pub mod views;

use council::{Pending, Window};
use events::{DipFacts, Event, LogEntry, Slice};
use limits::*;
use near_sdk::json_types::{U128, U64};
use near_sdk::{env, near, require, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseOrValue};
use nearly::Coin;
use ring::Ring;
use rules::{Day, Decision, Hoard, Inputs, Observation, Rules, Sample, Side, SliceKind, Unsettled};
use views::*;
use wyrm_types::{envelope, Preset, PresetName, Strategy, ENGINE_VERSION};

// Besides the gas it is given, each function-call receipt costs its creator about 4.9 Tgas of fees and each result a
// callback waits on about 9.4 Tgas, so eight parallel quotes cost some 355 Tgas. Only an observation that rests
// sends an auto-burn, so the burn's batch never adds to a trading chain.
const GAS_BALANCE: Gas = Gas::from_tgas(5);
const GAS_POOL: Gas = Gas::from_tgas(10);
const GAS_QUOTE: Gas = Gas::from_tgas(30);
const GAS_DEPOSIT: Gas = Gas::from_tgas(5);
const GAS_SWAP: Gas = Gas::from_tgas(180);
const GAS_ON_SWAP: Gas = Gas::from_tgas(10);
const GAS_ON_QUOTE: Gas = Gas::from_tgas(240);
const GAS_ON_OBSERVE: Gas = Gas::from_tgas(620);
pub const GAS_POKE_MIN: Gas = Gas::from_tgas(760);
const GAS_REGISTER: Gas = Gas::from_tgas(10);
const REGISTER_DEPOSIT: NearToken = NearToken::from_millinear(125);
const ONE_YOCTO: NearToken = NearToken::from_yoctonear(1);

// Upper bounds of one ring slot or pending entry in storage, record overhead included. Free slots are kept back from
// the reserve so a full ring can never fail a write for lack of balance.
const SAMPLE_SLOT_BYTES: u64 = 100;
const LOG_SLOT_BYTES: u64 = 700;
const SPENT_SLOT_BYTES: u64 = 90;
const PENDING_BYTES: u64 = 200;

/// The accounts a Dragon works with, fixed at init.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Peers {
    pub wrap: AccountId,
    pub dcl: AccountId,
    pub nearly: AccountId,
    pub tithe: AccountId,
    pub scales: AccountId,
    pub ash: AccountId,
    pub creator: AccountId,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Idle,
    Observing {
        step: u64,
        since: u64,
    },
    Quoting {
        step: u64,
        since: u64,
        side: Side,
        amount_in: u128,
    },
    Swapping {
        step: u64,
        since: u64,
        side: Side,
        amount_in: u128,
        quoted_out: u128,
        min_out: u128,
    },
}

impl Phase {
    fn in_flight(&self) -> Option<(u64, u64)> {
        match *self {
            Phase::Idle => None,
            Phase::Observing { step, since }
            | Phase::Quoting { step, since, .. }
            | Phase::Swapping { step, since, .. } => Some((step, since)),
        }
    }
}

/// A trade an observation decided, carried through the quotes to the swap so its event can show why it was made.
#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct Plan {
    pub kind: SliceKind,
    /// The amount the rules allow, before the slice cap. A sell's is gross, before the sell tax.
    pub amount_in: U128,
    pub tokens_per_near: U128,
    pub reference: Option<U128>,
    pub breath_at: U64,
    pub budget: U128,
    pub dip: Option<DipFacts>,
}

/// What `on_swap` needs to settle a swap, carried in the callback so a late result settles correctly after the
/// phase has moved on.
#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct Ticket {
    pub plan: Plan,
    pub at: U64,
    pub amount_in: U128,
    pub quoted_out: U128,
    pub min_out: U128,
    pub cap_bps: u16,
    pub day: U64,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Dragon {
    preset: PresetName,
    peers: Peers,
    /// The Council moves only this; the ladder and cadence are read from the preset.
    strategy: Strategy,
    /// Unknown until Nearly's launch lands, so state and not an init argument. Set once by bind.
    coin: Option<Coin>,
    last_launch_at: u64,
    phase: Phase,
    step: u64,
    day: Day,
    hoard: Hoard,
    unsettled: Unsettled,
    /// wNEAR the Dragon holds as far as it knows: the last balance read, moved by its own swaps since.
    wnear: u128,
    last_slice_at: u64,
    grants: Window,
    burns: Window,
    pending: Vec<Pending>,
    outflows: u64,
    samples: Ring<Sample>,
    log: Ring<LogEntry>,
}

#[near]
impl Dragon {
    #[init]
    pub fn new(preset: PresetName, peers: Peers) -> Self {
        let p = wyrm_types::preset(preset);
        p.check().unwrap_or_else(|e| env::panic_str(&e));
        p.allowed_on(&env::current_account_id())
            .unwrap_or_else(|e| env::panic_str(&e));
        Promise::new(peers.wrap.clone())
            .function_call(
                "storage_deposit",
                format!(
                    r#"{{"account_id":"{}","registration_only":true}}"#,
                    env::current_account_id()
                )
                .into_bytes(),
                REGISTER_DEPOSIT,
                GAS_REGISTER,
            )
            .detach();
        Self {
            preset,
            peers,
            strategy: p.strategy,
            coin: None,
            last_launch_at: 0,
            phase: Phase::Idle,
            step: 0,
            day: Day::default(),
            hoard: Hoard::default(),
            unsettled: Unsettled::default(),
            wnear: 0,
            last_slice_at: 0,
            grants: Window::new(b"g"),
            burns: Window::new(b"b"),
            pending: vec![],
            outflows: 0,
            samples: Ring::new(b"s", SAMPLES_CAP),
            log: Ring::new(b"l", LOG_CAP),
        }
    }

    /// One step: observe balances, the pool and the price, then maybe trade one slice.
    pub fn poke(&mut self) -> Promise {
        require!(self.coin.is_some(), "not bound");
        let now = env::block_timestamp();
        if let Some((_, since)) = self.phase.in_flight() {
            require!(now >= since + STALE, "in flight");
        }
        require!(env::prepaid_gas() >= GAS_POKE_MIN, "poke needs at least 760 Tgas");
        self.step += 1;
        let step = self.step;
        self.phase = Phase::Observing { step, since: now };
        let me = env::current_account_id();
        let coin = self.coin();
        let balance = format!(r#"{{"account_id":"{me}"}}"#);
        self.view_call(&coin.token, "ft_balance_of", balance.clone(), GAS_BALANCE)
            .and(self.view_call(&self.peers.wrap, "ft_balance_of", balance, GAS_BALANCE))
            .and(self.view_call(&self.peers.dcl, "get_pool", dcl::pool_args(&coin.pool_id), GAS_POOL))
            .and(self.quote(Side::Buy, ONE_NEAR))
            .then(Self::ext(me).with_static_gas(GAS_ON_OBSERVE).on_observe(U64(step)))
    }

    #[private]
    pub fn on_observe(&mut self, #[serializer(borsh)] step: U64) -> PromiseOrValue<String> {
        if !matches!(self.phase, Phase::Observing { step: s, .. } if s == step.0) {
            return PromiseOrValue::Value("stale step".into());
        }
        let now = env::block_timestamp();
        let wyrm_balance = dcl::result(0).and_then(|b| dcl::parse_u128(&b));
        let wnear_balance = dcl::result(1).and_then(|b| dcl::parse_u128(&b));
        let pool = dcl::result(2).and_then(|b| dcl::parse_pool(&b, self.coin().wrap_is_x));
        let quote = dcl::result(3).and_then(|b| dcl::parse_quote(&b));

        self.expire_pending(now);
        if let Some(b) = wyrm_balance.filter(|_| self.pending.is_empty()) {
            rules::reconcile_wyrm(&mut self.hoard, &mut self.unsettled, b, self.strategy.auto_burn_bps);
        }
        if let Some(w) = wnear_balance {
            rules::reconcile_wnear(&mut self.wnear, &mut self.hoard, &mut self.unsettled, w);
        }
        let preset = self.preset();
        let rules = Rules {
            strategy: &self.strategy,
            ladder: &preset.ladder,
            cadence: &preset.cadence,
        };
        let reserve = self.near_free() + self.wnear;
        if let Some(day) = rules::roll(&self.day, now, env::random_seed_array(), reserve, rules.cadence) {
            self.day = day;
        }
        let price = quote.filter(|_| pool.as_ref().is_some_and(|p| p.running));
        if let Some(tpn) = price {
            if rules::sample_due(self.samples.newest(), now) {
                let sample = match self.samples.newest() {
                    None => Sample::first(now, tpn),
                    Some(prev) => prev.next(now, tpn),
                };
                self.samples.push(sample);
            }
        }
        let reference = self.reference(now);
        let dip = match (price, reference) {
            (Some(_), Some(r)) => rules::confirmed_dip(&self.recent_samples(), r, &preset.ladder.steps_bps),
            _ => rules::Dip::default(),
        };
        self.day.budget = rules::day_budget(rules, &self.day, now, dip.depth);
        let decision = rules::decide(&Inputs {
            now,
            rules,
            day: &self.day,
            hoard: &self.hoard,
            reserve,
            last_slice_at: self.last_slice_at,
            market: price.map(|tokens_per_near| Observation {
                tokens_per_near,
                reference,
                dip: dip.depth,
            }),
        });
        let status = match decision {
            Decision::Rest(r) => format!("rest: {}", r.as_str()),
            Decision::Trade { kind, amount_in } => format!("{kind:?} slice: quoting {amount_in}"),
        };
        Event::Observe {
            at: U64(now),
            step,
            tokens_per_near: price.map(U128),
            reference: reference.map(U128),
            depth_near: pool.map(|p| U128(p.depth_near)),
            dip: dip.depth,
            reserve: U128(reserve),
            hoard_wyrm: U128(self.hoard.wyrm),
            breath_at: U64(self.day.breath_at),
            decision: status.clone(),
        }
        .emit();

        let (Decision::Trade { kind, amount_in }, Some(tpn)) = (decision, price) else {
            self.phase = Phase::Idle;
            if let Some(burn) = self.send_auto_burn(now) {
                burn.detach();
            }
            return PromiseOrValue::Value(status);
        };
        let side = kind.side();
        self.phase = Phase::Quoting {
            step: step.0,
            since: now,
            side,
            amount_in,
        };
        let plan = Plan {
            kind,
            amount_in: U128(amount_in),
            tokens_per_near: U128(tpn),
            reference: reference.map(U128),
            breath_at: U64(self.day.breath_at),
            budget: U128(self.day.budget),
            dip: (kind == SliceKind::Dip).then(|| DipFacts {
                step_bps: preset.ladder.steps_bps[dip.depth as usize - 1],
                confirmations: dip.confirmations,
                confirmed_since: U64(dip.since),
            }),
        };
        let inputs = quote_inputs(side, &rules::candidates(side, amount_in, tpn));
        let quotes = inputs[1..]
            .iter()
            .fold(self.quote(side, inputs[0]), |p, a| p.and(self.quote(side, *a)));
        // A sell's cap is measured against selling 1 NEAR worth of WYRM, net of the sell tax, quoted last.
        let quotes = if side == Side::Sell {
            quotes.and(self.quote(side, rules::sell_quote_input(tpn)))
        } else {
            quotes
        };
        let next = Self::ext(env::current_account_id())
            .with_static_gas(GAS_ON_QUOTE)
            .on_quote(step, plan);
        PromiseOrValue::Promise(quotes.then(next))
    }

    #[private]
    pub fn on_quote(
        &mut self,
        #[serializer(borsh)] step: U64,
        #[serializer(borsh)] plan: Plan,
    ) -> PromiseOrValue<String> {
        if !matches!(self.phase, Phase::Quoting { step: s, .. } if s == step.0) {
            return PromiseOrValue::Value("stale step".into());
        }
        let side = plan.kind.side();
        let tpn = plan.tokens_per_near.0;
        let candidates = rules::candidates(side, plan.amount_in.0, tpn);
        let inputs = quote_inputs(side, &candidates);
        let quotes: Vec<Option<u128>> = (0..inputs.len())
            .map(|i| dcl::result(i as u64).and_then(|b| dcl::parse_quote(&b)))
            .collect();
        let cap_bps = self.strategy.cap_bps;
        let unit = match side {
            Side::Buy => Some(rules::Unit::buy(tpn)),
            Side::Sell => dcl::result(inputs.len() as u64)
                .and_then(|b| dcl::parse_quote(&b))
                .map(|unit_out| rules::Unit {
                    unit_in: rules::sell_quote_input(tpn),
                    unit_out,
                }),
        };
        let picked = unit.and_then(|u| rules::pick_slice(&inputs, &quotes, u, cap_bps));
        let Some((input, quoted_out)) = picked else {
            self.phase = Phase::Idle;
            return PromiseOrValue::Value("rest: price impact over the slice cap".into());
        };
        let amount = candidates[inputs.iter().position(|a| *a == input).expect("a picked input")];
        let min_out = rules::min_out(quoted_out);
        let now = env::block_timestamp();
        self.last_slice_at = now;
        let coin = self.coin().clone();
        let swap = match side {
            Side::Buy => {
                let from_wnear = amount.min(self.wnear);
                self.wnear -= from_wnear;
                self.day.spent += amount;
                let deposit = amount - from_wnear;
                let wrap = Promise::new(self.peers.wrap.clone());
                let wrap = if deposit > 0 {
                    wrap.function_call(
                        "near_deposit",
                        b"{}".to_vec(),
                        NearToken::from_yoctonear(deposit),
                        GAS_DEPOSIT,
                    )
                } else {
                    wrap
                };
                // One receipt: if the swap call fails, the deposit is undone with it.
                wrap.function_call(
                    "ft_transfer_call",
                    dcl::swap_args(&self.peers.dcl, &coin.pool_id, amount, &coin.token, min_out, false),
                    ONE_YOCTO,
                    GAS_SWAP,
                )
            }
            Side::Sell => {
                self.hoard.wyrm -= amount;
                self.day.sold_wyrm += amount;
                Promise::new(coin.token.clone()).function_call(
                    "ft_transfer_call",
                    dcl::swap_args(&self.peers.dcl, &coin.pool_id, amount, &self.peers.wrap, min_out, true),
                    ONE_YOCTO,
                    GAS_SWAP,
                )
            }
        };
        self.phase = Phase::Swapping {
            step: step.0,
            since: now,
            side,
            amount_in: amount,
            quoted_out,
            min_out,
        };
        let ticket = Ticket {
            plan,
            at: U64(now),
            amount_in: U128(amount),
            quoted_out: U128(quoted_out),
            min_out: U128(min_out),
            cap_bps,
            day: U64(self.day.index),
        };
        let settle = Self::ext(env::current_account_id())
            .with_static_gas(GAS_ON_SWAP)
            .on_swap(step, ticket);
        PromiseOrValue::Promise(swap.then(settle))
    }

    /// Settles a swap from its ticket. The day's budget is always corrected; balances are restored only while the
    /// phase still belongs to this step, since after a replacement the next observation has already read them.
    #[private]
    pub fn on_swap(&mut self, #[serializer(borsh)] step: U64, #[serializer(borsh)] ticket: Ticket) -> String {
        let amount_in = ticket.amount_in.0;
        let used = dcl::result(0)
            .and_then(|b| dcl::parse_u128(&b))
            .unwrap_or(0)
            .min(amount_in);
        let unused = amount_in - used;
        let current = matches!(self.phase, Phase::Swapping { step: s, .. } if s == step.0);
        let same_day = self.day.index == ticket.day.0;
        let out = math::mul_div(ticket.quoted_out.0, used, amount_in.max(1));
        let kind = ticket.plan.kind;
        match kind.side() {
            Side::Buy => {
                if same_day {
                    self.day.spent -= unused.min(self.day.spent);
                }
                if current {
                    self.wnear += unused;
                }
                self.hoard.near_spent += used;
                self.unsettled.wyrm_out += rules::expected_net(out);
            }
            Side::Sell => {
                if same_day {
                    self.day.sold_wyrm -= unused.min(self.day.sold_wyrm);
                }
                if current {
                    self.hoard.wyrm += unused;
                }
                self.unsettled.wnear_out += out;
            }
        }
        if current {
            self.phase = Phase::Idle;
        }
        if used == 0 {
            self.record(Event::SwapFailed {
                at: ticket.at,
                slice: kind,
                amount_in: ticket.amount_in,
                reason: "refunded".into(),
            });
            return "swap_failed".into();
        }
        let plan = ticket.plan;
        let slice = Slice {
            at: ticket.at,
            breath_at: plan.breath_at,
            amount_in: U128(used),
            quoted_out: U128(out),
            min_out: ticket.min_out,
            tokens_per_near: plan.tokens_per_near,
            reference: plan.reference,
            cap_bps: ticket.cap_bps,
            budget: plan.budget,
        };
        self.record(match (kind, plan.dip) {
            (SliceKind::Dip, Some(dip)) => Event::DipSlice { slice, dip },
            (SliceKind::Hunt, _) => Event::HuntSlice(slice),
            _ => Event::BreathSlice(slice),
        });
        format!("{kind:?} slice: {used}")
    }

    /// The bound coin sent here with ft_transfer_call joins the hoard as a donation; wNEAR joins the reserve. Before
    /// bind only wNEAR is accepted. NEP-141's `{sender_id, amount, msg}` is read with the boundary's scanner, and the
    /// unused amount answered is always `"0"`.
    pub fn ft_on_transfer(&mut self) {
        let amount = env::input()
            .and_then(|b| dcl::object(&b).flatten()?.u128("amount"))
            .unwrap_or_else(|| env::panic_str("ft_on_transfer needs an amount"));
        let token = env::predecessor_account_id();
        if self.coin.as_ref().is_some_and(|c| c.token == token) {
            self.hoard.wyrm += amount;
            self.hoard.donated += amount;
        } else if token == self.peers.wrap {
            self.wnear += amount;
        } else {
            env::panic_str("the Dragon accepts only its bound coin and wNEAR");
        }
        env::value_return(br#""0""#);
    }

    /// The one mark of the real coin.
    pub fn get_coin(&self) -> Option<Coin> {
        self.coin.clone()
    }

    pub fn get_config(&self) -> ConfigView {
        let p = self.preset();
        ConfigView {
            preset: self.preset,
            preset_value: p,
            preset_hash: p.hash().into(),
            engine_version: ENGINE_VERSION,
            peers: self.peers.clone(),
            strategy: self.strategy,
            coin: self.coin.clone(),
            engine: EngineView {
                tax: envelope::TAX,
                spike_bps: SPIKE_BPS as u16,
                slippage_bps: SLIPPAGE_BPS as u16,
                dip_samples: DIP_SAMPLES as u8,
                dip_span_ns: U64(DIP_SPAN),
                outflow_bps: OUTFLOW_BPS as u16,
                outflow_window_bps: OUTFLOW_WINDOW_BPS as u16,
                outflow_window_ns: U64(OUTFLOW_WINDOW),
                stale_ns: U64(STALE),
                launch_margin: U128(nearly::LAUNCH_MARGIN.as_yoctonear()),
            },
        }
    }

    pub fn get_state(&self) -> StateView {
        let now = env::block_timestamp();
        let day = if self.day.index == now / DAY {
            self.day.clone()
        } else {
            Day::unobserved(now)
        };
        let near = self.near_free();
        let reserve = near + self.wnear;
        let newest = self.samples.newest().copied();
        let reference = self.reference(now);
        let recent = self.recent_samples();
        let preset = self.preset();
        let steps = &preset.ladder.steps_bps;
        let (run, confirmed) = match reference {
            Some(r) => (
                rules::dip_run(&recent, r, steps),
                rules::confirmed_dip(&recent, r, steps),
            ),
            None => (None, rules::Dip::default()),
        };
        let swapping = match self.phase {
            Phase::Swapping { side, .. } => Some(side),
            _ => None,
        };
        let inputs = Inputs {
            now,
            rules: Rules {
                strategy: &self.strategy,
                ladder: &preset.ladder,
                cadence: &preset.cadence,
            },
            day: &day,
            hoard: &self.hoard,
            reserve,
            last_slice_at: self.last_slice_at,
            market: newest.map(|s| Observation {
                tokens_per_near: s.tokens_per_near,
                reference,
                dip: confirmed.depth,
            }),
        };
        StateView {
            mood: rules::mood(&inputs, swapping, run.map_or(0, |r| r.depth)),
            phase: self.phase_view(),
            coin: self.coin.clone(),
            reserve: U128(reserve),
            reserve_near: U128(near),
            reserve_wnear: U128(self.wnear),
            spot: newest.map(|s| U128(s.tokens_per_near)),
            reference: reference.map(U128),
            hoard: HoardView::from(&self.hoard),
            day: DayView::from(&day),
            strategy: self.strategy,
            dip: run.map(|r| DipView {
                step: r.depth,
                step_bps: steps[r.depth as usize - 1],
                samples: r.confirmations,
                first_at: U64(r.since),
                confirmed: confirmed.depth >= r.depth,
            }),
            last_slice_at: U64(self.last_slice_at),
            last_launch_at: U64(self.last_launch_at),
            pending: self.pending.iter().map(PendingView::from).collect(),
            samples: self.samples.len(),
            log_next: U64(self.log.next_seq()),
        }
    }

    /// Entries from sequence number `from` (default: the oldest kept), oldest first, `limit` default 50, max 256:
    /// `[{"seq":"<n>","at":"<ns>","event":"<name>","data":{..}}]`. The stored JSON is returned as it is.
    pub fn get_log(&self, from: Option<U64>, limit: Option<u32>) {
        env::value_return(self.log_json(from, limit).as_bytes());
    }

    /// The newest `limit` samples (default 60, max 1,440), oldest first.
    pub fn get_samples(&self, limit: Option<u32>) -> Vec<SampleView> {
        let mut out: Vec<SampleView> = self
            .samples
            .newest_first(limit.unwrap_or(60).min(SAMPLES_CAP))
            .map(SampleView::from)
            .collect();
        out.reverse();
        out
    }
}

/// What DCL is asked to price for each candidate: a sell's net after the sell tax, since that is all the pool swaps.
fn quote_inputs(side: Side, candidates: &[u128]) -> Vec<u128> {
    match side {
        Side::Buy => candidates.to_vec(),
        Side::Sell => candidates.iter().map(|c| rules::sell_quote_input(*c)).collect(),
    }
}

impl Dragon {
    fn preset(&self) -> Preset {
        wyrm_types::preset(self.preset)
    }

    /// Callers run only once bound: `poke` refuses before, and the chain and outflows need it.
    fn coin(&self) -> &Coin {
        self.coin.as_ref().expect("not bound")
    }

    fn record(&mut self, event: Event) {
        let members = event.emit();
        if event.logged() {
            self.log.push(LogEntry {
                at: env::block_timestamp(),
                event: members,
            });
        }
    }

    fn log_json(&self, from: Option<U64>, limit: Option<u32>) -> String {
        let start = from.map_or(0, |f| f.0).max(self.log.first_seq());
        let limit = limit.unwrap_or(50).min(LOG_CAP) as u64;
        let entries: Vec<String> = (start..self.log.next_seq().min(start + limit))
            .filter_map(|seq| {
                let e = self.log.get(seq)?;
                Some(format!(r#"{{"seq":"{seq}","at":"{}",{}}}"#, e.at, e.event))
            })
            .collect();
        format!("[{}]", entries.join(","))
    }

    fn view_call(&self, to: &AccountId, method: &str, args: String, gas: Gas) -> Promise {
        Promise::new(to.clone()).function_call(method, args.into_bytes(), NearToken::from_yoctonear(0), gas)
    }

    fn quote(&self, side: Side, amount: u128) -> Promise {
        let coin = self.coin();
        let (input, output) = match side {
            Side::Buy => (&self.peers.wrap, &coin.token),
            Side::Sell => (&coin.token, &self.peers.wrap),
        };
        Promise::new(self.peers.dcl.clone()).function_call(
            "quote",
            dcl::quote_args(&coin.pool_id, input, output, amount),
            NearToken::from_yoctonear(0),
            GAS_QUOTE,
        )
    }

    fn reference(&self, now: u64) -> Option<u128> {
        rules::reference(self.samples.len(), |i| *self.samples.nth(i).expect("kept sample"), now)
    }

    fn recent_samples(&self) -> Vec<Sample> {
        self.samples.newest_first(DIP_SCAN).copied().collect()
    }

    /// NEAR above what storage needs now and what the rings and pending outflows may still grow into.
    fn near_free(&self) -> u128 {
        let headroom = self.samples.free_slots() as u64 * SAMPLE_SLOT_BYTES
            + self.log.free_slots() as u64 * LOG_SLOT_BYTES
            + (self.grants.free_slots() + self.burns.free_slots()) as u64 * SPENT_SLOT_BYTES
            + (MAX_PENDING - self.pending.len()) as u64 * PENDING_BYTES;
        let locked = (env::storage_usage() + headroom) as u128 * env::storage_byte_cost().as_yoctonear();
        env::account_balance().as_yoctonear().saturating_sub(locked)
    }

    fn phase_view(&self) -> PhaseView {
        match self.phase.clone() {
            Phase::Idle => PhaseView::Idle,
            Phase::Observing { step, since } => PhaseView::Observing {
                step: U64(step),
                since: U64(since),
            },
            Phase::Quoting {
                step,
                since,
                side,
                amount_in,
            } => PhaseView::Quoting {
                step: U64(step),
                since: U64(since),
                side,
                amount_in: U128(amount_in),
            },
            Phase::Swapping {
                step,
                since,
                side,
                amount_in,
                quoted_out,
                min_out,
            } => PhaseView::Swapping {
                step: U64(step),
                since: U64(since),
                side,
                amount_in: U128(amount_in),
                quoted_out: U128(quoted_out),
                min_out: U128(min_out),
            },
        }
    }
}

#[cfg(test)]
mod sim;
#[cfg(test)]
mod sim_tests;
#[cfg(test)]
mod tests;
