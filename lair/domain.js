import { NS_PER_DAY, formatBps, formatNear } from './format.js';

export const SCALE_COUNT = 100;
export const SALE_IDS = 90;
export const ROYALTY_BPS = 500n;
export const MIN_PRICE = 10n ** 23n;
export const MAX_OPEN_OFFERS = 10;
export const MAX_VOTE_TOKENS = 25;
export const STORAGE_FEE = 10n ** 22n;
export const DESCRIPTION_MAX_BYTES = 1000;
export const CLAIM_LOCK_NS = 10n * 60_000_000_000n;

export const TIERS = [
  { name: 'Ash', fromDays: 0 },
  { name: 'Ember', fromDays: 3 },
  { name: 'Molten', fromDays: 7 },
  { name: 'Obsidian', fromDays: 14 },
];

export function tierOf(holdStartNs, nowNs) {
  const held = nowNs - holdStartNs;
  let tier = TIERS[0];
  for (const t of TIERS) if (held >= BigInt(t.fromDays) * NS_PER_DAY) tier = t;
  return tier.name;
}

export function nextTier(holdStartNs, nowNs) {
  const held = nowNs - holdStartNs;
  const next = TIERS.find((t) => held < BigInt(t.fromDays) * NS_PER_DAY);
  return next ? { name: next.name, at: holdStartNs + BigInt(next.fromDays) * NS_PER_DAY } : null;
}

export function royaltySplit(price) {
  const royalty = (price * ROYALTY_BPS) / 10_000n;
  return { royalty, seller: price - royalty };
}

// scales resolves RoyaltyTo into one account: the creator, or the tithe for Holders.
export function royaltyTo(receiver, { creator, tithe }) {
  if (receiver === creator) return 'creator';
  if (receiver === tithe) return 'holders';
  return 'other';
}

// The creator's cut rounds down, as scales' `split`, and the Dragon gets the rest.
export function saleSplit(price, creatorBps) {
  const creator = (price * BigInt(creatorBps)) / 10_000n;
  return { creator, dragon: price - creator };
}

export const isSaleId = (id) => Number(id) >= 1 && Number(id) <= SALE_IDS;
export const isCreatorId = (id) => Number(id) > SALE_IDS && Number(id) <= SCALE_COUNT;

export const STATUSES = {
  yours: 'Yours',
  listed: 'For sale',
  held: 'Held',
  creator: "Creator's",
  unsold: 'Unsold',
  retired: 'Retired',
};

// Retirement mirrors scales: an unminted sale id once the sale is Closed.
export function statusOf(id, scale, { sale, account, creator }) {
  if (!scale) return { kind: sale.state === 'Closed' && isSaleId(id) ? 'retired' : 'unsold', price: null };
  const price = scale.listedPrice;
  if (account && scale.owner === account) return { kind: 'yours', price };
  if (price !== null) return { kind: 'listed', price };
  return { kind: isCreatorId(id) && scale.owner === creator ? 'creator' : 'held', price: null };
}

export function statusText({ kind, price }) {
  if (price === null) return STATUSES[kind];
  return kind === 'yours' ? `Yours, for sale at ${formatNear(price)} NEAR` : `For sale at ${formatNear(price)} NEAR`;
}

export function statusCounts(statuses) {
  const counts = Object.fromEntries(Object.keys(STATUSES).map((k) => [k, 0]));
  for (const s of statuses) counts[s.kind]++;
  return counts;
}

export const MOODS = {
  Sleeping: { line: 'Asleep on the hoard.' },
  Breathing: { line: 'It bought WYRM in the last half hour.' },
  Hunting: { line: 'It sold WYRM from the hoard in the last half hour, as the Council allows.' },
};

export const BUY_EVENTS = new Set(['breath_slice', 'dip_slice']);

const RECENT_NS = 30n * 60_000_000_000n;

// The mood shown comes from the log, never from the contract's live mood: that one turns Breathing when the day's
// buy window opens and Hunting when the price passes the sell line, before anything happens. A swap in the last half
// hour is already public in the log. `log` is newest first, as getDragon returns it.
export function moodOf(log, nowNs) {
  const swap = log.find((e) => BUY_EVENTS.has(e.event) || e.event === 'hunt_slice');
  if (!swap || nowNs - swap.at >= RECENT_NS) return 'Sleeping';
  return swap.event === 'hunt_slice' ? 'Hunting' : 'Breathing';
}

// NEAR per million WYRM, from a sample's WYRM base units per NEAR.
export function pricePerMillion(tokensPerNear, decimals = 18) {
  return (1e6 * 10 ** decimals) / Number(tokensPerNear);
}

// wyrm-types VoteClass::passes, gated by min_electorate first as scales does.
export const CLASSES = {
  Light: { turnout: [3, 10], text: '30% turnout and more yes than no' },
  Strict: { turnout: [2, 3], text: 'two thirds turnout and 80% yes' },
};

export function classOf(kind) {
  return kind.type === 'Grant' || (kind.type === 'SetPosture' && kind.hunt) ? 'Strict' : 'Light';
}

export function tally({ class: cls, electorate, yes, no }, minElectorate) {
  const [num, den] = CLASSES[cls].turnout;
  const votes = yes + no;
  const turnoutNeeded = Math.ceil((electorate * num) / den);
  const turnoutOk = votes * den >= electorate * num;
  const enough = electorate >= minElectorate;
  const share = cls === 'Strict' ? yes * 5 >= votes * 4 && votes > 0 : yes > no;
  const checks = [
    { ok: turnoutOk, text: `${votes} of ${turnoutNeeded} votes for turnout` },
    cls === 'Strict'
      ? { ok: share, text: `${votes ? Math.floor((yes * 100) / votes) : 0}% yes, 80% needed` }
      : { ok: share, text: share ? 'Yes leads' : 'Yes must lead no' },
  ];
  if (!enough) checks.push({ ok: false, text: `Needs ${minElectorate} Scales that can vote, has ${electorate}` });
  return {
    passing: enough && turnoutOk && share,
    marks: [{ at: turnoutNeeded, label: `${turnoutNeeded} votes for turnout` }],
    checks,
  };
}

export function bondReturns({ electorate, yes, no }) {
  return (yes + no) * 10 >= electorate * 3;
}

export function isClaiming(scale, nowNs) {
  return scale.claimingSince !== null && nowNs - scale.claimingSince < CLAIM_LOCK_NS;
}

export function canVoteOn(scale, proposal, seasonNs) {
  return scale.voting && scale.holdStart + seasonNs <= proposal.createdAt;
}

export function canPropose(scale, nowNs, seasonNs) {
  return scale.voting && scale.holdStart + seasonNs <= nowNs;
}

export function firstVoteAt(scales, seasonNs) {
  let first = null;
  for (const s of scales) if (s.voting && (first === null || s.holdStart < first)) first = s.holdStart;
  return first === null ? null : first + seasonNs;
}

export function votableScales(myScales, proposal, votedIds, seasonNs) {
  return myScales.filter((s) => canVoteOn(s, proposal, seasonNs) && !votedIds.has(s.id));
}

export function chunk(list, size) {
  const out = [];
  for (let i = 0; i < list.length; i += size) out.push(list.slice(i, i + size));
  return out;
}

// The proposal kinds, keyed by the names get_council() lists. `bound` names the StrategyBounds field that limits
// an input; `order` builds the ProposalKind JSON from the parsed form values.
export const KINDS = {
  SetBreath: {
    label: 'Set the breath',
    summary: "The daily budget, as a share of the day's opening reserve.",
    fields: [{ key: 'budget_bps', label: 'Daily budget', unit: '% of reserve', type: 'bps', bound: 'budget_bps' }],
    order: (a) => ({ Order: { SetBreath: { budget_bps: a.budget_bps } } }),
  },
  SetSlices: {
    label: 'Set the slice cap',
    summary: 'How far one slice may move the price.',
    fields: [{ key: 'cap_bps', label: 'Price impact cap per slice', unit: '% price move', type: 'bps', bound: 'cap_bps' }],
    order: (a) => ({ Order: { SetSlices: { cap_bps: a.cap_bps } } }),
  },
  'SetPosture on': {
    label: 'Hunt on',
    summary: 'Sell WYRM while the price is above a multiple of average cost, up to a daily limit.',
    fields: [
      { key: 'sell_multiple_bps', label: 'Sell line', unit: '% of average cost', type: 'bps', bound: 'sell_multiple_min_bps' },
      { key: 'max_sell_bps', label: 'Most sold per day', unit: '% of hoard', type: 'bps', bound: 'max_sell_bps' },
    ],
    order: (a) => ({ Order: { SetPosture: { hunt: { sell_multiple_bps: a.sell_multiple_bps, max_sell_bps: a.max_sell_bps } } } }),
  },
  'SetPosture off': {
    label: 'Hunt off',
    summary: 'Guard the hoard and never sell.',
    fields: [],
    order: () => ({ Order: { SetPosture: { hunt: null } } }),
  },
  Grant: {
    label: 'Grant WYRM',
    summary: 'Send WYRM from the hoard to an account.',
    fields: [
      { key: 'to', label: 'Recipient account', type: 'account' },
      { key: 'amount', label: 'Amount', unit: 'WYRM', type: 'wyrm' },
    ],
    order: (a) => ({ Order: { Grant: { to: a.to, amount: a.amount.toString() } } }),
  },
  Burn: {
    label: 'Burn WYRM',
    summary: 'Send WYRM from the hoard to ash, out of supply for good.',
    fields: [{ key: 'amount', label: 'Amount', unit: 'WYRM', type: 'wyrm' }],
    order: (a) => ({ Order: { Burn: { amount: a.amount.toString() } } }),
  },
  SetAutoBurn: {
    label: 'Set the auto-burn',
    summary: 'Burn a share of the WYRM every buy brings in.',
    fields: [{ key: 'bps', label: 'Share of each buy burned', unit: '%', type: 'bps', bound: 'auto_burn_bps' }],
    order: (a) => ({ Order: { SetAutoBurn: { bps: a.bps } } }),
  },
  Signal: {
    label: 'Signal',
    summary: 'A text poll. It calls nothing and ends Carried or Rejected.',
    fields: [],
    order: () => 'Signal',
  },
};

const MAX_U32 = 4_294_967_295;

// The sell line has only a floor.
export function boundOf(field, bounds) {
  const b = bounds[field.bound];
  return typeof b === 'number' ? { lo: b, hi: MAX_U32 } : b;
}

// Values outside the bounds make scales refuse the proposal. The outflow cap does not: scales accepts it and the
// Dragon refuses it at finalize, so the form warns and still lets it through.
export function boundErrors(kind, args, bounds) {
  return KINDS[kind].fields
    .filter((f) => f.bound)
    .map((f) => {
      const { lo, hi } = boundOf(f, bounds);
      return args[f.key] < lo || args[f.key] > hi ? `${f.label} must be ${formatBps(lo)}${hi < MAX_U32 ? ` to ${formatBps(hi)}` : ' or more'}.` : null;
    })
    .filter(Boolean);
}

// The Dragon measures each Grant and Burn on the hoard it holds when the order arrives (outflow_bps of it), and
// all of them in 30 days against outflow_window_bps.
export function outflowWarning(kind, args, { hoardWyrm, outflowBps, windowBps }) {
  if (kind !== 'Grant' && kind !== 'Burn') return null;
  if (args.amount === 0n) return 'The amount must be more than 0.';
  if (args.amount * 10_000n > hoardWyrm * BigInt(outflowBps))
    return `A ${kind} moves at most ${outflowBps / 100}% of the hoard, and ${windowBps / 100}% in 30 days.`;
  return null;
}

// WYRM's tithe rate as a share of volume: the creator's 0.48% of buy volume plus 60% of the token tax on every side.
export function titheRate(buyShare, taxBps) {
  return 0.0048 * buyShare + (taxBps / 10_000) * 0.6;
}

// Half the tithe goes to the 100 Scales in equal parts.
export function perScalePerDay(dailyVolumeNear, rate) {
  return (dailyVolumeNear * rate * 0.5) / 100;
}
