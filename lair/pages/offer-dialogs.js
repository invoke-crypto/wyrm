// The dialogs shared by the Scale page and the market, so a list, offer or
// accept reads the same wherever it starts.
import { tx } from '../chain.js';
import { MIN_PRICE, STORAGE_FEE, royaltySplit } from '../domain.js';
import { formatNear, parseNearInput, padScaleId } from '../format.js';
import { ask, html, toast } from '../ui.js';

function liveNote(dlg, input, note, describe) {
  const update = () => {
    const v = parseNearInput(input.value);
    note.textContent = v === null || v < MIN_PRICE ? 'At least 0.1 NEAR.' : describe(v);
  };
  dlg.addEventListener('ask-input', update);
  update();
}

function priceField(name, label, value = '') {
  return html`<label class="field">${label}
    <span class="near-input"><input name="${name}" inputmode="decimal" autocomplete="off" required pattern="\\d+(\\.\\d{0,24})?" value="${value}"><span>NEAR</span></span></label>
    <p class="note" aria-live="polite"></p>`;
}

function opened(describe) {
  queueMicrotask(() => {
    const dlg = document.querySelector('dialog.ask');
    liveNote(dlg, dlg.querySelector('input'), dlg.querySelector('.note'), describe);
  });
}

function parsePrice(form, name) {
  const v = parseNearInput(form.get(name));
  if (v === null || v < MIN_PRICE) {
    toast('Enter at least 0.1 NEAR.', 'error');
    return null;
  }
  return v;
}

export async function listDialog(tokenId, current, perform) {
  opened((v) => `You get ${formatNear(royaltySplit(v).seller, 3)} NEAR. ${formatNear(royaltySplit(v).royalty, 3)} NEAR (5%) is paid as royalty.`);
  const form = await ask({
    title: current ? `Change the price of Scale ${padScaleId(tokenId)}` : `List Scale ${padScaleId(tokenId)} for sale`,
    content: html`${priceField('price', 'Price', current ? formatNear(current, 3).replace(/,/g, '') : '')}
      <p class="hint">The Scale stays in your wallet until someone buys it. You can delist any time.</p>`,
    submit: current ? 'Update price' : 'List',
  });
  const price = form && parsePrice(form, 'price');
  if (price) await perform(tx.list(tokenId, price), `Scale ${padScaleId(tokenId)} listed for ${formatNear(price)} NEAR.`);
}

export async function offerDialog(tokenId, perform) {
  opened((v) => `You attach ${formatNear(v + STORAGE_FEE, 3)} NEAR. The 0.01 NEAR storage fee is kept; the rest comes back if you cancel.`);
  const form = await ask({
    title: tokenId ? `Make an offer on Scale ${padScaleId(tokenId)}` : 'Offer on any Scale',
    content: html`${priceField('amount', 'Your offer')}
      <p class="hint">${tokenId ? 'The owner can accept it at any time until you cancel.' : 'Any holder can accept it with a Scale of their choice.'}</p>`,
    submit: 'Make offer',
  });
  const amount = form && parsePrice(form, 'amount');
  if (amount) await perform(tx.makeOffer(tokenId, amount), `Offer of ${formatNear(amount)} NEAR made.`);
}

export async function acceptDialog(offer, myScales, perform) {
  const choices = offer.tokenId ? [offer.tokenId] : myScales;
  if (!choices.length) return toast('You have no Scale to sell.', 'error');
  const { seller, royalty } = royaltySplit(offer.amount);
  const form = await ask({
    title: `Sell for ${formatNear(offer.amount)} NEAR`,
    content: html`
      ${offer.tokenId
        ? html`<p>Scale ${padScaleId(offer.tokenId)} goes to ${offer.buyer}.</p>`
        : html`<label class="field">Which Scale
            <select name="token">${choices.map((id) => html`<option value="${id}">Scale ${padScaleId(id)}</option>`)}</select></label>`}
      <dl class="breakdown"><div><dt>You get</dt><dd>${formatNear(seller, 3)} NEAR</dd></div><div><dt>Royalty (5%)</dt><dd>${formatNear(royalty, 3)} NEAR</dd></div></dl>
      <p class="hint">Unclaimed NEAR in the Scale's vault goes with it. Claim first if you want to keep it.</p>`,
    submit: 'Sell',
  });
  if (!form) return;
  const tokenId = offer.tokenId ?? form.get('token');
  await perform(tx.acceptOffer(offer.id, tokenId), `Sold Scale ${padScaleId(tokenId)} for ${formatNear(offer.amount)} NEAR.`);
}
