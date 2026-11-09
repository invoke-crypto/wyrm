import { test } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { CLASS, MAX_VOTE_TOKENS, NEAR, SCENARIOS, ballots, bondReturns, context, decided, options, planCouncil } from "./drill.mjs";
import { b58encode } from "./chain.mjs";

const MIN = 60_000_000_000n;
const SEASON = 10n * MIN;
const VOTING = 30n * MIN;
const BOND = 1_010n * (NEAR / 1_000n);
const HOLDERS = ["thewyrm.testnet", "council.thewyrm.testnet"];
const PLANNED = ["Executed", "Executed", "Rejected", "Executed", "Carried", "Executed", "LeashHeld"];

// The status each scenario reaches: finalize's decision, then the Dragon's verdict on an executing Order. The Dragon
// refuses a Grant once the 30-day window would pass 100 bps of the hoard, which is the leash scenario's third Grant.
function play(electorate) {
  let granted = 0;
  return SCENARIOS.map((sc) => {
    const d = decided(sc, sc.votes(electorate), electorate);
    if (d !== "Executing") return d;
    if (sc.slug.includes("grant") || sc.slug === "leash-held") {
      if (granted + 40 > 100) return "LeashHeld";
      granted += 40;
    }
    return "Executed";
  });
}

test("the scenarios end Executed, Executed, Rejected, Executed, Carried, then the Grant that fills the window and LeashHeld", () => {
  assert.deepEqual(SCENARIOS.map((s) => s.expect), PLANNED);
  for (const e of [45, 50, 90]) assert.deepEqual(play(e), PLANNED, `electorate ${e}`);
});

test("for 50 Scales every tally fits the electorate, returns its bond, and lands on the side of the class rule it means to", () => {
  for (const e of [45, 50]) {
    for (const sc of SCENARIOS) {
      const { yes, no } = sc.votes(e);
      assert.ok(yes + no <= e, `${sc.slug} at ${e}: ${yes} + ${no} votes`);
      assert.ok(bondReturns(yes, no, e), `${sc.slug} at ${e} keeps its bond`);
      assert.equal(decided(sc, { yes, no }, e) !== "Rejected", sc.expect !== "Rejected", `${sc.slug} at ${e}`);
    }
  }
  const at50 = Object.fromEntries(SCENARIOS.map((sc) => [sc.slug, sc.votes(50)]));
  assert.deepEqual(at50, {
    setslices: { yes: 23, no: 6 },
    grant: { yes: 28, no: 6 },
    "hunt-fails": { yes: 38, no: 10 },
    burn: { yes: 23, no: 0 },
    signal: { yes: 15, no: 0 },
    "second-grant": { yes: 45, no: 0 },
    "leash-held": { yes: 45, no: 0 },
  });
});

test("the Strict tallies sit on the class edges: the Grant at two thirds turnout and 80% yes, Hunt at 79%", () => {
  for (const e of [45, 50]) {
    const grant = SCENARIOS.find((s) => s.slug === "grant").votes(e);
    const t = grant.yes + grant.no;
    assert.equal(t, Math.ceil((2 * e) / 3), "the least turnout that passes");
    assert.ok(grant.yes / t >= 0.8 && grant.yes / t < 0.83, `${grant.yes} of ${t}`);
    assert.equal(CLASS.Strict.passes(grant.yes - 1, grant.no + 1, e), false, "one yes fewer fails");
    const hunt = SCENARIOS.find((s) => s.slug === "hunt-fails").votes(e);
    const share = hunt.yes / (hunt.yes + hunt.no);
    assert.ok(share >= 0.79 && share < 0.8, `hunt at ${share}`);
    assert.equal(CLASS.Strict.passes(hunt.yes + 1, hunt.no - 1, e), true, "one more yes would pass");
  }
  assert.deepEqual(SCENARIOS.find((s) => s.slug === "grant").votes(45), { yes: 24, no: 6 }, "the sandbox suite's tally");
  assert.deepEqual(SCENARIOS.find((s) => s.slug === "hunt-fails").votes(45), { yes: 34, no: 9 });
});

test("Strict needs 45 voters and a Light Signal passes on 30% turnout", () => {
  const signal = SCENARIOS.find((s) => s.slug === "signal");
  assert.equal(decided(signal, signal.votes(45), 45), "Carried");
  assert.equal(decided(signal, { yes: 13, no: 0 }, 45), "Rejected", "29% turnout");
  const grant = SCENARIOS.find((s) => s.slug === "grant");
  assert.equal(decided(grant, { yes: 44, no: 0 }, 44), "Rejected", "under the Strict minimum electorate");
});

test("Grant, Burn and the leash Grant are 40, 10 and 40 bps of the hoard read when each opens", () => {
  const ctx = { hoard: "1000000", grantees: ["a.testnet", "b.testnet", "c.testnet"] };
  const kinds = Object.fromEntries(SCENARIOS.map((s) => [s.slug, s.kind(ctx)]));
  assert.deepEqual(kinds.grant, { Order: { Grant: { to: "a.testnet", amount: "4000" } } });
  assert.deepEqual(kinds.burn, { Order: { Burn: { amount: "1000" } } });
  assert.deepEqual(kinds["second-grant"], { Order: { Grant: { to: "b.testnet", amount: "4000" } } });
  assert.deepEqual(kinds["leash-held"], { Order: { Grant: { to: "c.testnet", amount: "4000" } } });
  assert.equal(kinds.signal, "Signal");
  assert.deepEqual(kinds["hunt-fails"], { Order: { SetPosture: { hunt: { sell_multiple_bps: 25_000, max_sell_bps: 1_000 } } } });
});

const scales = (counts) => {
  const out = [];
  let id = 1;
  counts.forEach((n, h) => { for (let i = 0; i < n; i++) out.push({ token_id: String(id++), owner: HOLDERS[h], voting: true, hold_start: "0" }); });
  return out;
};

test("ballots vote the lowest ids yes, the next ones no, by owner in chunks of 25, and skip Scales that voted", () => {
  const s = scales([30, 15]);
  const b = ballots(s, { yes: 34, no: 9 }, []);
  assert.deepEqual(b.map((x) => [x.signer, x.approve, x.tokens.length]), [
    [HOLDERS[0], true, 25], [HOLDERS[0], true, 5], [HOLDERS[1], true, 4], [HOLDERS[1], false, 9],
  ]);
  assert.ok(b.every((x) => x.tokens.length <= MAX_VOTE_TOKENS));
  assert.deepEqual(b[3].tokens, ["35", "36", "37", "38", "39", "40", "41", "42", "43"]);
  const again = ballots(s, { yes: 34, no: 9 }, b.slice(0, 2).flatMap((x) => x.tokens));
  assert.deepEqual(again.map((x) => [x.signer, x.approve, x.tokens.length]), [[HOLDERS[1], true, 4], [HOLDERS[1], false, 9]]);
  assert.deepEqual(ballots(s, { yes: 34, no: 9 }, b.flatMap((x) => x.tokens)), []);
});

const proposal = (id, slug, proposer, status, endsAt = 10n * VOTING) => ({
  id, description: `drill: ${slug}`, proposer, status, electorate: 45, ends_at: String(endsAt), created_at: "0",
});
const snap = (over = {}) => ({
  now: SEASON + 1n, closed: true, electorate: 45, season: SEASON, bond: BOND, proposals: [], votes: {},
  scales: scales([23, 22]), holders: HOLDERS, spendable: 10n * NEAR, hoard: "1000000", grantees: [HOLDERS[0], HOLDERS[1], HOLDERS[0]], ...over,
});

test("the planner opens one scenario per free proposer, in order, while the budget left covers each bond", () => {
  assert.deepEqual(planCouncil(snap()).actions.map((a) => [a.type, a.slug, a.proposer]), [
    ["propose", "setslices", HOLDERS[0]], ["propose", "grant", HOLDERS[1]],
  ]);
  const oneBond = planCouncil(snap({ spendable: BOND + 30n * (NEAR / 1_000n) }));
  assert.deepEqual(oneBond.actions.map((a) => a.slug), ["setslices"]);
  assert.match(oneBond.wait, /next bond needs/);
  const none = planCouncil(snap({ spendable: BOND }));
  assert.deepEqual(none.actions, []);
  assert.match(none.wait, /next bond needs/);
});

test("the planner waits for a closed sale, every Scale the holders vote with, and the end of seasoning", () => {
  assert.match(planCouncil(snap({ closed: false })).wait, /sale is still open/);
  assert.match(planCouncil(snap({ scales: scales([23, 21]) })).wait, /44 of 45/);
  const seasoning = planCouncil(snap({ now: SEASON - 1n }));
  assert.deepEqual(seasoning.actions, []);
  assert.equal(seasoning.until, SEASON);
});

test("the planner votes an open proposal, finalizes it once due, and never opens the leash Grant before both Grants executed", () => {
  const open = snap({ proposals: [proposal(0, "setslices", HOLDERS[0], "Open")] });
  const votes = planCouncil(open).actions.filter((a) => a.type === "vote");
  assert.deepEqual(votes.map((a) => [a.signer, a.approve, a.tokens.length]), [[HOLDERS[0], true, 20], [HOLDERS[0], false, 3], [HOLDERS[1], false, 2]]);
  const due = planCouncil({ ...open, now: 10n * VOTING });
  assert.deepEqual(due.actions[0], { type: "finalize", slug: "setslices", id: 0 });
  assert.ok(!due.actions.some((a) => a.type === "vote"), "no votes once the vote has ended");

  const done = (slug, i, status) => proposal(i, slug, HOLDERS[i % 2], status);
  const first = ["setslices", "grant", "hunt-fails", "burn", "signal"].map((s, i) => done(s, i, PLANNED[i]));
  const grantPending = planCouncil(snap({ proposals: [...first, done("second-grant", 5, "Executing")] }));
  assert.ok(!grantPending.actions.some((a) => a.slug === "leash-held"), "the second Grant has not executed yet");
  const ready = planCouncil(snap({ proposals: [...first, done("second-grant", 5, "Executed")] }));
  assert.deepEqual(ready.actions.map((a) => [a.type, a.slug]), [["propose", "leash-held"]]);
  assert.deepEqual(ready.actions[0].kind, { Order: { Grant: { to: HOLDERS[0], amount: "4000" } } });
});

test("a rerun on a finished drill plans nothing and reports done", () => {
  const all = SCENARIOS.map((sc, i) => proposal(i, sc.slug, HOLDERS[i % 2], sc.expect === "LeashHeld" ? { LeashHeld: { error: "window" } } : sc.expect));
  const plan = planCouncil(snap({ proposals: all }));
  assert.deepEqual(plan.actions, []);
  assert.equal(plan.done, true);
  assert.equal(planCouncil(snap({ proposals: all.slice(0, 6) })).done, false);
});

test("options refuse mainnet, a send without a budget, and fewer than 45 Scales", () => {
  assert.throws(() => options(["--network", "mainnet", "--root", "x.near"]), /testnet only/);
  assert.throws(() => options(["--network", "testnet", "--root", "x.testnet", "--send"]), /--budget/);
  assert.throws(() => options(["--network", "testnet", "--root", "x.testnet", "--scales", "44"]), /45/);
  const o = options(["--network", "testnet", "--root", "x.testnet", "--send", "--budget", "2.565022"]);
  assert.equal(o.budget, 2_565_022n * 10n ** 18n);
  assert.deepEqual(o.steps, ["buy", "close", "hoard", "council", "report"]);
});

test("the budget is net spend: a locked bond counts until its finalize returns it, and NEAR from elsewhere adds nothing", async () => {
  const seed = Buffer.alloc(32, 9);
  const pub = crypto.createPublicKey(crypto.createPrivateKey({ key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]), format: "der", type: "pkcs8" })).export({ format: "der", type: "spki" }).subarray(12);
  const balance = { [HOLDERS[0]]: 10n * NEAR, [HOLDERS[1]]: 0n };
  const rpc = async (_, params) => ({ amount: String(balance[params.account_id]), storage_usage: 0 });
  const gas = NEAR / 1_000n;
  const submit = async (tx) => {
    const a = tx.actions[0];
    balance[tx.signer] -= gas + a.deposit;
    if (a.method === "finalize") balance[HOLDERS[0]] += BOND;
    return {};
  };
  const opts = options(["--network", "testnet", "--root", HOLDERS[1], "--send", "--budget", "3"]);
  const ctx = context(opts, { rpc, credentials: { account_id: HOLDERS[0], private_key: `ed25519:${b58encode(Buffer.concat([seed, pub]))}` }, out: () => {}, submit });
  const bond = () => ctx.submit({ signer: HOLDERS[0], receiver: "scales", actions: [{ type: "call", method: "propose", deposit: BOND }] }, BOND);
  await bond();
  assert.equal(ctx.spent, BOND + gas, "a locked bond is spent");
  balance[HOLDERS[0]] += 10n * NEAR;
  await bond();
  assert.equal(ctx.spent, 2n * (BOND + gas), "10 NEAR from elsewhere did not lower the spend");
  await assert.rejects(bond(), /BLOCKED: scales.propose needs 1.0300 NEAR and the budget leaves 0.9780 NEAR/);
  assert.ok(balance[HOLDERS[0]] > 17n * NEAR, "the holders hold far more than the budget, as after the outside top-up");
  await ctx.submit({ signer: HOLDERS[0], receiver: "scales", actions: [{ type: "call", method: "finalize", deposit: 0n }] });
  assert.equal(ctx.spent, BOND + 3n * gas, "the finalize that returns a bond books it back");
  await bond();
  const plan = planCouncil(snap({ spendable: ctx.spendable() }));
  assert.deepEqual(plan.actions, [], "the planner opens no bond the budget cannot cover");
  assert.match(plan.wait, /next bond needs .* more of the budget/);
});
