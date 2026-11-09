import { getScale, getUnclaimed, getOffers, getCouncil, getSale, getSystem, tx, accountLink } from '../chain.js';
import { SCALE_COUNT, tierOf, nextTier, isClaiming, royaltySplit, royaltyTo, statusOf, statusText, CLAIM_LOCK_NS } from '../domain.js';
import { formatNear, formatAgo, formatIn, formatUtc, shortAccount, padScaleId } from '../format.js';
import { scaleSvg } from '../art.js';
import { html, raw, ask } from '../ui.js';
import { listDialog, offerDialog, acceptDialog } from './offer-dialogs.js';

export const title = (d) => `Scale ${padScaleId(d.id)}`;

export async function load([id]) {
  const n = String(Number(id));
  const [one, sale] = await Promise.all([getScale(n), getSale()]);
  if (!one) return { found: false, id: n, sale };
  const [unclaimed, offers, council, system] = await Promise.all([getUnclaimed([one.scale]), getOffers(), getCouncil(), getSystem()]);
  return {
    found: true,
    id: n,
    sale,
    ...one,
    unclaimed: unclaimed.get(one.scale.id),
    anyOffers: offers.filter((o) => o.tokenId === null).sort((a, b) => (b.amount > a.amount ? 1 : -1)),
    seasonNs: council.seasonNs,
    creator: system.creator,
    royaltyTo: royaltyTo(system.royaltyReceiver, system),
  };
}

const HISTORY = {
  Minted: (e) => html`Minted to ${accountLink(e.to, shortAccount(e.to))}${e.price ? ` for ${formatNear(e.price)} NEAR` : ''}`,
  Transferred: (e) => html`${shortAccount(e.from)} to ${accountLink(e.to, shortAccount(e.to))}${e.price ? ` for ${formatNear(e.price)} NEAR` : ', no sale'}`,
  Claimed: (e) => html`${shortAccount(e.to)} claimed ${formatNear(e.amount)} NEAR`,
  Swept: (e) => html`The vault's ${formatNear(e.amount, 4)} NEAR from before the mint went to the Dragon`,
};

function ownerPanel(d, st, now) {
  const busy = isClaiming(d.scale, now);
  const offers = [...d.offers, ...d.anyOffers.slice(0, 3)];
  return html`<div class="actions">
    <button class="btn btn-primary" data-act="claim" ${busy || !d.unclaimed ? 'disabled' : ''}>${busy ? 'Locked while its vault is swept' : `Claim ${formatNear(d.unclaimed ?? 0n)} NEAR`}</button>
    ${st.price === null
      ? html`<button class="btn" data-act="list">List for sale</button>`
      : html`<button class="btn" data-act="list">Change price</button><button class="btn btn-quiet" data-act="delist">Delist</button>`}
  </div>
  ${st.price !== null && d.unclaimed ? html`<p class="hint">A claim takes it off the market.</p>` : ''}
  ${offers.length
    ? html`<h2>Offers you can accept</h2>
      <ul class="rows">${offers.map((o) => html`<li class="row">
        <span class="row-id"><span><strong>${formatNear(o.amount)} NEAR</strong><small>from ${shortAccount(o.buyer)}${o.tokenId ? '' : ', for any Scale'}</small></span></span>
        <span class="row-acts"><button class="btn btn-small" data-act="accept" data-offer="${o.id}">Accept</button></span></li>`)}</ul>`
    : ''}`;
}

function visitorPanel(d, st, account) {
  const mineOffer = d.offers.find((o) => o.buyer === account);
  const best = d.offers.reduce((m, o) => (o.amount > m ? o.amount : m), 0n);
  return html`<div class="actions">
    ${st.price !== null ? html`<button class="btn btn-primary btn-big" data-act="buy">Buy for ${formatNear(st.price)} NEAR</button>` : ''}
    ${mineOffer
      ? html`<button class="btn btn-quiet" data-act="cancel" data-offer="${mineOffer.id}">Cancel your ${formatNear(mineOffer.amount)} NEAR offer</button>`
      : html`<button class="btn ${st.price === null ? 'btn-primary' : ''}" data-act="offer">Make an offer</button>`}
  </div>
  ${st.price !== null
    ? html`<p class="hint">The ${formatNear(d.unclaimed ?? 0n)} NEAR to claim comes with it. ${formatNear(royaltySplit(st.price).royalty, 3)} NEAR of the price is the 5% royalty to ${d.royaltyTo === 'holders' ? 'the tithe' : 'the creator'}.</p>`
    : html`<p class="hint">Not for sale. ${best ? `The best offer is ${formatNear(best)} NEAR.` : 'The holder can accept an offer at any time.'}</p>`}`;
}

function unminted(d) {
  const st = statusOf(d.id, null, { sale: d.sale });
  const real = Number(d.id) >= 1 && Number(d.id) <= SCALE_COUNT;
  return html`<section class="page page-scale">
    <p class="crumb"><a href="#/scales/all">All Scales</a></p>
    ${real
      ? html`<div class="scale-hero"><div class="scale-big">${raw(scaleSvg(st.kind, padScaleId(d.id)))}</div><div>
          <h1 tabindex="-1">Scale ${padScaleId(d.id)}</h1>
          <p class="status status-${st.kind}">${statusText(st)}</p>
          <p class="lede">${st.kind === 'retired' ? 'Nobody bought it before the sale closed, so it was never minted. Its share of every fee goes to the Dragon.' : d.sale.state === 'Open' ? 'Nobody has bought it yet. The sale mints the next id to whoever buys.' : 'The sale opens when the coin is bound.'}</p>
          ${d.sale.state === 'Open' ? html`<div class="actions"><a class="btn btn-primary" href="#/scales/sale">Go to the sale</a></div>` : ''}
        </div></div>`
      : html`<h1 tabindex="-1">There is no Scale ${d.id}</h1>`}
  </section>`;
}

export function view(d, { account, now }) {
  if (!d.found) return unminted(d);
  const s = d.scale;
  const st = statusOf(s.id, s, { sale: d.sale, account, creator: d.creator });
  const tier = tierOf(s.holdStart, now);
  const next = nextTier(s.holdStart, now);
  const seasoned = s.voting && s.holdStart + d.seasonNs <= now;
  const locked = isClaiming(s, now);
  return html`<section class="page page-scale">
    <p class="crumb"><a href="#/scales/all">All Scales</a></p>
    <div class="scale-hero">
      <div class="scale-big">${raw(scaleSvg(st.kind, padScaleId(s.id), st.price !== null))}</div>
      <div>
        <h1 tabindex="-1">Scale ${padScaleId(s.id)}</h1>
        <p class="status status-${st.kind}">${statusText(st)}</p>
        <p class="lede">Held by ${st.kind === 'yours' ? 'you' : shortAccount(s.owner)} for ${formatAgo(s.holdStart, now).replace(' ago', '')}. ${tier}${next ? `, ${next.name} ${formatIn(next.at, now)}` : ', fully hardened'}.</p>
        <dl class="figures figures-small">
          <div><dt>To claim</dt><dd>${d.unclaimed == null ? 'no vault yet' : formatNear(d.unclaimed)}<small>${d.unclaimed == null ? '' : 'NEAR'}</small></dd></div>
          <div><dt>Claimed so far</dt><dd>${formatNear(s.claimedTotal)}<small>NEAR</small></dd></div>
          <div><dt>Council vote</dt><dd>${!s.voting ? 'never' : seasoned ? 'yes' : 'not yet'}<small>${!s.voting ? "a creator's Scale" : seasoned ? '' : `from ${formatUtc(s.holdStart + d.seasonNs)}`}</small></dd></div>
        </dl>
        ${locked ? html`<p class="warn">Locked ${formatIn(s.claimingSince + CLAIM_LOCK_NS, now).replace('in ', 'for ')} more while its vault is swept.</p>` : ''}
        ${st.kind === 'yours' ? ownerPanel(d, st, now) : visitorPanel(d, st, account)}
        <p class="hint">Its vault is ${accountLink(s.vault)}.</p>
      </div>
    </div>
    <section aria-labelledby="hist-h">
      <h2 id="hist-h">History</h2>
      <ol class="log">${d.history.map((e) => html`<li><time title="${formatUtc(e.at)}">${formatAgo(e.at, now)}</time><span>${HISTORY[e.type]?.(e) ?? e.type}</span></li>`)}</ol>
    </section>
  </section>`;
}

export const actions = {
  claim: (_, { perform }, d) => perform(tx.claim(d.scale.id), `Claimed ${formatNear(d.unclaimed)} NEAR from Scale ${padScaleId(d.scale.id)}.`),
  list: (_, { perform }, d) => listDialog(d.scale.id, d.scale.listedPrice, perform),
  delist: (_, { perform }, d) => perform(tx.delist(d.scale.id), `Scale ${padScaleId(d.scale.id)} is off the market.`),
  buy: (_, { perform }, d) => perform(tx.buy(d.scale.id, d.scale.listedPrice), `Scale ${padScaleId(d.scale.id)} is yours.`),
  offer: (_, { perform }, d) => offerDialog(d.scale.id, perform),
  cancel: async (el, { perform }) => {
    if (await ask({ title: 'Cancel your offer?', content: html`<p>The NEAR comes back to your wallet.</p>`, submit: 'Cancel offer', danger: true }))
      await perform(tx.cancelOffer(el.dataset.offer), 'Offer cancelled.');
  },
  accept: (el, { perform }, d) => acceptDialog({ ...[...d.offers, ...d.anyOffers].find((o) => o.id === el.dataset.offer), tokenId: d.scale.id }, [], perform),
};
