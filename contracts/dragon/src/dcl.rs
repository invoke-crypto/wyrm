//! The boundary with wrap, the bound coin and Rhea DCL: promise results are parsed here into typed values, and a
//! malformed or failed answer becomes `None` instead of a panic that would strand the step in flight.
use near_sdk::{env, AccountId};

/// get_pool answers carry under a kilobyte; anything far larger is not a DCL answer.
const MAX_RESULT: usize = 8_192;

pub fn result(i: u64) -> Option<Vec<u8>> {
    env::promise_result_checked(i, MAX_RESULT).ok()
}

pub fn succeeded(i: u64) -> bool {
    result(i).is_some()
}

/// A JSON answer's top-level members as raw value text. The peers' answers are read through this one small scanner
/// rather than a serde type per answer, which kept the wasm under its size limit. Unknown members are skipped.
pub struct Object<'a>(Vec<(&'a str, &'a str)>);

/// `None` for anything but a well-formed object; `Some(None)` for `null`.
pub fn object(bytes: &[u8]) -> Option<Option<Object<'_>>> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text == "null" {
        return Some(None);
    }
    let b = text.as_bytes();
    if b.first() != Some(&b'{') || b.last() != Some(&b'}') {
        return None;
    }
    let mut members = vec![];
    let mut i = skip_ws(b, 1);
    while b[i] != b'}' {
        let key_end = value_end(b, i)?;
        let key = text.get(i + 1..key_end - 1).filter(|_| b[i] == b'"')?;
        i = skip_ws(b, key_end);
        if b.get(i) != Some(&b':') {
            return None;
        }
        let start = skip_ws(b, i + 1);
        let end = value_end(b, start)?;
        members.push((key, &text[start..end]));
        i = skip_ws(b, end);
        match b.get(i)? {
            b',' => {
                i = skip_ws(b, i + 1);
                if b.get(i) != Some(&b'"') {
                    return None;
                }
            }
            b'}' if i == b.len() - 1 => {}
            _ => return None,
        }
    }
    Some(Some(Object(members)))
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// The end of the value starting at `i`: a string with its escapes, a nested object or array, or a bare token.
fn value_end(b: &[u8], i: usize) -> Option<usize> {
    let (mut depth, mut in_string, mut k) = (0usize, false, i);
    while k < b.len() {
        let c = b[k];
        if in_string {
            match c {
                b'\\' => k += 1,
                b'"' => {
                    in_string = false;
                    if depth == 0 {
                        return Some(k + 1);
                    }
                }
                _ => {}
            }
        } else {
            match c {
                b'"' => in_string = true,
                b'{' | b'[' => depth += 1,
                b'}' | b']' if depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(k + 1);
                    }
                }
                b',' | b'}' | b']' if depth == 0 => return (k > i).then_some(k),
                c if depth == 0 && c.is_ascii_whitespace() => return (k > i).then_some(k),
                _ => {}
            }
        }
        k += 1;
    }
    (depth == 0 && !in_string && k > i).then_some(k)
}

impl<'a> Object<'a> {
    /// The one member of an object that has exactly one, as an externally tagged enum is encoded.
    pub fn only(&self) -> Option<(&'a str, &'a str)> {
        match self.0[..] {
            [member] => Some(member),
            _ => None,
        }
    }

    pub fn raw(&self, key: &str) -> Option<&'a str> {
        self.0.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    /// A string member with no escapes, which every id, step and mode Nearly and DCL answer is.
    pub fn str(&self, key: &str) -> Option<&'a str> {
        string(self.raw(key)?)
    }

    pub fn account(&self, key: &str) -> Option<AccountId> {
        self.str(key)?.parse().ok()
    }

    /// A u128 sent as a decimal string, as U128 is.
    pub fn u128(&self, key: &str) -> Option<u128> {
        self.str(key)?.parse().ok()
    }

    /// A bare JSON number.
    pub fn number(&self, key: &str) -> Option<u64> {
        self.raw(key)?.parse().ok()
    }

    pub fn is_null(&self, key: &str) -> bool {
        self.raw(key) == Some("null")
    }
}

/// A JSON string without escapes, unquoted.
pub fn string(raw: &str) -> Option<&str> {
    raw.strip_prefix('"')?
        .strip_suffix('"')
        .filter(|s| !s.contains(['\\', '"']))
}

pub fn parse_u128(bytes: &[u8]) -> Option<u128> {
    string(std::str::from_utf8(bytes).ok()?.trim())?.parse().ok()
}

pub struct Pool {
    /// The wrap side of total_x/total_y. Reported only, never sizing a slice: it counts far liquidity.
    pub depth_near: u128,
    pub running: bool,
}

pub fn parse_pool(bytes: &[u8], wrap_is_x: bool) -> Option<Pool> {
    let p = object(bytes)??;
    Some(Pool {
        depth_near: p.u128(if wrap_is_x { "total_x" } else { "total_y" })?,
        running: p.str("state")? == "Running",
    })
}

/// What bind reads of a pool: DCL answers `null` for an id it does not hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolSeen {
    pub token_x: AccountId,
    pub token_y: AccountId,
    pub state: String,
}

/// `None` when the view failed or did not parse, `Some(None)` when DCL holds no such pool.
pub fn parse_pool_seen(bytes: &[u8]) -> Option<Option<PoolSeen>> {
    let Some(p) = object(bytes)? else {
        return Some(None);
    };
    Some(Some(PoolSeen {
        token_x: p.account("token_x")?,
        token_y: p.account("token_y")?,
        state: p.str("state")?.into(),
    }))
}

/// Whether the answer names any pool at all. Launch refuses every existing pool, so its shape does not matter.
pub fn pool_exists(bytes: &[u8]) -> Option<bool> {
    object(bytes).map(|o| o.is_some())
}

/// DCL quotes a partial fill for an input the pool cannot fill, and zero for a wrong token. Zero is no price; a
/// partial fill fails the slice cap.
pub fn parse_quote(bytes: &[u8]) -> Option<u128> {
    object(bytes)??.u128("amount").filter(|a| *a > 0)
}

/// Checks the pool id names exactly the two tokens, sorted as DCL sorts, and says whether wrap is token x.
pub fn parse_pool_id(pool_id: &str, wrap: &AccountId, coin: &AccountId) -> Result<bool, String> {
    let parts: Vec<&str> = pool_id.split('|').collect();
    let [x, y, fee] = parts[..] else {
        return Err("pool_id must be \"<x>|<y>|<fee>\"".into());
    };
    if fee.parse::<u32>().is_err() {
        return Err("pool_id fee must be a number".into());
    }
    if x >= y {
        return Err("pool_id tokens must be sorted as DCL sorts them".into());
    }
    match (
        x == wrap.as_str(),
        x == coin.as_str(),
        y == wrap.as_str(),
        y == coin.as_str(),
    ) {
        (true, _, _, true) => Ok(true),
        (_, true, true, _) => Ok(false),
        _ => Err("pool_id must pair wrap with the coin".into()),
    }
}

// Outgoing arguments are written by hand: account ids and pool ids hold no character JSON escapes, and
// serde_json's Value would add float formatting to the wasm.
pub fn pool_args(pool_id: &str) -> String {
    format!(r#"{{"pool_id":"{pool_id}"}}"#)
}

pub fn quote_args(pool_id: &str, input: &AccountId, output: &AccountId, amount: u128) -> Vec<u8> {
    format!(
        r#"{{"pool_ids":["{pool_id}"],"input_token":"{input}","output_token":"{output}","input_amount":"{amount}","tag":null}}"#
    )
    .into_bytes()
}

/// A sell into wrap keeps its output as wNEAR (`skip_unwrap_near`), as every observed mainnet sell does, so it
/// arrives as a wrap transfer the reconcile can see.
pub fn swap_args(
    dcl: &AccountId,
    pool_id: &str,
    amount: u128,
    output: &AccountId,
    min_out: u128,
    sell: bool,
) -> Vec<u8> {
    let skip = if sell { r#",\"skip_unwrap_near\":true"# } else { "" };
    format!(
        r#"{{"receiver_id":"{dcl}","amount":"{amount}","msg":"{{\"Swap\":{{\"pool_ids\":[\"{pool_id}\"],\"output_token\":\"{output}\",\"min_output_amount\":\"{min_out}\"{skip}}}}}"}}"#
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::serde_json;

    fn id(s: &str) -> AccountId {
        s.parse().unwrap()
    }

    #[test]
    fn the_pool_id_must_pair_the_two_tokens_in_dcl_order() {
        let (wrap, wyrm) = (id("wrap.near"), id("wyrm.nearlytrade.near"));
        assert_eq!(
            parse_pool_id("wrap.near|wyrm.nearlytrade.near|10000", &wrap, &wyrm),
            Ok(true)
        );
        let (wrap, wyrm) = (id("wrap.near"), id("abc.near"));
        assert_eq!(parse_pool_id("abc.near|wrap.near|10000", &wrap, &wyrm), Ok(false));
        assert!(
            parse_pool_id("wrap.near|abc.near|10000", &wrap, &wyrm).is_err(),
            "unsorted"
        );
        assert!(parse_pool_id("abc.near|other.near|10000", &wrap, &wyrm).is_err());
        assert!(parse_pool_id("abc.near|wrap.near", &wrap, &wyrm).is_err());
    }

    #[test]
    fn answers_parse_and_garbage_becomes_none() {
        let pool = br#"{"pool_id":"a|b|1","token_x":"a.near","token_y":"b.near","total_x":"5","total_y":"7","current_point":-3,"state":"Running","extra":1}"#;
        let p = parse_pool(pool, false).unwrap();
        assert_eq!((p.depth_near, p.running), (7, true));
        assert!(parse_pool(b"null", true).is_none());
        assert!(parse_pool(b"{\"total_x\":5}", true).is_none());
        assert_eq!(parse_pool_seen(pool).unwrap().unwrap().token_y, id("b.near"));
        assert_eq!(parse_pool_seen(b"null"), Some(None));
        assert_eq!(parse_pool_seen(b"oops"), None);
        assert_eq!((pool_exists(pool), pool_exists(b"null")), (Some(true), Some(false)));
        assert_eq!(parse_quote(br#"{"amount":"42","tag":null}"#), Some(42));
        assert_eq!(parse_quote(br#"{"amount":"0"}"#), None);
        assert_eq!(parse_u128(br#""9""#), Some(9));
        assert_eq!(parse_u128(b"oops"), None);
    }

    #[test]
    fn the_scanner_reads_top_level_members_and_skips_the_rest() {
        let raw = br#" { "a" : "x.near", "n": 200, "nested": {"a": "no", "s": "}{,\"]"}, "list": [1, {"b": 2}], "z": null } "#;
        let o = object(raw).unwrap().unwrap();
        assert_eq!(o.str("a"), Some("x.near"), "the nested a is not the top-level one");
        assert_eq!(
            (o.number("n"), o.is_null("z"), o.raw("missing")),
            (Some(200), true, None)
        );
        assert_eq!(o.raw("list"), Some(r#"[1, {"b": 2}]"#));
        assert!(object(b"null").unwrap().is_none());
        for bad in [
            &b"[1]"[..],
            b"{\"a\" 1}",
            b"{\"a\":1",
            b"{\"a\":1,}",
            b"{\"a\":\"x\"}x",
            b"\xff",
        ] {
            assert!(object(bad).is_none(), "{bad:?}");
        }
        assert_eq!(
            object(br#"{"s":"a\"b"}"#).unwrap().unwrap().str("s"),
            None,
            "escaped strings are not ids"
        );
        assert_eq!(object(b"{}").unwrap().unwrap().raw("a"), None);
    }

    #[test]
    fn sells_keep_wnear_and_buys_say_nothing_of_it() {
        let msg = |sell| {
            let args: serde_json::Value =
                serde_json::from_slice(&swap_args(&id("dcl.near"), "a|b|1", 5, &id("b.near"), 4, sell)).unwrap();
            serde_json::from_str::<serde_json::Value>(args["msg"].as_str().unwrap()).unwrap()["Swap"].clone()
        };
        assert_eq!(msg(true)["skip_unwrap_near"], true);
        assert_eq!(msg(true)["min_output_amount"], "4");
        assert!(msg(false).get("skip_unwrap_near").is_none());
    }
}
