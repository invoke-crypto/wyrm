import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { formatReason, formatNear, parseNearInput, formatToken, parseTokenInput, formatBps, percentToBps, formatSpan, formatLength, shortAccount, padScaleId } from './format.js';

const NEAR = 10n ** 24n;

test('NEAR amounts format from yocto with grouping and rounding', () => {
  assert.equal(formatNear(0n), '0');
  assert.equal(formatNear(NEAR), '1');
  assert.equal(formatNear(1_234_567n * NEAR / 1000n), '1,234.57');
  assert.equal(formatNear(NEAR / 2n), '0.5');
  assert.equal(formatNear(1n), '<0.01');
  assert.equal(formatNear(4_199_999n * NEAR / 1_000_000n, 3), '4.2');
  assert.equal(formatNear(-NEAR), '-1');
});

test('typed NEAR parses to exact yocto, never through floats', () => {
  assert.equal(parseNearInput('1'), NEAR);
  assert.equal(parseNearInput('0.1'), NEAR / 10n);
  assert.equal(parseNearInput(' 4.2 '), 42n * NEAR / 10n);
  assert.equal(parseNearInput('0.000000000000000000000001'), 1n);
  for (const bad of ['', '1.2.3', '-1', 'abc', '1e3', '0.0000000000000000000000001']) assert.equal(parseNearInput(bad), null, bad);
});

test('tokens round-trip at their decimals', () => {
  assert.equal(formatToken(18_400_000n * 10n ** 18n, 18), '18,400,000');
  assert.equal(parseTokenInput('60000', 18), 60_000n * 10n ** 18n);
  assert.equal(parseTokenInput('1.5', 18), 15n * 10n ** 17n);
  assert.equal(parseTokenInput('x', 18), null);
});

test('basis points show as percent and parse back', () => {
  assert.equal(formatBps(3000), '30%');
  assert.equal(formatBps(250), '2.5%');
  assert.equal(formatBps(25000), '250%');
  assert.equal(percentToBps('2.5'), 250);
  assert.equal(percentToBps('90'), 9000);
  assert.equal(percentToBps('1.234'), null);
});

test('spans, accounts and ids', () => {
  assert.equal(formatSpan(90n * 1_000_000_000n), '1m');
  assert.equal(formatSpan(26n * 3600n * 1_000_000_000n), '1d 2h');
  assert.equal(shortAccount('ember.near'), 'ember.near');
  assert.equal(shortAccount('a'.repeat(64)), 'aaaaaa…aaaa');
  assert.equal(padScaleId(7), '007');
});

test('preset lengths read in words', () => {
  const MIN = 60_000_000_000n;
  assert.equal(formatLength(7n * 1440n * MIN), '7 days');
  assert.equal(formatLength(1440n * MIN), '1 day');
  assert.equal(formatLength(120n * MIN), '2 hours');
  assert.equal(formatLength(10n * MIN), '10 minutes');
});

test("the Dragon's outflow refusals read in WYRM and percent", () => {
  const rust = readFileSync(new URL('../contracts/dragon/src/limits.rs', import.meta.url), 'utf8');
  const fill = { what: 'grant', OUTFLOW_BPS: '50', OUTFLOW_WINDOW_BPS: '100', hoard: String(400_000_000n * 10n ** 18n), window_sum: String(40_000n * 10n ** 18n) };
  const reasons = [...rust.matchAll(/format!\(\s*"([^"]+)"/g)].map((m) => m[1].replace(/\{(\w+)\}/g, (_, k) => fill[k]));
  assert.deepEqual(reasons.map((r) => formatReason(r)), [
    'a grant is at most 0.5% of the hoard (400,000,000 WYRM)',
    'grants in any 30 days are at most 1% of the hoard; 40,000 WYRM already sent',
  ]);
  assert.equal(formatReason('launch 3651 is not done'), 'launch 3651 is not done');
});

test("the Dragon's launch deposit refusal reads in NEAR rounded up, and only numbers of 19 or more digits are WYRM", () => {
  const rust = readFileSync(new URL('../contracts/dragon/src/nearly.rs', import.meta.url), 'utf8');
  const [, deposit] = /format!\(\s*"(attach at least \{cost\} yoctoNEAR[^"]*)"/.exec(rust);
  assert.equal(formatReason(deposit.replace('{cost}', String(5_650_000n * 10n ** 18n))), "attach at least 5.65 NEAR, Nearly's quote plus the margin");
  assert.equal(formatReason(deposit.replace('{cost}', String(565n * 10n ** 22n + 1n))), "attach at least 5.6501 NEAR, Nearly's quote plus the margin");
  assert.equal(formatReason('sent 123456789012345678 back'), 'sent 123456789012345678 back');
  assert.equal(formatReason('sent 1234567890123456789 back'), 'sent 1.23 WYRM back');
});
