// Mock chain for ?mock=1. Serves fixtures in the raw contract JSON shapes, so the
// parsers in chain.js run exactly as they do against RPC, and applies the market,
// claim, sale and Council calls in memory so every flow can be clicked through.
// Refusals use the contracts' own panic messages.
import { config } from './config.js';
import { STORAGE_FEE, boundErrors, classOf, isSaleId, outflowWarning, tally } from './domain.js';

const load = (name) => fetch(new URL(`./fixtures/${name}.json`, import.meta.url)).then((r) => r.json());

// ?sold=N rewinds a closed sale to N sold, so the grid shows N+1 to 90 retired.
function applySold(world, sold) {
  if (sold === null || world.sale.state !== 'Closed') return;
  world.scales.tokens = world.scales.tokens.filter((t) => !isSaleId(t.token_id) || Number(t.token_id) <= sold);
  world.sale.sold = sold;
  world.sale.retired_from = sold < 90 ? sold + 1 : null;
}

function applyPreset(fixed, sale) {
  if (!config.mockPreset) return;
  const p = fixed.presets[config.mockPreset];
  for (const view of [fixed.council, fixed.scalesConfig, fixed.dragonConfig]) view.preset = config.mockPreset;
  fixed.council.season_ns = String(p.times.season);
  fixed.council.voting_ns = String(p.times.voting);
  fixed.scalesConfig.season_ns = String(p.times.season);
  fixed.dragonConfig.preset_value = { ...fixed.dragonConfig.preset_value, ...p, name: config.mockPreset };
  sale.price = p.sale.price;
}

export async function createMock() {
  const [world, fixed, accounts] = await Promise.all([load(`world-${config.mockSale}`), load('fixed'), load('accounts')]);
  applySold(world, config.mockSold);
  const { scales, market, council, dragon, sale } = world;
  applyPreset(fixed, sale);
  const NOW = BigInt(world.now);
  const C = config.contracts;
  const token = (id) => scales.tokens.find((t) => t.token_id === String(id));
  const page = (list, { from = 0, limit = 50 } = {}) => list.slice(from, from + limit);
  const proposal = (id) => council.proposals[id] ?? fail('no such proposal');
  const seasonNs = BigInt(fixed.council.season_ns);
  const bond = BigInt(fixed.council.bond) + BigInt(fixed.council.storage_fee);
  let nextOffer = Math.max(-1, ...market.offers.map((o) => o.id)) + 1;

  // Each world holds one Dragon state, keyed by the mood its log gives at `now` (generate.mjs).
  const [{ state, samples }] = Object.values(dragon.states);
  const views = {
    [`${C.dragon}.get_state`]: () => state,
    [`${C.dragon}.get_coin`]: () => dragon.coin,
    [`${C.dragon}.get_config`]: () => ({ ...fixed.dragonConfig, coin: dragon.coin, strategy: state.strategy }),
    [`${C.dragon}.get_samples`]: ({ limit = 60 } = {}) => samples.slice(-limit),
    [`${C.dragon}.get_log`]: ({ from, limit = 50 } = {}) => {
      if (from !== undefined && typeof from !== 'string') fail('Failed to deserialize input from JSON. Error: `invalid type: integer, expected a string`');
      const start = from === undefined ? 0 : Number(from);
      return dragon.log.filter((e) => Number(e.seq) >= start).slice(0, Math.min(limit, 256));
    },
    [`${C.scales}.get_config`]: () => ({ ...fixed.scalesConfig, sale }),
    [`${C.scales}.get_council`]: () => fixed.council,
    [`${C.scales}.get_scales`]: (args) => page(scales.tokens, { limit: 100, ...args }),
    [`${C.scales}.get_scale`]: ({ token_id }) => token(token_id) ?? null,
    [`${C.scales}.get_history`]: ({ token_id }) => scales.history[token_id] ?? [],
    [`${C.scales}.get_offers_for`]: ({ token_id }) => market.offers.filter((o) => o.token_id === token_id),
    [`${C.scales}.get_listings`]: (args) => page(market.listings, { limit: 100, ...args }),
    [`${C.scales}.get_offers`]: (args) => page(market.offers, args),
    [`${C.scales}.get_proposals`]: (args) => page(council.proposals, args),
    [`${C.scales}.get_proposal`]: ({ id }) => council.proposals[id] ?? null,
    [`${C.scales}.get_votes`]: ({ id }) => council.votes[id] ?? [],
    [`${C.scales}.get_sale`]: () => sale,
    [`${C.scales}.eligible_now`]: () => sale.sold,
  };

  function fail(msg) {
    throw new Error(msg);
  }

  function delist(t) {
    t.listed_price = null;
    market.listings = market.listings.filter((l) => l.token_id !== t.token_id);
  }

  function pushHistory(id, event) {
    const h = (scales.history[id] ??= []);
    h.push(event);
    if (h.length > 16) h.shift();
  }

  function transfer(t, to, price) {
    pushHistory(t.token_id, { Transferred: { from: t.owner, to, price: price.toString(), at: NOW.toString() } });
    t.owner = to;
    t.hold_start = NOW.toString();
    t.tier = 'Ash';
    delist(t);
  }

  function owned(signer, id) {
    const t = token(id) ?? fail('no such Scale');
    if (t.owner !== signer) fail('not the owner');
    return t;
  }

  // The Dragon's side of an order that passed: the outflow cap on the hoard, as admit runs it.
  function obey(kind) {
    const [[type, args]] = Object.entries(kind.Order);
    const hoardWyrm = BigInt(state.hoard.wyrm);
    const { outflow_bps: outflowBps, outflow_window_bps: windowBps } = fixed.dragonConfig.engine;
    const amount = args.amount === undefined ? undefined : BigInt(args.amount);
    const refused = outflowWarning(type, { amount }, { hoardWyrm, outflowBps, windowBps });
    return refused ? { LeashHeld: { error: `a ${type.toLowerCase()} is at most ${outflowBps} bps of the hoard (${hoardWyrm})` } } : 'Executed';
  }

  const calls = {
    list(signer, { token_id, price }) {
      const t = owned(signer, token_id);
      t.listed_price = price;
      market.listings = market.listings
        .filter((l) => l.token_id !== token_id)
        .concat({ token_id, price })
        .sort((a, b) => Number(a.token_id) - Number(b.token_id));
    },
    delist(signer, { token_id }) {
      delist(owned(signer, token_id));
    },
    buy(signer, { token_id }, deposit) {
      const l = market.listings.find((x) => x.token_id === token_id) ?? fail('not listed');
      if (BigInt(l.price) !== deposit) fail('attached deposit must equal the listed price');
      transfer(token(token_id), signer, deposit);
    },
    claim(signer, { token_id }) {
      const t = owned(signer, token_id);
      delist(t);
      const acc = accounts[t.vault] ?? fail('vault does not exist yet');
      const amount = BigInt(acc.amount) - config.vaultReserveYocto;
      if (amount <= 0n) fail('nothing to claim');
      acc.amount = config.vaultReserveYocto.toString();
      t.claimed_total = (BigInt(t.claimed_total) + amount).toString();
      pushHistory(token_id, { Claimed: { by: signer, amount: amount.toString(), at: NOW.toString() } });
    },
    make_offer(signer, { token_id = null }, deposit) {
      market.offers.push({ id: nextOffer++, buyer: signer, token_id, amount: (deposit - STORAGE_FEE).toString(), at: NOW.toString() });
    },
    cancel_offer(signer, { offer_id }) {
      const o = market.offers.find((x) => x.id === offer_id) ?? fail('no such offer');
      if (o.buyer !== signer) fail('not your offer');
      market.offers = market.offers.filter((x) => x !== o);
    },
    accept_offer(signer, { offer_id, token_id }) {
      const o = market.offers.find((x) => x.id === offer_id) ?? fail('no such offer');
      const t = owned(signer, token_id);
      if (o.token_id !== null && o.token_id !== token_id) fail('the offer is for another Scale');
      transfer(t, o.buyer, BigInt(o.amount));
      market.offers = market.offers.filter((x) => x !== o);
    },
    vote(signer, { proposal_id, token_ids, approve }) {
      const p = proposal(proposal_id);
      if (p.status !== 'Open' || NOW >= BigInt(p.ends_at)) fail('voting is closed');
      const list = (council.votes[proposal_id] ??= []);
      for (const id of token_ids) {
        const t = owned(signer, id);
        if (!t.voting || BigInt(t.hold_start) + seasonNs > BigInt(p.created_at)) fail(`Scale ${id} was not held a season before the proposal`);
        if (list.some((v) => v.token_id === id)) fail('the Scale already voted');
        list.push({ token_id: id, approve });
        approve ? p.yes++ : p.no++;
      }
      list.sort((a, b) => Number(a.token_id) - Number(b.token_id));
    },
    propose(signer, { kind, description, token_id }, deposit) {
      const t = owned(signer, token_id);
      if (!t.voting || BigInt(t.hold_start) + seasonNs > NOW) fail('the Scale is not seasoned');
      if (deposit < bond) fail('attach the bond and the storage fee');
      if (council.proposals.some((p) => p.proposer === signer && p.status === 'Open')) fail('you already have an open proposal');
      const parsed = kind === 'Signal' ? { type: 'Signal' } : (([[type, args]]) => ({ type, ...args }))(Object.entries(kind.Order));
      const cls = classOf(parsed);
      const rule = fixed.council.classes.find((c) => c.class === cls);
      if (sale.sold < rule.min_electorate) fail(`a ${cls} proposal needs ${rule.min_electorate} sold Scales`);
      if (rule.needs_closed_sale && sale.state !== 'Closed') fail('a Strict proposal needs the sale closed');
      const name = parsed.type === 'SetPosture' ? `SetPosture ${parsed.hunt ? 'on' : 'off'}` : parsed.type;
      const errors = parsed.type === 'Signal' ? [] : boundErrors(name, parsed.hunt ?? parsed, fixed.council.bounds);
      if (errors.length) fail(errors[0]);
      council.proposals.push({
        id: council.proposals.length, kind, class: cls, proposer: signer, description, bond: fixed.council.bond,
        created_at: NOW.toString(), ends_at: (NOW + BigInt(fixed.council.voting_ns)).toString(),
        electorate: sale.sold, yes: 0, no: 0, status: 'Open',
      });
    },
    finalize(_signer, { proposal_id }) {
      const p = proposal(proposal_id);
      if (p.status !== 'Open') fail('not open');
      if (NOW < BigInt(p.ends_at)) fail('voting still open');
      const min = fixed.council.classes.find((c) => c.class === p.class).min_electorate;
      const passing = tally(p, min).passing;
      p.status = !passing ? 'Rejected' : p.kind === 'Signal' ? 'Carried' : obey(p.kind);
      p.bond = '0';
    },
    buy_scale(signer, _args, deposit) {
      if (sale.state !== 'Open' || NOW >= BigInt(sale.ends_at)) fail('the sale is not open');
      if (sale.sold >= 90) fail('sold out');
      if (deposit < BigInt(sale.price)) fail('attached deposit is below the price');
      sale.sold++;
      const id = String(sale.sold);
      scales.tokens.push({
        token_id: id, owner: signer, voting: true, hold_start: NOW.toString(), tier: 'Ash', claimed_total: '0',
        claiming_since: NOW.toString(), listed_price: null, vault: config.vaultId(id),
      });
      scales.tokens.sort((a, b) => Number(a.token_id) - Number(b.token_id));
      scales.history[id] = [{ Minted: { to: signer, price: sale.price, at: NOW.toString() } }];
    },
  };

  return {
    now: () => NOW,
    you: world.you,
    view(contract, method, args) {
      const fn = views[`${contract}.${method}`];
      if (!fn) throw new Error(`mock has no view ${contract}.${method}`);
      return structuredClone(fn(args));
    },
    account: (id) => (accounts[id] ? { amount: accounts[id].amount, code_hash: accounts[id].code_hash, global_contract_hash: accounts[id].global_contract_hash } : null),
    keys: (id) => accounts[id]?.keys ?? [],
    call(signer, receiver, method, args, deposit) {
      if (!signer) throw new Error('Connect a wallet first');
      if (receiver !== C.scales || !calls[method]) throw new Error(`mock has no call ${receiver}.${method}`);
      calls[method](signer, args, deposit);
    },
  };
}
