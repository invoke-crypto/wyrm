import { getCouncil, getSale } from './chain.js';
import { config } from './config.js';
import { formatNear, formatLength } from './format.js';
import { html } from './ui.js';

export function drillBanner(council, sale) {
  if (council.preset !== 'drill') return null;
  return html`<p><strong>Testnet drill.</strong> Scales cost ${formatNear(sale.price)} NEAR, seasoning takes ${formatLength(council.seasonNs)} and a vote ${formatLength(council.votingNs)}. Nothing here is mainnet WYRM.</p>`;
}

export async function drillNotice() {
  const [council, sale] = await Promise.all([getCouncil(), getSale()]);
  return drillBanner(council, sale);
}

const select = (id, label, value, options) =>
  html`<label>${label} <select id="${id}">${options.map((o) => html`<option ${o === value ? 'selected' : ''}>${o}</option>`)}</select></label>`;

// No mood picker: the Dragon's mood comes from each world's log, and each log gives one.
export function mockBar() {
  return html`<p>Demo data. Nothing is sent on chain.</p>
    ${select('mock-sale', 'Sale', config.mockSale, ['waiting', 'open', 'closed'])}`;
}

const SCALE_TABS = ['sale', 'market', 'all', 'yours'];

const ROUTES = [
  { re: /^$/, page: 'dragon', nav: '' },
  { re: new RegExp(`^scales(?:/(${SCALE_TABS.join('|')}))?$`), page: 'scales', nav: 'scales' },
  { re: /^scales\/(\d{1,3})$/, page: 'scale', nav: 'scales' },
  { re: /^council$/, page: 'council', nav: 'council' },
  { re: /^verify$/, page: 'verify', nav: 'verify' },
];

const LEGACY_REDIRECTS = { market: '#/scales/market', holders: '#/scales/all', estimator: '#/scales/sale' };

// Where an address's hash leads: a page with its params, a redirect, or null for nowhere.
export function resolve(hash) {
  const path = hash.replace(/^#\/?/, '').replace(/\/$/, '');
  if (Object.hasOwn(LEGACY_REDIRECTS, path)) return { redirect: LEGACY_REDIRECTS[path] };
  for (const r of ROUTES) {
    const m = r.re.exec(path);
    if (m) return { page: r.page, nav: r.nav, params: m.slice(1) };
  }
  return null;
}
