import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { SCENARIOS } from './fixtures/generate.mjs';

const NEAR = 10n ** 24n;
const NS_DAY = 86_400_000_000_000n;
const near = (x) => (BigInt(Math.round(x * 1000)) * NEAR) / 1000n;
const read = (name) => JSON.parse(readFileSync(new URL(`./fixtures/${name}.json`, import.meta.url), 'utf8'));
const recorded = read('sandbox');
const worlds = Object.keys(SCENARIOS).map((name) => [name, read(`world-${name}`)]);
const [price] = new Set(recorded['scales.get_sale'].map((s) => s.price));
const states = (w) => Object.values(w.dragon.states).map((s) => s.state);

test('the mock worlds sell at the price the sandbox contracts recorded', () => {
  assert.equal(new Set(recorded['scales.get_sale'].map((s) => s.price)).size, 1);
  for (const [name, w] of worlds) {
    assert.equal(w.sale.price, price, `${name} sale`);
    for (const [id, history] of Object.entries(w.scales.history)) {
      for (const { Minted } of history.filter((e) => e.Minted)) assert.equal(Minted.price, Number(id) > 90 ? '0' : price, `${name} Scale ${id}`);
    }
  }
  const fixed = read('fixed');
  assert.equal(fixed.presets.hoarder.sale.price, price);
  assert.equal(fixed.dragonConfig.preset_value.sale.price, price);
});

test("each mock Dragon's reserve is its share of the Scales sold plus its other flows", () => {
  for (const [name, w] of worlds) {
    const { sold, held, out } = SCENARIOS[name];
    assert.equal(w.sale.sold, sold);
    const opening = (BigInt(sold) * BigInt(price) * BigInt(10_000 - w.sale.creator_bps)) / 10_000n + near(held);
    for (const st of states(w)) {
      assert.equal(BigInt(st.day.opening_reserve), w.dragon.coin ? opening : 0n, `${name} ${st.mood} opening reserve`);
      assert.equal(BigInt(st.reserve), opening - near(out), `${name} ${st.mood} reserve`);
    }
  }
  const closed = Object.fromEntries(worlds).closed;
  assert.equal(read('accounts')['dragon.thewyrm.near'].amount, states(closed)[0].reserve, "the Dragon account holds the closed world's reserve");
});

test("each mock Dragon's slices fit the day's budget, failed ones included", () => {
  for (const [name, w] of worlds) {
    for (const st of states(w)) {
      assert.equal(BigInt(st.day.budget), (BigInt(st.day.opening_reserve) * BigInt(st.strategy.budget_bps)) / 10_000n, `${name} ${st.mood} budget`);
      assert.ok(BigInt(st.day.spent) <= BigInt(st.day.budget), `${name} ${st.mood} spent`);
    }
    const days = new Map();
    for (const { event, at, data } of w.dragon.log.filter((e) => e.event === 'breath_slice' || e.event === 'dip_slice')) {
      const day = BigInt(at) / NS_DAY;
      if (day === BigInt(states(w)[0].day.index)) assert.equal(data.budget, states(w)[0].day.budget, `${name} ${event} at ${at}`);
      const d = days.get(day) ?? { budget: BigInt(data.budget), sum: 0n };
      d.sum += BigInt(data.amount_in);
      days.set(day, d);
    }
    for (const [day, { budget, sum }] of days) assert.ok(sum <= budget, `${name} day ${day}: slices ${sum} over the budget ${budget}`);
  }
});
