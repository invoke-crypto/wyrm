// Deployment settings. URL parameters override them for testing:
// ?network=testnet&root=wyrmling.thewyrm.near, ?network=sandbox&rpc=http://127.0.0.1:<port>,
// and in mock mode ?mock=1 with ?sale=waiting|open|closed, ?sold=40 and ?preset=drill.
const NETWORKS = {
  mainnet: {
    rpc: 'https://free.rpc.fastnear.com',
    explorer: 'https://nearblocks.io',
    root: 'thewyrm.near',
  },
  testnet: {
    rpc: 'https://test.rpc.fastnear.com',
    explorer: 'https://testnet.nearblocks.io',
    root: 'thewyrm.testnet',
  },
  sandbox: {
    rpc: 'http://127.0.0.1:3030',
    explorer: null,
    root: 'thewyrm.test.near',
  },
};

const params = new URLSearchParams(location.search);
const network = Object.hasOwn(NETWORKS, params.get('network')) ? params.get('network') : 'mainnet';
const net = NETWORKS[network];
const root = params.get('root') || net.root;
const sold = Number(params.get('sold'));

export const config = {
  network,
  root,
  rpc: params.get('rpc') || net.rpc,
  explorer: net.explorer,
  mock: params.get('mock') === '1',
  mockSale: ['waiting', 'open'].includes(params.get('sale')) ? params.get('sale') : 'closed',
  mockSold: Number.isInteger(sold) && sold >= 0 && sold <= 90 && params.has('sold') ? sold : null,
  mockPreset: params.get('preset') === 'drill' ? 'drill' : null,
  contracts: {
    tithe: `tithe.${root}`,
    dragon: `dragon.${root}`,
    scales: `scales.${root}`,
    ash: `ash.${root}`,
  },
  vaultId: (tokenId) => `${String(tokenId).padStart(3, '0')}.scales.${root}`,
  wyrmDecimals: 18,
  // Storage kept back in every vault; claimable = balance - this. Matches VAULT_RESERVE in scale-vault.
  vaultReserveYocto: 10n ** 22n,
  // Code hashes from the reproducible build at tag v2.0. Empty until the release build exists.
  expectedCodeHashes: { tithe: '', dragon: '', scales: '', vault: '' },
  holdersChat: 'https://t.me/',
  gas: { call: '100000000000000', heavy: '300000000000000' },
  // The waits before each retry of a rate-limited or failed RPC read, before jitter.
  rpcRetryMs: [500, 1000, 2000, 4000, 8000, 16000],
};
