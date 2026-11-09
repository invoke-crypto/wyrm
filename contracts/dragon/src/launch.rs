//! Launch and bind. The Dragon calls Nearly's `launch` itself, so Nearly records it as the coin's creator, and
//! `bind` then checks that record and the DCL pool before the Dragon trades. Before any NEAR leaves, launch checks
//! the wiring of the tithe and scales, the deposit and that no pool sits at the coin's predicted id. Bind writes no
//! in-flight state: a concurrent bind's callbacks find the coin set and do nothing.
use crate::nearly::{self, Wiring};
use crate::*;
use near_sdk::GasWeight;
use wyrm_types::CoinArgs;

/// launch: four views, the wiring callback with its get_pool, the pool callback and Nearly's 300 Tgas.
pub const LAUNCH_MIN_GAS: Gas = Gas::from_tgas(500);
pub const BIND_MIN_GAS: Gas = Gas::from_tgas(150);
/// Nearly's resume floors (lib.rs:4522, 4409) plus the Dragon's own receipt; V5-NOTES asks 300 for every resume.
pub const RESUME_MIN_GAS: Gas = Gas::from_tgas(300);
const GAS_WIRING_VIEW: Gas = Gas::from_tgas(10);
const GAS_NEARLY_VIEW: Gas = Gas::from_tgas(5);
const GAS_POOL_VIEW: Gas = Gas::from_tgas(10);
const GAS_ON_POOL_CHECKED: Gas = Gas::from_tgas(340);
const GAS_ON_WIRING: Gas = Gas::from_tgas(GAS_ON_POOL_CHECKED.as_tgas() + 40);
const GAS_ON_LAUNCH_SENT: Gas = Gas::from_tgas(10);
const GAS_OPEN_SALE: Gas = Gas::from_tgas(10);
const GAS_ON_POOL_SEEN: Gas = Gas::from_tgas(25);
const GAS_ON_LAUNCH_RECORD: Gas = Gas::from_tgas(GAS_ON_POOL_SEEN.as_tgas() + 35);

#[near]
impl Dragon {
    /// The creator launches the coin on Nearly through the Dragon. Attach at least Nearly's quote plus 0.086 NEAR;
    /// the rest comes back in the same transaction. A refusal before Nearly is called returns the whole deposit.
    #[payable]
    pub fn launch(&mut self, coin: CoinArgs) -> Promise {
        self.only_creator();
        require!(self.coin.is_none(), "already bound");
        require!(env::prepaid_gas() >= LAUNCH_MIN_GAS, "launch needs at least 500 Tgas");
        let now = env::block_timestamp();
        require!(
            now >= self.last_launch_at + LAUNCH_COOLDOWN,
            "launches are at least 10 minutes apart"
        );
        let coin = coin.parse().unwrap_or_else(|e| env::panic_str(&e));
        self.last_launch_at = now;
        let deposit = env::attached_deposit().as_yoctonear();
        let p = &self.peers;
        let dev_buy = coin.dev_buy.map_or("null".into(), |d| format!(r#""{}""#, d.0));
        let quote = format!(
            r#"{{"icon_bytes":{},"dev_buy":{dev_buy},"tax":true,"text_bytes":{}}}"#,
            coin.icon_bytes(),
            coin.text_bytes()
        );
        let slug = format!(r#"{{"slug":"{}"}}"#, nearly::sanitize_slug(&coin.symbol));
        self.view_call(&p.tithe, "get_config", "{}".into(), GAS_WIRING_VIEW)
            .and(self.view_call(&p.scales, "get_config", "{}".into(), GAS_WIRING_VIEW))
            .and(self.view_call(&p.nearly, "quote_launch", quote, GAS_WIRING_VIEW))
            .and(self.view_call(&p.nearly, "get_slug_count", slug, GAS_WIRING_VIEW))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_WIRING)
                    .on_wiring(coin.symbol.clone(), nearly::launch_args(&coin, &p.tithe), U128(deposit)),
            )
    }

    #[private]
    /// `args` are Nearly's launch arguments, written in `launch` from the parsed coin text.
    pub fn on_wiring(
        &mut self,
        #[serializer(borsh)] symbol: String,
        #[serializer(borsh)] args: Vec<u8>,
        #[serializer(borsh)] deposit: U128,
    ) -> PromiseOrValue<bool> {
        let wiring = Wiring {
            tithe: nearly::answer(0),
            scales: nearly::answer(1),
            quote: nearly::answer(2),
            slug_count: nearly::answer(3),
        };
        let me = env::current_account_id();
        let hash = String::from(&near_sdk::json_types::Base58CryptoHash::from(self.preset().hash()));
        let checked = nearly::check_wiring(&wiring, &me, &self.peers, &hash, deposit.0);
        let (slug_count, cost) = match checked {
            Ok(v) => v,
            Err(reason) => return PromiseOrValue::Value(self.refuse_launch(deposit.0, reason)),
        };
        let (_, pool_id) = nearly::predicted_pool(&symbol, slug_count, &self.peers.nearly, &self.peers.wrap);
        let get_pool = self.view_call(&self.peers.dcl, "get_pool", dcl::pool_args(&pool_id), GAS_POOL_VIEW);
        PromiseOrValue::Promise(
            get_pool.then(Self::ext(me).with_static_gas(GAS_ON_POOL_CHECKED).on_pool_checked(
                args,
                deposit,
                U128(cost),
            )),
        )
    }

    /// Refuses a pool already at the predicted id before any NEAR reaches Nearly: a pool there at another price
    /// would fail the launch after Nearly kept the money.
    #[private]
    pub fn on_pool_checked(
        &mut self,
        #[serializer(borsh)] args: Vec<u8>,
        #[serializer(borsh)] deposit: U128,
        #[serializer(borsh)] cost: U128,
    ) -> PromiseOrValue<bool> {
        match dcl::result(0).and_then(|b| dcl::pool_exists(&b)) {
            Some(false) => {}
            Some(true) => return PromiseOrValue::Value(self.refuse_launch(deposit.0, "pool exists".into())),
            None => return PromiseOrValue::Value(self.refuse_launch(deposit.0, "the pool view failed".into())),
        }
        let change = deposit.0 - cost.0;
        if change > 0 {
            Promise::new(self.peers.creator.clone())
                .transfer(NearToken::from_yoctonear(change))
                .detach();
        }
        let launch = Promise::new(self.peers.nearly.clone()).function_call_weight(
            "launch",
            args,
            NearToken::from_yoctonear(cost.0),
            nearly::LAUNCH_GAS,
            GasWeight(1),
        );
        PromiseOrValue::Promise(
            launch.then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_LAUNCH_SENT)
                    .with_unused_gas_weight(0)
                    .on_launch_sent(),
            ),
        )
    }

    /// `ok: false` means the step chain did not reach Done in this transaction. The launch may be parked: read
    /// `get_launch` with the id from Nearly's `launch_started` and resume it, never launch again. Nearly also answers
    /// `true` for some parks, so only `get_launch(id).step` says Done. Moves no NEAR.
    #[private]
    pub fn on_launch_sent(&mut self) -> bool {
        let ok = nearly::answer(0).is_some_and(|b| b.trim_ascii() == b"true");
        self.record(Event::LaunchSent { ok });
        ok
    }

    /// Binds the Dragon to a launch it created, once Nearly says Done. Writes no state.
    pub fn bind(&mut self, launch_id: U64) -> Promise {
        self.only_creator();
        require!(self.coin.is_none(), "already bound");
        require!(env::prepaid_gas() >= BIND_MIN_GAS, "bind needs at least 150 Tgas");
        let n = &self.peers.nearly;
        let id = launch_id_args(launch_id);
        self.view_call(n, "get_launch", id.clone(), GAS_NEARLY_VIEW)
            .and(self.view_call(n, "get_fee_to", id.clone(), GAS_NEARLY_VIEW))
            .and(self.view_call(n, "get_fee_opts", id.clone(), GAS_NEARLY_VIEW))
            .and(self.view_call(n, "get_tax", id, GAS_NEARLY_VIEW))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_LAUNCH_RECORD)
                    .on_launch_record(launch_id),
            )
    }

    #[private]
    pub fn on_launch_record(&mut self, #[serializer(borsh)] launch_id: U64) -> PromiseOrValue<bool> {
        if self.coin.is_some() {
            return PromiseOrValue::Value(false);
        }
        let record = nearly::LaunchRecord::parse(
            nearly::answer(0),
            nearly::answer(1),
            nearly::answer(2),
            nearly::answer(3),
        );
        let me = env::current_account_id();
        let coin = match nearly::check_record(&record, &me, &self.peers, launch_id) {
            Ok(c) => c,
            Err(r) => return PromiseOrValue::Value(self.refuse_bind(launch_id, r)),
        };
        let get_pool = self.view_call(
            &self.peers.dcl,
            "get_pool",
            dcl::pool_args(&coin.pool_id),
            GAS_POOL_VIEW,
        );
        PromiseOrValue::Promise(get_pool.then(Self::ext(me).with_static_gas(GAS_ON_POOL_SEEN).on_pool_seen(coin)))
    }

    #[private]
    pub fn on_pool_seen(&mut self, #[serializer(borsh)] coin: Coin) -> bool {
        if self.coin.is_some() {
            return false;
        }
        let pool = dcl::result(0).and_then(|b| dcl::parse_pool_seen(&b));
        if let Err(r) = nearly::check_pool(pool, &coin, &self.peers.wrap) {
            return self.refuse_bind(coin.launch_id, r);
        }
        self.record(Event::Bound {
            launch_id: coin.launch_id,
            token: coin.token.clone(),
            pool_id: coin.pool_id.clone(),
        });
        self.coin = Some(coin);
        Promise::new(self.peers.scales.clone())
            .function_call("open_sale", b"{}".to_vec(), NearToken::from_yoctonear(0), GAS_OPEN_SALE)
            .detach();
        true
    }

    /// Resumes a parked launch as its creator, which Nearly requires for a parked dev buy or a first buy that opens
    /// the pool. A resume can park again at DevBuy, so call it until `get_launch(id).step` is Done.
    pub fn nearly_resume(&mut self, launch_id: U64) -> Promise {
        require!(self.coin.is_none(), "already bound");
        require!(
            env::prepaid_gas() >= RESUME_MIN_GAS,
            "nearly_resume needs at least 300 Tgas"
        );
        Promise::new(self.peers.nearly.clone()).function_call_weight(
            "resume",
            launch_id_args(launch_id).into_bytes(),
            NearToken::from_yoctonear(0),
            nearly::RESUME_GAS,
            GasWeight(1),
        )
    }

    /// Cancels a launch parked before its dev buy, so its held NEAR returns to the Dragon. After bind, any launch but
    /// the bound one.
    pub fn nearly_cancel_stuck(&mut self, launch_id: U64) -> Promise {
        self.only_creator();
        require!(
            self.coin.as_ref().is_none_or(|c| c.launch_id != launch_id),
            "the bound launch is not stuck"
        );
        Promise::new(self.peers.nearly.clone()).function_call_weight(
            "cancel_stuck_launch",
            launch_id_args(launch_id).into_bytes(),
            NearToken::from_yoctonear(0),
            nearly::CANCEL_GAS,
            GasWeight(1),
        )
    }
}

fn launch_id_args(launch_id: U64) -> String {
    format!(r#"{{"launch_id":"{}"}}"#, launch_id.0)
}

impl Dragon {
    fn only_creator(&self) {
        require!(env::predecessor_account_id() == self.peers.creator, "only the creator");
    }

    /// The deposit of this same call goes back as a plain transfer: nothing reached Nearly.
    fn refuse_launch(&mut self, deposit: u128, reason: String) -> bool {
        if deposit > 0 {
            Promise::new(self.peers.creator.clone())
                .transfer(NearToken::from_yoctonear(deposit))
                .detach();
        }
        self.record(Event::LaunchRefused { reason });
        false
    }

    fn refuse_bind(&mut self, launch_id: U64, r: nearly::BindRefusal) -> bool {
        self.record(Event::BindRefused {
            launch_id,
            reason: r.reason(),
        });
        false
    }
}
