//! A native stand-in for the sandbox: it executes every receipt the Dragon creates, in dependency order, against two
//! simulated NEP-141 tokens (the coin can carry Nearly's 4% tax), a constant-product DCL pool with the same JSON
//! shapes as mock-dcl, and Nearly, tithe and scales views shaped as theirs. Each callback runs with exactly the gas the
//! Dragon attached to it, so a gas budget that does not add up fails here too. Wasm execution is not metered on the
//! mocked blockchain: the gas figures are fees and host calls only.
use super::*;
use near_sdk::mock::MockAction;
use near_sdk::serde_json::{self, json, Value};
use near_sdk::test_utils::{get_created_receipts, get_logs, VMContextBuilder};
use near_sdk::{test_vm_config, testing_env, PromiseResult, RuntimeFeesConfig};
use std::collections::HashMap;
use wyrm_types::{CoinArgs, Order};

pub const DRAGON: &str = "dragon.near";
pub const WRAP: &str = "wrap.near";
/// The coin, where Nearly puts symbol WYRM while its slug is unused.
pub const WYRM: &str = "wyrm.nearly.near";
pub const DCL: &str = "dcl.near";
pub const NEARLY: &str = "nearly.near";
pub const TITHE: &str = "tithe.near";
pub const COUNCIL: &str = "scales.near";
pub const ASH: &str = "ash.near";
pub const CREATOR: &str = "creator.near";
pub const KEEPER: &str = "keeper.near";
pub const POOL_ID: &str = "wrap.near|wyrm.nearly.near|10000";
pub const T: u128 = 1_000_000_000_000_000_000; // one WYRM
pub const POOL_NEAR: u128 = 1_000 * ONE_NEAR;
pub const POOL_WYRM: u128 = 1_000_000 * T;
pub const REGISTRATION: u128 = 1_250_000_000_000_000_000_000;
const FEE: u128 = 10_000; // of 1,000,000, as in the pool id
const QUOTE_TOTAL: u128 = 95_050_000_000_000_000_000_000;
const TAX_BPS: u128 = 400;

#[derive(Clone, Default)]
pub struct Pool {
    pub x: u128,
    pub y: u128,
    pub far_x: u128,
    pub far_y: u128,
}

#[derive(Clone)]
struct World {
    near: u128,
    tokens: HashMap<(String, String), u128>,
    pool: Pool,
    held: Vec<(String, String, u128)>,
}

pub struct Chain {
    pub d: Dragon,
    pub now: u64,
    pub seed: [u8; 32],
    w: World,
    pub fail_swaps: bool,
    pub hold_output: bool,
    /// Whether the coin takes Nearly's 4% on transfers out of and into the pool.
    pub taxed: bool,
    /// Holds every `on_outflow` back until `release_outflows`, so a poke can land between a transfer and its callback.
    pub hold_outflows: bool,
    /// An account whose incoming coin transfers fail, as an unregistered one's would.
    pub refuse: Option<&'static str>,
    held_outflows: Vec<(Vec<MockAction>, Vec<PromiseResult>)>,
    /// What Nearly's `launch` received, in NEAR.
    pub launched_with: Vec<u128>,
    /// Every event emitted since the last `take_events`, as (name, data).
    events: Vec<(String, Value)>,
    /// Ggas each Dragon method used, in call order, with the gas it was given.
    pub gas: Vec<(String, u64, u64)>,
}

/// A private callback's arguments, which the Dragon sends in borsh.
fn borsh_args<T: near_sdk::borsh::BorshDeserialize>(args: &[u8]) -> T {
    near_sdk::borsh::from_slice(args).expect("borsh callback args")
}

fn copy(r: &PromiseResult) -> PromiseResult {
    match r {
        PromiseResult::Successful(b) => PromiseResult::Successful(b.clone()),
        PromiseResult::Failed => PromiseResult::Failed,
    }
}

fn id(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn amount_arg(v: &Value, key: &str) -> u128 {
    v[key].as_str().unwrap().parse().unwrap()
}

pub fn peers() -> Peers {
    Peers {
        wrap: id(WRAP),
        dcl: id(DCL),
        nearly: id(NEARLY),
        tithe: id(TITHE),
        scales: id(COUNCIL),
        ash: id(ASH),
        creator: id(CREATOR),
    }
}

pub fn wyrm_coin() -> CoinArgs {
    CoinArgs {
        name: "WYRM".into(),
        symbol: "WYRM".into(),
        icon: Some("https://wyrm.example/icon.png".into()),
        description: "A dragon that buys its own coin.".into(),
        links: Default::default(),
        dev_buy: None,
    }
}

impl Chain {
    pub fn new(near: u128) -> Self {
        Self::with(near, PresetName::Hoarder, false)
    }

    /// A Dragon on `preset` that launched its coin through Nearly with exactly launch's gas floor and bound it with
    /// exactly bind's, so both budgets are checked on every setup.
    pub fn with(near: u128, preset: PresetName, taxed: bool) -> Self {
        testing_env!(VMContextBuilder::new()
            .current_account_id(id(DRAGON))
            .account_balance(NearToken::from_yoctonear(near))
            .build());
        let d = Dragon::new(preset, peers());
        let mut c = Chain {
            d,
            now: 20_400 * DAY + MINUTE,
            seed: [0; 32],
            w: World {
                near,
                tokens: HashMap::new(),
                pool: Pool::default(),
                held: vec![],
            },
            fail_swaps: false,
            hold_output: false,
            taxed,
            hold_outflows: false,
            refuse: None,
            held_outflows: vec![],
            launched_with: vec![],
            events: vec![],
            gas: vec![],
        };
        let receipts = get_created_receipts();
        c.execute(receipts);
        c.w.tokens.insert((WYRM.into(), DCL.into()), 1_000_000_000 * T);
        c.w.tokens.insert((WRAP.into(), DCL.into()), 1_000_000 * ONE_NEAR);
        // Nearly's CreateToken registers the creator, the Dragon, on the coin.
        c.w.tokens.insert((WYRM.into(), DRAGON.into()), 0);
        c.set_pool(POOL_NEAR, POOL_WYRM);
        let deposit = ONE_NEAR;
        c.call_with(CREATOR, launch::LAUNCH_MIN_GAS, deposit, |d| {
            drop(d.launch(wyrm_coin()))
        });
        assert_eq!(c.launched_with.len(), 1, "the launch reached Nearly");
        c.call(CREATOR, launch::BIND_MIN_GAS, |d| drop(d.bind(U64(0))));
        assert!(c.d.coin.is_some(), "bound");
        c.take_events();
        c.gas.clear();
        c
    }

    pub fn set_pool(&mut self, x: u128, y: u128) {
        self.w.pool.x = x;
        self.w.pool.y = y;
    }

    pub fn add_far_liquidity(&mut self, x: u128, y: u128) {
        self.w.pool.far_x += x;
        self.w.pool.far_y += y;
    }

    pub fn skip(&mut self, ns: u64) {
        self.now += ns;
    }

    pub fn balance(&self, token: &str, of: &str) -> u128 {
        *self.w.tokens.get(&(token.into(), of.into())).unwrap_or(&0)
    }

    pub fn near(&self) -> u128 {
        self.w.near
    }

    /// Credits tokens without a call to the holder, like a plain ft_transfer or a swap output.
    pub fn mint(&mut self, token: &str, to: &str, amount: u128) {
        *self.w.tokens.entry((token.into(), to.into())).or_insert(0) += amount;
    }

    pub fn release_held(&mut self) {
        for (token, to, amount) in std::mem::take(&mut self.w.held) {
            self.mint(&token, &to, amount);
        }
    }

    /// Runs every held `on_outflow` with the result its transfer had.
    pub fn release_outflows(&mut self) {
        for (actions, deps) in std::mem::take(&mut self.held_outflows) {
            self.callback(&actions, deps);
        }
    }

    pub fn take_events(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.events)
    }

    fn context(&self, pred: &str, gas: Gas, deposit: u128, results: Vec<PromiseResult>) {
        self.context_with(pred, gas, deposit, results, "");
    }

    fn context_with(&self, pred: &str, gas: Gas, deposit: u128, results: Vec<PromiseResult>, input: &str) {
        let mut ctx = VMContextBuilder::new()
            .current_account_id(id(DRAGON))
            .predecessor_account_id(id(pred))
            .signer_account_id(id(KEEPER))
            .block_timestamp(self.now)
            .random_seed(self.seed)
            .prepaid_gas(gas)
            .attached_deposit(NearToken::from_yoctonear(deposit))
            .account_balance(NearToken::from_yoctonear(self.w.near))
            .build();
        ctx.input = input.as_bytes().into();
        testing_env!(
            ctx,
            test_vm_config(),
            RuntimeFeesConfig::test(),
            Default::default(),
            results
        );
    }

    fn collect(&mut self, method: &str, given: Gas) {
        self.gas.push((
            method.into(),
            env::used_gas().as_gas() / 1_000_000_000,
            given.as_gas() / 1_000_000_000,
        ));
        for l in get_logs() {
            if let Some(j) = l.strip_prefix("EVENT_JSON:") {
                let v: Value = serde_json::from_str(j).unwrap();
                self.events
                    .push((v["event"].as_str().unwrap().into(), v["data"][0].clone()));
            }
        }
    }

    /// Calls a Dragon method as `pred` and runs every receipt it created.
    pub fn call<R>(&mut self, pred: &str, gas: Gas, f: impl FnOnce(&mut Dragon) -> R) -> R {
        self.call_with(pred, gas, 0, f)
    }

    pub fn call_with<R>(&mut self, pred: &str, gas: Gas, deposit: u128, f: impl FnOnce(&mut Dragon) -> R) -> R {
        self.call_input(pred, gas, deposit, "", f)
    }

    /// A call whose method reads its raw input itself.
    pub fn call_input<R>(
        &mut self,
        pred: &str,
        gas: Gas,
        deposit: u128,
        input: &str,
        f: impl FnOnce(&mut Dragon) -> R,
    ) -> R {
        self.w.near += deposit;
        self.context_with(pred, gas, deposit, vec![], input);
        let out = f(&mut self.d);
        let receipts = get_created_receipts();
        self.collect("call", gas);
        self.execute(receipts);
        out
    }

    pub fn poke(&mut self) -> Vec<(String, Value)> {
        self.take_events();
        self.call(KEEPER, GAS_POKE_MIN, |d| drop(d.poke()));
        self.take_events()
    }

    /// Makes the Dragon obey an order as scales does, with the 100 Tgas scales attaches.
    pub fn obey(&mut self, order: Order) {
        let input = json!({ "order": order }).to_string();
        self.call_input(COUNCIL, Gas::from_tgas(100), 0, &input, |d| drop(d.obey()));
    }

    /// ft_on_transfer of `amount` from `token`, as the token's ft_transfer_call makes it.
    pub fn transfer_in(&mut self, token: &str, amount: u128) {
        let input = format!(r#"{{"sender_id":"dev.near","amount":"{amount}","msg":""}}"#);
        self.call_input(token, Gas::from_tgas(30), 0, &input, |d| d.ft_on_transfer());
    }

    pub fn state(&mut self) -> Value {
        self.context(KEEPER, Gas::from_tgas(300), 0, vec![]);
        serde_json::to_value(self.d.get_state()).unwrap()
    }

    fn execute(&mut self, receipts: Vec<near_sdk::mock::Receipt>) {
        // A receipt's dependencies are positions in the mocked action log, where each receipt is logged as its
        // creation followed by its actions.
        let mut at_position = HashMap::new();
        let mut position = 0u64;
        for (k, r) in receipts.iter().enumerate() {
            at_position.insert(position, k);
            position += 1 + r.actions.len() as u64;
        }
        let mut results: Vec<PromiseResult> = Vec::with_capacity(receipts.len());
        for r in receipts {
            let deps: Vec<PromiseResult> = r
                .receipt_indices
                .iter()
                .map(|i| copy(&results[at_position[i]]))
                .collect();
            let result = if r.receiver_id.as_str() == DRAGON {
                if self.hold_outflows && method_of(&r.actions) == "on_outflow" {
                    self.held_outflows.push((r.actions.clone(), deps));
                    PromiseResult::Successful(vec![])
                } else {
                    self.callback(&r.actions, deps)
                }
            } else {
                self.external(r.receiver_id.as_str(), &r.actions)
            };
            results.push(result);
        }
    }

    fn callback(&mut self, actions: &[MockAction], deps: Vec<PromiseResult>) -> PromiseResult {
        let [MockAction::FunctionCallWeight {
            method_name,
            args,
            prepaid_gas,
            ..
        }] = actions
        else {
            panic!("a Dragon receipt holds one function call");
        };
        let method = String::from_utf8(method_name.clone()).unwrap();
        self.context(DRAGON, *prepaid_gas, 0, deps);
        let out: Vec<u8> = match method.as_str() {
            "on_observe" => {
                drop(self.d.on_observe(borsh_args(args)));
                vec![]
            }
            "on_quote" => {
                let (step, plan) = borsh_args(args);
                drop(self.d.on_quote(step, plan));
                vec![]
            }
            "on_swap" => {
                let (step, ticket) = borsh_args(args);
                serde_json::to_vec(&self.d.on_swap(step, ticket)).unwrap()
            }
            "on_outflow" => serde_json::to_vec(&self.d.on_outflow(borsh_args(args))).unwrap(),
            "on_wiring" => {
                let (symbol, launch, deposit) = borsh_args(args);
                drop(self.d.on_wiring(symbol, launch, deposit));
                vec![]
            }
            "on_pool_checked" => {
                let (launch, deposit, cost) = borsh_args(args);
                drop(self.d.on_pool_checked(launch, deposit, cost));
                vec![]
            }
            "on_launch_sent" => serde_json::to_vec(&self.d.on_launch_sent()).unwrap(),
            "on_launch_record" => {
                drop(self.d.on_launch_record(borsh_args(args)));
                vec![]
            }
            "on_pool_seen" => serde_json::to_vec(&self.d.on_pool_seen(borsh_args(args))).unwrap(),
            m => panic!("unexpected Dragon callback {m}"),
        };
        let receipts = get_created_receipts();
        self.collect(&method, *prepaid_gas);
        self.execute(receipts);
        PromiseResult::Successful(out)
    }

    /// A receipt to a token, the pool or a peer. Its actions are atomic: one failure undoes them all.
    fn external(&mut self, receiver: &str, actions: &[MockAction]) -> PromiseResult {
        let before = self.w.clone();
        let mut last = Ok(vec![]);
        for a in actions {
            let (method, args, deposit) = match a {
                MockAction::FunctionCallWeight {
                    method_name,
                    args,
                    attached_deposit,
                    ..
                } => (
                    String::from_utf8(method_name.clone()).unwrap(),
                    serde_json::from_slice::<Value>(args).unwrap(),
                    attached_deposit.as_yoctonear(),
                ),
                MockAction::Transfer { deposit, .. } => {
                    self.w.near -= deposit.as_yoctonear();
                    continue;
                }
                _ => continue,
            };
            self.w.near -= deposit;
            last = self.run(receiver, &method, &args, deposit);
            if last.is_err() {
                break;
            }
        }
        match last {
            Ok(bytes) => PromiseResult::Successful(bytes),
            Err(_) => {
                self.w = before;
                PromiseResult::Failed
            }
        }
    }

    fn run(&mut self, receiver: &str, method: &str, a: &Value, deposit: u128) -> Result<Vec<u8>, String> {
        let ok = |v: Value| Ok(serde_json::to_vec(&v).unwrap());
        match (receiver, method) {
            (_, "storage_deposit") => {
                let who = a["account_id"].as_str().unwrap();
                let fresh = !self.w.tokens.contains_key(&(receiver.into(), who.into()));
                self.mint(receiver, who, 0);
                self.w.near += deposit - if fresh { REGISTRATION } else { 0 };
                ok(json!({ "total": REGISTRATION.to_string(), "available": "0" }))
            }
            (_, "ft_balance_of") => ok(json!(self
                .balance(receiver, a["account_id"].as_str().unwrap())
                .to_string())),
            (WRAP, "near_deposit") => {
                self.mint(WRAP, DRAGON, deposit);
                ok(Value::Null)
            }
            (_, "ft_transfer") => {
                let to = a["receiver_id"].as_str().unwrap();
                if !self.w.tokens.contains_key(&(receiver.into(), to.into())) || self.refuse == Some(to) {
                    return Err(format!("The account {to} is not registered"));
                }
                self.debit(receiver, DRAGON, amount_arg(a, "amount"))?;
                self.mint(receiver, to, amount_arg(a, "amount"));
                ok(Value::Null)
            }
            (_, "ft_transfer_call") => {
                assert_eq!(a["receiver_id"], DCL);
                let amount = amount_arg(a, "amount");
                self.debit(receiver, DRAGON, amount)?;
                let msg: Value = serde_json::from_str(a["msg"].as_str().unwrap()).unwrap();
                let swap = &msg["Swap"];
                assert_eq!(swap["pool_ids"], json!([POOL_ID]));
                let output = swap["output_token"].as_str().unwrap().to_string();
                assert_eq!(
                    swap["skip_unwrap_near"] == json!(true),
                    output == WRAP,
                    "only sells keep wNEAR"
                );
                let sell_tax = if receiver == WYRM { self.tax(amount) } else { 0 };
                let into_pool = amount - sell_tax;
                let out = self.quote(receiver, into_pool);
                let min = amount_arg(swap, "min_output_amount");
                if self.fail_swaps || out < min {
                    // DCL panics, and ft_resolve_transfer refunds the whole amount, the tax with it.
                    self.mint(receiver, DRAGON, amount);
                    return ok(json!("0"));
                }
                self.mint(WYRM, WYRM, sell_tax);
                let p = &mut self.w.pool;
                if receiver == WRAP {
                    p.x += into_pool;
                    p.y -= out;
                } else {
                    p.y += into_pool;
                    p.x -= out;
                }
                let buy_tax = if output == WYRM { self.tax(out) } else { 0 };
                self.mint(WYRM, WYRM, buy_tax);
                if self.hold_output {
                    self.w.held.push((output, DRAGON.into(), out - buy_tax));
                } else {
                    self.mint(&output, DRAGON, out - buy_tax);
                }
                ok(json!(amount.to_string()))
            }
            (DCL, "get_pool") => {
                if a["pool_id"] != POOL_ID || !self.d_launched() {
                    return ok(Value::Null);
                }
                let p = &self.w.pool;
                ok(json!({
                    "pool_id": POOL_ID, "token_x": WRAP, "token_y": WYRM, "fee": 10000, "current_point": 0,
                    "total_x": (p.x + p.far_x).to_string(), "total_y": (p.y + p.far_y).to_string(), "state": "Running"
                }))
            }
            (DCL, "quote") => {
                let input = a["input_token"].as_str().unwrap();
                ok(json!({ "amount": self.quote(input, amount_arg(a, "input_amount")).to_string(), "tag": null }))
            }
            (TITHE, "get_config") => ok(json!({ "dragon": DRAGON, "scales": COUNCIL, "wrap": WRAP,
                                               "min_round": "1", "reserve": "1" })),
            (COUNCIL, "get_config") => {
                let hash = near_sdk::json_types::Base58CryptoHash::from(self.d.preset().hash());
                ok(
                    json!({ "preset_hash": hash, "engine_version": wyrm_types::ENGINE_VERSION, "dragon": DRAGON,
                           "tithe": TITHE, "creator": CREATOR, "vaults_created": 100 }),
                )
            }
            (COUNCIL, "open_sale") => ok(Value::Null),
            (NEARLY, "quote_launch") => ok(json!({ "total": QUOTE_TOTAL.to_string() })),
            (NEARLY, "get_slug_count") => ok(json!(0)),
            (NEARLY, "launch") => {
                assert_eq!(a["args"]["fee_to"], TITHE);
                self.launched_with.push(deposit);
                ok(json!(true))
            }
            (NEARLY, "get_launch") => ok(json!({
                "id": 0, "token": WYRM, "creator": DRAGON, "pool_id": POOL_ID, "quote": WRAP, "step": "Done",
                "lpt_id": format!("{POOL_ID}#0"),
            })),
            (NEARLY, "get_fee_to") => ok(json!(TITHE)),
            (NEARLY, "get_fee_opts") => ok(json!({ "mode": "creator" })),
            (NEARLY, "get_tax") => ok(
                json!({ "buy_bps": 400, "sell_bps": 400, "creator_bps": 10000, "burn_bps": 0,
                                              "holders_bps": 0 }),
            ),
            (r, m) => panic!("unexpected call {r}.{m}"),
        }
    }

    /// The pool exists once Nearly's launch has run; before, launch's check must find none.
    fn d_launched(&self) -> bool {
        !self.launched_with.is_empty()
    }

    fn tax(&self, amount: u128) -> u128 {
        if self.taxed {
            amount * TAX_BPS / BPS
        } else {
            0
        }
    }

    fn debit(&mut self, token: &str, from: &str, amount: u128) -> Result<(), String> {
        let bal = self
            .w
            .tokens
            .get_mut(&(token.into(), from.into()))
            .ok_or("not registered")?;
        if *bal < amount {
            return Err("The account doesn't have enough balance".into());
        }
        *bal -= amount;
        Ok(())
    }

    /// Constant product on the in-range reserves only, after the pool fee.
    pub fn quote(&self, input: &str, amount: u128) -> u128 {
        let p = &self.w.pool;
        let (r_in, r_out) = if input == WRAP { (p.x, p.y) } else { (p.y, p.x) };
        let net = amount * (1_000_000 - FEE) / 1_000_000;
        math::mul_div(r_out, net, r_in + net)
    }
}

fn method_of(actions: &[MockAction]) -> String {
    match actions {
        [MockAction::FunctionCallWeight { method_name, .. }] => String::from_utf8(method_name.clone()).unwrap(),
        _ => String::new(),
    }
}

pub fn named(evs: &[(String, Value)], name: &str) -> Vec<Value> {
    evs.iter().filter(|(n, _)| n == name).map(|(_, d)| d.clone()).collect()
}

pub fn slices(evs: &[(String, Value)]) -> Vec<(String, Value)> {
    evs.iter().filter(|(n, _)| n.ends_with("_slice")).cloned().collect()
}

pub fn u(v: &Value) -> u128 {
    v.as_str()
        .unwrap_or_else(|| panic!("not an amount: {v}"))
        .parse()
        .unwrap()
}
