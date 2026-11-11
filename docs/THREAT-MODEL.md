# WYRM threat model

This file lists what can go wrong with a keyless WYRM system, what the code does about each risk, and what is left over. It covers the 42 risks of the v2 design review, in the same numbering, ranked within each section by the harm each could do. Where the mainnet rehearsal measured something, the result is given with its evidence.

The rehearsal system is WYRMLING (`wyrmling.thewyrm.near`). It ran the same release as WYRM (v2.0 in [`verify/releases.json`](../verify/releases.json)) on mainnet against the real Nearly and the real DCL on Nov 10, 2026, and is torn down after its checks. Its transactions stay on chain. The contracts are described in [SPEC.md](SPEC.md) and the design in [design.md](design/v2/design.md). "Guess" marks a cause or outcome nobody observed.

## Mainnet gates

Seven facts could not be proven in a sandbox. WYRMLING checked each one on mainnet before WYRM deployed.

| Gate | Result | Evidence |
|---|---|---|
| 1. Gas of a contract-called `launch` | Pass | Launch 3730 reached Done with 500 Tgas attached and 85.11 Tgas burnt ([5rd53ue](https://nearblocks.io/txns/5rd53ueuRjhQLJ7z6YJYehBJ8SHE2D5mwcakdkPqn2XN)). |
| 2. Poke gas under real DCL | Pass | The first buy chain against the real DCL completed with the keeper's 1,000 Tgas, about 90 Tgas of real work, 0.06925 NEAR in and 64,494.86 WYRMLING out net, the quote less 4% ([Gyw5BevB](https://nearblocks.io/txns/Gyw5BevBJ3axmLAJW5nFbD91EL9UEsRT6V9LbC46xHuk)). |
| 3. Bind views fit their gas | Pass | Nearly's four views used 1.97 to 2.50 Tgas of their 5, and DCL's `get_pool` 2.73 of its 10 ([BbHfeGPt](https://nearblocks.io/txns/BbHfeGPtpq1Ev9xiH321Px4xNTdZB3phiukz8L7DxHbg)). |
| 4. Nearly's view shapes | Pass | `get_launch`, `get_fee_to`, `get_fee_opts` and `get_tax` carry every key the mock carries, with the same types. The real launch parsed `quote_launch` and `get_slug_count`. |
| 5. Nearly registers the creator | Pass | `storage_balance_of(dragon)` on the coin read 0.00125 NEAR right after the launch. |
| 6. A contract `fee_to` receives both shares | Pass | Nearly paid WYRMLING's tithe the pool-fee share and the tax share, each by plain transfer. The tax share was 0.310162 NEAR ([CWv8T2Bj](https://nearblocks.io/txns/CWv8T2BjcXirny5PHiW9iC3CioPp2tdH5pjLMw3pZYdT)). See item 6. |
| 7. Taxed sell and its refund | Pass | A wallet sold 279,131.52 WYRMLING, and the pool got 267,966.26, exactly 96.000%, with 11,165.26 taken as tax ([Ar6HtRhP](https://nearblocks.io/txns/Ar6HtRhP4j1WwgQ7ibkeEfa7agDSXQWPL7zeuzVL2BrA)). |

`npm run verify -- --network mainnet --root wyrmling.thewyrm.near` passed every check except check 1, because WYRMLING keeps its keys for teardown. [JUDGES.md](../JUDGES.md) lists the 15 attempts to rob WYRMLING and the refusal each one got.

## Gates before mainnet

1. **Gas of a contract-called `launch`.** The Dragon calls Nearly's `launch` with 300 Tgas and unused gas weight, and a launch short of gas parks instead of failing. The built `launch` refuses less than 500 Tgas attached, and the keeper resumes a parked launch through `nearly_resume`. Gate 1 passed: launch 3730 reached Done in one transaction, 85.11 Tgas burnt.
2. **Poke gas under real DCL.** The chain of a buy (observe batch, parallel quotes, deposit, 180 Tgas swap) must fit the 1,000 Tgas transaction limit. `poke` refuses below 760 Tgas, and the keeper attaches 1,000. The sandbox ran the chain against DCL's own wasm first ([GAS.md](GAS.md)). Gate 2 passed on WYRMLING's first buy. Gas-refund receipts of 7.52 Tgas each come on top of that work.
3. **The bind views fit their gas.** Bind gives Nearly's four views 5 Tgas each and DCL's `get_pool` 10. Gate 3 passed with at least half of each allowance unused.
4. **Mock-nearly's JSON shapes.** Bind parses four Nearly views, and launch parses two. The mock was built from recorded mainnet answers, not from the source alone. Gate 4 passed.
5. **Nearly registers the creator on the token.** The Dragon does not register itself at bind, because Nearly's CreateToken does it. Gate 5 passed.
6. **A contract `fee_to` receives the creator share and the tax share.** Nearly pays both into one `creator_fees` bucket for `fee_to`, and `push_creator_fees` pays that bucket out by plain transfer. Anyone may call the push once the bucket holds 0.05 NEAR. Gate 6 passed on WYRMLING. The pool-fee share reached its tithe in four pushes by Nearly's relayer, the first two being 0.004800 NEAR (0.48% of an outside trader's 1 NEAR of buys) and 0.003212 NEAR (0.48% of the rehearsal's own buys). The tax share takes longer, because Nearly's keeper collects and sells a launch's tax tokens off chain and skips small piles: over four days its smallest NEAR-pair sale was 0.2944 NEAR. Once the rehearsal's pile reached 0.53 NEAR, the keeper sold it and credited the creator 0.310162 NEAR, 60% of the proceeds as configured, and a push paid it to the tithe ([CWv8T2Bj](https://nearblocks.io/txns/CWv8T2BjcXirny5PHiW9iC3CioPp2tdH5pjLMw3pZYdT)). So in a quiet market WYRM's tax waits on the token account until the pile is worth about 0.3 NEAR, and its sale depends on Nearly's keeper running.
7. **The taxed sell and its refund.** The sandbox proves the refund of a failed taxed sell on the mock token, and gate 7 proved the token side on mainnet: a sell delivered exactly 96% to the pool. Hunt is off at launch for WYRM, so the Dragon makes no sells until a Strict vote turns Hunt on. A round trip of a buy and a sell lost about 9.7% to two taxes and two pool fees (measured).

## Griefing and launch failures

8. **Someone launches WYRM first.** Tickers are not reserved, so an impostor can take the plain `wyrm` token account and ours gets a numbered one. The Dragon binds only the token its own launch produced, and an impostor's record names a different creator, so it cannot be bound. What remains is brand confusion. The Lair, verify and the README name the real coin only through `dragon.get_coin()` on `dragon.thewyrm.near`.
9. **Someone creates the pool first so our launch fails.** The next token account is predictable, and a pool at another price makes Nearly fail the launch after taking the fee. The Dragon predicts the token and pool id from `get_slug_count` and refuses to send anything to Nearly when a pool already exists there. A pool created in the few blocks between that check and Nearly's CreatePool still fails the launch, and a change to Nearly's slug rule or pool fee would make the prediction miss. In both cases Nearly refunds the pool deposit to the Dragon, and the creator launches again.
10. **Nearly's owner changes a setting that refuses the launch.** A paused factory, unique symbols, disabled tax launches, a smaller icon limit, a disabled wNEAR quote or a fee raise above the 0.086 NEAR margin each refuse the launch at once. The deposit then returns to the Dragon and stays there as reserve, where it later buys the creator's own coin. Nobody else gains.
11. **Orphan launches.** A second launch by our own Dragon names our tithe as `fee_to` and looks real everywhere but `dragon.get_coin()`. `get_coin` is the only mark of the real coin, `launch_sent {ok: false}` means "resume, do not relaunch", and launches are at least 10 minutes apart. The creator can cancel an orphan parked before its dev buy with `nearly_cancel_stuck`, after bind too. The coin is set once, so nobody can bind an orphan. An orphan's fees still go to our tithe.
12. **Nearly moves new launches to another DCL.** A launch on another DCL than the Dragon's fails bind's pool check, and that system can never bind. The deploy runs launch and bind in one sitting. WYRMLING's launch landed on `dclv2.ref-labs.near` and bound.
13. **A coin names our tithe as `fee_to`.** Harmless. Its fees enrich our tithe, and it cannot be bound, because its creator is not our Dragon.
14. **Front-running or stalling bind.** Only the creator can call `bind`, and the record must name the Dragon, so there is nothing to front-run. Bind writes no in-flight state. A second bind's callback finds the coin set and does nothing. The robbery attempts on WYRMLING included a stranger's `bind`, refused with "only the creator".

## Keyless permanence

15. **A wrong preset value is permanent.** `Preset::check` refuses values outside the envelope, but a value inside it that later proves bad cannot be fixed. Only the Council moves the four tunable fields, inside their bounds. WYRMLING ran the same `hoarder` preset first.
16. **A wiring mistake in the deploy.** `launch` refuses until the tithe, scales and Dragon name each other and the creator, share a preset hash and engine version, and have 100 vaults. A mis-wired system never reaches Nearly, so `fee_to` is never spent on it. The deploy script reads the same three views before it deletes any key. WYRMLING's wiring check passed before its launch.
17. **Nearly changes its view shapes before we bind.** Bind's record check fails closed, and the Dragon stays unbound. A new system would be needed, because the tithe names the Dragon. The deploy runs launch and bind in one sitting and checks Nearly's code hash the same day.
18. **Nearly changes after bind.** Nearly's factory is owned by a DAO and can change its own contracts after WYRM binds. A change that reroutes fees would starve the tithe, and nothing keyless can follow it. A higher sell tax makes Hunt sells fail closed on `min_out`. A lower one widens the slippage room. Buys are unaffected, because DCL checks `min_out` on the gross amount. This is disclosed in the README and here, and no code mitigates it.
19. **New presets need new code.** The preset table is compiled, so a new preset means a new engine build and a new `ENGINE_VERSION`. Existing systems keep their code and preset forever. verify keeps the released code hashes and the presets of each release in `verify/releases.json`.
20. **A preset without the tax.** The tax is a constant, and the 2% slice-cap ceiling holds only with it. A preset without the tax would need a 0.5% ceiling, the sandwich break-even the Gym measured ([PARAMETERS.md](PARAMETERS.md#slice-cap-impact-and-sandwiches)). No such preset exists.

## Council

21. **No WYRM vote before the hackathon deadline.** Seasoning is 7 days and a vote 2 days, so the first WYRM proposal opens 7 days after its sale opens at bind. WYRM was bound on Nov 11, 2026, so its first vote can open on Nov 18. The Council was shown on testnet instead: the `drill` system `council.thewyrm.testnet` ran the same code against mock Nearly, mock DCL and a mock taxed token, labelled "testnet, mock Nearly". The sandbox runs the same six scenarios on the mainnet timings (`contracts/tests/tests/council_scenarios.rs`).
22. **A whale buys the Council.** The sale is 90 Scales at 2 NEAR, so 180 NEAR buys every vote. A holder of two thirds of the electorate passes any Strict vote. What a captured Council can do is bounded: grant or burn at most 50 bps of the hoard per order and 100 bps per 30 days each, move the budget, cap and Hunt line inside the preset's bounds, and set auto-burn up to 50%. It can never move NEAR, change a rule or touch the reserve.
23. **A Strict vote may never reach two thirds turnout.** Grant and Hunt-on may never pass in practice. That matches the intent.
24. **Concurrent Grants.** Several Grants may be open at once. Each is checked against the window when it finalizes, and one that would break the cap ends LeashHeld with the window named as the reason. The testnet drill ended a third Grant LeashHeld this way, with the reason "grants in any 30 days are at most 100 bps of the hoard".
25. **A starved finalize.** `finalize` refuses less than 150 Tgas prepaid, so nobody can push a passed order to LeashHeld by underfunding it. A test pins the floor.
26. **A light majority sets auto-burn to 50%.** That halves the hoard's growth. It enriches nobody, so it is a Light vote.
27. **Proposal and Signal spam.** Each proposal costs a 1 NEAR bond, which goes to the Dragon below 30% turnout, plus a 0.01 NEAR storage fee. Each proposer may have one open proposal.

## Unsold Scales and the tithe

28. **The retire call never lands.** `close_sale` retries while the tithe has not answered. Until it does, retired vaults keep receiving hundredths, and `sweep` moves them to the Dragon. Income is delayed, not lost.
29. **A scales bug retires a sold Scale.** The tithe accepts one number from scales once, and it can only describe the suffix from that id to 90. A test pins `retire(sold + 1)`.
30. **A mint-sweep race.** A tithe round can land between a mint and its sweep, so a sliver of the new owner's first income goes to the Dragon. It is at most one round's hundredth and errs toward the Dragon.
31. **A 10-minute lock after each mint.** A buyer cannot list or transfer a new Scale until its sweep answers. The Lair shows the lock.
32. **Holder royalties reach holders only in part.** With `RoyaltyTo::Holders` the royalty would go through the tithe, so half reaches the Dragon. No preset uses it. Every preset pays the 5% royalty to the creator.

## Factory

The factory and the Summon wizard are designed ([design.md section 2f](design/v2/design.md#2f-factory-new-summonroot) and [section 5c](design/v2/design.md#5c-factory-summon)) but not built. Risks 33 to 36 apply only once a factory exists.

33. **Impersonation through the factory.** The design refuses the labels `wyrm`, `wyrmling`, `thewyrm`, `summon` and `drill`. Look-alikes remain possible. Every Lair page names its system's creator, and WYRM is linked by its direct root, `thewyrm.near`.
34. **A broken summon.** The design checks every `new` argument before it deploys, so a batch can fail only on gas or storage funding. `refund(label)` would pay back the failed batches' funding.
35. **Cost barrier.** About 14.9 NEAR locked per summon plus Nearly's launch quote and the 0.086 NEAR margin, all paid by the creator (estimate).
36. **Our own factory storage.** About 9 to 10 NEAR locked once for the stored engine code (estimate). Not spent, because no factory is deployed.
37. **The vault code is missing on a network.** `create_vaults` fails until someone calls `publish_vault_code`. WYRMLING published the vault code on mainnet for 1.681 NEAR (measured), and every later system on mainnet, WYRM's included, uses it by hash for free. The drill deploy published it on testnet.

## Engine

38. **An outflow callback that never runs.** The reconcile waits while an outflow is in flight. An outflow older than 10 minutes counts as settled at the next observation, and its late callback changes nothing. A failed transfer found this way is booked as donated, so the hoard stays right, but its window share is not freed. The callback's gas is tested.
39. **The tax costs every Dragon buy 4%.** The Gym models it ([PARAMETERS.md](PARAMETERS.md#market-model)). The creator side of the Dragon's own tax returns through the tithe, half of it to the Dragon. WYRMLING's first buy arrived at exactly the quote less 4%.
40. **Nearly's seller sells the tax tokens on its own schedule.** Each sale lowers the price, which can read as a dip. The `hoarder` preset has no dip extras, so a dip changes the mood, not the spend. On WYRMLING the tax tokens were still unsold at the last reading (item 6).
41. **An ash deposit on every burn.** Each Burn and auto-burn batch attaches a storage deposit for ash on the token, and the token refunds it when ash is already registered.
42. **Wasm growth.** A larger scales wasm makes every system cost more storage. The v2.0 build is pinned by hash in `verify/releases.json`, and `contracts/build.sh` fails when any source file passes 1,000 lines.
