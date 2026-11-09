import { tx } from '../chain.js';
import { royaltySplit, MAX_OPEN_OFFERS } from '../domain.js';
import { formatNear, formatAgo, shortAccount, padScaleId } from '../format.js';
import { scaleSvg } from '../art.js';
import { html, raw, ask } from '../ui.js';
import { offerDialog } from './offer-dialogs.js';

const byPrice = (a, b) => (a.price < b.price ? -1 : a.price > b.price ? 1 : 0);
const byAmount = (a, b) => (b.amount > a.amount ? 1 : b.amount < a.amount ? -1 : 0);

export const offerTarget = (o) => (o.tokenId ? `For Scale ${padScaleId(o.tokenId)}` : 'For any Scale');

function offerRow(o, { account, mine, now }) {
  const canAccept = o.buyer !== account && (o.tokenId === null ? mine.size > 0 : mine.has(o.tokenId));
  return html`<li class="row">
    <span class="row-id"><span><strong>${formatNear(o.amount)} NEAR</strong><small>${o.tokenId ? html`For <a href="#/scales/${o.tokenId}">Scale ${padScaleId(o.tokenId)}</a>` : 'For any Scale'}, from ${o.buyer === account ? 'you' : shortAccount(o.buyer)}, ${formatAgo(o.at, now)}</small></span></span>
    <span class="row-acts">${o.buyer === account
      ? html`<button class="btn btn-small btn-quiet" data-act="cancel" data-offer="${o.id}">Cancel</button>`
      : canAccept ? html`<button class="btn btn-small" data-act="accept" data-offer="${o.id}">Accept</button>` : ''}</span>
  </li>`;
}

export function section(d, statuses, { account, now }) {
  const listings = [...statuses].filter(([, st]) => st.price !== null).map(([id, st]) => ({ id, st, price: st.price })).sort(byPrice);
  const offers = [...d.offers].sort(byAmount);
  const mine = new Set(d.scales.filter((s) => s.owner === account).map((s) => s.id));
  const myOffers = offers.filter((o) => o.buyer === account).length;
  return html`<h2>Listed by holders</h2>
    ${listings.length
      ? html`<ul class="rows">${listings.map(({ id, st, price }) => {
          const waiting = d.unclaimed.get(id);
          return html`<li class="row">
            <a class="row-id" href="#/scales/${id}">${raw(scaleSvg(st.kind, '', true))}<span><strong>Scale ${padScaleId(id)}</strong><small>${waiting ? `${formatNear(waiting)} NEAR to claim comes with it` : 'Nothing to claim inside'}</small></span></a>
            <span class="row-figure"><strong>${formatNear(price)} NEAR</strong><small>${formatNear(royaltySplit(price).royalty, 3)} royalty</small></span>
            <span class="row-acts">${st.kind === 'yours'
              ? html`<a class="btn btn-small btn-quiet" href="#/scales/yours">Yours</a>`
              : html`<button class="btn btn-small" data-act="buy" data-token="${id}" aria-label="Buy Scale ${padScaleId(id)} for ${formatNear(price)} NEAR">Buy</button>`}</span>
          </li>`;
        })}</ul>`
      : html`<p class="empty">Nothing is listed right now. Make an offer and a holder may take it.</p>`}
    <p class="hint">5% of every sale is a royalty to ${d.royaltyTo === 'holders' ? 'the tithe, shared like any fee' : 'the creator'}.</p>
    <div class="row-head">
      <h2>Offers</h2>
      <button class="btn btn-small" data-act="offerAny" ${myOffers >= MAX_OPEN_OFFERS ? 'disabled' : ''}>Offer on any Scale</button>
    </div>
    ${offers.length ? html`<ul class="rows">${offers.map((o) => offerRow(o, { account, mine, now }))}</ul>` : html`<p class="empty">No open offers.</p>`}`;
}

export const actions = {
  buy: (el, { perform }, d) => {
    const s = d.byId.get(el.dataset.token);
    return perform(tx.buy(s.id, s.listedPrice), `Scale ${padScaleId(s.id)} is yours.`);
  },
  offerAny: (_, { perform }) => offerDialog(null, perform),
  cancel: async (el, { perform }) => {
    if (await ask({ title: 'Cancel your offer?', content: html`<p>The NEAR comes back to your wallet.</p>`, submit: 'Cancel offer', danger: true }))
      await perform(tx.cancelOffer(el.dataset.offer), 'Offer cancelled.');
  },
};
