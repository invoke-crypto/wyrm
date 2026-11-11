import { config } from './config.js';

// Pinned. Loaded on demand so the read-only Lair never waits on it.
const CONNECTOR_URL = 'https://esm.sh/@hot-labs/near-connect@0.11.4';
const MOCK_KEY = 'lair.mockAccount';
const MOCK_DEFAULT = 'ember.near';

const listeners = new Set();
let connector = null;

export const wallet = {
  accountId: null,
  onChange(fn) {
    listeners.add(fn);
  },
  async restore() {
    if (config.mock) return setAccount(read(MOCK_KEY));
    const c = await getConnector();
    try {
      const { accounts } = await c.getConnectedWallet();
      setAccount(accounts[0]?.accountId ?? null);
    } catch {
      setAccount(null);
    }
  },
  async connect() {
    if (config.mock) {
      const id = new URLSearchParams(location.search).get('as') || MOCK_DEFAULT;
      write(MOCK_KEY, id);
      return setAccount(id);
    }
    await (await getConnector()).connect();
  },
  async disconnect() {
    if (config.mock) {
      write(MOCK_KEY, null);
      return setAccount(null);
    }
    await (await getConnector()).disconnect();
  },
  async signAndSend(transactions) {
    const w = await (await getConnector()).wallet();
    if (transactions.length === 1) return w.signAndSendTransaction({ ...transactions[0], network: config.network });
    return w.signAndSendTransactions({ transactions, network: config.network });
  },
};

function setAccount(id) {
  if (wallet.accountId === id) return;
  wallet.accountId = id;
  for (const fn of listeners) fn(id);
}

async function getConnector() {
  if (connector) return connector;
  const { NearConnector } = await import(CONNECTOR_URL);
  connector = new NearConnector({ network: config.network, providers: { [config.network]: config.rpcs } });
  connector.on('wallet:signIn', (e) => setAccount(e.accounts[0]?.accountId ?? null));
  connector.on('wallet:signOut', () => setAccount(null));
  return connector;
}

function read(key) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key, value) {
  try {
    if (value == null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {}
}
