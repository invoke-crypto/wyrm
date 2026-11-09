import { config } from './config.js';
import * as chain from './chain.js';
import { wallet } from './wallet.js';
import { html, toast } from './ui.js';
import { shortAccount } from './format.js';
import { drillNotice, mockBar, resolve } from './shell.js';

const PAGES = {
  dragon: () => import('./pages/dragon.js'),
  scales: () => import('./pages/scales.js'),
  scale: () => import('./pages/scale.js'),
  council: () => import('./pages/council.js'),
  verify: () => import('./pages/verify.js'),
};

const main = document.getElementById('main');
let current = null;
let renderSeq = 0;
let reading = 0;

function ctx() {
  return { account: wallet.accountId, now: chain.now(), perform, refresh: () => render() };
}

function markNav(nav) {
  for (const a of document.querySelectorAll('.nav a')) {
    if (a.dataset.nav === nav) a.setAttribute('aria-current', 'page');
    else a.removeAttribute('aria-current');
  }
}

// A read still in flight, as after an action, must land first, or the tab would show the data from before it.
const canRetab = (found) => !reading && current?.name === found.page && Boolean(current.page.retab);

async function render({ moveFocus = false, reload = true } = {}) {
  const seq = ++renderSeq;
  const found = resolve(location.hash);
  if (found?.redirect) return location.replace(found.redirect);
  markNav(found?.nav);
  if (!found) {
    current = null;
    main.innerHTML = String(html`<section class="page"><h1 tabindex="-1">Nothing lives here</h1><p><a href="#/">Back to the Dragon</a></p></section>`);
    return;
  }
  if (!reload && canRetab(found)) {
    current.params = found.params;
    current.page.retab(main, found.params, current.data, ctx());
    return;
  }
  main.setAttribute('aria-busy', 'true');
  reading++;
  try {
    const page = await PAGES[found.page]();
    const c = ctx();
    const data = await page.load(found.params, c);
    if (seq !== renderSeq) return;
    current = { name: found.page, page, params: found.params, data };
    delete document.body.dataset.mood;
    main.innerHTML = String(page.view(data, c, found.params));
    document.title = `${page.title(data)} · WYRM`;
    page.mounted?.(main, data, c, found.params);
    if (moveFocus) main.querySelector('h1')?.focus();
  } catch (e) {
    if (seq !== renderSeq) return;
    console.error(e);
    current = null;
    main.innerHTML = String(html`<section class="page"><h1 tabindex="-1">The Lair could not read the chain</h1>
      <p class="lede">${e.message}</p>
      ${/does not exist/.test(e.message)
        ? html`<p>The WYRM contracts are not on ${config.network} at these accounts yet. <a href="?mock=1">See the Lair with demo data</a>.</p>`
        : html`<p>Check your connection, or try another RPC with <code>?rpc=</code> in the address.</p>`}
      <button class="btn btn-primary" data-act="retry">Try again</button></section>`);
  } finally {
    if (--reading === 0) main.removeAttribute('aria-busy');
  }
}

async function perform(transactions, done) {
  if (!wallet.accountId) {
    await wallet.connect();
    if (!wallet.accountId) return false;
  }
  try {
    await chain.send(transactions);
    toast(done, 'ok');
    await render();
    return true;
  } catch (e) {
    console.warn(e);
    toast(e?.message ? `Not sent. ${e.message}` : 'Not sent.', 'error');
    return false;
  }
}

main.addEventListener('click', async (e) => {
  const el = e.target.closest('[data-act]');
  if (!el || !main.contains(el)) return;
  if (el.dataset.act === 'retry') return render({ moveFocus: true });
  if (el.dataset.act === 'connect') return wallet.connect().catch((err) => toast(err.message, 'error'));
  const handler = current?.page.actions?.[el.dataset.act];
  if (!handler) return;
  e.preventDefault();
  el.disabled = true;
  try {
    await handler(el, ctx(), current.data);
  } finally {
    el.disabled = false;
  }
});

main.addEventListener('submit', async (e) => {
  const form = e.target;
  const handler = current?.page.actions?.[form.dataset.submit];
  if (!handler) return;
  e.preventDefault();
  const button = form.querySelector('[type=submit]');
  if (button) button.disabled = true;
  try {
    await handler(form, ctx(), current.data);
  } finally {
    if (button) button.disabled = false;
  }
});

function renderWallet() {
  const slot = document.getElementById('wallet');
  slot.innerHTML = String(
    wallet.accountId
      ? html`<span class="who" title="${wallet.accountId}">${shortAccount(wallet.accountId)}</span><button class="btn btn-quiet btn-small" id="wallet-btn">Sign out</button>`
      : html`<button class="btn btn-small" id="wallet-btn">Connect wallet</button>`,
  );
  slot.querySelector('#wallet-btn').addEventListener('click', () => (wallet.accountId ? wallet.disconnect() : wallet.connect()).catch((e) => toast(e.message, 'error')));
}

async function renderDrill() {
  const content = await drillNotice();
  if (!content) return;
  const banner = document.getElementById('drill');
  banner.hidden = false;
  banner.innerHTML = String(content);
}

function renderMockBar() {
  if (!config.mock) return;
  const bar = document.getElementById('mockbar');
  bar.hidden = false;
  bar.innerHTML = String(mockBar());
  bar.querySelector('#mock-sale').addEventListener('change', (e) => {
    const p = new URLSearchParams(location.search);
    p.set('sale', e.target.value);
    location.search = p.toString();
  });
}

wallet.onChange(() => {
  renderWallet();
  render();
});
window.addEventListener('hashchange', () => render({ moveFocus: true, reload: false }));

renderWallet();
const first = resolve(location.hash);
if (first?.page) PAGES[first.page]();
await chain.ready();
renderMockBar();
render();
renderDrill().catch((e) => console.warn('preset', e));
wallet.restore().catch((e) => console.warn('wallet restore', e));
