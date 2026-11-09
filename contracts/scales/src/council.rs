use crate::*;
use near_sdk::{Gas, PromiseOrValue};
use wyrm_types::{bond_returns, HuntLine, Order, StrategyBounds, Verdict, VoteClass};

pub const BOND: NearToken = NearToken::from_near(1);
pub const MAX_DESCRIPTION: usize = 1_000;
pub const MAX_VOTE_TOKENS: usize = 25;
/// `finalize` refuses less prepaid gas, so nobody can starve a passed order of the gas `obey` needs.
pub const FINALIZE_GAS: Gas = Gas::from_tgas(150);
pub const OBEY_GAS: Gas = Gas::from_tgas(100);
const ON_EXECUTED_GAS: Gas = Gas::from_tgas(10);
const MAX_VERDICT_BYTES: usize = 1_024;

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum ProposalKind {
    Order(Order),
    /// A text poll. It can only end Carried or Rejected, and it calls nothing.
    Signal,
}

impl ProposalKind {
    pub fn class(&self) -> VoteClass {
        match self {
            ProposalKind::Order(order) => order.class(),
            ProposalKind::Signal => VoteClass::Light,
        }
    }
}

// Open -> Rejected | Carried (a Signal that passed) | Executing (an Order that passed), all in finalize.
// Executing -> Executed | LeashHeld (callback). Every status but Open and Executing is terminal.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Open,
    Rejected,
    Carried,
    Executing,
    Executed,
    LeashHeld { error: String },
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct Proposal {
    pub id: u64,
    pub kind: ProposalKind,
    /// Fixed at creation from the kind.
    pub class: VoteClass,
    pub proposer: AccountId,
    pub description: String,
    /// Zeroed when finalize sends it.
    pub bond: NearToken,
    pub created_at: u64,
    pub ends_at: u64,
    /// Sale Scales sold at creation. Unseasoned and silent Scales count as no-shows.
    pub electorate: u32,
    pub yes: ScaleSet,
    pub no: ScaleSet,
    pub status: Status,
}

impl Proposal {
    /// The minimum electorate gates first, because `VoteClass::passes` alone passes any electorate below it.
    pub(crate) fn passes(&self) -> bool {
        self.electorate >= self.class.min_electorate()
            && self.class.passes(self.yes.count(), self.no.count(), self.electorate)
    }
}

#[near(serializers = [json])]
pub struct ProposalView {
    pub id: u64,
    pub kind: ProposalKind,
    pub class: VoteClass,
    pub proposer: AccountId,
    pub description: String,
    pub bond: NearToken,
    pub created_at: U64,
    pub ends_at: U64,
    pub electorate: u32,
    pub yes: u32,
    pub no: u32,
    pub status: Status,
}

impl From<&Proposal> for ProposalView {
    fn from(p: &Proposal) -> Self {
        ProposalView {
            id: p.id,
            kind: p.kind.clone(),
            class: p.class,
            proposer: p.proposer.clone(),
            description: p.description.clone(),
            bond: p.bond,
            created_at: U64(p.created_at),
            ends_at: U64(p.ends_at),
            electorate: p.electorate,
            yes: p.yes.count(),
            no: p.no.count(),
            status: p.status.clone(),
        }
    }
}

#[near(serializers = [json])]
pub struct KindView {
    pub kind: String,
    pub class: VoteClass,
}

#[near(serializers = [json])]
pub struct ClassView {
    pub class: VoteClass,
    pub min_electorate: u32,
    pub needs_closed_sale: bool,
    pub rule: String,
}

#[near(serializers = [json])]
pub struct CouncilView {
    pub preset: PresetName,
    pub kinds: Vec<KindView>,
    pub classes: Vec<ClassView>,
    pub bounds: StrategyBounds,
    pub season_ns: U64,
    pub voting_ns: U64,
    pub bond: NearToken,
    pub storage_fee: NearToken,
    pub finalize_tgas: u64,
}

#[near_sdk::ext_contract(ext_dragon)]
#[allow(dead_code)]
trait Dragon {
    fn obey(&mut self, order: Order) -> Verdict;
}

#[near(serializers = [json])]
pub struct VoteView {
    pub token_id: TokenId,
    pub approve: bool,
}

#[near]
impl Scales {
    /// Opens a proposal backed by one of the caller's seasoned sale Scales. Takes `BOND` into escrow and keeps
    /// `STORAGE_FEE`.
    #[payable]
    pub fn propose(&mut self, kind: ProposalKind, description: String, token_id: TokenId) -> u64 {
        let proposer = env::predecessor_account_id();
        let now = env::block_timestamp();
        let paid = env::attached_deposit();
        let due = BOND.saturating_add(STORAGE_FEE);
        require!(paid >= due, "attach the 1 NEAR bond plus the storage fee");
        require!(description.len() <= MAX_DESCRIPTION, "description is over 1000 bytes");
        let preset = self.preset();
        let stake = ScaleId::expect(&token_id);
        require!(
            self.owned_by(stake, &proposer)
                .seasoned_at(stake, now, preset.times.season),
            "the Scale is not eligible to vote"
        );
        require!(
            !self.open_proposal.contains_key(&proposer),
            "you already have an open proposal"
        );
        if let ProposalKind::Order(order) = &kind {
            check(order.check(&preset));
        }
        let class = kind.class();
        let electorate = self.electorate();
        require!(
            electorate >= class.min_electorate(),
            "the electorate is too small for this proposal"
        );
        if class.needs_closed_sale() {
            require!(self.sale.over(now), "strict proposals open after the sale closes");
        }
        let id = self.proposals.len() as u64;
        let proposal = Proposal {
            id,
            kind,
            class,
            proposer: proposer.clone(),
            description,
            bond: BOND,
            created_at: now,
            ends_at: now + preset.times.voting,
            electorate,
            yes: ScaleSet::default(),
            no: ScaleSet::default(),
            status: Status::Open,
        };
        emit("proposal_created", ProposalView::from(&proposal));
        self.proposals.push(proposal);
        self.open_proposal.insert(proposer.clone(), id);
        self.escrow = self.escrow.saturating_add(BOND);
        let excess = paid.saturating_sub(due);
        if !excess.is_zero() {
            Promise::new(proposer).transfer(excess).detach();
        }
        self.assert_storage_paid();
        id
    }

    /// Counts the caller's Scales on an Open proposal. Each Scale counts once, whoever owns it later.
    #[payable]
    pub fn vote(&mut self, proposal_id: u64, token_ids: Vec<TokenId>, approve: bool) {
        near_sdk::assert_one_yocto();
        require!(
            !token_ids.is_empty() && token_ids.len() <= MAX_VOTE_TOKENS,
            "vote with 1 to 25 Scales"
        );
        let voter = env::predecessor_account_id();
        let season = self.preset().times.season;
        let mut p = self.proposal(proposal_id);
        require!(
            p.status == Status::Open && env::block_timestamp() < p.ends_at,
            "voting is closed"
        );
        for token_id in &token_ids {
            let id = ScaleId::expect(token_id);
            require!(
                self.owned_by(id, &voter).seasoned_at(id, p.created_at, season),
                "the Scale is not eligible on this proposal"
            );
            require!(!p.yes.union(p.no).contains(id), "the Scale already voted");
            if approve {
                p.yes.insert(id);
            } else {
                p.no.insert(id);
            }
        }
        self.proposals.set(proposal_id as u32, p);
        emit(
            "vote_cast",
            fields! { proposal_id: proposal_id, voter: &voter, token_ids: &token_ids, approve: approve },
        );
    }

    /// Closes an Open proposal after `ends_at` and sends the bond: back to the proposer when turnout reached 30%,
    /// otherwise to the Dragon. A passed Order reaches the Dragon in this transaction, so nothing can be banked.
    pub fn finalize(&mut self, proposal_id: u64) -> PromiseOrValue<Status> {
        require!(
            env::prepaid_gas() >= FINALIZE_GAS,
            "attach at least 150 Tgas to finalize"
        );
        let mut p = self.proposal(proposal_id);
        require!(p.status == Status::Open, "not open");
        require!(env::block_timestamp() >= p.ends_at, "voting still open");
        p.status = match (p.passes(), &p.kind) {
            (false, _) => Status::Rejected,
            (true, ProposalKind::Signal) => Status::Carried,
            (true, ProposalKind::Order(_)) => Status::Executing,
        };
        let bond = std::mem::replace(&mut p.bond, NearToken::from_yoctonear(0));
        self.escrow = self.escrow.saturating_sub(bond);
        self.open_proposal.remove(&p.proposer);
        let (yes, no) = (p.yes.count(), p.no.count());
        let bond_to = if bond_returns(yes, no, p.electorate) {
            p.proposer.clone()
        } else {
            self.dragon.clone()
        };
        Promise::new(bond_to.clone()).transfer(bond).detach();
        emit(
            "proposal_finalized",
            fields! { proposal_id: proposal_id, status: &p.status, yes: yes, no: no, electorate: p.electorate,
            bond: bond, bond_to: &bond_to },
        );
        let status = p.status.clone();
        let order = match &p.kind {
            ProposalKind::Order(order) if status == Status::Executing => Some(order.clone()),
            _ => None,
        };
        self.proposals.set(proposal_id as u32, p);
        let Some(order) = order else {
            return PromiseOrValue::Value(status);
        };
        ext_dragon::ext(self.dragon.clone())
            .with_static_gas(OBEY_GAS)
            .obey(order)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_EXECUTED_GAS)
                    .on_executed(proposal_id),
            )
            .into()
    }

    /// Records the Dragon's verdict. A call that failed or answered no verdict holds the leash too, so every Executing
    /// proposal ends.
    #[private]
    pub fn on_executed(&mut self, proposal_id: u64) -> Status {
        let mut p = self.proposal(proposal_id);
        let verdict = env::promise_result_checked(0, MAX_VERDICT_BYTES)
            .ok()
            .and_then(|v| decode_verdict(&v));
        p.status = match verdict {
            Some(Verdict::Executed) => Status::Executed,
            Some(Verdict::Refused { reason }) => Status::LeashHeld { error: reason },
            None => Status::LeashHeld {
                error: "dragon call failed".into(),
            },
        };
        emit(
            "proposal_executed",
            fields! { proposal_id: proposal_id, status: &p.status },
        );
        let status = p.status.clone();
        self.proposals.set(proposal_id as u32, p);
        status
    }

    /// What the Lair renders: the preset's enabled kinds with their classes, the class rules, the bounds and the
    /// timings.
    pub fn get_council(&self) -> CouncilView {
        let preset = self.preset();
        let b = preset.bounds;
        let hunt_on = HuntLine {
            sell_multiple_bps: b.sell_multiple_min_bps,
            max_sell_bps: b.max_sell_bps.lo,
        };
        // Each sample sits on its bound, so `check` refuses one only when its feature is disabled.
        let samples = [
            (
                "SetBreath",
                Order::SetBreath {
                    budget_bps: b.budget_bps.lo,
                },
            ),
            ("SetSlices", Order::SetSlices { cap_bps: b.cap_bps.lo }),
            ("SetPosture on", Order::SetPosture { hunt: Some(hunt_on) }),
            ("SetPosture off", Order::SetPosture { hunt: None }),
            (
                "Grant",
                Order::Grant {
                    to: self.dragon.clone(),
                    amount: U128(1),
                },
            ),
            ("Burn", Order::Burn { amount: U128(1) }),
            (
                "SetAutoBurn",
                Order::SetAutoBurn {
                    bps: b.auto_burn_bps.lo,
                },
            ),
        ];
        let mut kinds = Vec::new();
        for (kind, order) in samples {
            if order.check(&preset).is_ok() {
                kinds.push(KindView {
                    kind: kind.into(),
                    class: order.class(),
                });
            }
        }
        kinds.push(KindView {
            kind: "Signal".into(),
            class: ProposalKind::Signal.class(),
        });
        let rules = [
            (VoteClass::Light, "(yes + no) * 10 >= electorate * 3 and yes > no"),
            (
                VoteClass::Strict,
                "(yes + no) * 3 >= electorate * 2 and yes * 5 >= (yes + no) * 4",
            ),
        ];
        let classes = rules
            .map(|(class, rule)| ClassView {
                class,
                min_electorate: class.min_electorate(),
                needs_closed_sale: class.needs_closed_sale(),
                rule: rule.into(),
            })
            .into();
        CouncilView {
            preset: self.preset,
            kinds,
            classes,
            bounds: b,
            season_ns: U64(preset.times.season),
            voting_ns: U64(preset.times.voting),
            bond: BOND,
            storage_fee: STORAGE_FEE,
            finalize_tgas: FINALIZE_GAS.as_tgas(),
        }
    }

    /// Open proposals whose vote has ended, which anyone may finalize.
    pub fn get_due(&self) -> Vec<u64> {
        let now = env::block_timestamp();
        self.proposals
            .iter()
            .filter(|p| p.status == Status::Open && now >= p.ends_at)
            .map(|p| p.id)
            .collect()
    }

    pub fn get_proposal(&self, id: u64) -> Option<ProposalView> {
        u32::try_from(id)
            .ok()
            .and_then(|i| self.proposals.get(i))
            .map(ProposalView::from)
    }

    pub fn get_proposals(&self, from: Option<u32>, limit: Option<u32>) -> Vec<ProposalView> {
        self.proposals
            .iter()
            .skip(from.unwrap_or(0) as usize)
            .take(limit.unwrap_or(50) as usize)
            .map(ProposalView::from)
            .collect()
    }

    pub fn get_votes(&self, id: u64) -> Vec<VoteView> {
        let Some(p) = u32::try_from(id).ok().and_then(|i| self.proposals.get(i)) else {
            return vec![];
        };
        p.yes
            .union(p.no)
            .iter()
            .map(|t| VoteView {
                token_id: t.token_id(),
                approve: p.yes.contains(t),
            })
            .collect()
    }
}

/// `Verdict`'s two JSON forms, decoded without serde's enum machinery, which costs about 3 KB of wasm next to a
/// 405 KB limit. `a_verdict_decodes_from_its_own_json` pins both forms to wyrm-types' serializer.
pub(crate) fn decode_verdict(json: &[u8]) -> Option<Verdict> {
    if json == br#""Executed""# {
        return Some(Verdict::Executed);
    }
    let reason = json.strip_prefix(br#"{"Refused":{"reason":"#)?.strip_suffix(b"}}")?;
    serde_json::from_slice(reason)
        .ok()
        .map(|reason| Verdict::Refused { reason })
}

impl Scales {
    fn proposal(&self, id: u64) -> Proposal {
        u32::try_from(id)
            .ok()
            .and_then(|i| self.proposals.get(i))
            .cloned()
            .unwrap_or_else(|| env::panic_str("no such proposal"))
    }
}
