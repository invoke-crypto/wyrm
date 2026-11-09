//! The whole boundary with Nearly (nearlytrade.near, Nearlytrade e82bfa0, contracts/factory/src/lib.rs). The
//! launch arguments are written here, the coin's token and pool id are predicted here, and Nearly's JSON is parsed
//! here into one `LaunchRecord` that `check_record` judges. Nothing of Nearly's shapes goes further.
use crate::dcl::{self, PoolSeen};
use crate::Peers;
use near_sdk::json_types::{U128, U64};
use near_sdk::serde::Serialize;
use near_sdk::{env, near, serde_json, AccountId, Gas, NearToken};
use wyrm_types::{envelope, CoinArgs, Links, TaxPolicy, ENGINE_VERSION};

/// The floor sent to Nearly's `launch`; every unused gas of the call is added to it.
pub const LAUNCH_GAS: Gas = Gas::from_tgas(300);
/// Sent to `resume`, with the call's unused gas added. Nearly refuses a resume below 222 Tgas at AddLiquidity, below
/// 235 Tgas at DevBuy and below 263 Tgas for a first buy that opens the pool (lib.rs:4522, 4409, 1029).
pub const RESUME_GAS: Gas = Gas::from_tgas(280);
pub const CANCEL_GAS: Gas = Gas::from_tgas(30);
/// DCL_POOL_CREATE 0.1 less DCL_POOL_CHARGE 0.014 (lib.rs:50-55). Nearly's cost rises by this much when its pool
/// float runs low, and what it does not use returns to the creator, the Dragon, as `extra`.
pub const LAUNCH_MARGIN: NearToken = NearToken::from_millinear(86);
/// Nearly's config pool_fee, read at block 222922088.
pub const POOL_FEE: u32 = 10_000;
/// A record still at CreateToken keeps its icon, which Nearly allows up to 16 KB.
const MAX_ANSWER: usize = 20_000;
/// Nearly gives these slugs a suffix even when unused (RESERVED_SLUGS, lib.rs:4765).
const RESERVED_SLUGS: [&str; 1] = ["lock"];

pub fn answer(i: u64) -> Option<Vec<u8>> {
    env::promise_result_checked(i, MAX_ANSWER).ok()
}

/// Nearly's `launch` args: the coin text, fee_to = the tithe, fee_mode "creator" and the envelope tax. No quote
/// override (so wNEAR), no creator_share_bps and no init_point.
pub fn launch_args(coin: &CoinArgs, tithe: &AccountId) -> Vec<u8> {
    #[derive(Serialize)]
    #[serde(crate = "near_sdk::serde")]
    struct Args<'a> {
        name: &'a str,
        symbol: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        icon: Option<&'a str>,
        description: &'a str,
        links: &'a Links,
        #[serde(skip_serializing_if = "Option::is_none")]
        dev_buy: Option<U128>,
        fee_to: &'a AccountId,
        fee_mode: &'static str,
        tax: TaxPolicy,
    }
    #[derive(Serialize)]
    #[serde(crate = "near_sdk::serde")]
    struct Launch<'a> {
        args: Args<'a>,
    }
    serde_json::to_vec(&Launch {
        args: Args {
            name: &coin.name,
            symbol: &coin.symbol,
            icon: coin.icon.as_deref(),
            description: &coin.description,
            links: &coin.links,
            dev_buy: coin.dev_buy,
            fee_to: tithe,
            fee_mode: "creator",
            tax: envelope::TAX,
        },
    })
    .expect("launch args serialize")
}

/// Nearly's sanitize_slug (lib.rs:4776).
pub fn sanitize_slug(symbol: &str) -> String {
    let s: String = symbol
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .take(12)
        .collect();
    if s.is_empty() {
        "t".into()
    } else {
        s
    }
}

/// The token account and pool id Nearly will give this symbol, from `get_slug_count`: Nearly's short_name, the
/// reserved slugs and its sorted `"<a>|<b>|<pool_fee>"` (lib.rs:906-914).
pub fn predicted_pool(symbol: &str, slug_count: u32, nearly: &AccountId, wrap: &AccountId) -> (AccountId, String) {
    let slug = sanitize_slug(symbol);
    let taken = if slug_count == 0 && RESERVED_SLUGS.contains(&slug.as_str()) {
        1
    } else {
        slug_count
    };
    let short = if taken == 0 {
        slug
    } else {
        format!("{slug}-{}", taken + 1)
    };
    let token: AccountId = format!("{short}.{nearly}").parse().expect("a slug makes an account id");
    let (a, b) = if token.as_str() < wrap.as_str() {
        (token.as_str(), wrap.as_str())
    } else {
        (wrap.as_str(), token.as_str())
    };
    let pool_id = format!("{a}|{b}|{POOL_FEE}");
    (token, pool_id)
}

/// The coin, once bound. Set once and never cleared.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coin {
    pub launch_id: U64,
    pub token: AccountId,
    pub pool_id: String,
    pub wrap_is_x: bool,
}

#[derive(Clone, Debug)]
pub struct LaunchSeen {
    pub creator: AccountId,
    pub token: AccountId,
    pub pool_id: String,
    pub quote: AccountId,
    pub step: String,
    pub lpt_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxSeen {
    pub buy_bps: u64,
    pub sell_bps: u64,
    pub creator_bps: u64,
    pub burn_bps: u64,
    pub holders_bps: u64,
}

/// Nearly's answers to get_launch, get_fee_to, get_fee_opts and get_tax for one id. `None` is a view that failed or
/// did not parse; unknown fields are ignored.
pub struct LaunchRecord {
    pub launch: Option<Option<LaunchSeen>>,
    pub fee_to: Option<Option<AccountId>>,
    pub fee_mode: Option<String>,
    pub tax: Option<Option<TaxSeen>>,
}

impl LaunchRecord {
    pub fn parse(
        launch: Option<Vec<u8>>,
        fee_to: Option<Vec<u8>>,
        opts: Option<Vec<u8>>,
        tax: Option<Vec<u8>>,
    ) -> Self {
        let launch = launch.and_then(|b| {
            let Some(l) = dcl::object(&b)? else {
                return Some(None);
            };
            let lpt_id = if l.is_null("lpt_id") {
                None
            } else {
                Some(l.str("lpt_id")?.into())
            };
            Some(Some(LaunchSeen {
                creator: l.account("creator")?,
                token: l.account("token")?,
                pool_id: l.str("pool_id")?.into(),
                quote: l.account("quote")?,
                step: l.str("step")?.into(),
                lpt_id,
            }))
        });
        let fee_to = fee_to.and_then(|b| {
            let raw = std::str::from_utf8(&b).ok()?.trim();
            if raw == "null" {
                return Some(None);
            }
            Some(Some(dcl::string(raw)?.parse().ok()?))
        });
        let fee_mode = opts.and_then(|b| Some(dcl::object(&b)??.str("mode")?.into()));
        let tax = tax.and_then(|b| {
            let Some(t) = dcl::object(&b)? else {
                return Some(None);
            };
            Some(Some(TaxSeen {
                buy_bps: t.number("buy_bps")?,
                sell_bps: t.number("sell_bps")?,
                creator_bps: t.number("creator_bps")?,
                burn_bps: t.number("burn_bps")?,
                holders_bps: t.number("holders_bps")?,
            }))
        });
        Self {
            launch,
            fee_to,
            fee_mode,
            tax,
        }
    }
}

/// Every reason bind refuses, in checking order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindRefusal {
    ViewFailed,
    NoRecord,
    NotDone,
    NotOurs,
    NotNearPair,
    WrongFeeTo,
    WrongFeeMode,
    WrongTax,
    BadPoolId(String),
    NoPool,
    WrongPoolTokens,
    PoolNotRunning,
}

impl BindRefusal {
    pub fn reason(&self) -> String {
        match self {
            BindRefusal::ViewFailed => "a view failed".into(),
            BindRefusal::NoRecord => "no such launch".into(),
            BindRefusal::NotDone => "not done".into(),
            BindRefusal::NotOurs => "the launch's creator is not this Dragon".into(),
            BindRefusal::NotNearPair => "the launch is not on the wNEAR pair".into(),
            BindRefusal::WrongFeeTo => "the launch's fee_to is not the tithe".into(),
            BindRefusal::WrongFeeMode => "the launch's fee mode is not creator".into(),
            BindRefusal::WrongTax => "the launch's tax is not the engine tax".into(),
            BindRefusal::BadPoolId(e) => format!("bad pool id: {e}"),
            BindRefusal::NoPool => "no pool on our DCL".into(),
            BindRefusal::WrongPoolTokens => "the pool does not pair wrap with the coin".into(),
            BindRefusal::PoolNotRunning => "the pool is not running".into(),
        }
    }
}

/// Bind's checks 1 to 9 on Nearly's record. Pure.
pub fn check_record(r: &LaunchRecord, me: &AccountId, peers: &Peers, launch_id: U64) -> Result<Coin, BindRefusal> {
    let (Some(launch), Some(fee_to), Some(fee_mode), Some(tax)) = (&r.launch, &r.fee_to, &r.fee_mode, &r.tax) else {
        return Err(BindRefusal::ViewFailed);
    };
    let l = launch.as_ref().ok_or(BindRefusal::NoRecord)?;
    if l.step != "Done" || l.lpt_id.is_none() {
        return Err(BindRefusal::NotDone);
    }
    if &l.creator != me {
        return Err(BindRefusal::NotOurs);
    }
    if l.quote != peers.wrap {
        return Err(BindRefusal::NotNearPair);
    }
    if fee_to.as_ref() != Some(&peers.tithe) {
        return Err(BindRefusal::WrongFeeTo);
    }
    if fee_mode != "creator" {
        return Err(BindRefusal::WrongFeeMode);
    }
    let t = envelope::TAX;
    let want = TaxSeen {
        buy_bps: t.buy_bps.into(),
        sell_bps: t.sell_bps.into(),
        creator_bps: t.creator_bps.into(),
        burn_bps: t.burn_bps.into(),
        holders_bps: t.holders_bps.into(),
    };
    if tax.as_ref() != Some(&want) {
        return Err(BindRefusal::WrongTax);
    }
    let wrap_is_x = dcl::parse_pool_id(&l.pool_id, &peers.wrap, &l.token).map_err(BindRefusal::BadPoolId)?;
    Ok(Coin {
        launch_id,
        token: l.token.clone(),
        pool_id: l.pool_id.clone(),
        wrap_is_x,
    })
}

/// Bind's checks 10 to 12 on DCL's pool. `None` is a failed view. This catches a launch Nearly placed on another
/// DCL, which its owner can switch per launch (lib.rs:2939-2950).
pub fn check_pool(pool: Option<Option<PoolSeen>>, coin: &Coin, wrap: &AccountId) -> Result<(), BindRefusal> {
    let p = pool.ok_or(BindRefusal::ViewFailed)?.ok_or(BindRefusal::NoPool)?;
    let (x, y) = if coin.wrap_is_x {
        (wrap, &coin.token)
    } else {
        (&coin.token, wrap)
    };
    if &p.token_x != x || &p.token_y != y {
        return Err(BindRefusal::WrongPoolTokens);
    }
    if p.state != "Running" {
        return Err(BindRefusal::PoolNotRunning);
    }
    Ok(())
}

/// What `launch`'s four views answered.
pub struct Wiring {
    pub tithe: Option<Vec<u8>>,
    pub scales: Option<Vec<u8>>,
    pub quote: Option<Vec<u8>>,
    pub slug_count: Option<Vec<u8>>,
}

/// Launch's wiring check. Returns Nearly's slug count and the NEAR to send it (the quote plus the margin), or the
/// reason to refuse before any NEAR leaves.
pub fn check_wiring(
    w: &Wiring,
    me: &AccountId,
    peers: &Peers,
    preset_hash: &str,
    deposit: u128,
) -> Result<(u32, u128), String> {
    fn answer<'a>(what: &str, b: &'a Option<Vec<u8>>) -> Result<&'a [u8], String> {
        b.as_deref().ok_or_else(|| format!("{what} did not answer"))
    }
    fn object<'a>(what: &str, b: &'a Option<Vec<u8>>) -> Result<dcl::Object<'a>, String> {
        dcl::object(answer(what, b)?)
            .flatten()
            .ok_or_else(|| format!("{what} answered something else"))
    }
    let odd = |what: &str| format!("{what} answered something else");
    let tithe = object("tithe.get_config", &w.tithe)?;
    let named = |o: &dcl::Object, key: &str, want: &AccountId| o.account(key).as_ref() == Some(want);
    if !named(&tithe, "dragon", me) || !named(&tithe, "scales", &peers.scales) || !named(&tithe, "wrap", &peers.wrap) {
        return Err("the tithe is not wired to this Dragon, these scales and this wrap".into());
    }
    let scales = object("scales.get_config", &w.scales)?;
    if !named(&scales, "dragon", me)
        || !named(&scales, "tithe", &peers.tithe)
        || !named(&scales, "creator", &peers.creator)
    {
        return Err("scales is not wired to this Dragon, this tithe and this creator".into());
    }
    if scales.str("preset_hash") != Some(preset_hash) || scales.number("engine_version") != Some(ENGINE_VERSION.into())
    {
        return Err("scales runs another preset or engine version".into());
    }
    let vaults = scales
        .number("vaults_created")
        .ok_or_else(|| odd("scales.get_config"))?;
    if vaults != 100 {
        return Err(format!("scales has {vaults} of 100 vaults"));
    }
    let total = object("nearly.quote_launch", &w.quote)?
        .u128("total")
        .ok_or_else(|| odd("nearly.quote_launch"))?;
    let slug_count: u32 = std::str::from_utf8(answer("nearly.get_slug_count", &w.slug_count)?)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .ok_or_else(|| odd("nearly.get_slug_count"))?;
    let cost = total + LAUNCH_MARGIN.as_yoctonear();
    if deposit < cost {
        return Err(format!(
            "attach at least {cost} yoctoNEAR, Nearly's quote plus the margin"
        ));
    }
    Ok((slug_count, cost))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> AccountId {
        s.parse().unwrap()
    }

    fn peers() -> Peers {
        Peers {
            wrap: id("wrap.near"),
            dcl: id("dcl.near"),
            nearly: id("nearlytrade.near"),
            tithe: id("tithe.near"),
            scales: id("scales.near"),
            ash: id("ash.near"),
            creator: id("creator.near"),
        }
    }

    fn coin() -> CoinArgs {
        CoinArgs {
            name: "WYRM".into(),
            symbol: "WYRM".into(),
            icon: Some("https://wyrm.example/i.png".into()),
            description: "d".into(),
            links: Default::default(),
            dev_buy: None,
        }
    }

    #[test]
    fn launch_args_carry_fee_to_mode_and_tax() {
        let v: serde_json::Value = serde_json::from_slice(&launch_args(&coin(), &id("tithe.near"))).unwrap();
        let a = &v["args"];
        assert_eq!(a["fee_to"], "tithe.near");
        assert_eq!(a["fee_mode"], "creator");
        assert_eq!(
            a["tax"],
            serde_json::json!({ "buy_bps": 400, "sell_bps": 400, "creator_bps": 10_000, "burn_bps": 0, "holders_bps": 0 })
        );
        assert_eq!((a["name"].clone(), a["symbol"].clone()), ("WYRM".into(), "WYRM".into()));
        assert_eq!(a["icon"], "https://wyrm.example/i.png");
        for absent in ["quote", "creator_share_bps", "init_point", "dev_buy"] {
            assert!(a.get(absent).is_none(), "{absent} is left to Nearly");
        }
        let dev = CoinArgs {
            dev_buy: Some(U128(7)),
            icon: None,
            ..coin()
        };
        let v: serde_json::Value = serde_json::from_slice(&launch_args(&dev, &id("t.near"))).unwrap();
        assert_eq!(
            (v["args"]["dev_buy"].clone(), v["args"].get("icon")),
            ("7".into(), None)
        );
    }

    #[test]
    fn predicted_pool_follows_nearlys_slug_rule() {
        let (nearly, wrap) = (id("nearlytrade.near"), id("wrap.near"));
        let p = |sym: &str, n| predicted_pool(sym, n, &nearly, &wrap);
        assert_eq!(
            p("WYRM", 0),
            (
                id("wyrm.nearlytrade.near"),
                "wrap.near|wyrm.nearlytrade.near|10000".into()
            )
        );
        assert_eq!(p("WYRM", 1).0, id("wyrm-2.nearlytrade.near"));
        assert_eq!(p("WYRM", 2).0, id("wyrm-3.nearlytrade.near"));
        assert_eq!(p("LOCK", 0).0, id("lock-2.nearlytrade.near"), "lock is reserved");
        assert_eq!(p("LOCK", 1).0, id("lock-2.nearlytrade.near"));
        assert_eq!(
            p("Abc", 0),
            (
                id("abc.nearlytrade.near"),
                "abc.nearlytrade.near|wrap.near|10000".into()
            )
        );
        assert_eq!(
            p("ABCDEFGHIJKLM", 0).0,
            id("abcdefghijkl.nearlytrade.near"),
            "12 characters at most"
        );
    }

    fn record() -> LaunchRecord {
        let launch = br#"{"id":5,"token":"wyrm.nearlytrade.near","creator":"dragon.near","pool_id":"wrap.near|wyrm.nearlytrade.near|10000",
            "quote":"wrap.near","step":"Done","lpt_id":"x#1","icon":null,"inflight":false}"#;
        LaunchRecord::parse(
            Some(launch.to_vec()),
            Some(br#""tithe.near""#.to_vec()),
            Some(br#"{"mode":"creator","creator_share_bps":6000}"#.to_vec()),
            Some(br#"{"buy_bps":400,"sell_bps":400,"creator_bps":10000,"burn_bps":0,"holders_bps":0,"platform_bps":4000}"#.to_vec()),
        )
    }

    /// The `result` of a view recorded from mainnet in mock-nearly/fixtures.
    fn mainnet(fixture: &str) -> Option<Vec<u8>> {
        let v: serde_json::Value = serde_json::from_str(fixture).unwrap();
        Some(v["result"].to_string().into_bytes())
    }

    #[test]
    fn mainnet_answers_parse() {
        let r = LaunchRecord::parse(
            mainnet(include_str!("../../mock-nearly/fixtures/get_launch.json")),
            mainnet(include_str!("../../mock-nearly/fixtures/get_fee_to.json")),
            mainnet(include_str!("../../mock-nearly/fixtures/get_fee_opts.json")),
            mainnet(include_str!("../../mock-nearly/fixtures/get_tax.json")),
        );
        let l = r.launch.clone().unwrap().unwrap();
        assert_eq!(
            (l.creator.as_str(), l.token.as_str()),
            ("blueisgreen.near", "bigreen.nearlytrade.near")
        );
        assert_eq!((l.step.as_str(), l.quote.as_str()), ("Done", "wrap.near"));
        assert_eq!(l.pool_id, "bigreen.nearlytrade.near|wrap.near|10000");
        assert!(l.lpt_id.is_some());
        assert_eq!((r.fee_to.clone(), r.fee_mode.as_deref()), (Some(None), Some("creator")));
        assert_eq!(r.tax.clone().unwrap().unwrap().holders_bps, 3_400);
        let me: AccountId = "blueisgreen.near".parse().unwrap();
        let p = Peers {
            tithe: id("tithe.near"),
            ..peers()
        };
        assert_eq!(
            check_record(&r, &me, &p, U64(3657)),
            Err(BindRefusal::WrongFeeTo),
            "a real launch, not ours"
        );
        let pool = mainnet(include_str!("../../mock-nearly/fixtures/get_pool.json")).unwrap();
        let seen = dcl::parse_pool_seen(&pool).unwrap().unwrap();
        assert_eq!(
            (seen.token_x.as_str(), seen.state.as_str()),
            ("bigreen.nearlytrade.near", "Running")
        );
        let p = dcl::parse_pool(&pool, false).unwrap();
        assert!(p.running && p.depth_near > 0);
        let quote = mainnet(include_str!("../../mock-nearly/fixtures/quote_launch.json"));
        let w = Wiring {
            tithe: None,
            scales: None,
            quote,
            slug_count: mainnet(include_str!("../../mock-nearly/fixtures/get_slug_count.json")),
        };
        assert_eq!(w.slug_count.as_deref(), Some(&b"1"[..]));
        assert_eq!(
            dcl::object(w.quote.as_deref().unwrap()).unwrap().unwrap().u128("total"),
            Some(95_050_000_000_000_000_000_000)
        );
    }

    #[test]
    fn check_record_refuses_each_case_in_order() {
        let me = id("dragon.near");
        let ok = check_record(&record(), &me, &peers(), U64(5)).unwrap();
        assert_eq!(
            (ok.token.as_str(), ok.wrap_is_x, ok.launch_id),
            ("wyrm.nearlytrade.near", true, U64(5))
        );
        type Break = fn(&mut LaunchRecord);
        fn l(r: &mut LaunchRecord) -> &mut LaunchSeen {
            r.launch.as_mut().unwrap().as_mut().unwrap()
        }
        let cases: [(Break, BindRefusal); 12] = [
            (|r| r.tax = None, BindRefusal::ViewFailed),
            (|r| r.launch = Some(None), BindRefusal::NoRecord),
            (|r| l(r).step = "AddLiquidity".into(), BindRefusal::NotDone),
            (|r| l(r).lpt_id = None, BindRefusal::NotDone),
            (|r| l(r).creator = "creator.near".parse().unwrap(), BindRefusal::NotOurs),
            (|r| l(r).quote = "usdc.near".parse().unwrap(), BindRefusal::NotNearPair),
            (|r| r.fee_to = Some(None), BindRefusal::WrongFeeTo),
            (
                |r| r.fee_to = Some(Some("x.near".parse().unwrap())),
                BindRefusal::WrongFeeTo,
            ),
            (|r| r.fee_mode = Some("burn".into()), BindRefusal::WrongFeeMode),
            (|r| r.tax = Some(None), BindRefusal::WrongTax),
            (
                |r| r.tax.as_mut().unwrap().as_mut().unwrap().sell_bps = 300,
                BindRefusal::WrongTax,
            ),
            (
                |r| l(r).pool_id = "wyrm.nearlytrade.near|wrap.near|10000".into(),
                BindRefusal::BadPoolId("pool_id tokens must be sorted as DCL sorts them".into()),
            ),
        ];
        for (i, (brk, want)) in cases.into_iter().enumerate() {
            let mut r = record();
            brk(&mut r);
            assert_eq!(check_record(&r, &me, &peers(), U64(5)), Err(want), "case {i}");
        }
        // The order: a record that is not done and not ours says "not done" first.
        let mut r = record();
        l(&mut r).step = "CreatePool".into();
        l(&mut r).creator = "x.near".parse().unwrap();
        assert_eq!(check_record(&r, &me, &peers(), U64(5)), Err(BindRefusal::NotDone));

        let pool = |x: &str, y: &str, state: &str| {
            Some(Some(PoolSeen {
                token_x: x.parse().unwrap(),
                token_y: y.parse().unwrap(),
                state: state.into(),
            }))
        };
        let wrap = id("wrap.near");
        assert_eq!(
            check_pool(pool("wrap.near", "wyrm.nearlytrade.near", "Running"), &ok, &wrap),
            Ok(())
        );
        assert_eq!(check_pool(None, &ok, &wrap), Err(BindRefusal::ViewFailed));
        assert_eq!(check_pool(Some(None), &ok, &wrap), Err(BindRefusal::NoPool));
        assert_eq!(
            check_pool(pool("wrap.near", "fake.near", "Running"), &ok, &wrap),
            Err(BindRefusal::WrongPoolTokens)
        );
        assert_eq!(
            check_pool(pool("wrap.near", "wyrm.nearlytrade.near", "Paused"), &ok, &wrap),
            Err(BindRefusal::PoolNotRunning)
        );
    }
}
