import { getVerifyTargets, getSystem, getCoin, explorerAccount, accountLink } from '../chain.js';
import { config } from '../config.js';
import { html } from '../ui.js';

export const title = () => 'Verify';

export async function load() {
  const [{ core, vaultCode, vaults }, system, coin] = await Promise.all([getVerifyTargets(), getSystem(), getCoin()]);
  return { core, vaultCode, vaults, system, coin };
}

const NO_CODE = '11111111111111111111111111111111';

const NOT_READ = html`<span class="v v-unk">not read</span>`;

function hashCheck(t, role = t.role) {
  const hash = t.globalHash ?? t.codeHash;
  if (!t.exists) return { ok: false, text: 'account missing' };
  if (role === 'ash') return { ok: hash === NO_CODE, text: hash === NO_CODE ? 'no code' : hash };
  const expected = config.expectedCodeHashes[role];
  if (!expected) return { ok: null, text: hash };
  return { ok: hash === expected, text: hash };
}

function verdict(ok) {
  return ok === null ? html`<span class="v v-unk">not pinned</span>` : ok ? html`<span class="v v-ok">pass</span>` : html`<span class="v v-bad">fail</span>`;
}

function vaultCodeLine(vaultCode) {
  if (!vaultCode) return 'No vault exists yet, so none runs code.';
  if (!vaultCode.read) return html`Vault 001's code ${NOT_READ}; reload to try again.`;
  const h = hashCheck(vaultCode, 'vault');
  return html`Vault 001 runs <code class="hash">${h.text}</code> ${verdict(h.ok)}`;
}

export function view({ core, vaultCode, vaults, system, coin }) {
  const read = vaults.filter((v) => v.read);
  const unread = vaults.length - read.length;
  const badVaults = read.filter((v) => !v.exists || v.keys !== 0);
  const drillOnMainnet = system.preset === 'drill' && config.network === 'mainnet';
  return html`<section class="page">
    <header class="page-head">
      <h1 tabindex="-1">Verify</h1>
      <p class="lede">Nobody holds a key to the Dragon, the tithe or the Scales. Check it here, or on any explorer.</p>
    </header>
    <section aria-labelledby="sys-h">
      <h2 id="sys-h">The system</h2>
      <dl class="figures figures-small">
        <div><dt>Preset</dt><dd>${system.preset} ${verdict(!drillOnMainnet)}<small>${system.preset === 'drill' ? 'Testnet only. A mainnet system must never report it.' : 'Fixed for life at deploy.'}</small></dd></div>
        <div><dt>The coin</dt><dd>${coin ? accountLink(coin.token) : 'not bound yet'}<small>${coin ? `Nearly launch ${coin.launchId}. Only dragon.get_coin() names it.` : 'The sale opens at bind.'}</small></dd></div>
      </dl>
    </section>
    <section aria-labelledby="core-h">
      <h2 id="core-h">The contracts and ash</h2>
      <div class="table-wrap"><table>
        <thead><tr><th scope="col">Account</th><th scope="col">Access keys</th><th scope="col">Code hash</th></tr></thead>
        <tbody>${core.map((t) => {
          if (!t.read) return html`<tr><th scope="row">${accountLink(t.id)}</th><td>${NOT_READ}</td><td>${NOT_READ}</td></tr>`;
          const h = hashCheck(t);
          return html`<tr>
            <th scope="row">${accountLink(t.id)}</th>
            <td>${t.keys === null ? 'unknown' : t.keys} ${verdict(t.keys === null ? null : t.keys === 0)}</td>
            <td><code class="hash">${h.text}</code> ${verdict(h.ok)}</td></tr>`;
        })}</tbody>
      </table></div>
    </section>
    <section aria-labelledby="vault-h">
      <h2 id="vault-h">The 100 Scale vaults</h2>
      <p>${read.filter((v) => v.exists).length} of 100 exist. ${read.filter((v) => v.keys === 0).length} have zero access keys.${unread ? ` ${unread} could not be read from the RPC; reload to try again.` : ''}
        All 100 run the same global contract, fixed in the Scales at deploy. ${vaultCodeLine(vaultCode)}</p>
      <p class="hint">This page reads the code of vault 001. <code>npm run verify</code> checks all 100.</p>
      <ol class="vault-dots" aria-label="Vault status, 001 to 100">
        ${vaults.map((v, i) => {
          const state = !v.read ? 'not read' : !v.exists ? 'missing' : `${v.keys} keys`;
          return html`<li class="${!v.read ? 'unread' : !v.exists ? 'missing' : v.keys ? 'keyed' : 'ok'}"><a href="${explorerAccount(v.id) ?? '#/verify'}" target="_blank" rel="noopener" title="${v.id}: ${state}"><span class="sr">${v.id} ${state}</span>${String(i + 1).padStart(3, '0')}</a></li>`;
        })}
      </ol>
      ${badVaults.length
        ? html`<ul class="problems">${badVaults.map((v) => html`<li>${accountLink(v.id)} ${!v.exists ? 'does not exist yet. Anyone can call create_vaults() to make it.' : `has ${v.keys} access key${v.keys > 1 ? 's' : ''}.`}</li>`)}</ul>`
        : ''}
    </section>
    <p class="hint">The full audit, including the Nearly launch record, every tithe round and every Dragon decision, runs with <code>npm run verify</code> from the repository.</p>
  </section>`;
}
