import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chartModel, chartSvg, within } from './chart.js';

const MIN = 60_000;
const NS = 1_000_000n;
const D = 18;
const T1 = Date.UTC(2026, 10, 10, 15, 4);

// Samples every `every` minutes over `span` minutes, ending at T1, with prices from `price(i)` in NEAR per million.
function samples(span, every, price = (i) => 2.6 + 0.3 * Math.sin(i / 7)) {
  const out = [];
  for (let m = span, i = 0; m >= 0; m -= every, i++) out.push({ at: BigInt(T1 - m * MIN) * NS, tokensPerNear: BigInt(Math.round(1e6 / price(i))) * 10n ** 18n });
  return out;
}

const buyAt = (minutesAgo) => ({ event: 'dip_slice', at: BigInt(T1 - minutesAgo * MIN) * NS, amountIn: 10n ** 24n, quotedOut: 4n * 10n ** 23n });

function parse(svg) {
  const xs = [...svg.matchAll(/<text class="tick-x" data-at="(\d+)" x="([\d.]+)" y="([\d.]+)">([^<]*)<\/text>/g)].map((m) => ({ t: Number(m[1]), x: Number(m[2]), y: Number(m[3]), label: m[4] }));
  const ys = [...svg.matchAll(/<text class="tick-y" x="([\d.]+)" y="([\d.]+)">([^<]*)<\/text>/g)].map((m) => ({ x: Number(m[1]), y: Number(m[2]), label: m[3], value: Number(m[3].replace(/,/g, '')) }));
  const line = [.../<path class="line" d="([^"]*)"/.exec(svg)[1].matchAll(/[ML]([\d.]+) ([\d.]+)/g)].map((m) => ({ x: Number(m[1]), y: Number(m[2]) }));
  const base = Number(/<path class="area" d="[^"]*L[\d.]+ ([\d.]+)Z"/.exec(svg)[1]);
  const rows = [...svg.matchAll(/M0 ([\d.]+)H/g)].map((m) => Number(m[1]));
  const width = Number(/<svg[^>]* width="(\d+)"/.exec(svg)[1]);
  return { xs, ys, line, base, rows, width, coins: [...svg.matchAll(/<circle class="buy"/g)].length };
}

// 12 px text: about 7 px a character, an x label centred on its time, a price label ending at its x.
const box = (l, anchor) => {
  const w = l.label.length * 7;
  const left = anchor === 'middle' ? l.x - w / 2 : l.x - w;
  return { left, right: left + w, top: l.y - 10, bottom: l.y + 2 };
};
const overlap = (a, b) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

const SPANS = [['3 minutes', 3, 1], ['a live-like 20 minutes', 20, 5], ['6 hours', 360, 5], ['24 hours', 1440, 5], ['30 hours', 1800, 5]];
const WIDTHS = [358, 668, 1072];

test('the time axis is labelled from the start of the data to its end, at every span and width, with no label on another', () => {
  for (const [name, span, every] of SPANS) {
    for (const width of WIDTHS) {
      const where = `${name} at ${width} px`;
      const model = chartModel(samples(span, every), [], D);
      const { xs, ys } = parse(chartSvg(model, width, D));
      const [t0, t1] = [model.points[0].t, model.points.at(-1).t];
      assert.ok(xs.length >= 2, `${where}: ${xs.length} labels`);
      const step = xs[1].t - xs[0].t;
      for (let i = 1; i < xs.length; i++) assert.equal(xs[i].t - xs[i - 1].t, step, `${where}: evenly spaced`);
      assert.ok(xs[0].t >= t0 && xs.at(-1).t <= t1, `${where}: no label outside the data`);
      assert.ok(xs[0].t - t0 <= 1.25 * step, `${where}: the first label is near the start`);
      assert.ok(t1 - xs.at(-1).t < step, `${where}: the last label is within one step of the end`);
      for (const l of xs) {
        const b = box(l, 'middle');
        assert.ok(b.left >= 0 && b.right <= width, `${where}: ${l.label} fits the chart`);
        for (const p of ys) assert.ok(!overlap(b, box(p, 'end')), `${where}: ${l.label} clear of price ${p.label}`);
      }
      for (let i = 1; i < xs.length; i++) assert.ok(!overlap(box(xs[i - 1], 'middle'), box(xs[i], 'middle')), `${where}: ${xs[i - 1].label} clear of ${xs[i].label}`);
      for (const p of ys) assert.ok(box(p, 'end').left >= 0 && p.x <= width, `${where}: price ${p.label} inside the chart`);
    }
  }
});

test('six hours of samples ending at 15:04, 1072 px wide, are labelled every half hour through 15:00', () => {
  const { xs } = parse(chartSvg(chartModel(samples(360, 5), [], D), 1072, D));
  assert.deepEqual([xs[0].label, xs.at(-1).label], ['09:30', '15:00']);
});

test('the line runs through every sample and ends at the latest, on the right of the plot', () => {
  const model = chartModel(samples(360, 5), [], D);
  const { line } = parse(chartSvg(model, 668, D));
  assert.equal(line.length, model.points.length);
  assert.equal(Math.max(...line.map((v) => v.x)), line.at(-1).x);
});

test('the price labels bracket the line, and the line keeps a quarter gridline clear of the plot floor', () => {
  const model = chartModel(samples(60, 5, (i) => (i === 0 ? 3 : 2)), [], D);
  const { ys, line, base, rows } = parse(chartSvg(model, 668, D));
  assert.ok(Math.min(...ys.map((p) => p.value)) <= 2 && Math.max(...ys.map((p) => p.value)) >= 3, ys.map((p) => p.label).join(' '));
  const gap = Math.abs(rows[1] - rows[0]);
  assert.ok(base - Math.max(...line.map((v) => v.y)) >= gap / 4 - 0.1, 'the lowest price sits above the floor');
});

test("a range draws only that range's buys, and no buy outside the sampled span", () => {
  const log = [buyAt(-3), buyAt(60), buyAt(180), buyAt(1200), buyAt(1500)];
  const model = chartModel(samples(1440, 5), log, D);
  assert.equal(parse(chartSvg(model, 668, D)).coins, 3, 'not the buy before the first sample nor the one after the last');
  assert.equal(parse(chartSvg(within(model, '6h'), 668, D)).coins, 2, 'the last six hours');
});
