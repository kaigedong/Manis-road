//! Environment-driven server configuration.

use std::{env, net::SocketAddr, path::PathBuf};

use motu_core::signing_certificate_md5;

const DEFAULT_TILE_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
const DEFAULT_TILE_ATTRIBUTION: &str =
    "&copy; <a href=\"https://www.openstreetmap.org/copyright\">OpenStreetMap</a> contributors";

#[derive(Debug, Clone)]
pub struct WebConfig {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    /// Empty means use the core/default Motu base URL.
    pub api_base_url: String,
    /// Uppercase MD5 of the client signing certificate. Empty disables
    /// `safe-env-token`, which may make some official endpoints reject requests.
    pub signature_md5: String,
    pub tile_url: String,
    pub tile_attribution: String,
}

impl WebConfig {
    pub fn from_env() -> Result<Self, String> {
        let bind = env::var("MOTU_WEB_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8787".to_owned())
            .parse()
            .map_err(|error| format!("MOTU_WEB_ADDR is not a valid socket address: {error}"))?;
        let data_dir = PathBuf::from(
            env::var("MOTU_WEB_DATA_DIR").unwrap_or_else(|_| "var/motu-web".to_owned()),
        );
        let api_base_url = env::var("MOTU_WEB_API_BASE_URL")
            .unwrap_or_default()
            .trim()
            .to_owned();
        let signature_md5 = load_signature_md5()?;
        let tile_url = env::var("MOTU_WEB_TILE_URL")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_TILE_URL.to_owned());
        let tile_attribution = env::var("MOTU_WEB_TILE_ATTRIBUTION")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_TILE_ATTRIBUTION.to_owned());
        Ok(Self {
            bind,
            data_dir,
            api_base_url,
            signature_md5,
            tile_url,
            tile_attribution,
        })
    }
}

fn load_signature_md5() -> Result<String, String> {
    if let Ok(value) = env::var("MOTU_SIGNATURE_MD5") {
        return normalize_signature_md5(&value);
    }
    let Ok(path) = env::var("MOTU_CERT_DER") else {
        return Ok(String::new());
    };
    let path = path.trim();
    if path.is_empty() {
        return Ok(String::new());
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read MOTU_CERT_DER {path}: {error}"))?;
    let md5 = signing_certificate_md5(&bytes);
    if md5.trim().is_empty() {
        return Err("MOTU_CERT_DER must contain a non-empty certificate".to_owned());
    }
    Ok(md5)
}

fn normalize_signature_md5(value: &str) -> Result<String, String> {
    let value = value.trim().to_ascii_uppercase();
    if value.is_empty() {
        return Ok(value);
    }
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("MOTU_SIGNATURE_MD5 must be a 32-character MD5 hex string".to_owned());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_md5_is_normalized_to_uppercase() {
        let value = normalize_signature_md5(" 0123456789abcdef0123456789abcdef ").unwrap();
        assert_eq!(value, "0123456789ABCDEF0123456789ABCDEF");
    }

    #[test]
    fn empty_signature_md5_is_allowed() {
        assert_eq!(normalize_signature_md5("   ").unwrap(), "");
    }

    #[test]
    fn malformed_signature_md5_is_rejected() {
        assert!(normalize_signature_md5("abc").is_err());
        assert!(normalize_signature_md5("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_err());
    }
}
