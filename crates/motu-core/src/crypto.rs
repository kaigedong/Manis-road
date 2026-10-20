//! Request signing and payload protection recovered from Motu 4.1.9.
//!
//! Two layers exist in the original client:
//!
//! 1. `AppendEncryptionIntercepter` adds `timeStamp` and `sign` to every
//!    non-excluded call. `sign` is the lowercase MD5 of sorted `key=value&`
//!    pairs followed by the hard-coded secret.
//! 2. Endpoints marked `Security-Level: Encrypt` use
//!    `SecurityIntercepter`: AES-GCM with a per-request key, RSA-OAEP wrapped
//!    key, HMAC-SHA256 request/response signatures and
//!    `/api/security/public-key` bootstrap.
//!
//! The AAD passed to AES-GCM comes from the native `requireSalt` helper in
//! `libpro.so`. Runtime inspection with the official 4.1.9 build confirmed
//! that it returns the UTF-8 bytes of the salt after rejecting null/empty
//! input, so [`Aad::SaltUtf8`] is the behaviour of the original client.

use std::collections::BTreeMap;

use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes128Gcm, Aes256Gcm, KeyInit};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use rand::RngCore;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Oaep, RsaPublicKey};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha1::Sha1;
use sha2::{Sha256, Sha512};

use crate::error::{MotuError, Result};

/// Hard-coded signing secret found in `AppendEncryptionIntercepter`.
pub const SIGN_SECRET: &str = "6ccf59766be2ce07c8714fc54d768d42";
/// AES-CBC fallback key (`AppendEncryptionIntercepter.AES_KEY_BYTES`).
pub const FALLBACK_AES_KEY: &[u8] = b"rSsbdxSPjfsVWyVa";
/// AES-CBC fallback IV (`AppendEncryptionIntercepter.AES_IV_BYTES`).
pub const FALLBACK_AES_IV: &[u8] = b"VsvgRCrL9RDNcE8J";
/// Client version sent in the `version` header.
pub const APP_VERSION: &str = "4.1.9";
/// Android platform marker sent in the `platForm` header.
pub const PLATFORM: &str = "2";
/// Security config lifetime used by `SecurityKeyManager` (24 hours).
pub const SECURITY_CONFIG_TTL_MS: i64 = 86_400_000;

type HmacSha256 = Hmac<Sha256>;
type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Lowercase MD5 hex, matching Java `MD5Utils.md5PasswordTo32` for the login
/// password. The native signing helpers use [`encrypt_md5`] instead.
#[must_use]
pub fn md5_hex(input: &str) -> String {
    let digest = Md5::digest(input.as_bytes());
    hex_lower(&digest)
}

/// Uppercase MD5 hex, matching the native `AppSignatureHelper.md5` and
/// `AppendEncryptionIntercepter.encryptMD5` helpers. Request signatures and
/// `safe-env-token` use this case; the server is case-sensitive.
#[must_use]
pub fn encrypt_md5(input: &str) -> String {
    encrypt_md5_bytes(input.as_bytes())
}

/// Uppercase MD5 hex of arbitrary bytes, used for the APK signing
/// certificate. The certificate is DER, not UTF-8, so it cannot go through
/// [`encrypt_md5`].
#[must_use]
pub fn encrypt_md5_bytes(input: &[u8]) -> String {
    hex_lower(&Md5::digest(input)).to_ascii_uppercase()
}

/// MD5 of the APK signing certificate, as consumed by `safe-env-token`.
///
/// Returns an empty string when the certificate is unavailable, matching the
/// original client's behaviour of omitting `safe-env-token`.
#[must_use]
pub fn signing_certificate_md5(certificate_der: &[u8]) -> String {
    if certificate_der.is_empty() {
        String::new()
    } else {
        encrypt_md5_bytes(certificate_der)
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn base64_encode(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

fn base64_decode(value: &str) -> Result<Vec<u8>> {
    BASE64
        .decode(value)
        .map_err(|error| MotuError::Crypto(format!("invalid base64: {error}")))
}

/// Case-insensitive key ordering used by the Java signer.
fn sorted_keys(map: &Map<String, Value>) -> Vec<&String> {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by(|a, b| {
        a.to_ascii_lowercase()
            .cmp(&b.to_ascii_lowercase())
            .then_with(|| a.cmp(b))
    });
    keys
}

fn value_to_canonical(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) if text.is_empty() => None,
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).ok(),
    }
}

/// Canonical `k=v&...&key=secret` string before MD5.
///
/// Null and empty-string values are dropped, matching the Java interceptor.
#[must_use]
pub fn canonical_string(params: &Map<String, Value>) -> String {
    let mut out = String::new();
    for key in sorted_keys(params) {
        if let Some(value) = params.get(key).and_then(value_to_canonical) {
            out.push_str(key);
            out.push('=');
            out.push_str(&value);
            out.push('&');
        }
    }
    out.push_str("key=");
    out.push_str(SIGN_SECRET);
    out
}

/// Signature for a map of already-stringified parameters.
#[must_use]
pub fn sign_params(params: &BTreeMap<String, String>) -> String {
    let mut json = Map::new();
    for (key, value) in params {
        json.insert(key.clone(), Value::String(value.clone()));
    }
    encrypt_md5(&canonical_string(&json))
}

/// Sign a JSON request body and return `(signed_body, sign)`.
///
/// `timeStamp` is inserted as a decimal string, then the body is rebuilt with
/// the `sign` field, mirroring the Java interceptor's JSON branch.
pub fn sign_json_body(body: &Value, timestamp_ms: i64) -> Result<(Value, String)> {
    let Value::Object(object) = body else {
        return Err(MotuError::Api(
            "request body must be a JSON object to be signed".to_owned(),
        ));
    };

    let mut signable = object.clone();
    // Drop values the Java code removes before adding timeStamp.
    signable.retain(|_, value| {
        !value.is_null() && !matches!(value, Value::String(text) if text.is_empty())
    });
    signable.insert(
        "timeStamp".to_owned(),
        Value::String(timestamp_ms.to_string()),
    );
    let sign = encrypt_md5(&canonical_string(&signable));

    let mut signed = object.clone();
    signed.insert(
        "timeStamp".to_owned(),
        Value::String(timestamp_ms.to_string()),
    );
    signed.insert("sign".to_owned(), Value::String(sign.clone()));
    Ok((Value::Object(signed), sign))
}

/// Build the `safe-env-token` header.
///
/// `<unix_seconds>.<md5(deviceId + "-" + seconds + "-" + signatureMd5)>`.
#[must_use]
pub fn safe_env_token(device_id: &str, unix_seconds: u64, signature_md5: &str) -> String {
    let signature_md5 = signature_md5.to_ascii_uppercase();
    let digest = encrypt_md5(&format!("{device_id}-{unix_seconds}-{signature_md5}"));
    format!("{unix_seconds}.{digest}")
}

/// AES-128-CBC/PKCS#7 encryption used by the fallback interceptor.
pub fn aes_cbc_encrypt(plaintext: &str) -> Result<String> {
    let cipher = Aes128CbcEnc::new_from_slices(FALLBACK_AES_KEY, FALLBACK_AES_IV)
        .map_err(|error| MotuError::Crypto(format!("AES-CBC init failed: {error}")))?;
    let encrypted = cipher.encrypt_padded_vec_mut::<Pkcs7>(plaintext.as_bytes());
    Ok(base64_encode(&encrypted))
}

/// AES-128-CBC/PKCS#7 decryption used by the fallback interceptor.
pub fn aes_cbc_decrypt(ciphertext_b64: &str) -> Result<String> {
    if ciphertext_b64.is_empty() || ciphertext_b64 == "null" {
        return Err(MotuError::Crypto("empty ciphertext".to_owned()));
    }
    let encrypted = base64_decode(ciphertext_b64)?;
    let cipher = Aes128CbcDec::new_from_slices(FALLBACK_AES_KEY, FALLBACK_AES_IV)
        .map_err(|error| MotuError::Crypto(format!("AES-CBC init failed: {error}")))?;
    let decrypted = cipher
        .decrypt_padded_vec_mut::<Pkcs7>(&encrypted)
        .map_err(|error| MotuError::Crypto(format!("AES-CBC decrypt failed: {error}")))?;
    String::from_utf8(decrypted)
        .map_err(|error| MotuError::Crypto(format!("AES-CBC output is not UTF-8: {error}")))
}

/// AAD source for AES-GCM. See the module note about native `requireSalt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aad {
    /// UTF-8 bytes of the server salt. Confirmed against the official 4.1.9
    /// `requireSalt`, which rejects null/empty input and returns
    /// `salt.getBytes(StandardCharsets.UTF_8)`.
    SaltUtf8,
    /// Caller-supplied bytes, used by tests and by any future salt variant.
    Custom(Vec<u8>),
}

impl Aad {
    fn bytes(&self, salt: &str) -> Vec<u8> {
        match self {
            Self::SaltUtf8 => salt.as_bytes().to_vec(),
            Self::Custom(bytes) => bytes.clone(),
        }
    }
}

fn aes_gcm_encrypt_with(plaintext: &str, key: &[u8], iv: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    let payload = Payload {
        msg: plaintext.as_bytes(),
        aad,
    };
    if iv.len() != 12 {
        return Err(MotuError::Crypto(format!(
            "AES-GCM IV must be 12 bytes, got {}",
            iv.len()
        )));
    }
    let nonce = aes_gcm::Nonce::from_slice(iv);
    let result = if key.len() == 16 {
        Aes128Gcm::new_from_slice(key)
            .map_err(|error| MotuError::Crypto(error.to_string()))?
            .encrypt(nonce, payload)
    } else if key.len() == 32 {
        Aes256Gcm::new_from_slice(key)
            .map_err(|error| MotuError::Crypto(error.to_string()))?
            .encrypt(nonce, payload)
    } else {
        return Err(MotuError::Crypto(format!(
            "AES-GCM key must be 16 or 32 bytes, got {}",
            key.len()
        )));
    };
    result.map_err(|error| MotuError::Crypto(format!("AES-GCM encrypt failed: {error}")))
}

fn aes_gcm_decrypt_with(ciphertext: &[u8], key: &[u8], iv: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    let payload = Payload {
        msg: ciphertext,
        aad,
    };
    if iv.len() != 12 {
        return Err(MotuError::Crypto(format!(
            "AES-GCM IV must be 12 bytes, got {}",
            iv.len()
        )));
    }
    let nonce = aes_gcm::Nonce::from_slice(iv);
    let result = if key.len() == 16 {
        Aes128Gcm::new_from_slice(key)
            .map_err(|error| MotuError::Crypto(error.to_string()))?
            .decrypt(nonce, payload)
    } else if key.len() == 32 {
        Aes256Gcm::new_from_slice(key)
            .map_err(|error| MotuError::Crypto(error.to_string()))?
            .decrypt(nonce, payload)
    } else {
        return Err(MotuError::Crypto(format!(
            "AES-GCM key must be 16 or 32 bytes, got {}",
            key.len()
        )));
    };
    result.map_err(|error| MotuError::Crypto(format!("AES-GCM decrypt failed: {error}")))
}

/// AES-GCM encrypt with base64 key/IV and a salt-derived AAD.
pub fn aes_gcm_encrypt(
    plaintext: &str,
    key_b64: &str,
    iv_b64: &str,
    salt: &str,
    aad: &Aad,
) -> Result<String> {
    let key = base64_decode(key_b64)?;
    let iv = base64_decode(iv_b64)?;
    let ciphertext = aes_gcm_encrypt_with(plaintext, &key, &iv, &aad.bytes(salt))?;
    Ok(base64_encode(&ciphertext))
}

/// AES-GCM decrypt with base64 key/IV and a salt-derived AAD.
pub fn aes_gcm_decrypt(
    ciphertext_b64: &str,
    key_b64: &str,
    iv_b64: &str,
    salt: &str,
    aad: &Aad,
) -> Result<String> {
    let key = base64_decode(key_b64)?;
    let iv = base64_decode(iv_b64)?;
    let ciphertext = base64_decode(ciphertext_b64)?;
    let plaintext = aes_gcm_decrypt_with(&ciphertext, &key, &iv, &aad.bytes(salt))?;
    String::from_utf8(plaintext)
        .map_err(|error| MotuError::Crypto(format!("AES-GCM output is not UTF-8: {error}")))
}

/// HMAC-SHA256 with the **base64 text** of the key as the secret.
///
/// `HmacUtil.sign` calls `str2.getBytes()` on the base64 string rather than
/// decoding it; this function preserves that quirk.
pub fn hmac_sha256_b64(message: &str, key_b64_text: &str) -> Result<String> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key_b64_text.as_bytes())
        .map_err(|error| MotuError::Crypto(format!("HMAC init failed: {error}")))?;
    mac.update(message.as_bytes());
    Ok(base64_encode(&mac.finalize().into_bytes()))
}

/// Verify a base64 HMAC-SHA256 signature in constant time.
pub fn hmac_sha256_verify(message: &str, key_b64_text: &str, signature_b64: &str) -> Result<bool> {
    let expected = base64_decode(signature_b64)?;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key_b64_text.as_bytes())
        .map_err(|error| MotuError::Crypto(format!("HMAC init failed: {error}")))?;
    mac.update(message.as_bytes());
    Ok(mac.verify_slice(&expected).is_ok())
}

/// Encrypt a per-request AES key with the server's X.509 `SubjectPublicKeyInfo`.
///
/// `RsaUtil` uses `RSA/ECB/OAEPWithSHA-256AndMGF1Padding` with the platform
/// provider. That transform selects SHA-256 for the OAEP digest but leaves
/// MGF1 on its SHA-1 default, so MGF1 must be SHA-1 as well.
pub fn rsa_oaep_sha256_encrypt_b64(plaintext: &str, spki_der_b64: &str) -> Result<String> {
    let der = base64_decode(spki_der_b64)?;
    let public_key = RsaPublicKey::from_public_key_der(&der)
        .map_err(|error| MotuError::Crypto(format!("invalid RSA public key: {error}")))?;
    let mut rng = rand::thread_rng();
    let padding = Oaep::new_with_mgf_hash::<Sha256, Sha1>();
    let encrypted = public_key
        .encrypt(&mut rng, padding, plaintext.as_bytes())
        .map_err(|error| MotuError::Crypto(format!("RSA-OAEP encrypt failed: {error}")))?;
    Ok(base64_encode(&encrypted))
}

/// Generate a 32-byte AES key and 12-byte GCM IV, both base64 encoded.
#[must_use]
pub fn generate_key_and_iv() -> (String, String) {
    let mut key = [0_u8; 32];
    let mut iv = [0_u8; 12];
    let mut rng = rand::thread_rng();
    rng.fill_bytes(&mut key);
    rng.fill_bytes(&mut iv);
    (base64_encode(&key), base64_encode(&iv))
}

/// Server security bootstrap from `/api/security/public-key`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityConfig {
    pub key_id: String,
    pub public_key: String,
    pub salt: String,
    #[serde(default)]
    pub fetched_at_ms: i64,
    #[serde(default = "default_ttl_ms")]
    pub ttl_ms: i64,
}

const fn default_ttl_ms() -> i64 {
    SECURITY_CONFIG_TTL_MS
}

impl SecurityConfig {
    #[must_use]
    pub fn is_valid(&self, now_ms: i64) -> bool {
        !self.key_id.is_empty()
            && !self.public_key.is_empty()
            && !self.salt.is_empty()
            && now_ms.saturating_sub(self.fetched_at_ms) < self.ttl_ms
    }
}

/// JSON envelope posted to encrypted endpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityEnvelope {
    pub key_id: String,
    pub encrypted_key: String,
    pub iv: String,
    pub data: String,
    pub timestamp: i64,
    pub nonce: String,
    pub sign: String,
}

/// Encrypted payload returned by an encrypted endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityResponse {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub nonce: String,
    pub iv: String,
    pub data: String,
    pub sign: String,
}

/// Build an encrypted request envelope with caller-supplied key material.
///
/// `timestamp` is Unix seconds. `nonce` is the Java UUID without dashes.
pub fn build_envelope_with_material(
    config: &SecurityConfig,
    plaintext: &str,
    timestamp: i64,
    nonce: &str,
    key_b64: &str,
    iv_b64: &str,
    aad: &Aad,
) -> Result<SecurityEnvelope> {
    let data = aes_gcm_encrypt(plaintext, key_b64, iv_b64, &config.salt, aad)?;
    let encrypted_key = rsa_oaep_sha256_encrypt_b64(key_b64, &config.public_key)?;
    let message = format!("{}\n{}\n{}\n{}", config.key_id, timestamp, nonce, data);
    let sign = hmac_sha256_b64(&message, key_b64)?;
    Ok(SecurityEnvelope {
        key_id: config.key_id.clone(),
        encrypted_key,
        iv: iv_b64.to_owned(),
        data,
        timestamp,
        nonce: nonce.to_owned(),
        sign,
    })
}

/// Build an encrypted request envelope with fresh random key material,
/// returning the key so the response can be verified and decrypted.
pub fn build_envelope_start(
    config: &SecurityConfig,
    plaintext: &str,
    timestamp: i64,
    nonce: &str,
    aad: &Aad,
) -> Result<(SecurityEnvelope, String)> {
    let (key, iv) = generate_key_and_iv();
    let envelope =
        build_envelope_with_material(config, plaintext, timestamp, nonce, &key, &iv, aad)?;
    Ok((envelope, key))
}

/// Build an encrypted request envelope with fresh random key material.
pub fn build_envelope(
    config: &SecurityConfig,
    plaintext: &str,
    timestamp: i64,
    nonce: &str,
    aad: &Aad,
) -> Result<SecurityEnvelope> {
    build_envelope_start(config, plaintext, timestamp, nonce, aad).map(|(envelope, _key)| envelope)
}

/// Verify and decrypt an encrypted response body.
///
/// The response signature covers `timestamp\nnonce\ndata` and uses the same
/// per-request key as the request.
pub fn open_response(
    response: &SecurityResponse,
    key_b64: &str,
    salt: &str,
    aad: &Aad,
) -> Result<String> {
    let message = format!(
        "{}\n{}\n{}",
        response.timestamp, response.nonce, response.data
    );
    if !hmac_sha256_verify(&message, key_b64, &response.sign)? {
        return Err(MotuError::Crypto(
            "response HMAC verification failed".to_owned(),
        ));
    }
    aes_gcm_decrypt(&response.data, key_b64, &response.iv, salt, aad)
}

/// Generate a 32-character lowercase hex nonce (Java UUID without dashes).
#[must_use]
pub fn generate_nonce() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex_lower(&bytes)
}

/// SHA-256 hex helper used by integrity screens.
#[must_use]
pub fn sha256_hex(input: &[u8]) -> String {
    hex_lower(&Sha256::digest(input))
}

/// SHA-512 hex helper used by integrity screens.
#[must_use]
pub fn sha512_hex(input: &[u8]) -> String {
    hex_lower(&Sha512::digest(input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;
    use rsa::pkcs8::EncodePublicKey;
    use rsa::{RsaPrivateKey, traits::PublicKeyParts};

    #[test]
    fn md5_matches_known_vectors() {
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(encrypt_md5("abc"), "900150983CD24FB0D6963F7D28E17F72");
        assert_eq!(
            encrypt_md5_bytes(b"abc"),
            "900150983CD24FB0D6963F7D28E17F72"
        );
        assert_eq!(signing_certificate_md5(b""), "");
        assert_eq!(
            signing_certificate_md5(b"abc"),
            "900150983CD24FB0D6963F7D28E17F72"
        );
    }

    #[test]
    fn canonical_string_sorts_case_insensitively_and_skips_empty() {
        let mut params = Map::new();
        params.insert("b".into(), Value::String("2".into()));
        params.insert("A".into(), Value::String("1".into()));
        params.insert("empty".into(), Value::String(String::new()));
        params.insert("none".into(), Value::Null);
        assert_eq!(
            canonical_string(&params),
            "A=1&b=2&key=6ccf59766be2ce07c8714fc54d768d42"
        );
    }

    #[test]
    fn signed_body_inserts_timestamp_and_sign() {
        let body = serde_json::json!({"plateVal": "沪A12345", "cityCode": "310000"});
        let (signed, sign) = sign_json_body(&body, 1_700_000_000_000).unwrap();
        assert_eq!(signed["timeStamp"], "1700000000000");
        assert_eq!(signed["sign"], sign);
        assert_eq!(signed["plateVal"], "沪A12345");
        assert_eq!(sign, sign.to_ascii_uppercase());
    }

    #[test]
    fn safe_env_token_has_expected_shape() {
        let token = safe_env_token(
            "device-1",
            1_700_000_000,
            "900150983cd24fb0d6963f7d28e17f72",
        );
        let (seconds, digest) = token.split_once('.').unwrap();
        assert_eq!(seconds, "1700000000");
        assert_eq!(digest.len(), 32);
        assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(digest, digest.to_ascii_uppercase());
        assert_eq!(digest, "C56B53CB0EE12455D61EC17AEE1C246A");
    }

    #[test]
    fn aes_cbc_round_trips() {
        let plaintext = r#"{"code":0,"data":"摩途"}"#;
        let encrypted = aes_cbc_encrypt(plaintext).unwrap();
        assert_ne!(encrypted, plaintext);
        assert_eq!(aes_cbc_decrypt(&encrypted).unwrap(), plaintext);
    }

    #[test]
    fn aes_gcm_round_trips_with_salt_aad() {
        let (key, iv) = generate_key_and_iv();
        let encrypted = aes_gcm_encrypt("hello", &key, &iv, "salt", &Aad::SaltUtf8).unwrap();
        let decrypted = aes_gcm_decrypt(&encrypted, &key, &iv, "salt", &Aad::SaltUtf8).unwrap();
        assert_eq!(decrypted, "hello");
    }

    #[test]
    fn aes_gcm_rejects_wrong_aad() {
        let (key, iv) = generate_key_and_iv();
        let encrypted = aes_gcm_encrypt("hello", &key, &iv, "salt", &Aad::SaltUtf8).unwrap();
        assert!(aes_gcm_decrypt(&encrypted, &key, &iv, "other", &Aad::SaltUtf8).is_err());
    }

    #[test]
    fn require_salt_aad_is_utf8_salt() {
        let salt = "TW90dVNlY3VyZUFwaVYyU2FsdDIwMjYwNzIy";
        assert_eq!(Aad::SaltUtf8.bytes(salt), salt.as_bytes());
    }

    #[test]
    fn aes_gcm_matches_known_vector_with_live_salt_aad() {
        // Vector generated independently with Node's AES-256-GCM using the
        // salt published by the official service on 2026-10-20.
        let key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
        let iv = "AAECAwQFBgcICQoL";
        let salt = "TW90dVNlY3VyZUFwaVYyU2FsdDIwMjYwNzIy";
        let plaintext = r#"{"message":"操作成功","code":0}"#;
        let expected = "PCC7fraWo3zoY62pV3r1iT5KYbxgntXjGkvH5nINZZA7INPNpxjnu+foUo3XM6klYzPK";
        let encrypted = aes_gcm_encrypt(plaintext, key, iv, salt, &Aad::SaltUtf8).unwrap();
        assert_eq!(encrypted, expected);
        assert_eq!(
            aes_gcm_decrypt(expected, key, iv, salt, &Aad::SaltUtf8).unwrap(),
            plaintext
        );
    }

    #[test]
    fn hmac_uses_base64_key_text() {
        let signature = hmac_sha256_b64("message", "a2V5").unwrap();
        assert!(hmac_sha256_verify("message", "a2V5", &signature).unwrap());
        assert!(!hmac_sha256_verify("other", "a2V5", &signature).unwrap());
    }

    #[test]
    fn rsa_oaep_uses_sha256_digest_and_sha1_mgf1() {
        let mut rng = OsRng;
        let private = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public = private.to_public_key();
        assert!(public.n().bits() >= 2048);
        let spki = public.to_public_key_der().unwrap();
        let encoded = BASE64.encode(spki.as_bytes());
        let encrypted = rsa_oaep_sha256_encrypt_b64("a2V5", &encoded).unwrap();
        let ciphertext = BASE64.decode(encrypted).unwrap();
        let padding = Oaep::new_with_mgf_hash::<Sha256, Sha1>();
        let decrypted = private.decrypt(padding, &ciphertext).unwrap();
        assert_eq!(decrypted, b"a2V5");
        // Java's OAEPWithSHA-256AndMGF1Padding defaults MGF1 to SHA-1; the
        // Rust default (SHA-256 for both) must not be used by the client.
        assert!(private.decrypt(Oaep::new::<Sha256>(), &ciphertext).is_err());
    }

    #[test]
    fn envelope_signature_and_payload_round_trip() {
        let mut rng = OsRng;
        let private = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public = private.to_public_key();
        let spki = public.to_public_key_der().unwrap();
        let config = SecurityConfig {
            key_id: "kid-1".into(),
            public_key: BASE64.encode(spki.as_bytes()),
            salt: "server-salt".into(),
            fetched_at_ms: 1_700_000_000_000,
            ttl_ms: 86_400_000,
        };
        let (key, iv) = generate_key_and_iv();
        let envelope = build_envelope_with_material(
            &config,
            r#"{"a":1}"#,
            1_700_000_000,
            "nonce",
            &key,
            &iv,
            &Aad::SaltUtf8,
        )
        .unwrap();
        assert!(
            hmac_sha256_verify(
                &format!("kid-1\n1700000000\nnonce\n{}", envelope.data),
                &key,
                &envelope.sign
            )
            .unwrap()
        );
        let decrypted = aes_gcm_decrypt(
            &envelope.data,
            &key,
            &envelope.iv,
            &config.salt,
            &Aad::SaltUtf8,
        )
        .unwrap();
        assert_eq!(decrypted, r#"{"a":1}"#);
    }

    #[test]
    fn security_config_ttl_is_enforced() {
        let config = SecurityConfig {
            key_id: "k".into(),
            public_key: "p".into(),
            salt: "s".into(),
            fetched_at_ms: 1_000,
            ttl_ms: 500,
        };
        assert!(config.is_valid(1_499));
        assert!(!config.is_valid(1_500));
    }
}
