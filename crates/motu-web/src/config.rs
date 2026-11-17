//! Environment-driven server configuration.

use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use motu_core::signing_certificate_md5;

/// Certificate the server falls back to when neither signature variable is
/// set. It lives inside the (git-ignored) data directory so a locally
/// extracted certificate survives restarts without ending up in the repo.
const LOCAL_CERT_FILE: &str = "signing-cert.der";

/// `0` means "never expire": spent money / ticket history is kept until the
/// user explicitly refreshes or clears the cache.
const DEFAULT_CACHE_TTL_SECS: u64 = 0;
/// One week: official camera points are effectively static.
const DEFAULT_SYNC_TTL_SECS: u64 = 7 * 24 * 60 * 60;

const DEFAULT_TILE_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
const DEFAULT_TILE_ATTRIBUTION: &str =
    "&copy; <a href=\"https://www.openstreetmap.org/copyright\">OpenStreetMap</a> contributors";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignatureSource {
    #[default]
    None,
    Md5Env,
    CertEnv,
    DataDirCert,
}

impl SignatureSource {
    /// Configuration key this value came from; empty when unconfigured.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Md5Env => "MOTU_SIGNATURE_MD5",
            Self::CertEnv => "MOTU_CERT_DER",
            Self::DataDirCert => LOCAL_CERT_FILE,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebConfig {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    /// Empty means use the core/default Motu base URL.
    pub api_base_url: String,
    /// Uppercase MD5 of the client signing certificate. Empty disables
    /// `safe-env-token`, which may make some official endpoints reject requests.
    pub signature_md5: String,
    /// Where [`Self::signature_md5`] came from, for logging and `/api/config`.
    pub signature_source: SignatureSource,
    /// How long a cached camera detail/area layer is served without a network
    /// refresh. `0` (the default) never expires; manual refresh still works.
    pub cache_ttl_secs: u64,
    /// How long a recorded sync area counts as covered, so a viewport inside it
    /// is served from cache instead of re-querying Motu. `0` always queries.
    pub sync_ttl_secs: u64,
    /// When set, no upstream request is made at all; only local data and the
    /// offline road graph are used.
    pub offline: bool,
    /// Path to the binary road graph built by `motu-graph build`.
    pub graph_path: PathBuf,
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
        let (signature_md5, signature_source) = load_signature_md5(&data_dir)?;
        let cache_ttl_secs = env::var("MOTU_WEB_CACHE_TTL_SECS")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|error| format!("MOTU_WEB_CACHE_TTL_SECS is not a number: {error}"))
            })
            .transpose()?
            .unwrap_or(DEFAULT_CACHE_TTL_SECS);
        let sync_ttl_secs = env::var("MOTU_WEB_SYNC_TTL_SECS")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|error| format!("MOTU_WEB_SYNC_TTL_SECS is not a number: {error}"))
            })
            .transpose()?
            .unwrap_or(DEFAULT_SYNC_TTL_SECS);
        let offline = env::var("MOTU_WEB_OFFLINE")
            .ok()
            .is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            });
        let graph_path = PathBuf::from(
            env::var("MOTU_WEB_GRAPH").unwrap_or_else(|_| "var/offline/graph.bin".to_owned()),
        );
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
            signature_source,
            cache_ttl_secs,
            sync_ttl_secs,
            offline,
            graph_path,
            tile_url,
            tile_attribution,
        })
    }
}

fn load_signature_md5(data_dir: &Path) -> Result<(String, SignatureSource), String> {
    resolve_signature_md5(
        env::var("MOTU_SIGNATURE_MD5").ok(),
        env::var("MOTU_CERT_DER").ok(),
        data_dir,
    )
}

/// Precedence: explicit MD5, explicit certificate path, then the local
/// certificate inside the data directory.
fn resolve_signature_md5(
    env_md5: Option<String>,
    env_cert: Option<String>,
    data_dir: &Path,
) -> Result<(String, SignatureSource), String> {
    if let Some(value) = env_md5 {
        let md5 = normalize_signature_md5(&value)?;
        let source = if md5.is_empty() {
            SignatureSource::None
        } else {
            SignatureSource::Md5Env
        };
        return Ok((md5, source));
    }
    if let Some(path) = env_cert.map(|value| value.trim().to_owned()).filter(|p| !p.is_empty()) {
        let md5 = read_certificate_md5(Path::new(&path))
            .map_err(|error| format!("MOTU_CERT_DER {path}: {error}"))?;
        return Ok((md5, SignatureSource::CertEnv));
    }
    let local = data_dir.join(LOCAL_CERT_FILE);
    if local.is_file() {
        let md5 = read_certificate_md5(&local)
            .map_err(|error| format!("{}: {error}", local.display()))?;
        return Ok((md5, SignatureSource::DataDirCert));
    }
    Ok((String::new(), SignatureSource::None))
}

fn read_certificate_md5(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("cannot read: {error}"))?;
    let md5 = signing_certificate_md5(&bytes);
    if md5.trim().is_empty() {
        return Err("certificate must be non-empty".to_owned());
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

    #[test]
    fn local_certificate_is_used_when_environment_is_unset() {
        let directory = tempfile::tempdir().expect("tempdir");
        let bytes = b"certificate bytes";
        std::fs::write(directory.path().join(LOCAL_CERT_FILE), bytes).expect("write");

        let (md5, source) = resolve_signature_md5(None, None, directory.path()).expect("resolve");
        assert_eq!(source, SignatureSource::DataDirCert);
        assert_eq!(md5, motu_core::encrypt_md5_bytes(bytes));
    }

    #[test]
    fn environment_md5_wins_over_local_certificate() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join(LOCAL_CERT_FILE), b"certificate bytes")
            .expect("write");

        let (md5, source) = resolve_signature_md5(
            Some("0123456789abcdef0123456789abcdef".to_owned()),
            None,
            directory.path(),
        )
        .expect("resolve");
        assert_eq!(source, SignatureSource::Md5Env);
        assert_eq!(md5, "0123456789ABCDEF0123456789ABCDEF");
    }

    #[test]
    fn empty_environment_md5_disables_the_signature() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join(LOCAL_CERT_FILE), b"certificate bytes")
            .expect("write");

        let (md5, source) =
            resolve_signature_md5(Some("  ".to_owned()), None, directory.path()).expect("resolve");
        assert_eq!(source, SignatureSource::None);
        assert!(md5.is_empty());
    }

    #[test]
    fn missing_local_certificate_stays_unconfigured() {
        let directory = tempfile::tempdir().expect("tempdir");
        let (md5, source) = resolve_signature_md5(None, None, directory.path()).expect("resolve");
        assert_eq!(source, SignatureSource::None);
        assert!(md5.is_empty());
    }

    #[test]
    fn empty_local_certificate_is_rejected() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join(LOCAL_CERT_FILE), b"").expect("write");
        assert!(resolve_signature_md5(None, None, directory.path()).is_err());
    }
}
