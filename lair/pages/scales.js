import { getScales, getUnclaimed, getOffers, getSale, getSystem, tx } from '../chain.js';
import { SCALE_COUNT, SALE_IDS, CLAIM_LOCK_NS, STATUSES, isClaiming, saleSplit, statusOf, statusCounts, statusText, royaltyTo, tierOf } from '../domain.js';
import { formatNear, formatBps, formatIn, padScaleId } from '../format.js';
import { scaleSvg } from '../art.js';
import { html, raw, ask } from '../ui.js';
import { listDialog, acceptDialog } from './offer-dialogs.js';
import * as earnings from './earnings.js';
import * as market from './market.js';

export const title = () => 'Scales';

export async function load() {
  const [scales, offers, sale, system, estimate] = await Promise.all([getScales(), getOffers(), getSale(), getSystem(), earnings.load()]);
  const unclaimed = await getUnclaimed(scales);
  return { scales, unclaimed, offers, sale, byId: new Map(scales.map((s) => [s.id, s])), creator: system.creator, royaltyTo: royaltyTo(system.royaltyReceiver, system), estimate };
}

const IDS = Array.from({ length: SCALE_COUNT }, (_, i) => String(i + 1));

function statuses(d, account) {
  return new Map(IDS.map((id) => [id, statusOf(id, d.byId.get(id) ?? null, { sale: d.sale, account, creator: d.creator })]));
}

export function tabOf(param, account, sale) {
  if (param && (param !== 'yours' || account)) return param;
  return sale.state === 'Closed' ? 'market' : 'sale';
}

function salePanel(d, now) {
  const { sale } = d;
  const price = formatNear(sale.price);
  if (sale.state === 'Waiting')
    return html`<div class="sale">
      <p class="sale-state">The sale opens when the coin is bound.</p>
      <dl class="figures"><div><dt>Scales</dt><dd>${SALE_IDS}</dd></div><div><dt>Price</dt><dd>${price}<small>NEAR</small></dd></div></dl>
    </div>`;
  if (sale.state === 'Open')
    return html`<div class="sale">
      <dl class="figures">
        <div><dt>Left</dt><dd>${SALE_IDS - sale.sold}<small>of ${SALE_IDS}</small></dd></div>
        <div><dt>Price</dt><dd>${price}<small>NEAR</small></dd></div>
        <div><dt>Ends</dt><dd data-countdown>${formatIn(sale.endsAt, now)}</dd></div>
      </dl>
      <button class="btn btn-primary btn-big" data-act="mint">Buy a Scale for ${price} NEAR</button>
    </div>`;
  const retired = SALE_IDS - sale.sold;
  return html`<div class="sale">
    <p class="sale-state">The sale closed with ${sale.sold} of ${SALE_IDS} sold.</p>
    ${retired ? html`<p class="hint">Scales ${padScaleId(sale.sold + 1)} to ${padScaleId(SALE_IDS)} are retired. Nobody bought them, and their share of every fee goes to the Dragon.</p>` : ''}
    <a class="btn btn-primary btn-big" href="#/scales/market">See Scales for sale</a>
  </div>`;
}

function paid(d) {
  let total = 0n;
  for (const s of d.scales) total += s.claimedTotal + (d.unclaimed.get(s.id) ?? 0n);
  return total;
}

function cell(id, st) {
  const row = Math.floor((Number(id) - 1) / 10);
  const name = `Scale ${padScaleId(id)}, ${statusText(st).replace(/^\w/, (c) => c.toLowerCase())}`;
  const inner = raw(scaleSvg(st.kind, padScaleId(id), st.price !== null));
  const minted = st.kind !== 'unsold' && st.kind !== 'retired';
  return html`<li class="cell" data-status="${st.kind}" style="--row:${row}">${minted
    ? html`<a href="#/scales/${id}" aria-label="${name}" title="${name}">${inner}</a>`
    : html`<span title="${name}">${inner}<span class="sr">${name}</span></span>`}</li>`;
}

function allPanel(map) {
  const counts = statusCounts([...map.values()]);
  return html`<div class="chips" role="group" aria-label="Show Scales">
      <button class="chip" aria-pressed="true" data-filter="all">All ${SCALE_COUNT}</button>
      ${Object.entries(STATUSES).filter(([k]) => counts[k]).map(([k, label]) => html`<button class="chip" aria-pressed="false" data-filter="${k}" data-status="${k}"><span class="swatch" aria-hidden="true"></span>${label} <span class="count">${counts[k]}</span></button>`)}
    </div>
    <ol class="hide" id="hide">${IDS.map((id) => cell(id, map.get(id)))}</ol>`;
}

function claimable(mine, d, now) {
  return mine.filter((s) => (d.unclaimed.get(s.id) ?? 0n) > 0n && !isClaiming(s, now));
}

function yourRow(s, st, d, now) {
  const waiting = d.unclaimed.get(s.id) ?? 0n;
  const locked = isClaiming(s, now);
  const id = padScaleId(s.id);
  return html`<li class="row">
    <a class="row-id" href="#/scales/${s.id}">${raw(scaleSvg(st.kind, '', st.price !== null))}<span><strong>Scale ${id}</strong><small>${st.price === null ? 'Not listed' : `Listed at ${formatNear(st.price)} NEAR`}, ${tierOf(s.holdStart, now)}</small></span></a>
    <span class="row-figure"><strong>${formatNear(waiting)}</strong><small>NEAR to claim</small></span>
    <span class="row-acts">
      <button class="btn btn-small" data-act="claim" data-token="${s.id}" ${locked || !waiting ? 'disabled' : ''} aria-label="Claim ${formatNear(waiting)} NEAR from Scale ${id}">${locked ? 'Locked' : 'Claim'}</button>
      ${st.price === null
        ? html`<button class="btn btn-small btn-quiet" data-act="list" data-token="${s.id}" aria-label="List Scale ${id} for sale">List</button>`
        : html`<button class="btn btn-small btn-quiet" data-act="list" data-token="${s.id}" aria-label="Change the price of Scale ${id}">Change price</button><button class="btn btn-small btn-quiet" data-act="delist" data-token="${s.id}" aria-label="Delist Scale ${id}">Delist</button>`}
    </span>
  </li>`;
}

function acceptable(offers, mine, account) {
  const ids = new Set(mine.map((s) => s.id));
  return offers.filter((o) => o.buyer !== account && (o.tokenId === null ? ids.size > 0 : ids.has(o.tokenId))).sort((a, b) => (b.amount > a.amount ? 1 : -1));
}

function yoursPanel(d, map, account, now) {
  const mine = d.scales.filter((s) => s.owner === account);
  if (!mine.length)
    return html`<p class="sale-state">You hold no Scale yet.</p><a class="btn btn-primary" href="#/scales/${d.sale.state === 'Open' ? 'sale' : 'market'}">Get a Scale</a>`;
  const ready = claimable(mine, d, now);
  const total = ready.reduce((n, s) => n + d.unclaimed.get(s.id), 0n);
  const offers = acceptable(d.offers, mine, account);
  return html`<div class="yours-head">
      <p class="sale-state">${mine.length} Scale${mine.length > 1 ? 's' : ''}, ${formatNear(total)} NEAR to claim</p>
      ${ready.length > 1 ? html`<button class="btn btn-primary" data-act="claimAll">Claim all ${formatNear(total)} NEAR</button>` : ''}
    </div>
    <ul class="rows rows-yours">${mine.map((s) => yourRow(s, map.get(s.id), d, now))}</ul>
    <p class="hint">A claim takes a listed Scale off the market.</p>
    ${offers.length
      ? html`<h2>Offers for you</h2><ul class="rows">${offers.map((o) => html`<li class="row">
          <span class="row-id"><span><strong>${formatNear(o.amount)} NEAR</strong><small>${market.offerTarget(o)} from ${o.buyer}</small></span></span>
          <span class="row-acts"><button class="btn btn-small" data-act="accept" data-offer="${o.id}">Accept</button></span></li>`)}</ul>`
      : ''}`;
}

const TAB_NAMES = { sale: 'Sale', market: 'Market', all: 'All 100', yours: 'Yours' };

export function view(d, { account, now }, [param] = []) {
  const map = statuses(d, account);
  const tab = tabOf(param, account, d.sale);
  const mineCount = account ? d.scales.filter((s) => s.owner === account).length : 0;
  const tabs = Object.keys(TAB_NAMES).filter((t) => t !== 'yours' || account);
  const panels = {
    sale: html`${salePanel(d, now)}${earnings.section(d.estimate, paid(d))}`,
    market: market.section(d, map, { account, now }),
    all: allPanel(map),
    yours: account ? yoursPanel(d, map, account, now) : '',
  };
  return html`<section class="page page-scales">
    <header class="page-head">
      <h1 tabindex="-1">Scales</h1>
      <p class="lede">A hundred Scales share half of every fee WYRM's trading pays. Whoever holds one claims its share.</p>
    </header>
    <div class="tabs" role="tablist" aria-label="Scales">${tabs.map((t) => html`<a role="tab" id="tab-${t}" href="#/scales/${t}" aria-controls="panel-${t}" aria-selected="${String(t === tab)}" tabindex="${t === tab ? 0 : -1}">${TAB_NAMES[t]}${t === 'yours' ? html` <span class="count">${mineCount}</span>` : ''}</a>`)}</div>
    ${tabs.map((t) => html`<div class="tab-panel" role="tabpanel" id="panel-${t}" aria-labelledby="tab-${t}" tabindex="0" ${t === tab ? '' : 'hidden'}>${panels[t]}</div>`)}
  </section>`;
}

export function retab(root, [param], d, { account }) {
  const tab = tabOf(param, account, d.sale);
  for (const a of root.querySelectorAll('[role=tab]')) {
    const on = a.id === `tab-${tab}`;
    a.setAttribute('aria-selected', String(on));
    a.tabIndex = on ? 0 : -1;
  }
  for (const p of root.querySelectorAll('[role=tabpanel]')) p.hidden = p.id !== `panel-${tab}`;
}

export function mounted(root, d, ctx) {
  earnings.mount(root);
  const list = root.querySelector('[role=tablist]');
  list.addEventListener('keydown', (e) => {
    const tabs = [...list.querySelectorAll('[role=tab]')];
    const i = tabs.indexOf(document.activeElement);
    const next = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: tabs.length - 1 }[e.key];
    if (i < 0 || next === undefined) return;
    e.preventDefault();
    const tab = tabs[(next + tabs.length) % tabs.length];
    tab.focus();
    // replaceState fires no hashchange, so the tab is shown here. Arrowing through tabs is not a visit to each, so
    // it must not leave history entries; a click on a tab still pushes one.
    history.replaceState(history.state, '', tab.getAttribute('href'));
    retab(root, [tab.id.slice('tab-'.length)], d, ctx);
  });
  const hide = root.querySelector('#hide');
  for (const chip of root.querySelectorAll('[data-filter]')) {
    chip.addEventListener('click', () => {
      for (const c of root.querySelectorAll('[data-filter]')) c.setAttribute('aria-pressed', String(c === chip));
      const f = chip.dataset.filter;
      hide.dataset.filter = f;
      for (const li of hide.children) li.inert = f !== 'all' && li.dataset.status !== f;
    });
  }
}

async function buyDialog(sale, now, perform) {
  const { creator, dragon } = saleSplit(sale.price, sale.creatorBps);
  const form = await ask({
    title: `Buy Scale ${padScaleId(sale.sold + 1)}`,
    content: html`<dl class="breakdown">
        <div><dt>You pay</dt><dd>${formatNear(sale.price)} NEAR</dd></div>
        <div><dt>To the creator, ${formatBps(sale.creatorBps)}</dt><dd>${formatNear(creator, 3)} NEAR</dd></div>
        <div><dt>To the Dragon's reserve, ${formatBps(10_000 - sale.creatorBps)}</dt><dd>${formatNear(dragon, 3)} NEAR</dd></div>
        <div><dt>The sale ends</dt><dd>${formatIn(sale.endsAt, now)}</dd></div>
      </dl>
      <p class="hint">The next id is minted to you. For up to ${Number(CLAIM_LOCK_NS / 60_000_000_000n)} minutes it is locked while its vault is swept to the Dragon.</p>`,
    submit: `Pay ${formatNear(sale.price)} NEAR`,
  });
  if (form) await perform(tx.buyScale(sale.price), 'Scale bought. It is in your wallet.');
}

const mineOf = (d, account) => d.scales.filter((s) => s.owner === account);

export const actions = {
  ...market.actions,
  mint: (_, { perform, now }, d) => buyDialog(d.sale, now, perform),
  claimAll: (_, { perform, account, now }, d) => {
    const ready = claimable(mineOf(d, account), d, now);
    return perform(tx.claimAll(ready.map((s) => s.id)), `Claimed from ${ready.length} Scales.`);
  },
  claim: (el, { perform }, d) => {
    const id = el.dataset.token;
    return perform(tx.claim(id), `Claimed ${formatNear(d.unclaimed.get(id))} NEAR from Scale ${padScaleId(id)}.`);
  },
  list: (el, { perform }, d) => listDialog(el.dataset.token, d.byId.get(el.dataset.token).listedPrice, perform),
  delist: (el, { perform }) => perform(tx.delist(el.dataset.token), `Scale ${padScaleId(el.dataset.token)} is off the market.`),
  accept: (el, { perform, account }, d) => acceptDialog(d.offers.find((o) => o.id === el.dataset.offer), mineOf(d, account).map((s) => s.id), perform),
};
