// Test-only stand-in for Rhea DCL (dclv2.ref-labs.near). It answers get_pool, quote and the ft_transfer_call Swap
// message with the same JSON shapes as the real contract, priced by a constant-product pool that tests set directly.
// As on mainnet, a quote on a missing pool panics E403 and get_pool answers null.
// Switches let a test make every swap fail on slippage, hold swap output back for a late delivery, and add liquidity
// far from the price that shows in total_x/total_y but never touches a quote. Never deploy this on mainnet.
use near_sdk::json_types::U128;
use near_sdk::serde::Deserialize;
use near_sdk::store::LookupMap;
use near_sdk::{env, near, require, serde_json, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseOrValue};

const FEE_DENOMINATOR: u128 = 1_000_000;
const POINT_DELTA: i32 = 40;
const POINT_RANGE: i32 = 800_000;
// DCL keeps this share of each swap fee as the protocol fee it reports in total_fee_*_charged.
const PROTOCOL_FEE_PCT: u128 = 20;

#[near(serializers = [borsh])]
pub struct Pool {
    token_x: AccountId,
    token_y: AccountId,
    fee: u32,
    // The reserves the constant-product price is made of.
    x: u128,
    y: u128,
    // Out-of-range liquidity: counted in total_x/total_y, never swapped against.
    far_x: u128,
    far_y: u128,
    state: String,
    volume: Volume,
}

/// The running totals DCL reports in get_pool.
#[near(serializers = [borsh])]
#[derive(Default)]
pub struct Volume {
    x_in: u128,
    y_in: u128,
    x_out: u128,
    y_out: u128,
    fee_x: u128,
    fee_y: u128,
}

#[near(serializers = [borsh])]
pub struct Held {
    token: AccountId,
    receiver: AccountId,
    amount: u128,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MockDcl {
    pools: LookupMap<String, Pool>,
    fail_swaps: bool,
    hold_output: bool,
    held: Vec<Held>,
}

#[near(serializers = [json])]
pub struct PoolInfo {
    pool_id: String,
    token_x: AccountId,
    token_y: AccountId,
    fee: u32,
    point_delta: u32,
    current_point: i32,
    liquidity: U128,
    liquidity_x: U128,
    max_liquidity_per_point: U128,
    total_fee_x_charged: U128,
    total_fee_y_charged: U128,
    volume_x_in: U128,
    volume_y_in: U128,
    volume_x_out: U128,
    volume_y_out: U128,
    total_liquidity: U128,
    total_order_x: U128,
    total_order_y: U128,
    total_x: U128,
    total_y: U128,
    state: String,
    whitelist: Option<Vec<AccountId>>,
    tax: Option<serde_json::Value>,
}

#[near(serializers = [json])]
pub struct QuoteResult {
    amount: U128,
    tag: Option<String>,
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
enum TransferMsg {
    Swap {
        pool_ids: Vec<String>,
        output_token: AccountId,
        min_output_amount: U128,
        swap_out_recipient: Option<AccountId>,
    },
}

#[near]
impl MockDcl {
    #[init]
    pub fn new() -> Self {
        Self {
            pools: LookupMap::new(b"p"),
            fail_swaps: false,
            hold_output: false,
            held: vec![],
        }
    }

    /// Creates or resets a pool. The tokens and fee come from the id, `"<x>|<y>|<fee>"` sorted as DCL sorts.
    pub fn set_pool(&mut self, pool_id: String, total_x: U128, total_y: U128, state: Option<String>) {
        let parts: Vec<&str> = pool_id.split('|').collect();
        assert!(parts.len() == 3 && parts[0] < parts[1], "malformed pool id");
        let far = self.pools.get(&pool_id).map(|p| (p.far_x, p.far_y)).unwrap_or((0, 0));
        let pool = Pool {
            token_x: parts[0].parse().expect("token_x"),
            token_y: parts[1].parse().expect("token_y"),
            fee: parts[2].parse().expect("fee"),
            x: total_x.0,
            y: total_y.0,
            far_x: far.0,
            far_y: far.1,
            state: state.unwrap_or_else(|| "Running".into()),
            volume: Volume::default(),
        };
        self.pools.insert(pool_id, pool);
    }

    /// Creates an empty pool, as Nearly's CreateToken step asks DCL to. Refuses an id that exists or is malformed.
    pub fn create_pool(&mut self, pool_id: String) -> String {
        require!(!self.pools.contains_key(&pool_id), "pool already exists");
        self.set_pool(pool_id.clone(), U128(0), U128(0), None);
        pool_id
    }

    pub fn add_far_liquidity(&mut self, pool_id: String, x: U128, y: U128) {
        let pool = self.pools.get_mut(&pool_id).expect("no pool");
        pool.far_x += x.0;
        pool.far_y += y.0;
    }

    pub fn set_fail_swaps(&mut self, fail: bool) {
        self.fail_swaps = fail;
    }

    pub fn set_hold_output(&mut self, hold: bool) {
        self.hold_output = hold;
    }

    /// Delivers every held swap output.
    pub fn release_held(&mut self) {
        for h in std::mem::take(&mut self.held) {
            send(h.token, h.receiver, h.amount);
        }
    }

    pub fn get_pool(&self, pool_id: String) -> Option<PoolInfo> {
        self.pools.get(&pool_id).map(|p| PoolInfo {
            pool_id: pool_id.clone(),
            token_x: p.token_x.clone(),
            token_y: p.token_y.clone(),
            fee: p.fee,
            point_delta: POINT_DELTA as u32,
            current_point: point(p.x, p.y),
            liquidity: U128(0),
            liquidity_x: U128(0),
            max_liquidity_per_point: U128(u128::MAX / (2 * POINT_RANGE / POINT_DELTA + 1) as u128),
            total_fee_x_charged: U128(p.volume.fee_x),
            total_fee_y_charged: U128(p.volume.fee_y),
            volume_x_in: U128(p.volume.x_in),
            volume_y_in: U128(p.volume.y_in),
            volume_x_out: U128(p.volume.x_out),
            volume_y_out: U128(p.volume.y_out),
            total_liquidity: U128(0),
            total_order_x: U128(0),
            total_order_y: U128(0),
            total_x: U128(p.x + p.far_x),
            total_y: U128(p.y + p.far_y),
            state: p.state.clone(),
            whitelist: None,
            tax: None,
        })
    }

    pub fn quote(
        &self,
        pool_ids: Vec<String>,
        input_token: AccountId,
        output_token: AccountId,
        input_amount: U128,
        tag: Option<String>,
    ) -> QuoteResult {
        let pool = pool_ids
            .first()
            .and_then(|id| self.pools.get(id))
            .expect("E403: pool not exist");
        let amount = swap_out(pool, &input_token, &output_token, input_amount.0).unwrap_or(0);
        QuoteResult {
            amount: U128(amount),
            tag,
        }
    }

    pub fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> PromiseOrValue<U128> {
        let input_token = env::predecessor_account_id();
        let TransferMsg::Swap {
            pool_ids,
            output_token,
            min_output_amount,
            swap_out_recipient,
        } = serde_json::from_str(&msg).expect("E28: illegal msg in ft_transfer_call");
        assert!(!self.fail_swaps, "E204: slippage error");
        assert!(pool_ids.len() == 1, "mock-dcl swaps through one pool");
        let pool = self.pools.get_mut(&pool_ids[0]).expect("E403: pool not exist");
        assert!(pool.state == "Running", "E405: pool paused");
        let out = swap_out(pool, &input_token, &output_token, amount.0).expect("E404: invalid token");
        assert!(out >= min_output_amount.0, "E204: slippage error");
        let fee = amount.0 - mul_div(amount.0, FEE_DENOMINATOR - pool.fee as u128, FEE_DENOMINATOR);
        let protocol = fee * PROTOCOL_FEE_PCT / 100;
        let v = &mut pool.volume;
        if input_token == pool.token_x {
            pool.x += amount.0;
            pool.y -= out;
            (v.x_in, v.y_out, v.fee_x) = (v.x_in + amount.0, v.y_out + out, v.fee_x + protocol);
        } else {
            pool.y += amount.0;
            pool.x -= out;
            (v.y_in, v.x_out, v.fee_y) = (v.y_in + amount.0, v.x_out + out, v.fee_y + protocol);
        }
        let receiver = swap_out_recipient.unwrap_or(sender_id);
        if self.hold_output {
            self.held.push(Held {
                token: output_token,
                receiver,
                amount: out,
            });
        } else {
            send(output_token, receiver, out);
        }
        PromiseOrValue::Value(U128(0))
    }
}

fn swap_out(pool: &Pool, input: &AccountId, output: &AccountId, amount: u128) -> Option<u128> {
    let (r_in, r_out) = if *input == pool.token_x && *output == pool.token_y {
        (pool.x, pool.y)
    } else if *input == pool.token_y && *output == pool.token_x {
        (pool.y, pool.x)
    } else {
        return None;
    };
    let net = mul_div(amount, FEE_DENOMINATOR - pool.fee as u128, FEE_DENOMINATOR);
    // Like real DCL, an input larger than the pool can fill gets at most the whole reserve, never an overflow.
    match r_in.checked_add(net) {
        Some(0) => Some(0),
        Some(d) => Some(mul_div(r_out, net, d)),
        None => Some(r_out),
    }
}

// Reserves times amounts overflow u128. Same algorithm as dragon/src/math.rs; the mock cannot depend on the Dragon.
fn mul_div(a: u128, b: u128, d: u128) -> u128 {
    const LO: u128 = u64::MAX as u128;
    let (a1, a0, b1, b0) = (a >> 64, a & LO, b >> 64, b & LO);
    let (p00, p01, p10, p11) = (a0 * b0, a0 * b1, a1 * b0, a1 * b1);
    let mid = (p00 >> 64) + (p01 & LO) + (p10 & LO);
    let lo = (p00 & LO) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    assert!(hi < d, "quote overflow");
    let (mut rem, mut q) = (hi, 0u128);
    for i in (0..128).rev() {
        let carry = rem >> 127;
        rem = (rem << 1) | ((lo >> i) & 1);
        q <<= 1;
        if carry == 1 || rem >= d {
            rem = rem.wrapping_sub(d);
            q |= 1;
        }
    }
    q
}

// DCL's point is log base 1.0001 of the price of x in y.
fn point(x: u128, y: u128) -> i32 {
    if x == 0 || y == 0 {
        return 0;
    }
    ((y as f64 / x as f64).ln() / 1.0001f64.ln()).floor() as i32
}

fn send(token: AccountId, receiver: AccountId, amount: u128) {
    Promise::new(token)
        .function_call(
            "ft_transfer",
            serde_json::json!({ "receiver_id": receiver, "amount": U128(amount) })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            Gas::from_tgas(10),
        )
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(x: u128, y: u128) -> Pool {
        Pool {
            token_x: "a.near".parse().unwrap(),
            token_y: "b.near".parse().unwrap(),
            fee: 10_000,
            x,
            y,
            far_x: 0,
            far_y: 0,
            state: "Running".into(),
            volume: Volume::default(),
        }
    }

    #[test]
    fn output_never_passes_the_reserve() {
        let (a, b): (AccountId, AccountId) = ("a.near".parse().unwrap(), "b.near".parse().unwrap());
        let p = pool(1_000 * 10u128.pow(24), 10u128.pow(27));
        let huge = swap_out(&p, &a, &b, 1_000_000_000 * 10u128.pow(24)).unwrap();
        assert!(huge > 0 && huge <= p.y);
        assert!(swap_out(&p, &a, &b, u128::MAX).unwrap() <= p.y);
        let full = pool(u128::MAX - 1, 10u128.pow(27));
        assert_eq!(
            swap_out(&full, &a, &b, u128::MAX),
            Some(full.y),
            "an input past u128 takes the whole reserve"
        );
        assert_eq!(swap_out(&p, &b, &b, 1), None);
    }

    #[test]
    fn the_fee_is_taken_from_the_input() {
        let (a, b): (AccountId, AccountId) = ("a.near".parse().unwrap(), "b.near".parse().unwrap());
        let p = pool(10u128.pow(30), 10u128.pow(30));
        assert_eq!(swap_out(&p, &a, &b, 1_000_000), Some(989_999));
    }
}
