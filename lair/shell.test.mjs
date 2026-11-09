import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// The shell as app.js starts it from an address: mock mode, the open sale, a drill preset, and an old ?mood=Hunting
// link, which no longer picks anything.
globalThis.location = { search: '?mock=1&sale=open&mood=Hunting&preset=drill' };
globalThis.fetch = async (url) => ({ json: async () => JSON.parse(readFileSync(url, 'utf8')) });
const chain = await import('./chain.js');
const { mockBar, drillNotice, resolve } = await import('./shell.js');
const dragon = await import('./pages/dragon.js');
await chain.ready();

test("the mock bar picks only the sale; the open world's log gives Breathing whatever ?mood= says", async () => {
  const bar = String(mockBar());
  const options = [...bar.matchAll(/<option ([^>]*)>(\w+)<\/option>/g)].map(([, attrs, name]) => (attrs.includes('selected') ? `*${name}` : name));
  assert.deepEqual(options, ['waiting', '*open', 'closed']);
  assert.doesNotMatch(bar, /mock-mood/);
  assert.equal((await dragon.load([], { now: chain.now() })).mood, 'Breathing');
});

test('each Scales tab has its own address, and the old Market, Holders and Estimator links land on the tab that does their job', () => {
  for (const tab of ['sale', 'market', 'all', 'yours']) assert.deepEqual(resolve(`#/scales/${tab}`), { page: 'scales', nav: 'scales', params: [tab] }, tab);
  assert.deepEqual(resolve('#/scales'), { page: 'scales', nav: 'scales', params: [undefined] }, 'no tab: the page picks');
  assert.deepEqual(resolve('#/scales/market/'), { page: 'scales', nav: 'scales', params: ['market'] });
  assert.deepEqual(resolve('#/scales/42'), { page: 'scale', nav: 'scales', params: ['42'] });
  assert.equal(resolve('#/scales/offers'), null);
  assert.deepEqual(resolve('#/market'), { redirect: '#/scales/market' });
  assert.deepEqual(resolve('#/holders'), { redirect: '#/scales/all' });
  assert.deepEqual(resolve('#/estimator'), { redirect: '#/scales/sale' });
  assert.deepEqual(resolve(''), { page: 'dragon', nav: '', params: [] });
  assert.deepEqual(resolve('#/'), { page: 'dragon', nav: '', params: [] });
  assert.equal(resolve('#/marketplace'), null);
});

test('the drill banner reads its price and times from the drill preset', async () => {
  assert.match(String(await drillNotice()), /Scales cost 0\.1 NEAR, seasoning takes 10 minutes and a vote 30 minutes\./);
});
