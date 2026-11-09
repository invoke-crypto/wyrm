## Measured gas (sandbox)

This section is hand-written in `docs/GAS.md` and appended by `gym/parameters.mjs`. The numbers come from the near-workspaces sandbox (near-sandbox 2.13.4, protocol 86), on the release wasm built by `contracts/build.sh`. Rerun them with `npm run test:wsl -- integration bind_sandbox unsold_sandbox council_scenarios tax_sandbox dcl_real` and read the `GAS` lines in each sandbox log.

Burnt gas is what the transaction consumed across all its receipts. The transaction limit is on gas attached, and contracts attach fixed gas to the calls they make, so the attached column is the one a caller must meet. The v1 column is the frozen v1 trunk, rerun on the same machine on Nov 6. A dash means v1 had no such call.

Each column was read at one point in the transaction's life:

- **At commit.** The whole-transaction column and the v1 column come from near-workspaces, which answers at `broadcast_tx_commit`. That answer leaves out the gas-refund receipts that run later, about 0.22 Tgas each.
- **At FINAL.** The same calls read with `wait_until: FINAL`, which counts every receipt. They were re-read on Nov 9 in near-sandbox 2.13.4 (protocol 86) through `keeper/deploy.mjs` and one-call scripts, for the rows a deploy and a tithe round reach. A dash means the row was not re-read.
- **The called method's own receipt.** One receipt, which does not change between the two points.

A row that fans out to many accounts gains the most at FINAL. One `distribute` to 100 vaults read 49.04 Tgas at commit and 71.59 Tgas at FINAL: 101 more refund receipts. Every row with a dash reads higher at FINAL by its late refunds, most of all those that fan out: `sweep`, the tithe round after retirement and the real-DCL poke chains. Attached gas covers every receipt, so the 300 Tgas `distribute` attaches still leaves 228 Tgas spare at FINAL.

| Operation | Burnt, whole transaction, at commit | Burnt, whole transaction, at FINAL | Burnt, the called method's own receipt | v1, whole transaction, at commit | Attached by the caller |
|---|---|---|---|---|---|
| `dragon.launch(coin)`, no dev buy, through Nearly to Done | 46.44 Tgas | 46.75 Tgas | 3.49 Tgas | - | 500 Tgas, required |
| `dragon.launch(coin)` with a 1 NEAR dev buy | 59.00 Tgas | - | 3.50 Tgas | - | 500 Tgas, required |
| `dragon.bind(launch_id)`, with the `open_sale` kick | 18.64 Tgas | 18.92 Tgas | 3.44 Tgas | - | 150 Tgas, required |
| `dragon.nearly_resume(id)` of a launch parked at CreatePool | 12.81 Tgas | - | 1.79 Tgas | - | 300 Tgas, required |
| `dragon.poke()`, a full buy chain against mock-dcl | 40.94 Tgas | - | 3.44 Tgas | 40.51 Tgas | 1,000 Tgas, floor 760 |
| `dragon.poke()`, a full buy chain against the real DCL wasm | 142.68 Tgas | - | 3.45 Tgas | - | 1,000 Tgas, floor 760 |
| `dragon.poke()`, a buy chain against the real DCL wasm at the floor | 149.67 Tgas | - | 3.45 Tgas | - | 760 Tgas |
| `dragon.poke()`, a Hunt sell chain on the taxed coin | 41.70 to 41.85 Tgas | - | 3.44 Tgas | - | 1,000 Tgas |
| `dragon.poke()`, an observation with nothing to do | 11.94 Tgas | 12.10 to 12.12 Tgas | 3.44 Tgas | 11.83 Tgas | 1,000 Tgas |
| `scales.finalize`, SetSlices (Light) | 7.76 Tgas | - | 3.03 Tgas | 6.96 Tgas (`execute`) | 150 Tgas, enforced |
| `scales.finalize`, Grant paid (Strict) | 12.45 Tgas | - | 3.05 Tgas | 11.58 Tgas (`execute`) | 150 Tgas, enforced |
| `scales.finalize`, Burn to ash (Light), ash registered in the same batch | 14.69 Tgas | - | 3.04 Tgas | - | 150 Tgas, enforced |
| `scales.finalize`, Grant held by the 30-day window (ends LeashHeld) | 8.11 Tgas | - | 3.05 Tgas | - | 150 Tgas, enforced |
| `scales.finalize`, Grant refused by the token (ends LeashHeld) | 12.56 Tgas | - | 3.04 Tgas | 11.65 Tgas (`execute`) | 150 Tgas, enforced |
| `scales.finalize`, a rejected order or a Signal | 3.03 to 3.04 Tgas | - | 2.28 Tgas | - | 150 Tgas, enforced |
| `scales.close_sale()`, telling the tithe the first unsold id | 6.01 Tgas | - | 2.38 Tgas | - | 300 Tgas in the test, 50 designed |
| `scales.sweep(token_ids)`, 20 retired vaults to the Dragon | 42.92 Tgas | - | 8.53 Tgas | - | 300 Tgas |
| `scales.create_vaults()`, 25 keyless vaults on the global code | 253.92 to 254.05 Tgas | 254.20 to 254.33 Tgas | 31.11 Tgas | 253.90 to 254.04 Tgas | 300 Tgas |
| `scales.buy_scale()`, with its mint sweep | 8.59 Tgas | 9.04 to 9.07 Tgas | 3.47 Tgas | 3.64 Tgas | 300 Tgas |
| `scales.buy(token_id)`, a market sale with royalty | 3.87 Tgas | - | 2.89 Tgas | 3.90 Tgas | 300 Tgas |
| `scales.claim(token_id)`, vault release and `on_claimed` | 6.91 Tgas | - | 2.61 Tgas | 6.93 Tgas | 300 Tgas |
| `tithe.distribute()`, half to the Dragon and 100 vault transfers | 48.87 Tgas | 71.52 to 71.59 Tgas | 25.80 Tgas | 48.90 Tgas | 300 Tgas |
| `tithe.distribute()` after retirement from 4, 13 vault transfers | 8.33 Tgas | - | 4.67 Tgas | - | 300 Tgas |

The poke floor is 760 Tgas. `poke` with 759 Tgas attached is refused with "poke needs at least 760 Tgas", and a buy chain at exactly 760 Tgas completes, against mock-dcl and against the real DCL wasm. `launch` and `bind` complete at exactly 500 and 150 Tgas, and refuse 499 and 149. The design named 450 Tgas for `launch`. The Dragon requires 500, because its wiring callback attaches 380 Tgas to cover Nearly's 300 and its own callbacks.

### The poke chain against real DCL

`dcl_real` imports the code of `dclv2.ref-labs.near` from mainnet (version 2.4.2, code hash `3G34Nhjmir5JPYPtijaA9iCNnHDPR7h3wfC9H77woEGo`, 1,166,649 bytes) with fresh state. It seeds one pool, `wrap|coin|10000`, with a single position from point -80,000 to -58,000 around 1,000 WYRM per NEAR. Three runs burnt the same gas to the hundredth.

| Receipt | Real DCL | mock-dcl |
|---|---|---|
| Dragon `poke`, `on_observe`, `on_quote`, `on_swap` | 3.45, 5.22, 2.61, 1.82 Tgas | 3.44, 5.21, 2.62, 1.81 Tgas |
| DCL `get_pool` | 2.59 Tgas | 1.36 Tgas |
| DCL `quote`, the reference and seven halvings | 11.88 to 12.09 Tgas each, 95.9 Tgas in all | 1.30 Tgas each, 10.4 Tgas in all |
| DCL `ft_on_transfer`, the swap | 14.39 Tgas | 1.90 Tgas |
| DCL `callback_post_withdraw_asset` | 2.48 Tgas | - |
| Whole transaction, at commit | 142.68 Tgas | 40.94 Tgas |

The real chain burns 3.5 times the mock chain. Eight quotes account for 86 Tgas of the 102 Tgas gap and the swap for 12.5. Each quote burns about 12 Tgas against the 30 Tgas the Dragon attaches, and the swap 14.4 Tgas against the 180 Tgas the Dragon attaches. These match the mainnet receipts in docs/research/RHEA-DCL.md, where quotes burn 4.9 to 12.5 Tgas and swaps 8.1 to 14.1 Tgas. The Dragon's own receipts burn the same as on the mock. So the 760 Tgas floor holds with six halvings, and needs no cut to five. Quote gas grows with the points a swap crosses, so a pool with many positions, or a slice far larger than this one (29.4 NEAR), costs more per quote. WYRMLING's first buy on mainnet is the check for that.

What the numbers say:

- `create_vaults` is the tightest call. Each new vault burns about 8.6 Tgas, so a batch of 25 burns about 254 Tgas, 254.33 at FINAL, under the 280 Tgas limit, with 45 Tgas of headroom at 300 Tgas attached.
- `buy_scale` burns 5 Tgas more than v1, which is the mint sweep: a vault release and its callback.
- A tithe round after retirement sends fewer transfers, so it costs a sixth of a full round.
- The mock chain burns 0.43 Tgas more than v1. The Dragon's own receipts burn slightly less than v1, and the rest comes from the larger v2 mocks.
