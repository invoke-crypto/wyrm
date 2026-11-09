# WYRM v2 invariants

Every invariant of the [v2 design](design.md) and where it is enforced. **Type** means an illegal state cannot be written. **Runtime** means a `require!`, a refusal path or a checked result. **Test** names the test that pins it, and a test after a type or runtime check is the regression guard. New tests are marked (new).

## Presets and bounds

| # | Invariant | Enforced |
|---|---|---|
| P1 | Every preset's launch strategy lies inside its own bounds, and its bounds lie inside the envelope. | Runtime: `Preset::check`, in a unit test over every name and again in each engine's `new`. `contracts/build.sh` runs the tests before the release build. Test: `every_preset_checks` (new) |
| P2 | Budget ceiling plus the sum of dip extras is at most 8,000 bps, and dip steps strictly increase. | Runtime: `Preset::check` rule 2. Test: `every_preset_checks`, plus `the_envelope_refuses_each_broken_rule` (new) |
| P3 | Every launch asks Nearly for the same tax, 400 bps a side with the creator side 10,000, and requires it back at bind. | Type: `envelope::TAX` is the only tax value, and `Preset` has no tax field. Test: `launch_args_carry_fee_to_mode_and_tax` (new), `bind_refuses_a_tax_other_than_the_constant` (new) |
| P4 | A disabled feature launches off, and Hunt may launch on only where the Hunt feature exists. | Runtime: `Preset::check` rules 3 and 4. Test: `every_preset_checks` (new) |
| P5 | A preset usable on mainnet has 7-day seasoning, 2-day votes and a 14-day sale. | Runtime: `Preset::check` rule 5. Test: `only_the_drill_preset_has_short_times` (new) |
| P6 | The drill preset runs only on an account that ends in `.testnet`. | Runtime: `Preset::allowed_on` in both engines' `new` and in `summon`. Test: `drill_is_refused_off_testnet` (new), verify check 2 |
| P7 | Every strategy number in the table equals the Gym's `gym/out/presets.json`. | Test: `the_table_matches_the_gym` (new, reads the file with `include_str!`) |
| P8 | A preset value exists in one place. Engines store the preset's name, not its values, apart from the Dragon's live `strategy`. | Type: `preset: PresetName` in state, values from `wyrm_types::preset(name)`. Test: review |
| P9 | scales and dragon of one system run the same preset and the same engine release. | Runtime: `on_wiring` compares `preset_hash` and `ENGINE_VERSION` from both `get_config` views with its own. A version constant bumped per release stands in for "same build", because a contract cannot read another account's code hash. Test: `launch_refuses_a_preset_or_version_mismatch` (new), verify check 2 for the code hashes |
| P10 | Nothing in a preset, the envelope, the vote rules or the windows changes after init. | Type: no setter exists, and the table is compiled. Test: verify check 2 (code hash) |
| P11 | No Council order sets a value outside the preset's bounds, uses a disabled feature, or grants or burns zero. | Runtime: `Order::check` at `propose` and inside `admit` at `obey`. Test: `every_order_inside_and_outside_its_bounds` (changed from `every_council_method_inside_and_outside_its_limits...`), `disabled_features_refuse_at_propose_and_at_obey` (new), `a_zero_grant_or_burn_is_refused` (new) |
| P12 | The Council can move only the budget, the cap, the Hunt line and auto-burn. Dip ladder, cadence, tax, spike guard, gap and slippage are fixed for life. | Type: `Order` has no variant for them. Test: `order_variants_cover_only_tunable_fields` (new, exhaustive match) |
| P13 | The collection text is bounded: name at most 64 bytes, symbol at most 16 ASCII letters and digits, icon and each tier medium an https or ipfs URL of at most 512 bytes. | Runtime: `CollectionArgs::check` in scales `new` and in `summon`. Test: `collection_args_refuse_each_broken_rule` (new) |

## Launch

| # | Invariant | Enforced |
|---|---|---|
| L1 | Only the creator calls `launch`, `bind` and `nearly_cancel_stuck`. | Runtime: predecessor equals `peers.creator`. Test: `launch_and_bind_refuse_a_stranger` (new) |
| L2 | Nearly is never asked to name a tithe that does not name this Dragon, these scales and this wrap, or a scales that names another Dragon, tithe or creator, another preset hash or engine version, or has fewer than 100 vaults. | Runtime: `on_wiring`. Test: `launch_refuses_bad_wiring_and_returns_the_whole_deposit` (new) |
| L3 | Launch args always carry `fee_to = tithe`, `fee_mode = "creator"`, `envelope::TAX`, and no quote, share or init-point override. | Type: `CoinArgs` has no such fields, and `launch_args` writes them. Test: `launch_args_carry_fee_to_mode_and_tax` (new) |
| L4 | Coin text passes Nearly's synchronous text rules. The icon is an https or ipfs URL of at most 512 bytes with no `"`, `\` or control characters, so its JSON length equals its byte length. | Runtime: `CoinArgs::parse`. Test: `coin_args_follow_nearlys_limits` (new) |
| L5 | No NEAR reaches Nearly when a DCL pool already exists at the coin's predicted pool id. | Runtime: `on_pool_checked` after `predicted_pool`. Test: `launch_refuses_a_precreated_pool_and_returns_the_deposit` (new, sandbox), `predicted_pool_follows_nearlys_slug_rule` (new, cases for `-2`, `-3` and the reserved `lock`) |
| L6 | The Dragon sends Nearly exactly `quote.total + LAUNCH_MARGIN`, and returns the rest of the same call's deposit to the creator. Sent plus returned equals the attached deposit. | Runtime: `on_pool_checked`, computed from callback arguments. Test: `launch_sends_the_quote_and_margin_and_returns_the_rest` (new, sandbox) |
| L7 | The Dragon never sends the creator NEAR that Nearly returned. | Runtime: the only transfers to the creator are in `on_wiring` and `on_pool_checked`, before Nearly is called. Test: `a_refused_launch_leaves_the_deposit_in_the_dragon` (new, sandbox with a panicking mock-nearly), verify check 8 |
| L8 | Two launches are at least 10 minutes apart, and no launch happens once a coin is bound. | Runtime: `last_launch_at` and `coin.is_none()`. Test: `launch_waits_ten_minutes_and_never_runs_once_bound` (new) |
| L9 | After bind the Dragon calls Nearly only to cancel a stuck launch other than the bound one. | Runtime: `launch`, `bind` and `nearly_resume` refuse once bound. `nearly_cancel_stuck` refuses the bound id. Test: `after_bind_only_an_orphan_can_be_cancelled` (new), verify check 8 |

## Binding

| # | Invariant | Enforced |
|---|---|---|
| B1 | The Dragon trades only after it is bound. | Type: only `Some(Coin)` holds a token and pool id. Runtime: `poke` refuses `"not bound"`. Test: `poke_refuses_until_bound` (new) |
| B2 | `coin` is set at most once and never cleared. | Type: no code path writes `None` after init. Runtime: `on_pool_seen` writes only while `coin` is None. Test: `bind_runs_once` (new) |
| B3 | Two binds in flight together bind once. The later callback does nothing. | Runtime: `on_launch_record` and `on_pool_seen` return early when `coin` is Some. Test: `two_concurrent_binds_bind_once` (new) |
| B4 | The bound launch's creator is this Dragon. | Runtime: `check_record`. Test: `bind_refuses_a_launch_someone_else_created` (new, sandbox) |
| B5 | The bound launch pays its creator fees to this tithe, in creator mode, on the wNEAR pair, with `envelope::TAX`. | Runtime: `check_record`. Test: one refusal test per field (new), `bind_refuses_a_tax_other_than_the_constant` (new) |
| B6 | The bound launch is Done with a position. | Runtime: `check_record` (`step == "Done"`, `lpt_id` set). Test: `bind_refuses_a_launch_not_done` (new) |
| B7 | The pool id is Nearly's record, pairs wrap with the bound token in DCL order, and is never typed by hand. | Runtime: `dcl::parse_pool_id` on the record. Test: v1 `dcl` tests and `bind_takes_pool_id_from_the_record` (new) |
| B8 | The bound pool exists on `peers.dcl`, holds wrap and the coin, and is Running. | Runtime: `on_pool_seen`. Test: `bind_refuses_a_pool_on_another_dcl` (new, sandbox), `bind_refuses_a_paused_pool` (new) |
| B9 | The Dragon is registered on its token before its first buy. | Structure: Nearly's CreateToken registers the creator (lib.rs:4508-4513). Test: mock-nearly does the same, and `a_bound_dragon_receives_its_first_buy` (new, sandbox). verify reads `storage_balance_of(dragon)` on the token. |

## Council

| # | Invariant | Enforced |
|---|---|---|
| C1 | Grant and turning Hunt on are Strict: turnout at least 2/3 and yes at least 80% of votes cast. Everything else, Burn, auto-burn, Hunt off and Signal included, is Light: turnout at least 30% and yes > no. | Type: `Order::class` is an exhaustive match, stored on the proposal at creation. Runtime: `VoteClass::passes` in integer math. Test: `thresholds_follow_the_integer_rules` (changed, both classes at every electorate from 1 to 90), `each_order_has_its_class` (new) |
| C2 | A Strict proposal needs the sale Closed and at least 45 voting Scales sold. A Light one needs 10. | Runtime: `propose`. Test: `a_strict_proposal_waits_for_the_sale_and_45` (changed from 35) |
| C3 | Every vote lasts the preset's voting time (2 days on mainnet), with no early close. | Runtime: `ends_at = created_at + times.voting`, `finalize` needs `now >= ends_at`. Test: `nothing_finalizes_early` (changed) |
| C4 | A Scale votes once per proposal, only if it is a sale Scale held since `created_at - times.season`. | Type: `ScaleSet` yes and no, `ScaleId::votes()`. Runtime: `vote`. Test: v1 `a_scale_counts_once_however_often_it_changes_hands`, `proposing_and_voting_need_your_own_seasoned_scale` |
| C5 | Creator Scales 91 to 100 never vote or propose. | Type: `ScaleId::votes()` derived from the id, no `voting` field. Test: `creator_scales_never_vote_or_propose` (changed ids) |
| C6 | The electorate is the sold count at creation, so unseasoned and retired Scales never shrink it and retired ones never join it. | Runtime: `electorate = sold`. Test: `the_electorate_counts_unseasoned_scales` (changed), `retired_scales_never_join_the_electorate` (new) |
| C7 | A passed order reaches the Dragon exactly once, in its finalize transaction, and cannot be banked. | Type: there is no Passed state, and only Open can be finalized. Test: `a_second_finalize_pays_nothing` (kept), `finalize_calls_the_dragon_once_and_holds_it_while_executing` (changed from execute) |
| C8 | A finalize with less than 150 Tgas prepaid changes nothing. | Runtime: `require!(prepaid_gas >= FINALIZE_GAS)` before any write. Test: `finalize_refuses_below_its_gas_floor` (new) |
| C9 | Every Executing proposal ends Executed or LeashHeld, and a LeashHeld proposal carries the Dragon's reason. | Type: `obey` and `on_outflow` answer a `Verdict`, and the Dragon panics only on a wrong caller. Runtime: `on_executed` always writes a status. Test: `the_callback_records_executed_or_leash_held` (kept), `leash_held_names_the_limit` (new) |
| C10 | A Signal never executes anything. | Type: `ProposalKind::Signal` carries no order, and a passing Signal ends Carried. Test: `a_signal_carries_and_executes_nothing` (new) |
| C11 | The bond returns at 30% turnout and otherwise goes to the Dragon, once. | Runtime: the bond is zeroed before the send. Test: v1 bond tests |
| C12 | Only scales can make the Dragon obey. | Runtime: `obey` checks the predecessor and panics otherwise. Test: `only_the_council_moves_settings...` (changed) |
| C13 | No proposal moves NEAR or changes a rule, a bound or a timing. | Type: no `Order` variant does. Test: review, verify check 9 |

## Hoard and outflows

| # | Invariant | Enforced |
|---|---|---|
| H1 | A grant is at most 50 bps of `hoard.wyrm`, and grants in any 30 days total at most 100 bps. | Runtime: `admit` and the `grants` window. Test: v1 `grants_in_any_30_days_stay_within_one_percent_of_the_hoard` (changed to `obey`) |
| H2 | The same caps hold for Burn, in its own window. | Runtime: the `burns` window. Test: `burns_in_any_30_days_stay_within_one_percent` (new) |
| H3 | A Burn and an auto-burn pay only `ash`, and `ash` has no keys and no code. Each one deposits ash's storage on the token in the same batch. | Runtime: the recipient is `peers.ash`, and the batch is `storage_deposit(ash)` then `ft_transfer`. Deploy: ash is created with no AddKey and no code. Test: `a_first_burn_registers_ash` (new, sandbox), verify check 1 |
| H4 | Auto-burn takes `auto_burn_bps` of the WYRM each Dragon buy delivered, never of donations, and moves it from `hoard.wyrm` to `ash_due`. | Runtime: `reconcile_wyrm` moves only the bought part. Test: `auto_burn_takes_its_share_of_bought_not_donated` (new, proptest) |
| H5 | A failed outflow returns its amount where it came from, `hoard.wyrm` for Grant and Burn and `ash_due` for auto-burn, and frees its window share. | Runtime: `on_outflow`. Test: v1 `a_failed_grant_restores_the_hoard_and_frees_its_share`, extended to Burn, and `a_failed_auto_burn_returns_to_ash_due` (new) |
| H6 | WYRM owed to ash is never granted, burned by a Council order, or sold by Hunt. | Runtime: the Grant and Burn caps and `sell_room` read `hoard.wyrm`, which excludes `ash_due`. Test: `ash_due_is_outside_every_cap` (new) |
| H7 | The Dragon sends NEAR only into swaps, into Nearly's `launch` (the quote plus the margin of the same call), back to the creator (the unsent rest of the same call), and into storage deposits. | Runtime: no other transfer exists. Test: verify check 8 |

## Engine (v1, kept, with the tax, the cadence and the new reconcile)

| # | Invariant | Enforced |
|---|---|---|
| E1 | Daily spend never exceeds the budget, and no slice exceeds the impact cap. | Runtime: `rules`. Test: v1 rules property tests |
| E2 | With a spread, the breath budget opens linearly over `spread_minutes` and the spread ends inside the UTC day. With no spread it opens at the breath minute, as v1. | Runtime: `Day::starting` and `day_budget`. Test: `a_spread_opens_the_budget_linearly_inside_the_day` (new, mirrors the Gym's `allowance`) |
| E3 | Two slices are at least `gap_seconds` apart. | Runtime: `decide`. Test: `the_gap_comes_from_the_preset` (new) |
| E4 | Buy `min_out` is the gross quote less 150 bps. | Runtime: `rules::min_out(quoted_out)`. Test: `buy_min_out_is_on_the_gross_quote` (new, sandbox with the taxed mock-ft) |
| E5 | Sell quotes and sell `min_out` are on the net input after the sell tax. | Runtime: `sell_quote_input`. Test: `a_taxed_sell_quotes_the_net_and_passes_min_out` (new) |
| E6 | Every outflow is counted once. The WYRM reconcile does not run while `pending` holds an outflow younger than 10 minutes, and otherwise `seen = balance - ash_due`. `wyrm_bought` counts only what arrived, at most the expected net of swaps since the last reconcile, and the rest is donated. | Runtime: `on_observe` skips `reconcile_wyrm` while an outflow is in flight, and `reconcile_wyrm` takes no in-flight argument. Test: v1 `donations_never_lower_the_average_cost` with a tax case, and `a_poke_between_an_outflow_and_its_callback_books_nothing` (new, sim: the transfer lands, a poke observes, then `on_outflow` runs, and neither `hoard.wyrm` nor `donated` moves twice), `a_lost_outflow_callback_expires_after_ten_minutes` (new) |
| E7 | A failed taxed sell returns the gross amount to the hoard. | Runtime: the token's resolve refund and reconcile. Test: `a_refunded_taxed_sell_restores_the_gross` (new, sandbox) |
| E8 | A late callback of a replaced step changes nothing but the day budget. | Runtime: the step match. Test: v1 `a_stale_step_is_replaced...`, `a_late_refund_returns_the_day_budget...` |
| E9 | Quote 30 Tgas, swap 180 Tgas, sells carry `skip_unwrap_near: true`, and the poke chain fits in 1,000 Tgas with a 760 Tgas floor. | Runtime: constants. Test: `the_poke_buy_chain_fits_in_1000_tgas...` (changed floor), `sell_args_skip_unwrap` (new) |
| E10 | No Dragon source file passes 1,000 lines. | Build: `contracts/build.sh` fails on a longer file. |

## Scales and sale

| # | Invariant | Enforced |
|---|---|---|
| S1 | Exactly 100 ids exist: 90 sale ids and 10 creator ids minted at init. | Type: `SALE_IDS` and `CREATOR_IDS`. Test: `creator_scales_91_to_100_belong_to_the_creator_and_never_vote` (changed) |
| S2 | The sale opens only when the Dragon binds, once, for the preset's sale length. | Type: `Sale` enum. Runtime: `open_sale` accepts only the Dragon and acts only while Waiting, and never panics after the caller check. Test: `the_sale_opens_on_the_dragons_call_and_never_reopens` (new), `open_sale_refuses_anyone_but_the_dragon` (new) |
| S3 | Each sale pays exactly `price * creator_bps / 10,000` to the creator and the rest to the Dragon. | Runtime: one split function in `buy_scale`. Test: `sale_splits_every_price_creator_and_dragon` (changed) |
| S4 | Income a vault received before its Scale was minted goes to the Dragon. | Runtime: the mint sweep under the claim lock. Test: `a_mint_sweeps_pre_sale_income_to_the_dragon` (new) |
| S5 | An unminted Scale's vault can be released only to the Dragon. | Runtime: `sweep` requires unminted and names `dragon`. Test: `sweep_pays_only_the_dragon_and_only_for_unminted` (new) |
| S6 | No Scale is minted after the sale closes. | Runtime: `buy_scale` requires Open and `now < ends_at`. Test: `the_sale_closes_at_ends_at_and_unsold_ids_are_never_minted` (changed) |
| S7 | The retired ids are exactly `sold + 1` to 90, and scales tells the tithe until it acknowledges. | Runtime: `close_sale`, `on_retired`. Test: `close_sale_retires_the_unsold_suffix_and_retries_the_tithe` (new) |
| S8 | The resale royalty is 5% and goes to `royalty_receiver`, which is the creator for `RoyaltyTo::Creator` and the tithe for `RoyaltyTo::Holders`. `nft_payout` has at most two entries. | Runtime: `pay_sale`, `nft_payout`. Test: v1 market and nft tests, run for both `RoyaltyTo` values |
| S9 | A claim in flight blocks every transfer, listing, sale and accepted offer, and this covers the mint sweep. | Runtime: `claim_in_flight`. Test: v1 `a_claim_in_flight_blocks...` |
| S10 | Escrowed NEAR never pays for storage. | Runtime: `assert_storage_paid`. Test: v1 `escrowed_near_never_pays_for_storage` |

## Tithe and vault

| # | Invariant | Enforced |
|---|---|---|
| T1 | Before retirement, each round sends half to the Dragon and 1/100 of the rest to each vault, and the dust stays. | Runtime. Test: v1 `round_splits_half_and_hundredths_and_keeps_dust` |
| T2 | After retirement, the Dragon's transfer adds one hundredth of the vault half per retired id, and retired vaults get nothing. Live vaults get exactly what they got before. | Runtime: `distribute`. Test: `a_round_after_retirement_pays_the_retired_shares_to_the_dragon` (new) |
| T3 | Only scales can retire, only ids from a single `first_unsold` (1 to 90) up to 90, and only once. A repeat of the same value is a no-op. | Type: one `Option<u8>` describes a suffix. Runtime: `retire`. Test: `retire_takes_one_suffix_once_from_scales_only` (new) |
| T4 | The tithe never spends its storage stake. | Runtime. Test: v1 `keeps_the_storage_stake...` |
| T5 | Only the parent can release a vault. | Runtime. Test: v1 `release_refuses_anyone_but_the_parent` |

## Factory

| # | Invariant | Enforced |
|---|---|---|
| F1 | A label is used once, and reserved labels never. | Runtime: the `summons` insert happens before any account is created, and RESERVED is checked. Test: `a_label_is_used_once` (new) |
| F2 | The factory deploys only the code whose hash was fixed in `new`. | Runtime: `store_code` checks sha256. Test: `store_code_takes_only_the_fixed_hash` (new) |
| F3 | A summoned system is wired to itself and to its preset by name. | Runtime: the factory computes every peer from the label. Test: `factory_sandbox` reads each `get_config` (new) |
| F4 | Every argument of every `new` in a summon is checked before any account is created, so a batch can fail only on gas or storage funding. | Runtime: `summon` runs `Preset::check`, `allowed_on` and `CollectionArgs::check`. Test: `summon_refuses_bad_collection_text_before_deploying` (new) |
| F5 | A summon's excess goes back at once. A Broken summon records the failed batches' funding as `owed`, and `refund(label)` pays it once, only when the factory's free balance covers `owed_total`. | Runtime: `on_assembled`, `refund`. Test: `a_broken_summon_records_what_is_owed` (new), `refund_pays_only_what_the_factory_holds` (new) |
| F6 | The factory holds no coin, calls no Nearly method and publishes no global code. | Structure: no such code. Test: review |
| F7 | `quote_summon` covers every byte the summon deploys. | Runtime: the quote reads the stored code lengths. Test: `the_quote_pays_for_the_stored_code` (new) |
