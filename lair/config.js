// Deployment settings. URL parameters override them for testing:
// ?network=testnet&root=wyrmling.thewyrm.near, ?network=sandbox&rpc=http://127.0.0.1:<port> (a comma list for several),
// and in mock mode ?mock=1 with ?sale=waiting|open|closed, ?sold=40 and ?preset=drill.
const NETWORKS = {
  mainnet: {
    // Free public RPCs that allow browser reads. Each visitor starts on a random one, and a rate limit moves the Lair
    // to the next.
    rpcs: ['https://free.rpc.fastnear.com', 'https://near.drpc.org', 'https://rpc.shitzuapes.xyz', 'https://near-mainnet.gateway.tatum.io', 'https://rpc.mainnet.fastnear.com'],
    explorer: 'https://nearblocks.io',
    root: 'thewyrm.near',
  },
  testnet: {
    rpcs: ['https://test.rpc.fastnear.com'],
    explorer: 'https://testnet.nearblocks.io',
    root: 'thewyrm.testnet',
  },
  sandbox: {
    rpcs: ['http://127.0.0.1:3030'],
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
  rpcs: params.get('rpc')?.split(',') ?? net.rpcs,
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
  // Code hashes of the v2.0 reproducible build, as in verify/releases.json. Only WYRM on mainnet runs that build.
  expectedCodeHashes:
    network === 'mainnet' && params.get('mock') !== '1'
      ? {
          tithe: 'CnDz34PWKiiXECKsT3f2KJvyKwgaVdUB233dC7d5HSYU',
          dragon: 'FoaJWAPrkgnPmgSVyuBT9bxMeFTvJf2JhJJxWkxjYExc',
          scales: 'DUhg2PuvGCoETpELJLNy9b3oZWXCkW6Fentn8AzMBTSb',
          vault: 'A8oxJyQcpZqXP94eZsTMU5ENG9nMuefKETsBggmx23XT',
        }
      : { tithe: '', dragon: '', scales: '', vault: '' },
  holdersChat: 'https://t.me/',
  gas: { call: '100000000000000', heavy: '300000000000000' },
  // The waits before each retry of a rate-limited or failed RPC read, once every RPC in the list refused it, before jitter.
  rpcRetryMs: [500, 1000, 2000, 4000, 8000, 16000],
};
