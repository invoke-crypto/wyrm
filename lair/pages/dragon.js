import { getDragon } from '../chain.js';
import { config } from '../config.js';
import { MOODS, BUY_EVENTS, moodOf } from '../domain.js';
import { formatReason, formatNear, formatToken, formatBps, formatSpan, formatUtc, shortAccount } from '../format.js';
import { chartModel, chartSvg, chartSummary, mountChart, pointText, whenText, rangesOf, defaultRange, within } from '../chart.js';
import { dragonSvg } from '../art.js';
import { html, raw } from '../ui.js';

const D = config.wyrmDecimals;
const wyrm = (x) => formatToken(x, D);
const NEAR = 10n ** 24n;
const reason = (text) => html`<span title="${text}">${formatReason(text, D)}</span>`;

export const title = (d) => `The Dragon is ${d.mood.toLowerCase()}`;
export const load = async (_, { now }) => {
  const d = await getDragon();
  return { ...d, mood: moodOf(d.log, now) };
};

const bought = (e) => `Bought ${wyrm(e.quotedOut)} WYRM for ${formatNear(e.amountIn)} NEAR.`;

const LOG = {
  breath_slice: bought,
  dip_slice: bought,
  hunt_slice: (e) => `Sold ${wyrm(e.amountIn)} WYRM for ${formatNear(e.quotedOut)} NEAR.`,
  swap_failed: (e) =>
    e.slice === 'Hunt'
      ? `A sale of ${wyrm(e.amountIn)} WYRM failed, and the WYRM went back to the hoard.`
      : `A buy with ${formatNear(e.amountIn)} NEAR failed, and the NEAR went back to the reserve.`,
  grant_paid: (e) => `Granted ${wyrm(e.amount)} WYRM to ${shortAccount(e.to)}, as the Council ordered.`,
  burn_paid: (e) => `Burned ${wyrm(e.amount)} WYRM to ash, as the Council ordered.`,
  auto_burn: (e) => `Burned ${wyrm(e.amount)} WYRM from its buys.`,
  outflow_failed: (e) => `A ${e.kind} of ${wyrm(e.amount)} WYRM failed, and the WYRM went back.`,
  settings_changed: (e) =>
    `The Council set the budget to ${formatBps(e.strategy.budgetBps)}, the slice cap to ${formatBps(e.strategy.capBps)}, Hunt ${e.strategy.hunt ? 'on' : 'off'} and auto-burn to ${formatBps(e.strategy.autoBurnBps)}.`,
  order_refused: (e) => html`Refused a Council order: ${reason(e.reason)}.`,
  launch_sent: (e) => (e.ok ? 'Sent the coin launch to Nearly.' : 'Sent the coin launch to Nearly. It may have parked, so it needs a resume.'),
  launch_refused: (e) => html`Refused a launch: ${reason(e.reason)}.`,
  bind_refused: (e) => html`Refused to bind launch ${e.launchId}: ${reason(e.reason)}.`,
  bound: () => 'Bound its coin and opened the Scale sale.',
};

function averageCost(h) {
  return h.wyrmBought === 0n ? null : (h.nearSpent * 10n ** BigInt(D) * 1_000_000n) / h.wyrmBought;
}

// Grants and burns are Council orders; auto-burn is the share of each buy the Dragon burns on its own.
function outflows({ granted, burned, autoBurned }) {
  const council = [granted && `granted ${wyrm(granted)} WYRM`, burned && `burned ${wyrm(burned)} WYRM`].filter(Boolean);
  return [
    council.length ? `The Council ${council.join(' and ')} from the hoard.` : '',
    autoBurned ? `The Dragon burned ${wyrm(autoBurned)} WYRM from its buys.` : '',
  ].filter(Boolean).join(' ');
}

const CHART_WIDTH = 720;
const LOG_SHOWN = 8;

function priceChart(full) {
  if (full.points.length < 2) return html`<p class="empty">The Dragon has not sampled the price yet.</p>`;
  const range = defaultRange(full);
  const model = within(full, range);
  const last = model.points.at(-1);
  return html`<figure class="chart">
    <figcaption class="chart-head">
      <p class="readout" aria-live="polite"><strong>${pointText(last)}</strong> <span>${whenText(last.t)}</span></p>
      <p class="chart-key"><span class="key-coin" aria-hidden="true"></span>A buy by the Dragon</p>
      ${rangesOf(full).length > 1 ? html`<div class="ranges" role="group" aria-label="Time span">${rangesOf(full).map(([id, label]) => html`<button class="chip" data-range="${id}" aria-pressed="${String(id === range)}">${label}</button>`)}</div>` : ''}
    </figcaption>
    <div class="plot" role="img" tabindex="0" aria-label="${chartSummary(model)}">${raw(chartSvg(model, CHART_WIDTH, D))}</div>
  </figure>`;
}

function logItem(e, now) {
  return html`<li class="ev-${BUY_EVENTS.has(e.event) ? 'buy' : e.event}"><time datetime="${new Date(Number(e.at / 1_000_000n)).toISOString()}" title="${formatUtc(e.at)}">${formatSpan(now - e.at)} ago</time><span>${LOG[e.event]?.(e) ?? e.event}</span></li>`;
}

function hero(s, mood) {
  const lede = s.coin
    ? 'WYRM is a coin on NEAR. Half of every trading fee feeds the Dragon, a contract nobody holds a key to, and it buys WYRM and keeps it. The other half pays 100 Scales.'
    : 'WYRM is a coin on NEAR, and the Dragon has none yet. It wakes when its launch is bound, and the Scale sale opens then.';
  return html`<div class="hero">
    <div class="hero-art">${raw(dragonSvg(mood, Math.min(1, Number(s.reserve / NEAR) / 300)))}</div>
    <div class="hero-text">
      <h1 tabindex="-1" class="mood-word">${mood}</h1>
      <p class="mood-line">${MOODS[mood].line}</p>
      <p class="lede">${lede}</p>
      <div class="actions"><a class="btn btn-primary" href="#/scales">Get a Scale</a><a class="btn btn-quiet" href="#/verify">Check it yourself</a></div>
    </div>
  </div>`;
}

export function view({ state: s, samples, log, mood }, { now }) {
  const avg = averageCost(s.hoard);
  const gone = outflows(s.hoard);
  const model = chartModel(samples, log, D);
  return html`<section class="page page-dragon">
    ${hero(s, mood)}
    <section aria-labelledby="hoard-h" class="hoard">
      <h2 id="hoard-h" class="sr">The hoard</h2>
      <dl class="figures">
        <div><dt>In reserve</dt><dd>${formatNear(s.reserve)}<small>NEAR</small></dd></div>
        <div><dt>In the hoard</dt><dd>${wyrm(s.hoard.wyrm)}<small>WYRM</small></dd></div>
        <div><dt>Spent on WYRM</dt><dd>${formatNear(s.hoard.nearSpent)}<small>NEAR</small></dd></div>
        <div><dt>Average cost</dt><dd>${avg === null ? 'none yet' : formatNear(avg, 3)}<small>${avg === null ? '' : 'NEAR per million'}</small></dd></div>
      </dl>
      ${gone ? html`<p class="hint">${gone}</p>` : ''}
    </section>

    <section aria-labelledby="price-h">
      <h2 id="price-h">Price</h2>
      ${priceChart(model)}
    </section>

    <section aria-labelledby="log-h">
      <h2 id="log-h">What the Dragon did</h2>
      ${log.length
        ? html`<ol class="log">${log.slice(0, LOG_SHOWN).map((e) => logItem(e, now))}</ol>
          ${log.length > LOG_SHOWN ? html`<details class="more"><summary>Show all ${log.length}</summary><ol class="log">${log.slice(LOG_SHOWN).map((e) => logItem(e, now))}</ol></details>` : ''}`
        : html`<p class="empty">No launch, buys or Council orders yet.</p>`}
    </section>
  </section>`;
}

// The mood tints the whole page while the Dragon is on screen; app.js clears it on every other page.
export function mounted(root, { samples, log, mood }) {
  document.body.dataset.mood = mood;
  const figure = root.querySelector('.chart');
  if (figure) mountChart(figure, chartModel(samples, log, D), D);
}
