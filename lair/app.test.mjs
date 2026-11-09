import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// app.js runs here against a small DOM built from index.html, in mock mode with ember.near connected, so tab
// switches, keys, taps and actions go through the same handlers a browser calls.

const VOID = new Set(['br', 'hr', 'img', 'input', 'link', 'meta', 'source', 'wbr']);
const ENT = { amp: '&', lt: '<', gt: '>', quot: '"', '#39': "'" };
const decode = (s) => s.replace(/&(amp|lt|gt|quot|#39);/g, (_, e) => ENT[e]);
let layoutWidth = 1072;

class El {
  constructor(tag, attrs = []) {
    this.tagName = tag.toUpperCase();
    this.attrs = new Map(attrs);
    this.children = [];
    this.parent = null;
    this.listeners = {};
  }
  getAttribute(n) {
    return this.attrs.get(n) ?? null;
  }
  setAttribute(n, v) {
    this.attrs.set(n, String(v));
  }
  removeAttribute(n) {
    this.attrs.delete(n);
  }
  hasAttribute(n) {
    return this.attrs.has(n);
  }
  toggleAttribute(n, force) {
    const on = force ?? !this.attrs.has(n);
    if (on) this.attrs.set(n, '');
    else this.attrs.delete(n);
    return on;
  }
  get id() {
    return this.getAttribute('id') ?? '';
  }
  get hidden() {
    return this.attrs.has('hidden');
  }
  set hidden(v) {
    this.toggleAttribute('hidden', Boolean(v));
  }
  set inert(v) {
    this.toggleAttribute('inert', Boolean(v));
  }
  get tabIndex() {
    return Number(this.getAttribute('tabindex') ?? -1);
  }
  set tabIndex(v) {
    this.setAttribute('tabindex', v);
  }
  set disabled(v) {
    this.toggleAttribute('disabled', Boolean(v));
  }
  set className(v) {
    this.setAttribute('class', v);
  }
  get classList() {
    const get = () => new Set((this.getAttribute('class') ?? '').split(/\s+/).filter(Boolean));
    const put = (s) => this.setAttribute('class', [...s].join(' '));
    return {
      contains: (c) => get().has(c),
      add: (c) => put(get().add(c)),
      remove: (c) => put((s => (s.delete(c), s))(get())),
      toggle: (c, on = !get().has(c)) => (on ? this.classList.add(c) : this.classList.remove(c), on),
    };
  }
  get dataset() {
    const key = (k) => `data-${k.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;
    return new Proxy({}, {
      get: (_, k) => this.getAttribute(key(k)) ?? undefined,
      set: (_, k, v) => (this.setAttribute(key(k), v), true),
      deleteProperty: (_, k) => (this.removeAttribute(key(k)), true),
    });
  }
  set innerHTML(s) {
    this.children = [];
    parseInto(this, String(s));
  }
  set textContent(s) {
    this.children = [String(s)];
  }
  get textContent() {
    return this.children.map((c) => (typeof c === 'string' ? c : c.textContent)).join('');
  }
  get clientWidth() {
    return layoutWidth;
  }
  getBoundingClientRect() {
    return { left: 0, top: 0, width: Number(this.getAttribute('width') ?? layoutWidth) };
  }
  append(...nodes) {
    for (const n of nodes) {
      n.parent = this;
      this.children.push(n);
    }
  }
  remove() {
    if (this.parent) this.parent.children = this.parent.children.filter((c) => c !== this);
    this.parent = null;
  }
  contains(el) {
    for (let e = el; e; e = e.parent) if (e === this) return true;
    return false;
  }
  focus() {
    document.activeElement = this;
  }
  *descendants() {
    for (const c of this.children) {
      if (typeof c === 'string') continue;
      yield c;
      yield* c.descendants();
    }
  }
  matches(sel) {
    return sel.split(',').some((s) => {
      const parts = s.trim().split(/\s+/);
      if (!one(this, parts.at(-1))) return false;
      let e = this.parent;
      for (let i = parts.length - 2; i >= 0; i--) {
        while (e && !one(e, parts[i])) e = e.parent;
        if (!e) return false;
        e = e.parent;
      }
      return true;
    });
  }
  querySelectorAll(sel) {
    return [...this.descendants()].filter((e) => e.matches(sel));
  }
  querySelector(sel) {
    return this.querySelectorAll(sel)[0] ?? null;
  }
  closest(sel) {
    for (let e = this; e instanceof El; e = e.parent) if (e.matches(sel)) return e;
    return null;
  }
  addEventListener(type, fn) {
    (this.listeners[type] ??= []).push(fn);
  }
  dispatch(type, props = {}) {
    const ev = { type, target: this, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; }, ...props };
    for (let e = this; e; e = e.parent) for (const fn of e.listeners[type] ?? []) fn(ev);
    return ev;
  }
}

function one(el, simple) {
  const re = /([a-z][\w-]*)|#([\w-]+)|\.([\w-]+)|\[([\w-]+)(?:=(?:"([^"]*)"|'([^']*)'|([^\]]*)))?\]/gy;
  let m;
  let at = 0;
  while (at < simple.length && (re.lastIndex = at, (m = re.exec(simple)))) {
    at = re.lastIndex;
    if (m[1] && el.tagName !== m[1].toUpperCase()) return false;
    if (m[2] && el.id !== m[2]) return false;
    if (m[3] && !el.classList.contains(m[3])) return false;
    if (m[4]) {
      const want = m[5] ?? m[6] ?? m[7];
      if (!el.hasAttribute(m[4]) || (want !== undefined && el.getAttribute(m[4]) !== want)) return false;
    }
  }
  if (at !== simple.length) throw new Error(`selector ${simple}`);
  return true;
}

function parseInto(root, s) {
  const stack = [root];
  const tokens = /<!--[\s\S]*?-->|<(\/?)([a-zA-Z][\w:-]*)((?:\s+[^\s=>/]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)*)\s*(\/?)>|([^<]+)/g;
  for (const [all, close, tag, attrText, self, text] of s.matchAll(tokens)) {
    const top = stack.at(-1);
    if (text !== undefined) top.children.push(decode(text));
    else if (all.startsWith('<!--')) continue;
    else if (close) {
      const i = stack.findLastIndex((e) => e.tagName === tag.toUpperCase());
      if (i > 0) stack.length = i;
    } else {
      const attrs = [...attrText.matchAll(/([^\s=/]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g)].map((a) => [a[1], decode(a[2] ?? a[3] ?? a[4] ?? '')]);
      const el = new El(tag, attrs);
      top.append(el);
      if (!self && !VOID.has(tag.toLowerCase())) stack.push(el);
    }
  }
}

const body = new El('body');
parseInto(body, /<body>([\s\S]*)<\/body>/.exec(readFileSync(new URL('./index.html', import.meta.url), 'utf8'))[1]);
const windowListeners = {};
globalThis.document = {
  body,
  title: '',
  activeElement: null,
  getElementById: (id) => body.querySelector(`#${id}`),
  querySelector: (s) => body.querySelector(s),
  querySelectorAll: (s) => body.querySelectorAll(s),
  createElement: (tag) => new El(tag),
};
const later = globalThis.setTimeout;
globalThis.setTimeout = (fn, ms) => later(fn, ms).unref();
// As in a browser: setting the hash (a link) pushes an entry, and it and Back fire hashchange after the current task;
// replaceState changes the entry and fires nothing.
const fireHashchange = () => (windowListeners.hashchange ?? []).forEach((fn) => fn({ type: 'hashchange' }));
const hashchange = () => later(fireHashchange);
globalThis.window = { addEventListener: (type, fn) => (windowListeners[type] ??= []).push(fn) };
const entries = ['#/scales/sale'];
globalThis.location = {
  search: '?mock=1&sale=open',
  get hash() {
    return entries.at(-1);
  },
  set hash(h) {
    entries.push(h);
    hashchange();
  },
  replace: (h) => ((entries[entries.length - 1] = h), hashchange()),
};
globalThis.history = {
  state: null,
  get length() {
    return entries.length;
  },
  replaceState: (_, __, url) => (entries[entries.length - 1] = url),
};
const visit = (hash) => (location.hash = hash);
const back = () => (entries.pop(), hashchange());
const store = new Map([['lair.mockAccount', 'ember.near']]);
globalThis.localStorage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v), removeItem: (k) => store.delete(k) };
globalThis.fetch = async (url) => ({ json: async () => JSON.parse(readFileSync(url, 'utf8')) });
// Like a browser's first notification, after layout: the element's laid-out width.
globalThis.ResizeObserver = class {
  constructor(fn) {
    this.fn = fn;
  }
  observe(el) {
    later(() => this.fn([{ target: el, contentRect: { width: el.clientWidth } }]));
  }
};

const main = document.getElementById('main');
const tick = () => new Promise((r) => setImmediate(r));
async function settled() {
  await new Promise((r) => later(r, 1));
  for (let i = 0; i < 500; i++) {
    await tick();
    if (!main.hasAttribute('aria-busy') && main.querySelector('h1')) return;
  }
  throw new Error('the page never finished loading');
}

await import('./app.js');
await settled();
const { wallet } = await import('./wallet.js');
assert.equal(wallet.accountId, 'ember.near');
await settled();

const shownPanels = () => main.querySelectorAll('[role=tabpanel]').filter((p) => !p.hidden).map((p) => p.id);
const selectedTabs = () => main.querySelectorAll('[role=tab]').filter((t) => t.getAttribute('aria-selected') === 'true').map((t) => t.id);
const focusable = () => main.querySelectorAll('[role=tab]').filter((t) => t.tabIndex === 0).map((t) => t.id);
function onTab(tab, where) {
  assert.deepEqual(shownPanels(), [`panel-${tab}`], `${where}: only its panel shows`);
  assert.deepEqual(selectedTabs(), [`tab-${tab}`], `${where}: only its tab is selected`);
  assert.deepEqual(focusable(), [`tab-${tab}`], `${where}: only its tab is in the tab order`);
  assert.equal(location.hash, `#/scales/${tab}`, `${where}: the address names it`);
}

test('arrow keys, Home and End move along the Scales tabs and wrap, replacing the history entry instead of adding one', () => {
  onTab('sale', 'start');
  const before = history.length;
  const press = (key) => document.activeElement.dispatch('keydown', { key });
  main.querySelector('#tab-sale').focus();
  for (const [key, tab] of [['ArrowRight', 'market'], ['ArrowRight', 'all'], ['ArrowRight', 'yours'], ['ArrowRight', 'sale'], ['ArrowLeft', 'yours'], ['Home', 'sale'], ['End', 'yours'], ['ArrowLeft', 'all']]) {
    press(key);
    onTab(tab, key);
    assert.equal(document.activeElement.id, `tab-${tab}`, `${key}: focus follows`);
  }
  assert.equal(history.length, before, 'no history entry per key press');
  assert.equal(press('a').defaultPrevented, false, 'other keys pass through');
});

test('a tab link and Back switch tabs in place: one panel shows and its tab is selected', async () => {
  visit('#/scales/market');
  await settled();
  onTab('market', 'a link');
  visit('#/scales/yours');
  await settled();
  onTab('yours', 'a second link');
  back();
  await settled();
  onTab('market', 'Back');
});

test('a tab switch while an action reloads waits for the fresh read instead of showing the old data', async () => {
  visit('#/scales/yours');
  await settled();
  const button = main.querySelectorAll('[data-act=claim]').find((b) => !b.hasAttribute('disabled'));
  const row = () => button && main.querySelectorAll('.rows-yours .row').find((r) => r.querySelector(`[data-token="${button.dataset.token}"]`)).querySelector('.row-figure strong').textContent;
  assert.notEqual(row(), '0', 'a Scale with NEAR to claim');
  button.dispatch('click');
  for (let i = 0; i < 1000 && !main.hasAttribute('aria-busy'); i++) await Promise.resolve();
  assert.ok(main.hasAttribute('aria-busy'), 'the reload after the claim has started');
  entries.push('#/scales/all');
  fireHashchange();
  await settled();
  onTab('all', 'switched mid-read');
  const fresh = main.querySelectorAll('.rows-yours .row').find((r) => r.querySelector(`[data-token="${button.dataset.token}"]`));
  assert.equal(fresh.querySelector('.row-figure strong').textContent, '0', 'the Yours panel shows the claim');
});

test('at 390 px a tap on a buy coin shows that buy, even with another coin a few pixels away', async () => {
  layoutWidth = 358;
  visit('#/');
  await settled();
  await new Promise((r) => later(r, 5));
  assert.equal(main.querySelector('.chart-svg').getAttribute('width'), '358', 'redrawn at the laid-out width');
  const coins = main.querySelectorAll('circle.buy').map((c) => ({ cx: Number(c.getAttribute('cx')), title: c.textContent }));
  const close = coins.some((c, i) => i && c.cx - coins[i - 1].cx < 12);
  assert.ok(coins.length >= 3 && close, 'the open world has two buys within a tap of each other at this width');
  const plot = main.querySelector('.plot');
  for (const c of coins) {
    plot.dispatch('pointerdown', { clientX: c.cx, pointerType: 'touch' });
    const readout = main.querySelector('.readout').textContent;
    assert.equal(readout.replace(/ (\d\d:\d\d UTC)/, ', $1'), c.title, `tap at ${c.cx}`);
  }
  layoutWidth = 1072;
});
