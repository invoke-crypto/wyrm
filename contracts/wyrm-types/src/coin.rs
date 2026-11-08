use crate::collection::check_url;
use near_sdk::json_types::U128;
use near_sdk::near;

pub const MAX_LINK_BYTES: usize = 200;

/// Nearly's `Links`, field for field.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Links {
    pub website: Option<String>,
    pub twitter: Option<String>,
    pub telegram: Option<String>,
}

impl Links {
    fn each(&self) -> impl Iterator<Item = (&'static str, &String)> {
        [
            ("website", &self.website),
            ("twitter", &self.twitter),
            ("telegram", &self.telegram),
        ]
        .into_iter()
        .filter_map(|(field, link)| link.as_ref().map(|l| (field, l)))
    }
}

/// The creator's coin text, parsed with Nearly's own synchronous `launch` rules so that Nearly cannot refuse it.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoinArgs {
    /// No '"', '\' or control characters. Nearly charges json_len(name), which counts escapes, against a fixed
    /// 140 bytes in quote_launch.
    pub name: String,
    pub symbol: String,
    /// No '"', '\' or control characters. Nearly's launch charges for json_len(icon), but quote_launch takes the
    /// raw icon_bytes, so an escaped icon would make the quoted deposit fall short.
    pub icon: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub links: Links,
    pub dev_buy: Option<U128>,
}

impl CoinArgs {
    pub fn parse(self) -> Result<CoinArgs, String> {
        let name = self.name.trim().to_string();
        let symbol = self.symbol.trim().to_string();
        let name_chars = name.chars().count();
        if !(1..=32).contains(&name_chars) {
            return Err(format!("name must be 1 to 32 characters, got {name_chars}"));
        }
        refuse_escapes("name", &name)?;
        let symbol_chars = symbol.chars().count();
        if !(1..=12).contains(&symbol_chars) {
            return Err(format!("symbol must be 1 to 12 characters, got {symbol_chars}"));
        }
        if !symbol.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(format!("symbol must be ASCII letters and digits, got {symbol:?}"));
        }
        let description_chars = self.description.chars().count();
        if description_chars > 500 {
            return Err(format!(
                "description must be at most 500 characters, got {description_chars}"
            ));
        }
        for (field, link) in self.links.each() {
            if link.len() > MAX_LINK_BYTES {
                return Err(format!(
                    "links.{field} must be at most {MAX_LINK_BYTES} bytes, got {}",
                    link.len()
                ));
            }
        }
        if let Some(icon) = &self.icon {
            check_url("icon", icon)?;
            refuse_escapes("icon", icon)?;
        }
        Ok(CoinArgs { name, symbol, ..self })
    }

    /// Nearly's `text_bytes` for quote_launch.
    pub fn text_bytes(&self) -> u32 {
        let links: usize = self.links.each().map(|(_, l)| l.len()).sum();
        (self.name.len() + self.symbol.len() + self.description.len() + links) as u32
    }

    pub fn icon_bytes(&self) -> u32 {
        self.icon.as_ref().map_or(0, |i| i.len() as u32)
    }
}

fn refuse_escapes(field: &str, text: &str) -> Result<(), String> {
    if text.chars().any(|c| c == '"' || c == '\\' || c.is_control()) {
        return Err(format!("{field} must not hold '\"', '\\' or control characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wyrm() -> CoinArgs {
        CoinArgs {
            name: " WYRM ".into(),
            symbol: "WYRM ".into(),
            icon: Some(format!("https://{}", "i".repeat(504))),
            description: "A dragon that buys its own coin.".into(),
            links: Links {
                website: Some("https://thewyrm.near.page".into()),
                twitter: None,
                telegram: None,
            },
            dev_buy: None,
        }
    }

    #[test]
    fn coin_args_follow_nearlys_limits() {
        let coin = wyrm().parse().unwrap();
        assert_eq!((coin.name.as_str(), coin.symbol.as_str()), ("WYRM", "WYRM"));
        assert_eq!(coin.icon_bytes(), 512);
        assert_eq!(coin.text_bytes(), 4 + 4 + 32 + 25);

        type Break = fn(&mut CoinArgs);
        let cases: [(Break, &str); 15] = [
            (
                |c| c.name = format!("{}\u{1b}", "\u{1F409}".repeat(31)),
                "name must not hold",
            ),
            (|c| c.name = "WY\"RM".into(), "name must not hold"),
            (|c| c.name = "WY\\RM".into(), "name must not hold"),
            (|c| c.icon = Some("https://x.io/a\"b.png".into()), "icon must not hold"),
            (|c| c.icon = Some("https://x.io/a\\b.png".into()), "icon must not hold"),
            (|c| c.icon = Some("https://x.io/a\nb.png".into()), "icon must not hold"),
            (
                |c| c.icon = Some(format!("https://{}", "i".repeat(505))),
                "icon must be at most 512 bytes, got 513",
            ),
            (
                |c| c.icon = Some("http://x.io/i.png".into()),
                "icon must start with https:// or ipfs://",
            ),
            (
                |c| c.icon = Some("data:image/png;base64,AA".into()),
                "icon must start with https:// or ipfs://",
            ),
            (
                |c| c.symbol = "S".repeat(13),
                "symbol must be 1 to 12 characters, got 13",
            ),
            (|c| c.symbol = "W-RM".into(), "symbol must be ASCII letters and digits"),
            (|c| c.symbol = "   ".into(), "symbol must be 1 to 12 characters, got 0"),
            (|c| c.name = "n".repeat(33), "name must be 1 to 32 characters, got 33"),
            (
                |c| c.description = "d".repeat(501),
                "description must be at most 500 characters, got 501",
            ),
            (
                |c| c.links.telegram = Some("t".repeat(201)),
                "links.telegram must be at most 200 bytes, got 201",
            ),
        ];
        for (break_it, needle) in cases {
            let mut c = wyrm();
            break_it(&mut c);
            let err = c.parse().unwrap_err();
            assert!(err.contains(needle), "{err}");
        }

        let mut edge = wyrm();
        edge.name = "é".repeat(32);
        edge.symbol = "S".repeat(12);
        edge.description = "d".repeat(500);
        edge.icon = Some("ipfs://bafy".into());
        edge.links.twitter = Some("t".repeat(200));
        assert!(edge.parse().is_ok());
    }
}
