import { getProposals, getVotedTokens, getScales, getSale, getElectorateNow, getDragonState, getDragonConfig, getCouncil, tx } from '../chain.js';
import { config } from '../config.js';
import {
  KINDS, CLASSES, tally, votableScales, canPropose, firstVoteAt, boundOf, boundErrors, outflowWarning, bondReturns, DESCRIPTION_MAX_BYTES,
} from '../domain.js';
import { formatReason, formatNear, formatToken, formatBps, formatIn, formatAgo, formatUtc, formatLength, shortAccount, padScaleId, percentToBps, parseTokenInput } from '../format.js';
import { html, toast } from '../ui.js';

export const title = () => 'The Council';

export async function load(_, { account }) {
  const [proposals, scales, sale, electorate, dragon, dragonConfig, council] = await Promise.all([
    getProposals(), getScales(), getSale(), getElectorateNow(), getDragonState(), getDragonConfig(), getCouncil(),
  ]);
  const open = proposals.filter((p) => p.status.type === 'Open');
  const voted = new Map(await Promise.all(open.map(async (p) => [p.id, await getVotedTokens(p.id)])));
  return {
    proposals, voted, scales, sale, electorate, council,
    mine: scales.filter((s) => s.owner === account),
    strategy: dragon.strategy,
    outflow: { hoardWyrm: dragon.hoard.wyrm, outflowBps: dragonConfig.outflowBps, windowBps: dragonConfig.windowBps },
  };
}

const STATUS = {
  Open: 'Voting',
  Rejected: 'Rejected',
  Carried: 'Carried',
  Executing: 'Running on the Dragon',
  Executed: 'Done',
  LeashHeld: 'The leash held',
};

const kindName = (k) => (k.type === 'SetPosture' ? `SetPosture ${k.hunt ? 'on' : 'off'}` : k.type);
const wyrm = (x) => formatToken(x, config.wyrmDecimals);
const leashReason = (text) => html`<code title="${text}">${formatReason(text, config.wyrmDecimals)}</code>`;

function describeKind(k) {
  switch (k.type) {
    case 'SetBreath':
      return `Daily budget ${formatBps(k.budget_bps)} of the reserve`;
    case 'SetSlices':
      return `Each slice moves the price by at most ${formatBps(k.cap_bps)}`;
    case 'SetPosture':
      return k.hunt ? `Hunt on: sell above ${formatBps(k.hunt.sell_multiple_bps)} of cost, at most ${formatBps(k.hunt.max_sell_bps)} of the hoard a day` : 'Hunt off: guard the hoard';
    case 'Grant':
      return `${wyrm(k.amount)} WYRM to ${k.to}`;
    case 'Burn':
      return `Burn ${wyrm(k.amount)} WYRM to ash`;
    case 'SetAutoBurn':
      return `Burn ${formatBps(k.bps)} of every buy`;
    case 'Signal':
      return 'A text poll';
  }
}

function classRule(cls, council) {
  const c = council.classes[cls];
  return `${CLASSES[cls].text}, with an electorate of at least ${c.minElectorate} sold Scales${c.needsClosedSale ? ', after the sale closes' : ''}`;
}

function bar(label, parts, max, marks) {
  const pct = (n) => `${Math.min(100, (n / Math.max(max, 1)) * 100)}%`;
  return html`<div class="bar-row">
    <span class="bar-label">${label}</span>
    <div class="bar" role="img" aria-label="${label}: ${parts.map((p) => `${p.n} ${p.cls}`).join(', ')} of ${max}. ${marks.map((m) => m.label).join('. ')}.">
      ${parts.map((p) => html`<span class="seg seg-${p.cls}" style="width:${pct(p.n)}"></span>`)}
      ${marks.map((m) => html`<span class="mark" style="left:${pct(m.at)}"><span>${m.label}</span></span>`)}
    </div>
  </div>`;
}

function tallyView(p, council) {
  const t = tally(p, council.classes[p.class].minElectorate);
  return html`<div class="tally">
    ${bar('Votes', [{ cls: 'yes', n: p.yes }, { cls: 'no', n: p.no }], p.electorate, t.marks)}
    <p class="counts"><span class="k-yes">${p.yes} yes</span> <span class="k-no">${p.no} no</span> <span>of ${p.electorate} Scales that could vote</span></p>
    <ul class="checks">${t.checks.map((c) => html`<li class="${c.ok ? 'ok' : 'nok'}">${c.text}</li>`)}</ul>
  </div>`;
}

function voteControls(p, d, account, now) {
  if (p.endsAt <= now) return html`<button class="btn" data-act="finalize" data-id="${p.id}">Count the votes</button><p class="hint">Voting closed ${formatAgo(p.endsAt, now)}. Anyone can count it, and a passed order reaches the Dragon in the same transaction.</p>`;
  if (!account) return html`<p class="hint">Connect a wallet to vote with your Scales.</p>`;
  const can = votableScales(d.mine, p, d.voted.get(p.id) ?? new Set(), d.council.seasonNs);
  if (!can.length) {
    const any = d.mine.some((s) => s.voting);
    return html`<p class="hint">${any ? `None of your Scales can vote on this. A Scale votes once per proposal, and only if you held it ${formatLength(d.council.seasonNs)} before the proposal opened.` : 'You hold no Scale with a vote.'}</p>`;
  }
  const ids = can.map((s) => padScaleId(s.id)).join(', ');
  return html`<div class="actions">
    <button class="btn btn-yes" data-act="vote" data-id="${p.id}" data-approve="1">Yes with ${can.length} Scale${can.length > 1 ? 's' : ''}</button>
    <button class="btn btn-no" data-act="vote" data-id="${p.id}" data-approve="0">No with ${can.length}</button>
  </div><p class="hint">Votes with Scale ${ids}.</p>`;
}

function proposalCard(p, d, account, now) {
  const label = KINDS[kindName(p.kind)].label;
  return html`<article class="proposal status-${p.status.type}" aria-labelledby="p-${p.id}">
    <header>
      <h3 id="p-${p.id}"><span class="pid">#${p.id}</span> ${label} <span class="cls cls-${p.class}">${p.class}</span></h3>
      <span class="status">${STATUS[p.status.type]}</span>
    </header>
    <p class="kind-line">${describeKind(p.kind)}</p>
    ${p.description ? html`<blockquote>${p.description}</blockquote>` : ''}
    <p class="meta">By ${shortAccount(p.proposer)}. ${p.status.type === 'Open' ? `Voting ends ${formatIn(p.endsAt, now)}, at ${formatUtc(p.endsAt)}.` : `Opened ${formatUtc(p.createdAt)}.`}</p>
    ${tallyView(p, d.council)}
    ${p.status.type === 'Open' ? voteControls(p, d, account, now) : ''}
    ${p.status.type === 'LeashHeld' ? html`<p class="leash-error">The Dragon refused it: ${leashReason(p.status.error)}</p>` : ''}
    ${p.status.type !== 'Open' && p.status.type !== 'Executing' ? html`<p class="hint">Bond ${bondReturns(p) ? 'returned to the proposer' : 'went to the Dragon, turnout stayed under 30%'}.</p>` : ''}
  </article>`;
}

function field(f, kind, d) {
  const id = `f-${kind.replace(' ', '-')}-${f.key}`;
  if (f.type === 'account')
    return html`<label class="field" for="${id}">${f.label}<input id="${id}" name="${f.key}" required autocomplete="off" spellcheck="false" pattern="[a-z0-9_\\-.]{2,64}" placeholder="name.near"></label>`;
  if (f.type === 'wyrm') {
    const cap = (d.outflow.hoardWyrm * BigInt(d.outflow.outflowBps)) / 10_000n;
    return html`<label class="field" for="${id}">${f.label} <small>${f.unit}, at most ${wyrm(cap)} (${formatBps(d.outflow.outflowBps)} of the hoard), ${formatBps(d.outflow.windowBps)} in 30 days</small>
      <input id="${id}" name="${f.key}" inputmode="decimal" required placeholder="${wyrm(cap / 2n).replace(/,/g, '')}"></label>`;
  }
  const { lo, hi } = boundOf(f, d.council.bounds);
  const current = { budget_bps: d.strategy.budgetBps, cap_bps: d.strategy.capBps, bps: d.strategy.autoBurnBps, sell_multiple_bps: d.strategy.hunt?.sellMultipleBps ?? 25_000, max_sell_bps: d.strategy.hunt?.maxSellBps ?? 1000 }[f.key];
  const range = hi < 4_294_967_295 ? `${formatBps(lo)} to ${formatBps(hi)}` : `at least ${formatBps(lo)}`;
  return html`<label class="field" for="${id}">${f.label} <small>${f.unit}, ${range}</small>
    <input id="${id}" name="${f.key}" type="number" inputmode="decimal" step="0.01" min="${lo / 100}" ${hi < 4_294_967_295 ? html`max="${hi / 100}"` : ''} required value="${current / 100}"></label>`;
}

function readArgs(kind, form) {
  const args = {};
  for (const f of KINDS[kind].fields) {
    if (f.type === 'bps') args[f.key] = percentToBps(form.get(f.key));
    if (f.type === 'account') args[f.key] = String(form.get(f.key) ?? '').trim().toLowerCase();
    if (f.type === 'wyrm') args[f.key] = parseTokenInput(form.get(f.key), config.wyrmDecimals);
    if (args[f.key] === null) return { error: `${f.label} is not a number.` };
  }
  return { args };
}

function kindBlock(cls, d) {
  const c = d.council.classes[cls];
  if (d.electorate < c.minElectorate) return `Needs an electorate of ${c.minElectorate} sold Scales, and it is ${d.electorate}.`;
  if (c.needsClosedSale && d.sale.state !== 'Closed') return 'Opens after the sale closes.';
  return null;
}

function formBlock(d, account, now, firstAt) {
  if (firstAt === null) return 'No Scale is sold yet, so nobody can propose. The first sale Scale can vote a season after it is bought.';
  if (firstAt > now) return `No Scale can vote yet. The first one is seasoned ${formatIn(firstAt, now)}, at ${formatUtc(firstAt)}.`;
  if (!account) return 'Connect a wallet that holds a seasoned Scale to propose.';
  if (!d.mine.some((s) => canPropose(s, now, d.council.seasonNs))) return `You need a sale Scale you have held for ${formatLength(d.council.seasonNs)}.`;
  if (d.proposals.some((p) => p.proposer === account && p.status.type === 'Open')) return 'You already have an open proposal. One at a time.';
  return null;
}

function proposeForm(d, account, now, firstAt) {
  const block = formBlock(d, account, now, firstAt);
  const stake = d.mine.filter((s) => canPropose(s, now, d.council.seasonNs));
  const kinds = d.council.kinds.filter((k) => KINDS[k.kind]);
  const first = kinds.find((k) => !kindBlock(k.class, d));
  return html`<form class="propose" data-submit="propose" novalidate>
    ${block ? html`<p class="warn" id="propose-block">${block}</p>` : ''}
    <fieldset class="propose-body" ${block ? 'disabled' : ''} ${block ? html`aria-describedby="propose-block"` : ''}>
      <fieldset class="kinds"><legend>What should change</legend>
        ${kinds.map((k) => {
          const why = kindBlock(k.class, d);
          return html`<label class="kind"><input type="radio" name="kind" value="${k.kind}" ${k === first ? 'checked' : ''} ${why ? 'disabled' : ''}>
            <span><strong>${KINDS[k.kind].label}</strong> <span class="cls cls-${k.class}">${k.class}</span>
              <small>${KINDS[k.kind].summary}${why ? ` ${why}` : ''}</small></span></label>`;
        })}
      </fieldset>
      ${kinds.map((k) => html`<div class="kind-fields" data-kind="${k.kind}" hidden>${KINDS[k.kind].fields.map((f) => field(f, k.kind, d))}</div>`)}
      <p class="leash-warn" aria-live="polite" hidden></p>
      <label class="field">Why <small class="bytes">0 / ${DESCRIPTION_MAX_BYTES} bytes</small><textarea name="description" rows="3" required></textarea></label>
      <label class="field">Stake with <select name="token">${stake.length ? stake.map((s) => html`<option value="${s.id}">Scale ${padScaleId(s.id)}</option>`) : html`<option value="">No seasoned Scale</option>`}</select></label>
      <p class="hint">You attach ${formatNear(d.council.bond + d.council.storageFee)} NEAR: a ${formatNear(d.council.bond)} NEAR bond and a ${formatNear(d.council.storageFee)} NEAR storage fee. The bond comes back if 30% of the Scales that can vote turn out, otherwise it feeds the Dragon. The vote lasts ${formatLength(d.council.votingNs)}.</p>
      <button class="btn btn-primary" type="submit">Propose</button>
    </fieldset>
  </form>`;
}

const DECIDED_SHOWN = 3;

export function view(d, { account, now }) {
  const firstAt = firstVoteAt(d.scales, d.council.seasonNs);
  const open = d.proposals.filter((p) => ['Open', 'Executing'].includes(p.status.type));
  const held = d.proposals.filter((p) => p.status.type === 'LeashHeld');
  const past = d.proposals.filter((p) => !['Open', 'Executing'].includes(p.status.type));
  return html`<section class="page page-council">
    <header class="page-head">
      <h1 tabindex="-1">The Council</h1>
      <p class="lede">Scale holders steer the Dragon inside limits written in its code. No proposal can move NEAR, and the thresholds are not up for a vote.</p>
    </header>
    ${firstAt !== null && firstAt > now
      ? html`<div class="banner countdown" role="status"><p><strong>The first Scale can vote ${formatIn(firstAt, now)}</strong>, at ${formatUtc(firstAt)}. A sale Scale votes once it has been held for ${formatLength(d.council.seasonNs)}.</p></div>`
      : ''}
    <section class="classes" aria-labelledby="classes-h">
      <h2 id="classes-h">How a vote passes</h2>
      <p class="hint">${d.electorate} sold Scales can vote. A vote lasts ${formatLength(d.council.votingNs)}, and a Scale counts only if held ${formatLength(d.council.seasonNs)} before the proposal opened.</p>
      <dl class="class-list">${Object.keys(d.council.classes).map((cls) => html`<div>
        <dt><span class="cls cls-${cls}">${cls}</span></dt>
        <dd>${classRule(cls, d.council)}.<small>${d.council.kinds.filter((k) => k.class === cls).map((k) => KINDS[k.kind]?.label ?? k.kind).join(', ')}</small></dd></div>`)}</dl>
    </section>

    <section aria-labelledby="open-h">
      <h2 id="open-h">In play</h2>
      ${open.length ? open.map((p) => proposalCard(p, d, account, now)) : html`<p class="empty">No proposal is open.</p>`}
    </section>

    <details class="panel propose-wrap">
      <summary><h2 id="propose-h">Make a proposal</h2></summary>
      ${proposeForm(d, account, now, firstAt)}
    </details>

    <section aria-labelledby="leash-h" class="leash">
      <h2 id="leash-h">The leash held</h2>
      <p class="hint">Proposals that passed and that the Dragon still refused, in its own words.</p>
      ${held.length
        ? html`<ul class="held">${held.map((p) => html`<li><strong>#${p.id} ${describeKind(p.kind)}</strong><span>${p.description}</span>${leashReason(p.status.error)}</li>`)}</ul>`
        : html`<p class="empty">Nobody has tried yet.</p>`}
    </section>

    <section aria-labelledby="past-h">
      <h2 id="past-h">Decided</h2>
      ${past.length ? past.slice(0, DECIDED_SHOWN).map((p) => proposalCard(p, d, account, now)) : html`<p class="empty">Nothing decided yet.</p>`}
      ${past.length > DECIDED_SHOWN ? html`<details class="more"><summary>Show all ${past.length}</summary>${past.slice(DECIDED_SHOWN).map((p) => proposalCard(p, d, account, now))}</details>` : ''}
    </section>
  </section>`;
}

export function mounted(root, d) {
  const form = root.querySelector('form.propose');
  if (!form) return;
  const warn = form.querySelector('.leash-warn');
  const bytes = form.querySelector('.bytes');
  const update = () => {
    const kind = form.querySelector('[name=kind]:checked')?.value;
    for (const box of form.querySelectorAll('.kind-fields')) {
      box.hidden = box.dataset.kind !== kind;
      for (const input of box.querySelectorAll('input')) input.disabled = box.hidden;
    }
    bytes.textContent = `${new TextEncoder().encode(form.description.value).length} / ${DESCRIPTION_MAX_BYTES} bytes`;
    const { args } = kind ? readArgs(kind, new FormData(form)) : {};
    const errors = args ? boundErrors(kind, args, d.council.bounds) : [];
    const outflow = args?.amount != null ? outflowWarning(kind, args, d.outflow) : null;
    warn.hidden = !errors.length && !outflow;
    warn.textContent = errors.length ? `Scales will refuse this: ${errors.join(' ')}` : outflow ? `The Dragon will refuse this at the count: ${outflow} You can still propose it, and it will land on the leash list.` : '';
  };
  form.addEventListener('input', update);
  update();
}

export const actions = {
  vote: (el, { perform }, d) => {
    const p = d.proposals.find((x) => x.id === el.dataset.id);
    const ids = votableScales(d.mine, p, d.voted.get(p.id) ?? new Set(), d.council.seasonNs).map((s) => s.id);
    const approve = el.dataset.approve === '1';
    return perform(tx.vote(p.id, ids, approve), `Voted ${approve ? 'yes' : 'no'} on #${p.id} with ${ids.length} Scale${ids.length > 1 ? 's' : ''}.`);
  },
  finalize: (el, { perform }) => perform(tx.finalize(el.dataset.id), `Votes on #${el.dataset.id} counted.`),
  propose: (form, { perform }, d) => {
    const data = new FormData(form);
    const kind = data.get('kind');
    if (!kind) return toast('Pick what should change.', 'error');
    const { args, error } = readArgs(kind, data);
    if (error) return toast(error, 'error');
    const errors = boundErrors(kind, args, d.council.bounds);
    if (errors.length) return toast(errors[0], 'error');
    const description = String(data.get('description') ?? '').trim();
    if (!description) return toast('Say why, in a sentence or two.', 'error');
    if (new TextEncoder().encode(description).length > DESCRIPTION_MAX_BYTES) return toast(`Keep the reason under ${DESCRIPTION_MAX_BYTES} bytes.`, 'error');
    return perform(tx.propose(KINDS[kind].order(args), description, data.get('token'), d.council.bond + d.council.storageFee), 'Proposal opened.');
  },
};
