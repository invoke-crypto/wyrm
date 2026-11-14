# Checking WYRM yourself

WYRM claims that its Dragon has no keys, runs the code in this repository, and has followed its rules on every trade. You do not have to trust these claims. This page shows how to check them against mainnet in a few minutes, with no wallet and no API key.

## Run verify

You need a copy of this repository and Node.js 22.5 or newer. Nothing else is required. The repository has no npm dependencies, so there is no `npm install` step.

1. Get the code.

   ```sh
   git clone https://github.com/invoke-crypto/wyrm.git
   cd wyrm
   ```

2. Check your Node version. It must print `v22.5.0` or higher.

   ```sh
   node --version
   ```

3. Run verify against WYRM on mainnet.

   ```sh
   npm run verify -- --network mainnet --root thewyrm.near
   ```

verify prints one line for each of its 15 checks, `PASS` or `FAIL`, with the check's number, its title and a summary. It ends with `all checks PASSED` and the number of RPC requests it made, and exits with code 0. Any failing check prints the evidence that failed and makes the command exit with code 1. A timed run against WYRM took 134 s, with 328 RPC requests (measured Nov 14).

Most of that time is one read. Nine checks replay the history of the Dragon, the tithe and the Scales, and the Dragon's history holds 2,367 transactions. The free FastNEAR transactions API returns 20 transactions for each request and answers about one request a second. verify sends those requests one at a time, waits when the API answers with a rate limit, and prints a progress line to stderr. If you run verify from a tool with a command timeout, set it to 5 minutes.

For a first look, run only the six checks that read current state. They took 5 s (measured Nov 14):

```sh
npm run verify -- --network mainnet --root thewyrm.near --only 1,2,3,4,10,12
```

verify only reads. It asks a NEAR RPC node for contract state and the FastNEAR transactions API for each account's history. Both are free and need no key.

### Options

- `--verbose` prints every item each check judged, not only the failures, and logs each RPC request to stderr.
- `--only 1,3,4` runs only the named checks.
- `--rpc URL[,URL]` replaces the RPC list with your own, as one URL or a comma-separated list.

Free RPC nodes rate-limit busy clients. On mainnet verify knows four: `https://rpc.mainnet.fastnear.com`, `https://near.drpc.org`, `https://rpc.shitzuapes.xyz` and `https://free.rpc.fastnear.com`. When a node answers with a rate limit, verify moves to the next one and stays there. It waits only after every node in the list has refused in turn, and it gives up after eight rounds. If your run fails with "could not read" and a `-429` error, run it again later or pass your own nodes with `--rpc`.

## What each check proves

The checks live in [`verify/checks.mjs`](verify/checks.mjs). Each one reads the chain, then judges what it read with plain arithmetic. The system's accounts are `tithe.thewyrm.near`, `dragon.thewyrm.near`, `scales.thewyrm.near`, `ash.thewyrm.near` and the 100 vaults `001.scales.thewyrm.near` to `100.scales.thewyrm.near`.

**1. Zero access keys on tithe, dragon, scales, ash and the 100 vaults; ash has no code.** Every one of the 104 accounts exists and has no access key, and ash runs no code. Nobody can sign for these accounts again, not even their creator. The code can never be replaced, and no hidden method can be called with a key. WYRM burns tokens by sending them to ash, and with no key and no code nothing can move them out.

**2. The code is a release in verify/releases.json, and scales and dragon run that release's preset.** The code hash on every account equals the v2.0 hash in [`verify/releases.json`](verify/releases.json), which comes from a reproducible build of this repository (`contracts/build.sh --reproducible`). The Dragon and scales report the same preset, preset hash and engine version, and they equal the release's. On mainnet the testnet-only `drill` preset fails this check. This check proves that the contracts you can read in `contracts/` are the contracts running.

**3. The coin is dragon.get_coin(): launched by the dragon, Done, paying the tithe.** The real coin is whatever `dragon.get_coin()` names. Nearly's record of that launch must name the Dragon as its creator, the same token and pool as `get_coin`, step Done, and the tithe as the account that receives its fees. A coin that only copies the WYRM symbol has a different creator, so it cannot pass. When verify finds a launch with the same symbol, it lists it as not bound.

**4. Tax 400 bps each side, all to the creator side; pool fee mode creator; the pool runs wrap and the coin.** Nearly's `get_tax` for the launch must show 400 bps on buys and on sells, with the whole creator side (10,000 bps) and nothing burned or paid to holders. The pool fee must be paid in creator mode, and the DCL pool must be Running with wNEAR and the coin. Nearly pays the creator side of both to the tithe, which is how the Dragon and the Scales earn.

**5. Every tithe_round paid half to the dragon and 1/100 of the rest to each unretired vault.** For every `tithe_round` event in the tithe's history, verify adds up the actual transfers in that transaction. The Dragon gets half, plus the shares of retired vaults once the sale has closed. Each other vault gets exactly one hundredth of the other half. The dust left over is under 100 yoctoNEAR, and no transfer goes anywhere else.

**6. Every dragon buy followed the breath or a confirmed dip.** Each logged buy slice is checked against the rule it claims. A breath slice comes at or after that day's breath minute, on the same UTC day. A dip slice comes after the price fell past a dip step, confirmed by at least 3 samples over at least 10 minutes. No slice bought while the price stood more than 15% above its 24-hour reference. Then verify replays the Dragon's whole log through [`gym/dragon-rules.mjs`](gym/dragon-rules.mjs), the same rules the Gym tuned, and requires that the replay makes the same decision at every step.

**7. Daily spend within budget and every slice within the price-impact cap.** For each slice, the wNEAR the Dragon actually sent to DCL in that transaction must equal the amount it logged. Each slice must pass the impact rule, quote times 10,000 at least the amount times the 1 NEAR price times (10,000 minus the cap). For each UTC day, the total actually sent must stay within that day's budget.

**8. The dragon's outgoing receipts went only to its peers and its coin, wiring reads to the tithe, refunds to the creator.** verify lists every receipt the Dragon ever sent. Each must go to wNEAR, DCL, Nearly, scales or the coin. The only call allowed to the tithe is the `get_config` read during launch, and the only receipts allowed to the creator are plain transfers, the change from the creator's own launch deposit. Any other destination fails the check, so there is no hidden way to pay anyone.

**9. The Council: every executed order passes the preset, went Open to Executing at finalize, and every leash hold has its refusal.** The Dragon and scales must name each other, and the Dragon's live strategy must sit inside the preset's bounds. For every proposal, an Executed order must pass the preset's rules and must have become Executing in its own finalize transaction. A LeashHeld proposal must show the Dragon's refusal, a failed outflow or a failed receipt in that transaction.

**10. 100 Scales at most, every vault NNN.scales.\<root\> with parent scales; 91-100 are the creator's and never vote.** At most 100 Scales exist, with ids 1 to 100. Every vault exists at its fixed name and reports scales as its parent. Scales 91 to 100 are minted and cannot vote. Every sold Scale is minted and can vote, and no id the closed sale retired is minted.

**11. Every Scale sale paid the creator's cut to the creator, the rest to the dragon, and swept the vault.** For every Scale bought in the sale, the transaction paid the creator exactly the preset's cut (10% of 2 NEAR) and the Dragon the rest. The same transaction swept the Scale's vault to the Dragon, so a buyer never collects income from before the sale.

**12. No dev wallet: Nearly's dev-buy recipient is the dragon.** Nearly sends a launch's dev buy to the launch's creator. The creator is the Dragon, so any tokens bought at launch went into the hoard. No wallet received tokens at launch.

**13. Grants and burns each stay within 50 bps of the hoard per order and 100 bps per rolling 30 days.** For every paid grant and burn, the amount must be at most 0.5% of the hoard at that time. All of the same kind paid in the 30 days up to it must add up to at most 1% of that hoard. A captured Council can therefore never empty the hoard.

**14. Every buy delivered amount_out less the buy tax, as a net transfer and a `tax` transfer, and amount_out met min_out.** For every buy, the coin's transfer from DCL to the Dragon must split into one `tax` transfer of exactly 4% and a net transfer of the rest. The gross amount must be at least the slice's `min_out`. This proves that the tax is what the Dragon was built for and that no swap was filled below its slippage limit.

**15. Every hunt sell sent its gross, DCL got it less the sell tax, and the wNEAR back met min_out on that net.** Hunt is off at launch, so this check normally reports 0 sells. If the Council turns Hunt on, every sell must send its logged amount, DCL must receive it less exactly 4%, and the wNEAR returned must meet the `min_out` the Dragon set on that net amount.

## WYRM on mainnet

| What | Value |
|---|---|
| Root | `thewyrm.near` |
| Dragon | [`dragon.thewyrm.near`](https://nearblocks.io/address/dragon.thewyrm.near) |
| Coin | `dragon.get_coin()`: launch 3757, token [`wyrm.nearlytrade.near`](https://nearblocks.io/address/wyrm.nearlytrade.near) |
| Preset | `hoarder`, release v2.0 |

## Attempts to rob the Dragon

Before WYRM deployed, the same release ran on mainnet as a rehearsal system, WYRMLING, at `wyrmling.thewyrm.near`. WYRMLING kept its keys so it could be torn down afterwards to recover its storage deposits, which is why check 1 fails on it and passes only on WYRM. On Nov 10, 2026, a fresh wallet with no role in the system (`1eb10a7e9968c9d5a24d57eb0d0d4b37b1f7e642c7dd5a90b424065ce42b9376`, "burner" below) tried 15 ways to take value or control from it. Every attempt was refused by the contract itself, with its own reason. WYRMLING is torn down after its checks, but these transactions stay on chain.

| # | Contract | Method | What it tried | Refusal | Transaction |
|---|---|---|---|---|---|
| 1 | dragon | `obey` | order the Dragon to grant WYRMLING from its hoard to the burner, without a vote | only scales | [E8oYJP7z](https://nearblocks.io/txns/E8oYJP7zC7MCDUe6paHtbxpzCvzRS6JB3bPHNdHbFy1g) |
| 2 | dragon | `on_swap` | call the swap callback directly to fake a settled swap | Method on_swap is private | [3SGWFbZY](https://nearblocks.io/txns/3SGWFbZYnDsPKtZxejeHWEiNtgMW7UwYFTWHgVWbLc4A) |
| 3 | dragon | `on_outflow` | call the outflow callback directly to fake a failed grant | Method on_outflow is private | [7XktsikB](https://nearblocks.io/txns/7XktsikBUXdyyQvF868HJaRoosjQaJYb8owfkyuYNgWV) |
| 4 | dragon | `ft_on_transfer` | report a token deposit that never happened, to inflate the hoard | the Dragon accepts only its bound coin and wNEAR | [J3uPBud4](https://nearblocks.io/txns/J3uPBud4H5yLeAQPPMchyedVSsDvtZcHhh5sioqhrm6c) |
| 5 | dragon | `launch` | launch a coin of its own through the Dragon | only the creator | [H65QyumX](https://nearblocks.io/txns/H65QyumXg3X4NazzAkBmTAVVT7tJaL287ftBegENoQea) |
| 6 | dragon | `bind` | bind the Dragon to launch 3730 | only the creator | [8StqKXy3](https://nearblocks.io/txns/8StqKXy34HqD8rp94TmoRR6G7rhkqes8z7cRbUpdWQ89) |
| 7 | dragon | `nearly_cancel_stuck` | cancel the bound launch | only the creator | [7yPJcf7P](https://nearblocks.io/txns/7yPJcf7PDurcBwmG1CoVwayi2w3isVNa61mrHjv6MDsn) |
| 8 | tithe | `retire` | retire every sale Scale, so their income goes elsewhere | only scales can retire | [7JfPKf9T](https://nearblocks.io/txns/7JfPKf9TtHXnYf9NXpBDrgR6vFYfuw2rSCa8rjqU8JeH) |
| 9 | tithe | `on_wrap_balance` | call the unwrap callback directly | Method on_wrap_balance is private | [9btQjtSE](https://nearblocks.io/txns/9btQjtSEMUWp3pnLrAsoLsfC9FAhpYSGgGLFfxk5feDm) |
| 10 | scales | `open_sale` | reopen or restart the Scale sale | only the dragon opens the sale | [43vBvkkT](https://nearblocks.io/txns/43vBvkkTvh9PamtMpQXwj5KzCuEXX4W8XqJPxwAvMPqq) |
| 11 | scales | `claim` | claim the income of Scale 1, which another account owns | not the owner | [FPBET7jU](https://nearblocks.io/txns/FPBET7jUpWgaFHF1d2HGz4odmWWLhnWdzkK4Bvk55VXq) |
| 12 | scales | `nft_transfer` | move Scale 1 to the burner | not the owner or approved | [D3BjacvH](https://nearblocks.io/txns/D3BjacvHvmL44ekTGNrQeJQLD1mioFtFuR25bKWWh4oQ) |
| 13 | scales | `on_executed` | call the Council callback directly to fake a verdict | Method on_executed is private | [9R1WHzjE](https://nearblocks.io/txns/9R1WHzjEqFFxYyCp7yzLNbAAZNw3tL1KZz61XAY7gXUe) |
| 14 | 001.scales | `release` | empty Scale 1's vault to the burner | only the parent may release | [BbayTiqB](https://nearblocks.io/txns/BbayTiqBRf4os3XyvHHAVAzi2Xrq4cqVKgcpnfYMUh44) |
| 15 | dragon | `new` | initialise the Dragon a second time | The contract has already been initialized | [8xj3ghqa](https://nearblocks.io/txns/8xj3ghqaqYCC9WbnKL2m7wBUjT9yejiMh5aBVeyhpTtx) |

Attempt 15 named the system's real peers and the creator `wyrmling.thewyrm.near`. Any `new` call fails the same way, whatever its arguments. A 16th transaction, an earlier `new` with incomplete arguments, failed on argument parsing before the contract ran, so it is not counted.

What the threat model expects, and what else WYRMLING measured on mainnet, is in [docs/THREAT-MODEL.md](docs/THREAT-MODEL.md).
