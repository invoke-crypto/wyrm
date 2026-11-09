import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';

// Pages render here as they do in the browser: each page's load() reads through chain.js over RPC, and the
// RPC answers from a mock world, so the real parsers run. `world` picks which one.
let world = null;
globalThis.location = { search: '' };
globalThis.fetch = async (url, init) => {
  if (String(url).startsWith('file:')) return { json: async () => JSON.parse(readFileSync(url, 'utf8')) };
  const { params } = JSON.parse(init.body);
  const result = {
    call_function: () => ({ result: [...new TextEncoder().encode(JSON.stringify(world.view(params.account_id, params.method_name, JSON.parse(atob(params.args_base64)))))] }),
    view_account: () => world.account(params.account_id) ?? { error: `account ${params.account_id} does not exist` },
    view_access_key_list: () => ({ keys: world.keys(params.account_id) }),
  }[params.request_type]();
  return { ok: true, json: async () => ({ result }) };
};
const chain = await import('./chain.js');
const { config } = await import('./config.js');
const { createMock } = await import('./mock.js');
const { drillBanner } = await import('./shell.js');
const scales = await import('./pages/scales.js');
const council = await import('./pages/council.js');
const dragon = await import('./pages/dragon.js');
const scale = await import('./pages/scale.js');
const verify = await import('./pages/verify.js');
const { formatSpan, formatUtc } = await import('./format.js');
const { moodOf } = await import('./domain.js');

async function enter(settings) {
  Object.assign(config, { mockSale: 'closed', mockSold: null, mockPreset: null }, settings);
  world = await createMock();
  return { account: null, now: world.now() };
}

async function render(page, ctx, params = []) {
  return String(page.view(await page.load(params, ctx), ctx, params));
}

test('a waiting sale says it opens when the coin is bound, and offers no buy button', async () => {
  const waiting = await render(scales, await enter({ mockSale: 'waiting' }));
  assert.match(waiting, /The sale opens when the coin is bound\./);
  assert.doesNotMatch(waiting, /data-act="mint"/);
  const open = await render(scales, await enter({ mockSale: 'open' }));
  assert.match(open, /data-act="mint"/, 'an open sale does offer it');
});

test('with no seasoned Scale the Council counts down to the first vote and disables the form', async () => {
  const page = await render(council, await enter({ mockSale: 'open' }));
  assert.match(page, /The first Scale can vote in 5d \d+h<\/strong>, at 2026-11-16 /);
  assert.match(page, /<fieldset class="propose-body" disabled/);
  assert.match(page, /<input type="radio" name="kind" value="SetBreath"/, 'the forms are still shown');
});

test("the drill banner shows only on a drill system, with the preset's price and times", async () => {
  await enter({ mockPreset: 'drill' });
  const [drill, sale] = [await chain.getCouncil(), await chain.getSale()];
  assert.match(String(drillBanner(drill, sale)), /Scales cost 0\.1 NEAR, seasoning takes 10 minutes and a vote 30 minutes\./);
  assert.match(String(drillBanner(drill, { ...sale, price: 25n * 10n ** 22n })), /Scales cost 0\.25 NEAR,/);
  await enter({});
  assert.equal(drillBanner(await chain.getCouncil(), await chain.getSale()), null);
});

const SALES = ['waiting', 'open', 'closed'];
const MIN = 60_000_000_000n;

test("the Dragon's mood is read from its log, never from the contract's live mood", async () => {
  const mood = async (ctx) => /<h1 tabindex="-1" class="mood-word">(\w+)<\/h1>/.exec(await render(dragon, ctx))[1];
  assert.equal(await mood(await enter({ mockSale: 'waiting' })), 'Sleeping', 'a log with no swap');
  const ctx = await enter({ mockSale: 'closed' });
  assert.equal(await mood(ctx), 'Breathing', 'the dip buy 5 minutes ago');
  const D = config.contracts.dragon;
  const view = world.view;
  const shifted = (minutes) => view(D, 'get_log', { limit: 256 }).map((e) => ({ ...e, at: String(BigInt(e.at) - BigInt(minutes) * MIN) }));
  world.view = (contract, method, args) =>
    method === 'get_state' ? { ...view(contract, method, args), mood: 'Breathing', phase: { Swapping: { side: 'Buy' } } } : method === 'get_log' ? shifted(40) : view(contract, method, args);
  const page = await render(dragon, ctx);
  assert.match(page, /class="mood-word">Sleeping</, 'the contract says Breathing, and the last buy was 45 minutes ago');
  assert.match(page, /data-mood="Sleeping"/, 'the art sleeps too');
  const sold = { seq: '99', at: String(ctx.now - 10n * MIN), event: 'hunt_slice', data: { amount_in: '1000000000000000000000', quoted_out: '3000000000000000000000' } };
  world.view = (contract, method, args) => (method === 'get_log' ? [...shifted(40), sold] : view(contract, method, args));
  assert.equal(await mood(ctx), 'Hunting', 'a sell 10 minutes ago');
  assert.equal(dragon.title(await dragon.load([], ctx)), 'The Dragon is hunting');
});

test('each mock world holds the one Dragon state its log gives', async () => {
  for (const sale of SALES) {
    const w = JSON.parse(readFileSync(new URL(`./fixtures/world-${sale}.json`, import.meta.url), 'utf8'));
    const ctx = await enter({ mockSale: sale });
    const { log } = await chain.getDragon();
    assert.deepEqual(Object.keys(w.dragon.states), [moodOf(log, ctx.now)], sale);
  }
});

// What would help someone trade against the Dragon: when it buys next, the dip steps (20%, 35% and 50% below the
// reference in every preset), the reference price and its lines, and how much of today's budget is left. The DCL's
// 20% fee share in the estimator's assumptions is not a step, hence the words after the percent.
const SIGNALS = [/next breath/i, /breath time/i, /reference/i, /\bdip/i, /\bsteps?\b(?!=)/i, /\b(20|35|50)%(,| and| or| below| under| drop| fall| down)/i, /spent today/i, /budget is spent/i, /<line[\s/>]/];

function noSignal(text, signals, where) {
  for (const re of signals) {
    const m = re.exec(text);
    assert.equal(m && text.slice(Math.max(0, m.index - 60), m.index + 40), null, `${where} shows ${re}`);
  }
}

// Every page the mock can show, in every sale world, with the raw Dragon views. `perturb` rewrites the get_state
// the pages read.
const PAGES = { dragon, scales, scale, council, verify };
async function* allPages(perturb = (state) => state) {
  for (const sale of SALES) {
    const ctx = await enter({ mockSale: sale });
    const D = config.contracts.dragon;
    const view = world.view;
    world.view = (contract, method, args) => (contract === D && method === 'get_state' ? perturb(view(contract, method, args), ctx.now) : view(contract, method, args));
    const raw = { state: view(D, 'get_state', {}), samples: view(D, 'get_samples', { limit: 1440 }), log: view(D, 'get_log', { limit: 256 }) };
    const id = view(config.contracts.scales, 'get_scales', { from: 0, limit: 1 })[0].token_id;
    for (const [name, page] of Object.entries(PAGES)) {
      yield { name, where: `${sale} ${name}`, raw, now: ctx.now, page: await render(page, ctx, name === 'scale' ? [id] : []) };
    }
  }
}

test('no page of any world shows when or why the Dragon buys', async () => {
  for await (const { name, where, page } of allPages()) noSignal(page, name === 'council' ? SIGNALS.slice(0, 4) : SIGNALS, where);
});

test('no page of any world shows the breath time, as a time or as a span', async () => {
  let pages = 0;
  for await (const { where, raw, now, page } of allPages()) {
    const breathAt = BigInt(raw.state.day.breath_at);
    assert.ok(!page.includes(formatUtc(breathAt)), `${where}: the breath time`);
    assert.ok(!page.includes(formatSpan(breathAt - now)), `${where}: the span to the breath`);
    pages++;
  }
  assert.equal(pages, 15);
});

// The contract's live mood, phase and day, the dip, the reference and the spot, set to other values a contract
// could hold. A page that read any of them would render differently.
function relive(state, now) {
  return {
    ...state,
    mood: state.mood === 'Hunting' ? 'Sleeping' : 'Hunting',
    phase: { Swapping: { amount_in: '1', min_out: '0', quoted_out: '1', side: 'Sell', since: String(now), step: '1' } },
    day: { ...state.day, breath_at: String(now + 17n * MIN), spent: '0', sold_wyrm: '7', index: String(BigInt(state.day.index) + 1n) },
    dip: { confirmed: false, first_at: String(now), samples: 1, step: 3, step_bps: 5000 },
    reference: '123456789',
    spot: '987654321',
  };
}

test("no page reads the contract's live mood, phase or day: every page renders the same when they change", async () => {
  const before = [];
  for await (const { page } of allPages()) before.push(page);
  let i = 0;
  for await (const { where, page } of allPages(relive)) assert.equal(page, before[i++], where);
  assert.equal(i, 15);
});

test('no Lair module reads the day, the phase or the breath time from a raw view', () => {
  const dir = new URL('./', import.meta.url);
  const modules = [...readdirSync(dir).filter((f) => f.endsWith('.js')), ...readdirSync(new URL('pages/', dir)).map((f) => `pages/${f}`)];
  assert.ok(modules.includes('chain.js') && modules.includes('pages/council.js'));
  for (const f of modules) {
    const src = readFileSync(new URL(f, dir), 'utf8');
    for (const re of [/\.day\b/, /\[\s*['"]day['"]\s*\]/, /breath_at/, /\.phase\b/]) assert.doesNotMatch(src, re, f);
  }
});

async function* dragonPages() {
  for await (const p of allPages()) if (p.name === 'dragon') yield p;
}

function splitTimeAxis(page) {
  const ticks = [...page.matchAll(/<text class="tick-x" data-at="(\d+)"[^>]*>([^<]*)<\/text>/g)].map((m) => ({ at: Number(m[1]), label: m[2] }));
  return { ticks, rest: page.replace(/<text class="tick-x"[^>]*>[^<]*<\/text>/g, '') };
}

test('the Dragon page shows only past times: a log entry, the last sample, its own time axis, and spans ago', async () => {
  for await (const { where, raw, page } of dragonPages()) {
    const { ticks, rest } = splitTimeAxis(page);
    const lastSample = raw.samples.length ? Number(BigInt(raw.samples.at(-1).at) / 1_000_000n) : 0;
    for (const t of ticks) assert.ok(t.at <= lastSample, `${where}: axis label ${t.label} is after the last sample`);
    if (raw.samples.length > 1) assert.ok(ticks.length >= 2, `${where}: the time axis is labelled`);
    const shown = new Set([...raw.log.map((e) => e.at), raw.samples.at(-1)?.at].filter(Boolean).map((at) => formatUtc(BigInt(at)).slice(11, 16)));
    for (const [hhmm] of rest.matchAll(/(?<![\d:])\d{2}:\d{2}(?!\d)/g)) assert.ok(shown.has(hhmm), `${where}: ${hhmm} is no log entry's time and not the last sample's`);
    for (const m of rest.matchAll(/\d+(?:d \d+h|h \d+m|m|s)(?! ago| of samples)/g)) assert.fail(`${where}: a span that is not in the past: ${rest.slice(m.index - 40, m.index + 30)}`);
  }
});

test('every chart a mock world shows fits its mood: a Breathing Dragon has its recent buy drawn on the line', async () => {
  for await (const { where, raw, now, page } of dragonPages()) {
    const mood = /class="mood-word">(\w+)</.exec(page)[1];
    const buys = raw.log.filter((e) => e.event === 'breath_slice' || e.event === 'dip_slice').map((e) => BigInt(e.at));
    assert.equal(raw.log.filter((e) => e.event === 'hunt_slice').length, 0, `${where}: no mock world logs a sell, so none is Hunting`);
    if (mood === 'Sleeping') {
      assert.ok(buys.every((at) => now - at >= 30n * MIN), `${where}: Sleeping, with no buy in the last half hour`);
      continue;
    }
    assert.equal(mood, 'Breathing', where);
    const last = buys.reduce((a, b) => (a > b ? a : b));
    assert.ok(now - last < 30n * MIN, `${where}: the buy that makes it Breathing is recent`);
    assert.ok(last <= BigInt(raw.samples.at(-1).at), `${where}: and inside the chart`);
    const hhmm = formatUtc(last).slice(11, 16);
    assert.match(page, new RegExp(`<circle class="buy"[^>]*><title>The Dragon bought [\\d,]+ WYRM for [\\d.]+ NEAR, ${hhmm} UTC`), `${where}: its coin is drawn`);
  }
});

function lineYAt(d, x) {
  const pts = [...d.matchAll(/[ML]([\d.]+) ([\d.]+)/g)].map((m) => [Number(m[1]), Number(m[2])]);
  const j = pts.findIndex(([px]) => px >= x);
  if (j <= 0) return pts[Math.max(j, 0)][1];
  const [[x0, y0], [x1, y1]] = [pts[j - 1], pts[j]];
  return y0 + ((y1 - y0) * (x - x0)) / (x1 - x0 || 1);
}

test('the price chart draws the line, its fill and quiet gridlines, and no threshold, step or reference line', async () => {
  for await (const { where, raw, page } of dragonPages()) {
    if (raw.samples.length < 2) continue;
    const svg = /<svg class="chart-svg"[^>]*>([\s\S]*?)<\/svg>/.exec(page)[1];
    assert.doesNotMatch(svg, /<(line|polyline|polygon|rect)/, `${where}: only paths, text and the buy coins`);
    assert.deepEqual([...svg.matchAll(/<path class="(\w+)"/g)].map((m) => m[1]).sort(), ['area', 'cross', 'grid', 'line'], `${where}: the shapes`);
    const grid = /<path class="grid" d="([^"]*)"/.exec(svg)[1];
    const rows = [...grid.matchAll(/M0 ([\d.]+)H\d+/g)].map((m) => Number(m[1]));
    assert.equal(grid.replace(/M0 [\d.]+H\d+/g, ''), '', `${where}: every gridline is a full-width horizontal`);
    const labels = [...svg.matchAll(/<text class="tick-y" x="\d+" y="([\d.]+)">([\d.,]+)<\/text>/g)].map((m) => +(Number(m[1]) - 4).toFixed(1));
    assert.deepEqual(rows, labels, `${where}: each gridline is a labelled price on the axis, so none marks a threshold`);
    const prices = raw.samples.map((q) => 1e24 / Number(q.tokens_per_near));
    const ys = [...svg.matchAll(/<text class="tick-y"[^>]*>([\d.,]+)</g)].map((m) => Number(m[1].replace(/,/g, '')));
    assert.ok(ys.length >= 2 && Math.min(...ys) <= Math.max(...prices) && Math.max(...ys) >= Math.min(...prices), `${where}: the labels span the prices`);
  }
});

test("the price chart puts one coin on the line at each buy in the Dragon's log, naming what it bought and when", async () => {
  let coins = 0;
  for await (const { where, raw, page } of dragonPages()) {
    if (raw.samples.length < 2) continue;
    const [t0, t1] = [BigInt(raw.samples[0].at), BigInt(raw.samples.at(-1).at)];
    const want = raw.log.filter((e) => ['breath_slice', 'dip_slice'].includes(e.event) && BigInt(e.at) >= t0 && BigInt(e.at) <= t1);
    const line = /<path class="line" d="([^"]*)"/.exec(page)[1];
    const got = [...page.matchAll(/<circle class="buy" data-buy="\d+" cx="([\d.]+)" cy="([\d.]+)" r="6"><title>The Dragon bought ([\d,]+) WYRM for ([\d.]+) NEAR, (\d\d:\d\d) UTC/g)];
    assert.equal(got.length, want.length, `${where}: one coin per buy`);
    assert.match(page, new RegExp(`The Dragon bought ${want.length} times? in that window\.`));
    for (const [i, m] of got.entries()) {
      const [cx, cy] = [Number(m[1]), Number(m[2])];
      assert.ok(Math.abs(lineYAt(line, cx) - cy) < 0.6, `${where}: coin ${i} sits on the line`);
      assert.equal(m[3], ((BigInt(want[i].data.quoted_out) + 5n * 10n ** 17n) / 10n ** 18n).toLocaleString('en-US'), `${where}: coin ${i} names the WYRM bought`);
      assert.equal(m[5], formatUtc(BigInt(want[i].at)).slice(11, 16), `${where}: coin ${i} names its time`);
    }
    const xs = got.map((m) => Number(m[1]));
    assert.deepEqual(xs, [...xs].sort((a, b) => a - b), `${where}: coins in time order`);
    coins += got.length;
  }
  assert.ok(coins >= 4, 'the fixtures have buys to mark');
});

test('a Dragon with no buys in its samples draws the price and no coins', async () => {
  const ctx = await enter({ mockSale: 'closed' });
  const view = world.view;
  world.view = (contract, method, args) => (method === 'get_log' ? [] : view(contract, method, args));
  const page = await render(dragon, ctx);
  assert.match(page, /<path class="line"/);
  assert.doesNotMatch(page, /class="buy"/);
  assert.match(page, /The Dragon bought 0 times in that window\./);
});

test('the hoard line names only what left it: Council grants and burns, and the auto-burn as the Dragon burning from its buys', async () => {
  const ctx = await enter({ mockSale: 'closed' });
  const view = world.view;
  const line = async (hoard) => {
    world.view = (contract, method, args) => {
      const r = view(contract, method, args);
      return method === 'get_state' ? { ...r, hoard: { ...r.hoard, granted: '0', burned: '0', auto_burned: '0', ...hoard } } : r;
    };
    return /<section aria-labelledby="hoard-h"[\s\S]*?<\/section>/.exec(await render(dragon, ctx))[0].match(/<p class="hint">([^<]*)<\/p>/)?.[1] ?? null;
  };
  const k = (n) => String(BigInt(n) * 10n ** 21n);
  assert.equal(await line({}), null, 'nothing left the hoard');
  assert.equal(await line({ granted: k(40) }), 'The Council granted 40,000 WYRM from the hoard.');
  assert.equal(await line({ burned: k(20) }), 'The Council burned 20,000 WYRM from the hoard.');
  assert.equal(await line({ auto_burned: k(5) }), 'The Dragon burned 5,000 WYRM from its buys.');
  assert.equal(await line({ granted: k(40), burned: k(20), auto_burned: k(5) }), 'The Council granted 40,000 WYRM and burned 20,000 WYRM from the hoard. The Dragon burned 5,000 WYRM from its buys.');
});

test("the leash list gives the Dragon's reason in WYRM and percent, with the raw text on hover", async () => {
  const page = await render(council, await enter({ mockSale: 'closed' }));
  const raw = 'grants in any 30 days are at most 100 bps of the hoard; 40000000000000000000000 already sent';
  assert.match(page, new RegExp(`<code title="${raw}">grants in any 30 days are at most 1% of the hoard; 40,000 WYRM already sent</code>`));
});

test('the coin and its launch id show on Verify and on no other page, and the shell has no coin strip', async () => {
  const shell = readFileSync(new URL('./index.html', import.meta.url), 'utf8');
  assert.doesNotMatch(shell, /id="coin"|coin-strip/, 'index.html');
  for await (const { name, where, page } of allPages()) {
    if (name === 'verify') continue;
    assert.doesNotMatch(page, /nearlytrade|3651/, where);
  }
  const page = await render(verify, await enter({ mockSale: 'closed' }));
  assert.match(page, /wyrm\.nearlytrade\.near<\/a><small>Nearly launch 3651\./);
});

const SCALES = config.contracts.scales;
function cells(page) {
  const hide = page.slice(page.indexOf('id="hide"'));
  return new Map([...hide.matchAll(/<li class="cell" data-status="(\w+)"[^>]*>(?:<a href="#\/scales\/\d+" aria-label="([^"]*)"|<span title="([^"]*)")/g)].map((m) => {
    const label = m[2] ?? m[3];
    return [String(Number(/Scale (\d+)/.exec(label)[1])), { kind: m[1], label }];
  }));
}

test("one status per Scale reads the same in the grid, on the Scale's page and in Yours", async () => {
  const ctx = { ...(await enter({ mockSale: 'closed' })), account: 'ember.near' };
  const mine = world.view(SCALES, 'get_scales', { from: 0, limit: 100 }).filter((t) => t.owner === 'ember.near' && t.listed_price === null)[0].token_id;
  world.call('ember.near', SCALES, 'list', { token_id: mine, price: String(3n * 10n ** 24n) }, 1n);
  const grid = await render(scales, ctx, ['all']);
  const hide = grid.slice(grid.indexOf('id="hide"'));
  assert.equal([...hide.matchAll(/<li class="cell" data-status="/g)].length, 100, 'the hide holds all 100');
  const all = cells(grid);
  assert.equal(all.size, 100);
  assert.equal(all.get(mine).kind, 'yours');
  assert.match(all.get(mine).label, /, yours, for sale at 3 NEAR$/);
  assert.match(await render(scale, ctx, [mine]), /<p class="status status-yours">Yours, for sale at 3 NEAR<\/p>/);
  assert.match(await render(scales, ctx, ['yours']), new RegExp(`<strong>Scale ${mine.padStart(3, '0')}</strong><small>Listed at 3 NEAR,`));

  const creatorListed = world.view(SCALES, 'get_scales', { from: 0, limit: 100 }).find((t) => Number(t.token_id) > 90 && t.listed_price !== null).token_id;
  assert.equal(all.get(creatorListed).kind, 'listed', "a creator's Scale that is listed is for sale");
  assert.match(all.get(creatorListed).label, /, for sale at [\d.]+ NEAR$/);
  assert.match(await render(scale, ctx, [creatorListed]), /<p class="status status-listed">For sale at [\d.]+ NEAR<\/p>/);
  assert.deepEqual(all.get('60'), { kind: 'retired', label: 'Scale 060, retired' });
  assert.match(await render(scale, ctx, ['60']), /<p class="status status-retired">Retired<\/p>/);
});

test('the Scales tabs: each tab names its panel, only the chosen one shows, and Yours needs a wallet', async () => {
  const ctx = await enter({ mockSale: 'open' });
  const page = await render(scales, ctx, ['market']);
  const tabs = [...page.matchAll(/<a role="tab" id="tab-(\w+)" href="#\/scales\/(\w+)" aria-controls="panel-(\w+)" aria-selected="(\w+)" tabindex="(-?\d)"/g)].map((m) => m.slice(1));
  assert.deepEqual(tabs.map((t) => t[0]), ['sale', 'market', 'all'], 'no Yours tab without a wallet');
  for (const [id, href, panel] of tabs) assert.ok(id === href && id === panel, id);
  assert.deepEqual(tabs.filter((t) => t[3] === 'true').map((t) => [t[0], t[4]]), [['market', '0']]);
  const shown = [...page.matchAll(/role="tabpanel" id="panel-(\w+)" aria-labelledby="tab-\w+" tabindex="0" (hidden)?>/g)].filter((m) => !m[2]).map((m) => m[1]);
  assert.deepEqual(shown, ['market']);
  assert.match(await render(scales, { ...ctx, account: 'ember.near' }, ['yours']), /aria-selected="true" tabindex="0">Yours <span class="count">\d+<\/span>/);
  const { tabOf } = scales;
  assert.equal(tabOf(undefined, null, { state: 'Open' }), 'sale', 'an open sale opens on the sale');
  assert.equal(tabOf(undefined, null, { state: 'Closed' }), 'market', 'a closed sale opens on the market');
  assert.equal(tabOf('yours', null, { state: 'Open' }), 'sale', 'Yours with no wallet falls back');
});
