import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  tierOf, nextTier, tally, bondReturns, canVoteOn, votableScales, firstVoteAt, royaltySplit, royaltyTo, saleSplit, statusOf, statusCounts, STATUSES, classOf,
  boundErrors, outflowWarning, titheRate, perScalePerDay, pricePerMillion, isClaiming, chunk, MOODS, moodOf,
} from './domain.js';
import { NS_PER_DAY } from './format.js';

const NOW = 1_000n * NS_PER_DAY;
const SEASON = 7n * NS_PER_DAY;
const NEAR = 10n ** 24n;
const bounds = JSON.parse(readFileSync(new URL('./fixtures/sandbox.json', import.meta.url), 'utf8'))['scales.get_council'].bounds;
const estimator = JSON.parse(readFileSync(new URL('./fixtures/estimator.json', import.meta.url), 'utf8'));

test('tier hardens at 3, 7 and 14 days of holding', () => {
  const held = (d) => tierOf(NOW - d, NOW);
  assert.equal(held(0n), 'Ash');
  assert.equal(held(3n * NS_PER_DAY - 1n), 'Ash');
  assert.equal(held(3n * NS_PER_DAY), 'Ember');
  assert.equal(held(7n * NS_PER_DAY), 'Molten');
  assert.equal(held(14n * NS_PER_DAY - 1n), 'Molten');
  assert.equal(held(14n * NS_PER_DAY), 'Obsidian');
  assert.deepEqual(nextTier(NOW - 4n * NS_PER_DAY, NOW), { name: 'Molten', at: NOW + 3n * NS_PER_DAY });
  assert.equal(nextTier(NOW - 20n * NS_PER_DAY, NOW), null);
});

const light = (electorate, yes, no) => tally({ class: 'Light', electorate, yes, no }, 10);
const strict = (electorate, yes, no) => tally({ class: 'Strict', electorate, yes, no }, 45);

test('Light passes with (yes+no)*10 >= electorate*3 and yes > no, from 10 Scales', () => {
  assert.equal(light(67, 11, 9).passing, false, '20 votes is under 30% of 67');
  assert.equal(light(67, 12, 9).passing, true, '21 votes reaches 30% of 67');
  assert.equal(light(70, 11, 10).passing, true, '21 of 70 is exactly 30%');
  assert.equal(light(70, 10, 10).passing, false, 'a tie fails');
  assert.equal(light(9, 9, 0).passing, false, 'under the minimum electorate');
  assert.equal(light(67, 13, 5).marks[0].at, 21);
});

test('Strict passes with two thirds turnout and 80% yes, from 45 Scales', () => {
  assert.equal(strict(50, 28, 7).passing, true, '35 of 50 turn out, 80% yes');
  assert.equal(strict(50, 27, 8).passing, false, '77% yes');
  assert.equal(strict(50, 33, 0).passing, false, '33 votes is under two thirds of 50');
  assert.equal(strict(50, 34, 0).passing, true, '34 of 50 is two thirds');
  assert.equal(strict(44, 44, 0).passing, false, 'under the minimum electorate');
  assert.equal(strict(50, 0, 0).marks[0].at, 34);
  assert.ok(strict(44, 44, 0).checks.some((c) => !c.ok && /45/.test(c.text)));
});

test('the vote class follows the order: Grant and Hunt on are Strict', () => {
  assert.equal(classOf({ type: 'Grant' }), 'Strict');
  assert.equal(classOf({ type: 'SetPosture', hunt: { sell_multiple_bps: 20000, max_sell_bps: 1 } }), 'Strict');
  for (const k of [{ type: 'SetPosture', hunt: null }, { type: 'SetBreath' }, { type: 'SetSlices' }, { type: 'Burn' }, { type: 'SetAutoBurn' }, { type: 'Signal' }]) assert.equal(classOf(k), 'Light', k.type);
});

test('bond returns at 30% turnout of the electorate', () => {
  assert.equal(bondReturns({ electorate: 70, yes: 1, no: 20 }), true);
  assert.equal(bondReturns({ electorate: 70, yes: 1, no: 19 }), false);
});

test('a sale Scale votes when held a season before the proposal opened, once', () => {
  const p = { createdAt: NOW };
  const a = { id: '1', voting: true, holdStart: NOW - SEASON };
  const b = { id: '2', voting: true, holdStart: NOW - SEASON + 1n };
  const creator = { id: '91', voting: false, holdStart: 0n };
  assert.equal(canVoteOn(a, p, SEASON), true);
  assert.equal(canVoteOn(b, p, SEASON), false);
  assert.equal(canVoteOn(creator, p, SEASON), false);
  assert.deepEqual(votableScales([a, b, creator], p, new Set(), SEASON).map((s) => s.id), ['1']);
  assert.deepEqual(votableScales([a], p, new Set(['1']), SEASON), []);
});

test('the first vote opens a season after the earliest-held sale Scale', () => {
  const minted = [{ voting: false, holdStart: 0n }, { voting: true, holdStart: NOW + 5n }, { voting: true, holdStart: NOW }];
  assert.equal(firstVoteAt(minted, SEASON), NOW + SEASON);
  assert.equal(firstVoteAt([{ voting: false, holdStart: 0n }], SEASON), null, "the creator's Scales never vote");
});

test('a claim or a mint sweep locks the token for 10 minutes', () => {
  assert.equal(isClaiming({ claimingSince: null }, NOW), false);
  assert.equal(isClaiming({ claimingSince: NOW - 9n * 60_000_000_000n }, NOW), true);
  assert.equal(isClaiming({ claimingSince: NOW - 10n * 60_000_000_000n }, NOW), false);
});

test('royalty is 5% with no yocto lost, to the creator or to the holders through the tithe', () => {
  const price = 4_200_000_000_000_000_000_000_001n;
  const { royalty, seller } = royaltySplit(price);
  assert.equal(royalty, 210_000_000_000_000_000_000_000n);
  assert.equal(royalty + seller, price);
  const accounts = { creator: 'creator.thewyrm.near', tithe: 'tithe.thewyrm.near' };
  assert.equal(royaltyTo('creator.thewyrm.near', accounts), 'creator');
  assert.equal(royaltyTo('tithe.thewyrm.near', accounts), 'holders');
});

test('a sale Scale pays the creator its cut, rounded down, and the Dragon the rest', () => {
  assert.deepEqual(saleSplit(2n * NEAR, 1000), { creator: NEAR / 5n, dragon: (9n * NEAR) / 5n });
  assert.deepEqual(saleSplit(19n, 1000), { creator: 1n, dragon: 18n });
});

const token = (owner, listedPrice = null) => ({ owner, listedPrice });
const OPEN = { state: 'Open', sold: 40 };
const CLOSED = { state: 'Closed', sold: 40 };
const at = (sale, account = null) => ({ sale, account, creator: 'creator.near' });

test("each Scale has one status: unsold, retired, held, for sale, creator's or yours", () => {
  assert.deepEqual(statusOf('41', null, at(OPEN, 'me.near')), { kind: 'unsold', price: null }, 'a sale id not bought yet');
  assert.equal(statusOf('1', null, at({ state: 'Waiting', sold: 0 })).kind, 'unsold', 'before the sale opens');
  assert.deepEqual(statusOf('41', null, at(CLOSED)), { kind: 'retired', price: null }, 'unbought when the sale closed');
  assert.deepEqual(statusOf('7', token('kestrel.near'), at(OPEN, 'me.near')), { kind: 'held', price: null });
  assert.deepEqual(statusOf('7', token('kestrel.near', 5n * NEAR), at(CLOSED, 'me.near')), { kind: 'listed', price: 5n * NEAR });
  assert.deepEqual(statusOf('95', token('creator.near'), at(CLOSED)), { kind: 'creator', price: null });
  assert.deepEqual(statusOf('95', token('creator.near', 6n * NEAR), at(CLOSED)), { kind: 'listed', price: 6n * NEAR }, "a creator's Scale that is listed is for sale");
  assert.deepEqual(statusOf('95', token('alice.near'), at(CLOSED)), { kind: 'held', price: null }, "a creator's Scale sold on is held like any other");
  assert.deepEqual(statusOf('7', token('creator.near'), at(CLOSED)), { kind: 'held', price: null }, 'a sale Scale the creator bought is held');
  assert.deepEqual(statusOf('7', token('me.near'), at(OPEN, 'me.near')), { kind: 'yours', price: null });
  assert.deepEqual(statusOf('7', token('me.near', 3n * NEAR), at(OPEN, 'me.near')), { kind: 'yours', price: 3n * NEAR }, 'yours and listed keeps its price');
  assert.equal(statusOf('7', token('me.near'), at(OPEN)).kind, 'held', 'with no wallet nobody is you');
});

test("after a close with 40 sold the 100 statuses split 40 held, 50 retired and 10 creator's", () => {
  const all = Array.from({ length: 100 }, (_, i) => statusOf(String(i + 1), i < 40 || i >= 90 ? token(i >= 90 ? 'creator.near' : 'h.near') : null, at(CLOSED)));
  assert.deepEqual(statusCounts(all), { yours: 0, listed: 0, held: 40, creator: 10, unsold: 0, retired: 50 });
  assert.deepEqual(Object.keys(STATUSES), Object.keys(statusCounts([])), 'every status has a label');
});

test("the form's bounds are the preset's: auto-burn stops at 50%, Hunt's sell line starts at 200%", () => {
  assert.deepEqual(boundErrors('SetAutoBurn', { bps: 5000 }, bounds), []);
  assert.equal(boundErrors('SetAutoBurn', { bps: 5001 }, bounds).length, 1);
  assert.deepEqual(boundErrors('SetSlices', { cap_bps: 50 }, bounds), []);
  assert.deepEqual(boundErrors('SetSlices', { cap_bps: 201 }, bounds), ['Price impact cap per slice must be 0.5% to 2%.']);
  assert.equal(boundErrors('SetBreath', { budget_bps: 999 }, bounds).length, 1);
  assert.deepEqual(boundErrors('SetPosture on', { sell_multiple_bps: 19999, max_sell_bps: 1501 }, bounds), ['Sell line must be 200% or more.', 'Most sold per day must be 0.01% to 15%.']);
  assert.deepEqual(boundErrors('Signal', {}, bounds), []);
});

test('a Grant or Burn over 0.5% of the hoard is warned, not refused', () => {
  const ctx = { hoardWyrm: 10_000n, outflowBps: 50, windowBps: 100 };
  assert.equal(outflowWarning('Grant', { amount: 50n }, ctx), null);
  assert.match(outflowWarning('Grant', { amount: 51n }, ctx), /0.5% of the hoard, and 1% in 30 days/);
  assert.match(outflowWarning('Burn', { amount: 0n }, ctx), /more than 0/);
  assert.equal(outflowWarning('SetSlices', { cap_bps: 50 }, ctx), null);
});

test("the estimator counts 0.48% of buy volume plus 60% of the 4% tax, and a Scale's hundredth of half", () => {
  const share = estimator.buy_share.pooled;
  assert.ok(Math.abs(titheRate(share, 400) - estimator.scenarios.tax_400.rate) < 1e-6, 'tax_400 rate');
  assert.ok(Math.abs(titheRate(share, 0) - estimator.scenarios.no_tax.rate) < 1e-6, 'no_tax rate');
  for (const b of estimator.brackets) {
    const per = perScalePerDay(b.daily_volume_near, estimator.scenarios.tax_400.rate);
    assert.ok(Math.abs(per - b.per_scale_near_per_day.tax_400) / per < 1e-3, `${b.daily_volume_near} NEAR a day`);
  }
});

test('a sample of 300,000 WYRM per NEAR is a price of 3.33 NEAR per million WYRM', () => {
  assert.ok(Math.abs(pricePerMillion(300_000n * 10n ** 18n) - 10 / 3) < 1e-9);
});

test('the Lair has a line for Sleeping, Breathing and Hunting, and no Stirring or Gorged mood', () => {
  assert.deepEqual(Object.keys(MOODS), ['Sleeping', 'Breathing', 'Hunting']);
  for (const m of Object.keys(MOODS)) assert.ok(MOODS[m].line);
});

test("the mood is the Dragon's last swap in the log: a buy or a sell in the last 30 minutes, else Sleeping", () => {
  const MIN = 60_000_000_000n;
  const e = (event, minutesAgo) => ({ event, at: NOW - BigInt(minutesAgo) * MIN });
  assert.equal(moodOf([e('breath_slice', 10)], NOW), 'Breathing', 'a buy 10 minutes ago');
  assert.equal(moodOf([e('dip_slice', 10)], NOW), 'Breathing', 'a dip buy 10 minutes ago');
  assert.equal(moodOf([e('breath_slice', 40)], NOW), 'Sleeping', 'a buy 40 minutes ago');
  assert.equal(moodOf([e('dip_slice', 30)], NOW), 'Sleeping', 'a buy exactly 30 minutes ago');
  assert.equal(moodOf([e('hunt_slice', 10)], NOW), 'Hunting', 'a sell 10 minutes ago');
  assert.equal(moodOf([e('hunt_slice', 40)], NOW), 'Sleeping', 'a sell 40 minutes ago');
  assert.equal(moodOf([], NOW), 'Sleeping', 'an empty log');
  assert.equal(moodOf([e('hunt_slice', 2), e('breath_slice', 5)], NOW), 'Hunting', 'the newest swap decides');
  assert.equal(moodOf([e('breath_slice', 2), e('hunt_slice', 5)], NOW), 'Breathing', 'the newest swap decides');
  assert.equal(moodOf([e('auto_burn', 1), e('swap_failed', 2), e('breath_slice', 3)], NOW), 'Breathing', 'a burn or failed swap after a buy keeps it Breathing');
  assert.equal(moodOf([e('grant_paid', 1), e('swap_failed', 2), e('breath_slice', 45)], NOW), 'Sleeping', 'only a swap that happened counts');
  assert.equal(moodOf([e('breath_slice', 29)], NOW), 'Breathing', 'a buy 29 minutes ago');
  assert.equal(moodOf([e('auto_burn', 1), e('breath_slice', 40)], NOW), 'Sleeping', 'a late auto-burn is not a buy');
});

test('votes go in batches of at most 25 tokens', () => {
  assert.deepEqual(chunk([...Array(26).keys()], 25).map((c) => c.length), [25, 1]);
});
