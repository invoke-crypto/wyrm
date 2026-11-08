use near_contract_standards::non_fungible_token::metadata::{NFTContractMetadata, NFT_METADATA_SPEC};
use near_sdk::near;

pub const MAX_URL_BYTES: usize = 512;

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionArgs {
    pub name: String,
    pub symbol: String,
    pub icon: Option<String>,
    pub tier_media: [String; 4],
}

impl CollectionArgs {
    pub fn check(&self) -> Result<(), String> {
        if !(1..=64).contains(&self.name.len()) {
            return Err(format!("name must be 1 to 64 bytes, got {}", self.name.len()));
        }
        if !(1..=16).contains(&self.symbol.len()) {
            return Err(format!("symbol must be 1 to 16 bytes, got {}", self.symbol.len()));
        }
        if !self.symbol.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(format!(
                "symbol must be ASCII letters and digits, got {:?}",
                self.symbol
            ));
        }
        if let Some(icon) = &self.icon {
            check_url("icon", icon)?;
        }
        for (i, media) in self.tier_media.iter().enumerate() {
            check_url(&format!("tier_media[{i}]"), media)?;
        }
        Ok(())
    }

    pub fn metadata(&self) -> NFTContractMetadata {
        NFTContractMetadata {
            spec: NFT_METADATA_SPEC.to_string(),
            name: self.name.clone(),
            symbol: self.symbol.clone(),
            icon: self.icon.clone(),
            base_uri: None,
            reference: None,
            reference_hash: None,
        }
    }
}

pub(crate) fn check_url(field: &str, url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("ipfs://")) {
        return Err(format!("{field} must start with https:// or ipfs://"));
    }
    if url.len() > MAX_URL_BYTES {
        return Err(format!(
            "{field} must be at most {MAX_URL_BYTES} bytes, got {}",
            url.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wyrm_scales() -> CollectionArgs {
        CollectionArgs {
            name: "WYRM Scales".into(),
            symbol: "SCALE".into(),
            icon: Some("https://thewyrm.near.page/icon.svg".into()),
            tier_media: ["hatchling", "drake", "wyrm", "elder"].map(|t| format!("ipfs://bafy/{t}.png")),
        }
    }

    #[test]
    fn collection_args_follow_their_limits() {
        assert_eq!(wyrm_scales().check(), Ok(()));
        let m = wyrm_scales().metadata();
        assert_eq!(
            (m.spec.as_str(), m.name.as_str(), m.symbol.as_str()),
            (NFT_METADATA_SPEC, "WYRM Scales", "SCALE")
        );
        m.assert_valid();

        type Break = fn(&mut CollectionArgs);
        let cases: [(Break, &str); 9] = [
            (|c| c.name = "n".repeat(65), "name must be 1 to 64 bytes, got 65"),
            (|c| c.name.clear(), "name must be 1 to 64 bytes, got 0"),
            (|c| c.symbol = "S".repeat(17), "symbol must be 1 to 16 bytes, got 17"),
            (
                |c| c.symbol = "W-SCALE".into(),
                "symbol must be ASCII letters and digits",
            ),
            (
                |c| c.icon = Some("http://x.io/i.png".into()),
                "icon must start with https:// or ipfs://",
            ),
            (
                |c| c.tier_media[2] = format!("https://{}", "m".repeat(505)),
                "tier_media[2] must be at most 512 bytes, got 513",
            ),
            (
                |c| c.tier_media[0] = "data:image/png;base64,AA".into(),
                "tier_media[0] must start with",
            ),
            (
                |c| c.icon = Some(format!("ipfs://{}", "i".repeat(506))),
                "icon must be at most 512 bytes, got 513",
            ),
            (|c| c.symbol = "SCALÉ".into(), "symbol must be ASCII letters and digits"),
        ];
        for (break_it, needle) in cases {
            let mut c = wyrm_scales();
            break_it(&mut c);
            let err = c.check().unwrap_err();
            assert!(err.contains(needle), "{err}");
        }

        let mut edge = wyrm_scales();
        edge.name = "n".repeat(64);
        edge.symbol = "S".repeat(16);
        edge.icon = None;
        edge.tier_media[3] = format!("https://{}", "m".repeat(504));
        assert_eq!(edge.check(), Ok(()));
    }
}
