// Drawn at the shown pixel width, not scaled by viewBox, because viewBox scaling would scale the axis text too.
import { BUY_EVENTS, pricePerMillion } from './domain.js';
import { formatNear, formatToken } from './format.js';

const H = 260;
const PAD = { top: 16, right: 52, bottom: 30, left: 4 };
const MS = 1_000_000n;
const MIN_MS = 60_000;
const Y_MARGIN_STEPS = 0.25;
const TAP_PX = 12;
const TIME_STEPS = [1, 2, 5, 10, 15, 30, 60, 120, 180, 360, 720, 1440, 2880, 10080].map((m) => m * MIN_MS);

const fig = (p) => p.toLocaleString('en-US', { maximumSignificantDigits: 4 });
const hhmm = (t) => new Date(t).toISOString().slice(11, 16);
const DAY = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' });
const day = (t) => DAY.format(t);

function priceAt(points, t) {
  const i = points.findIndex((q) => q.t >= t);
  if (i <= 0) return points[Math.max(i, 0)].p;
  const [a, b] = [points[i - 1], points[i]];
  return a.p + ((b.p - a.p) * (t - a.t)) / (b.t - a.t || 1);
}

export function chartModel(samples, newestFirst, decimals) {
  const points = samples.map((s) => ({ t: Number(s.at / MS), p: pricePerMillion(s.tokensPerNear, decimals) }));
  if (points.length < 2) return { points, buys: [] };
  const [t0, t1] = [points[0].t, points.at(-1).t];
  const buys = newestFirst
    .filter((e) => BUY_EVENTS.has(e.event))
    .map((e) => ({ e, t: Number(e.at / MS) }))
    .filter((b) => b.t >= t0 && b.t <= t1)
    .reverse()
    .map((b) => ({ ...b, p: priceAt(points, b.t) }));
  return { points, buys };
}

const RANGES = [['6h', '6H', 6 * 60 * MIN_MS], ['24h', '24H', 24 * 60 * MIN_MS], ['all', 'All', Infinity]];

export function rangesOf({ points }) {
  const span = points.at(-1).t - points[0].t;
  return RANGES.filter(([, , ms]) => ms < span || ms === Infinity);
}

export function defaultRange(model) {
  const ranges = rangesOf(model).map(([id]) => id);
  if (!model.buys.length) return ranges.includes('24h') ? '24h' : 'all';
  return ranges.find((id) => within(model, id).buys.length === model.buys.length);
}

export function within(model, id) {
  const [, , ms] = RANGES.find((r) => r[0] === id);
  const from = model.points.at(-1).t - ms;
  return { points: model.points.filter((q) => q.t >= from), buys: model.buys.filter((b) => b.t >= from) };
}

function niceStep(range, count) {
  const raw = range / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  return [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= raw);
}

function yScale(points) {
  const ps = points.map((q) => q.p);
  const [lo, hi] = [Math.min(...ps), Math.max(...ps)];
  const step = niceStep(hi - lo || hi * 0.1 || 1, 4);
  const [y0, y1] = [Math.floor((lo - step * Y_MARGIN_STEPS) / step) * step, Math.ceil((hi + step * Y_MARGIN_STEPS) / step) * step];
  const ticks = [];
  for (let v = y0 + step; v < y1 - step / 2; v += step) ticks.push(v);
  const bottom = H - PAD.bottom;
  return { y: (p) => bottom - ((p - y0) / (y1 - y0)) * (bottom - PAD.top), ticks };
}

function frame({ points }, width) {
  const [t0, t1] = [points[0].t, points.at(-1).t];
  const x = (t) => PAD.left + ((t - t0) / (t1 - t0 || 1)) * (width - PAD.left - PAD.right);
  return { t0, t1, x, width, ...yScale(points) };
}

function timeTicks(t0, t1, width) {
  const most = Math.max(2, Math.floor(width / 84));
  const step = TIME_STEPS.find((s) => (t1 - t0) / s <= most) ?? TIME_STEPS.at(-1);
  const out = [];
  for (let t = Math.ceil(t0 / step) * step; t <= t1; t += step) out.push(t);
  return { ticks: out, daily: step >= 1440 * MIN_MS };
}

// The x labels sit on their own row under the plot, below the lowest price label, so only the left edge can clip one.
const labelHalf = (label) => label.length * 3.7;

function buyText(b, decimals) {
  return `The Dragon bought ${formatToken(b.e.quotedOut, decimals)} WYRM for ${formatNear(b.e.amountIn)} NEAR`;
}

export function pointText(q) {
  return `${fig(q.p)} NEAR per million WYRM`;
}

export const whenText = (t) => `${hhmm(t)} UTC, ${day(t)}`;

export function chartSvg(model, width, decimals) {
  const { points, buys } = model;
  const height = H;
  const { t0, t1, x, y, ticks } = frame(model, width);
  const line = points.map((q, i) => `${i ? 'L' : 'M'}${x(q.t).toFixed(1)} ${y(q.p).toFixed(1)}`).join('');
  const base = (height - PAD.bottom).toFixed(1);
  const area = `${line}L${x(t1).toFixed(1)} ${base}L${x(t0).toFixed(1)} ${base}Z`;
  const grid = ticks.map((v) => `M0 ${y(v).toFixed(1)}H${width - PAD.right + 8}`).join('');
  const time = timeTicks(t0, t1, width);
  const xLabel = (t) => (time.daily || hhmm(t) === '00:00' ? day(t) : hhmm(t));
  return `<svg class="chart-svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" aria-hidden="true">
<defs><linearGradient id="price-fill" x1="0" y1="0" x2="0" y2="1"><stop offset="0" class="fill-top"/><stop offset="1" class="fill-bottom"/></linearGradient></defs>
<path class="grid" d="${grid}"/>
${ticks.map((v) => `<text class="tick-y" x="${width}" y="${(y(v) + 4).toFixed(1)}">${fig(v)}</text>`).join('')}
${time.ticks.map((t) => [t, xLabel(t)]).filter(([t, label]) => x(t) >= labelHalf(label)).map(([t, label]) => `<text class="tick-x" data-at="${t}" x="${x(t).toFixed(1)}" y="${height - 8}">${label}</text>`).join('')}
<path class="area" d="${area}"/>
<path class="line" d="${line}"/>
<path class="cross" d="M0 0" hidden/>
${buys.map((b, i) => `<circle class="buy" data-buy="${i}" cx="${x(b.t).toFixed(1)}" cy="${y(b.p).toFixed(1)}" r="6"><title>${buyText(b, decimals)}, ${whenText(b.t)}</title></circle>`).join('')}
<circle class="dot" r="4.5" cx="0" cy="0" hidden/>
</svg>`;
}

export function chartSummary({ points, buys }) {
  const span = points.at(-1).t - points[0].t;
  const hours = Math.round(span / 3_600_000);
  return `WYRM's price over ${hours} hours, from ${fig(points[0].p)} to ${fig(points.at(-1).p)} NEAR per million WYRM. The Dragon bought ${buys.length} time${buys.length === 1 ? '' : 's'} in that window.`;
}

export function mountChart(figure, full, decimals) {
  const plot = figure.querySelector('.plot');
  const readout = figure.querySelector('.readout');
  if (!plot || full.points.length < 2) return;
  let model = within(full, figure.querySelector('[data-range][aria-pressed=true]')?.dataset.range ?? 'all');
  let index = model.points.length - 1;
  const first = plot.querySelector('svg');
  let geom = { svg: first, ...frame(model, Number(first.getAttribute('width'))) };
  const latest = () => {
    const q = model.points.at(-1);
    readout.innerHTML = `<strong>${pointText(q)}</strong> <span>${whenText(q.t)}</span>`;
  };

  const draw = (width) => {
    plot.innerHTML = chartSvg(model, width, decimals);
    geom = { svg: plot.querySelector('svg'), ...frame(model, width) };
  };

  const show = (q, buy) => {
    const { svg, x, y } = geom;
    const cx = x(q.t).toFixed(1);
    const cross = svg.querySelector('.cross');
    cross.setAttribute('d', `M${cx} ${PAD.top}V${H - PAD.bottom}`);
    cross.removeAttribute('hidden');
    const dot = svg.querySelector('.dot');
    dot.setAttribute('cx', cx);
    dot.setAttribute('cy', y(q.p).toFixed(1));
    dot.toggleAttribute('hidden', buy !== null);
    for (const c of svg.querySelectorAll('.buy')) c.classList.toggle('on', buy !== null && c.dataset.buy === String(model.buys.indexOf(buy)));
    readout.innerHTML = buy
      ? `<strong>${buyText(buy, decimals)}</strong> <span>${whenText(buy.t)}</span>`
      : `<strong>${pointText(q)}</strong> <span>${whenText(q.t)}</span>`;
  };

  const reset = () => {
    geom.svg.querySelector('.cross').setAttribute('hidden', '');
    geom.svg.querySelector('.dot').setAttribute('hidden', '');
    for (const c of geom.svg.querySelectorAll('.buy.on')) c.classList.remove('on');
    latest();
  };

  const nearest = (t) => {
    let best = 0;
    for (let i = 1; i < model.points.length; i++) if (Math.abs(model.points[i].t - t) < Math.abs(model.points[best].t - t)) best = i;
    return best;
  };

  const at = (clientX) => {
    const rect = geom.svg.getBoundingClientRect();
    const px = ((clientX - rect.left) / rect.width) * geom.width;
    const t = geom.t0 + ((px - PAD.left) / (geom.width - PAD.left - PAD.right)) * (geom.t1 - geom.t0);
    index = nearest(t);
    let buy = null;
    for (const b of model.buys) {
      const d = Math.abs(geom.x(b.t) - px);
      if (d <= TAP_PX && (!buy || d < Math.abs(geom.x(buy.t) - px))) buy = b;
    }
    show(buy ?? model.points[index], buy);
  };

  for (const b of figure.querySelectorAll('[data-range]')) {
    b.addEventListener('click', () => {
      for (const o of figure.querySelectorAll('[data-range]')) o.setAttribute('aria-pressed', String(o === b));
      model = within(full, b.dataset.range);
      index = model.points.length - 1;
      draw(geom.width);
      latest();
    });
  }
  plot.addEventListener('pointermove', (e) => at(e.clientX));
  plot.addEventListener('pointerdown', (e) => at(e.clientX));
  plot.addEventListener('pointerleave', (e) => e.pointerType === 'mouse' && reset());
  plot.addEventListener('blur', reset);
  plot.addEventListener('keydown', (e) => {
    const step = { ArrowLeft: -1, ArrowRight: 1, Home: -Infinity, End: Infinity }[e.key];
    if (step === undefined) return;
    e.preventDefault();
    index = Math.min(model.points.length - 1, Math.max(0, index + (e.shiftKey ? step * 12 : step)));
    const buy = model.buys.find((b) => nearest(b.t) === index) ?? null;
    show(buy ?? model.points[index], buy);
  });
  // The view drew the chart at a guessed width and CSS scales it to fit. The observer's first call comes after
  // layout and before paint, so the redraw at the real width costs no forced layout and no wrongly scaled frame.
  new ResizeObserver(([entry]) => {
    const width = Math.max(240, Math.round(entry.contentRect.width));
    if (Math.abs(width - geom.width) > 1) draw(width);
  }).observe(plot);
}
