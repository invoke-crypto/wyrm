# WYRM

This dragon has no keys.

WYRM is a memecoin on [Nearly](https://nearly.trade) with a treasury that nobody controls. The treasury is a contract called the Dragon. It holds NEAR, buys WYRM once a day in small slices through the coin's own pool, and keeps what it buys in its hoard. After deploy, every WYRM account has its access keys deleted, so nobody can change the code, take the NEAR or skip the rules. That includes the person who launched it.

The money comes from trading. Every buy and every sell of WYRM pays a 4% token tax. Nearly pays the creator's part of that tax, and of the pool fee, to a contract called the tithe. The tithe splits each payment in two. Half goes to the Dragon. The other half goes in equal hundredths to 100 vaults, one for each Scale.

Scales are NFTs. 90 are sold at a flat 2 NEAR for 14 days after the coin launches, and 10 go to the creator. A Scale's vault collects its share of every tithe payment, and its owner can claim it at any time. Scales are also the Council. A Council vote can tune the Dragon or grant and burn WYRM from the hoard, but only inside limits fixed in the code. No vote can move NEAR out of the Dragon or change a rule.

The Dragon's buying rules come from the Gym, a replay of real Nearly coins (`gym/`). On coins it was not tuned on, buying once a day got a median 1.69 times more WYRM per NEAR than buying at once ([docs/PARAMETERS.md](docs/PARAMETERS.md#the-values)).

## WYRM on mainnet

Website: [thewyrm.xyz](https://thewyrm.xyz). X: [@thewyrmisnear](https://x.com/thewyrmisnear).

WYRM's root account is `thewyrm.near`, and its Dragon is `dragon.thewyrm.near`. The real coin is the one `dragon.get_coin()` names: launch 3757 on Nearly, token account `wyrm.nearlytrade.near`. A coin that only copies the WYRM symbol is not WYRM, and its Nearly record names a different creator.

Read the coin yourself:

```sh
curl -s https://rpc.mainnet.fastnear.com -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"query","params":{"request_type":"call_function","finality":"final","account_id":"dragon.thewyrm.near","method_name":"get_coin","args_base64":"e30="}}'
```

The inner `result` holds `{ launch_id, token, pool_id, wrap_is_x }` as a list of JSON bytes.

To check every promise against the chain, follow [JUDGES.md](JUDGES.md). It runs 15 checks with Node.js and no API key. Against WYRM the six checks that read only current state took 5 s, and all 15 took about 2 minutes (measured Nov 14). The free history API sets that time, so give the full run a timeout of 5 minutes.

## Try it

Each step needs only this repository and Node.js 22.5 or newer. There is no `npm install`.

| Do | See |
|---|---|
| Open [thewyrm.xyz](https://thewyrm.xyz) | The Dragon's NEAR reserve, its hoard of WYRM, and a log line for its first buy: 0.49 NEAR on Nov 12. |
| `npm run verify -- --network mainnet --root thewyrm.near --only 1,2,3,4,10,12` | Six `PASS` lines in about 5 s: 104 accounts with zero access keys, the code hashes of release v2.0, the coin the Dragon launched, the 4% tax, the 100 Scales and no dev wallet. |
| `npm run verify -- --network mainnet --root thewyrm.near` | 15 `PASS` lines and `all checks PASSED` in about 2 minutes. The extra checks replay every fee split and every Dragon buy from the chain's history. |
| `npm test` | 233 tests pass in about 7 s. |
| Open [thewyrm.xyz/#/scales](https://thewyrm.xyz/#/scales) with a NEAR wallet | Buy a Scale for 2 NEAR. It appears under Yours, and Claim pays its share of the fees to your wallet. |

The same release first ran on mainnet as a rehearsal system, WYRMLING (`wyrmling.thewyrm.near`), against the real Nearly and the real DCL. It passed the seven mainnet gates of the design and refused 15 attempts to rob it. [docs/THREAT-MODEL.md](docs/THREAT-MODEL.md) has the results.

## The Council demo (testnet, mock Nearly)

A WYRM vote cannot happen before the hackathon deadline. A Scale must be held for 7 days before it can vote, and a vote lasts 2 days. The Council was therefore shown on testnet, on the drill system at `council.thewyrm.testnet`. It runs the same code with the testnet-only `drill` preset: a 0.1 NEAR Scale, a 2-hour sale, 10 minutes of seasoning and 30-minute votes. Nearly exists only on mainnet, so the drill system trades against our own mock Nearly, mock DCL pool and mock taxed token.

In the demo video, the Council's vote history is labelled "testnet rehearsal". The drill ran seven proposals:

- A Light vote set the slice cap to 1%, and it passed.
- A Strict grant passed.
- A Strict vote to turn Hunt on got 79% yes, and it failed.
- A burn to ash passed.
- A Signal, a text poll, ended Carried.
- A second grant passed.
- A third grant passed the vote, but the Dragon refused it. Grants in any 30 days are at most 1% of the hoard, so the proposal ended LeashHeld with that reason.

Explore it at [testnet.nearblocks.io](https://testnet.nearblocks.io/address/scales.council.thewyrm.testnet), or open the Lair against it (below). The sandbox runs the same six kinds of scenario on the mainnet timings in [`contracts/tests/tests/council_scenarios.rs`](contracts/tests/tests/council_scenarios.rs).

## What to know before you buy

- **Nearly can change after bind.** Nearly's factory is owned by a DAO and can change its own contracts after WYRM binds to its coin. A change that reroutes fees would starve the tithe, and a keyless system cannot follow it. A higher sell tax would make the Dragon's sells fail safely. Nothing in WYRM can prevent this.
- **The Dragon pays the tax too.** Each Dragon buy costs the 1% pool fee plus the 4% tax. Half of the creator part comes back through the tithe.
- **A captured Council is bounded, not powerless.** 180 NEAR buys every sale Scale. Holders of two thirds of the votes can grant or burn up to 0.5% of the hoard per order and 1% per 30 days each, move the budget, cap and Hunt line inside fixed bounds, and set auto-burn up to 50%.

The full list is [docs/THREAT-MODEL.md](docs/THREAT-MODEL.md).

## Designed, not built

The factory and its Summon wizard would let anyone launch their own keyless Dragon from one transaction. They are designed in [docs/design/v2/design.md section 2f](docs/design/v2/design.md#2f-factory-new-summonroot) and [section 5c](docs/design/v2/design.md#5c-factory-summon), and the contracts already take a preset by name so a factory could deploy them. The factory was not built for the hackathon. WYRM deploys directly with `keeper/deploy.mjs`.

## Repository map

| Path | What it holds |
|---|---|
| [`contracts/`](contracts) | The Rust contracts. `tithe`, `dragon`, `scales` and `scale-vault` are deployed. `wyrm-types` holds the shared types and the preset table. `mock-nearly`, `mock-dcl` and `mock-ft` stand in for Nearly, DCL and a taxed token in tests and on testnet. `state-cleanup` empties a rehearsal account before its teardown. `tests/` holds the sandbox tests. |
| [`keeper/`](keeper) | `deploy.mjs` deploys, launches, binds and tears down a system one step at a time. `keeper.mjs` is the loop that pokes the Dragon, distributes the tithe, finalizes votes and closes the sale. Anyone may run it. `drill.mjs` runs the testnet Council demo. `chain.mjs` is the RPC client. |
| [`verify/`](verify) | `verify.mjs` and its 15 checks, `audit-dragon.mjs`, which replays the Dragon's decisions, and `releases.json`, the code hashes of each release. |
| [`gym/`](gym) | The replay of real Nearly coins that chose the presets. `npm run gym` writes `gym/out/` and `docs/PARAMETERS.md`. |
| [`lair/`](lair) | The website: the Dragon, the Scales market, the Council and a Verify page. Plain ES modules, no build step. |
| [`media/`](media) | The coin icon and the four Scale tier images, with their SVG sources. |
| [`docs/`](docs) | [SPEC.md](docs/SPEC.md), the contract spec. [THREAT-MODEL.md](docs/THREAT-MODEL.md). [design.md](docs/design/v2/design.md) and [invariants.md](docs/design/v2/invariants.md), the v2 design. [PARAMETERS.md](docs/PARAMETERS.md) and [GAS.md](docs/GAS.md), the measurements. [CONCEPT.md](docs/CONCEPT.md), the brand. `research/`, what was read of Nearly and DCL. |

## Run the tests

The JavaScript tests need Node.js 22.5 or newer and no dependencies:

```sh
npm test
```

The contract tests need Rust 1.99.0 (pinned in `contracts/rust-toolchain.toml`) and `cargo-near`. The sandbox tests start a local NEAR node, which has no Windows build, so run them on Linux, or in WSL from Windows:

```sh
npm run test:contracts   # cargo test --workspace, on Linux
npm run test:wsl         # from Windows: syncs the tree into WSL, builds every wasm, runs the unit tests
```

`npm run build:contracts` builds every contract. `bash contracts/build.sh --reproducible` builds the four deployed contracts in the pinned Docker image from committed code. `verify/releases.json` holds the hashes of the v2.0 build.

`node docs/check-docs.mjs` checks the links in these documents, looks for stale v1 numbers, and compares SPEC.md's method tables with the contracts.

## Run the Lair

The Lair is static files. Serve the `lair/` folder with any static file server and open it in a browser:

```sh
python -m http.server 8000 -d lair
```

`http://localhost:8000/` reads WYRM on mainnet. `?mock=1` runs on built-in sample data. `?network=testnet&root=council.thewyrm.testnet` reads the drill system and shows the drill banner on every page.

## Built at NEAR HACKATHON

WYRM is a solo entry by [invoke-crypto](https://github.com/invoke-crypto), built for NEAR HACKATHON. The first commit is from Nov 2, 2026, and no code predates the hackathon.

It fits two tracks:

- **Memecoins With Mechanics.** WYRM is a NEP-141 coin whose trading fees fund a treasury with fixed rules and no owner.
- **NFTs That Do Something.** Each Scale is a NEP-171 NFT with its own sub-account, such as `042.scales.thewyrm.near`. That account holds the Scale's earnings, and the Scale carries a Council vote.

What NEAR makes possible here:

- **Deleted access keys.** An account with no keys can never be redeployed or drained, so the Dragon, the tithe and the Scales have no owner. `npm run verify` checks it.
- **Named sub-accounts.** Each of the 100 vaults is an account of its own, so a Scale's income moves with the Scale when it is sold.
- **Global contracts.** The 100 vaults run one published copy of the vault code.

Who did what. invoke-crypto designed WYRM: the mechanism, the economics, the Council's limits and every product decision. invoke-crypto also directed the Gym study, reviewed the work and ran the mainnet launch. The code, tests and documents were written with Claude Code, an AI coding assistant, under that direction and review.

What WYRM builds on:

- **Contracts:** Rust with `near-sdk` and `near-contract-standards` 5.29.1. The sandbox tests use `near-workspaces` 0.23 and `near-sandbox`.
- **Tools:** the keeper, verify and the Gym are plain Node.js with no npm dependencies.
- **Lair:** plain JavaScript with no framework. Wallets connect through `@hot-labs/near-connect` 0.11.4. The fonts are Bricolage Grotesque and Grenze Gotisch.
- **On chain:** WYRM launches on [Nearly](https://nearly.trade) and trades in a Rhea DCL pool (`dclv2.ref-labs.near`) against `wrap.near`.
- **Data:** reads go through public RPCs, and verify reads history from FastNEAR's indexer.

## License

MIT. See [LICENSE](LICENSE).
