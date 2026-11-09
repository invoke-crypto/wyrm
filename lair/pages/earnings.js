import { perScalePerDay } from '../domain.js';
import { formatNear } from '../format.js';
import { html } from '../ui.js';

let estimate = null;
export function load() {
  estimate ??= fetch(new URL('../fixtures/estimator.json', import.meta.url)).then((r) => r.json());
  return estimate;
}

const num = (x, digits) => x.toLocaleString('en-US', { maximumFractionDigits: digits, minimumFractionDigits: digits });
const near = (x) => x.toLocaleString('en-US', { maximumSignificantDigits: 3 });
const pct = (x, digits = 2) => `${num(x * 100, digits)}%`;

export function section(e, paid) {
  const rate = e.scenarios.tax_400.rate;
  const median = e.topCoinMedianDay.p50;
  return html`<section class="earnings" aria-labelledby="earn-h">
    <h2 id="earn-h">What a Scale earns</h2>
    <p class="earn-line"><label for="own-volume">At</label> <input id="own-volume" type="number" inputmode="decimal" min="0" step="any" value="${Math.round(median)}" aria-label="Daily trading volume in NEAR"> NEAR of trading a day,
      <output class="own-result" for="own-volume" data-rate="${rate}"></output></p>
    ${paid > 0n ? html`<p class="hint">Scales have been paid ${formatNear(paid)} NEAR so far.</p>` : ''}
    <details class="assume">
      <summary>How this is worked out</summary>
      <p>A Scale gets a hundredth of half of what the tithe receives, ${pct(rate, 3)} of each day's volume: 0.48% of buy volume (${pct(e.buy_share.pooled, 1)} of volume is buys) and 60% of the 4% token tax on every trade. The median day of the ${e.topCoinDailyVolume.n} top Nearly coins measured was ${num(median, 0)} NEAR. A retired Scale's share goes to the Dragon, so a held Scale's share never changes.</p>
      <ul>${e.assumptions.map((a) => html`<li>${a}</li>`)}</ul>
    </details>
  </section>`;
}

export function mount(root) {
  const input = root.querySelector('#own-volume');
  if (!input) return;
  const out = root.querySelector('.own-result');
  const rate = Number(out.dataset.rate);
  const update = () => {
    const v = Number(input.value);
    const day = perScalePerDay(v, rate);
    out.textContent = Number.isFinite(v) && v >= 0 ? `a Scale earns ${near(day)} NEAR a day, ${near(day * 30)} NEAR in 30 days.` : 'enter a volume in NEAR.';
  };
  input.addEventListener('input', update);
  update();
}
