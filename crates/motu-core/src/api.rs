//! HTTP surface recovered from `com.motu.api.service.*` and the `OkHttp`
//! interceptor chain in `com.motumap.base.http`.
//!
//! Interceptor order matters and is reproduced in [`build_request`] plus
//! [`SecurityClient`]:
//!
//! 1. `AppendHeaderParamIntercepter` adds platform/token/device/version headers.
//! 2. `AppendEncryptionIntercepter` adds `timeStamp` and the MD5 `sign`.
//! 3. `SecurityIntercepter` encrypts the already-signed body for endpoints
//!    marked `Security-Level: Encrypt`.
//!
//! Endpoints in [`EXCLUDED_FROM_SIGNING`] are passed through unsigned.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::crypto::{
    self, Aad, SecurityConfig, SecurityResponse, build_envelope_start, open_response,
    sign_json_body,
};
use crate::error::{MotuError, Result};
use crate::model::ApiEnvelope;

/// Recovered Retrofit base URL.
pub const BASE_URL: &str = "https://motu.motumap.com/";
/// Security bootstrap path used by `SecurityIntercepter`.
pub const PUBLIC_KEY_PATH: &str = "api/security/public-key";
/// Header carrying the client version (`version`).
pub const HEADER_VERSION: &str = "version";
/// Header carrying the Android platform marker (`platForm`).
pub const HEADER_PLATFORM: &str = "platForm";
/// Header toggling the encrypted protocol.
pub const HEADER_SECURITY_LEVEL: &str = "Security-Level";
/// Value of [`HEADER_SECURITY_LEVEL`] for encrypted endpoints.
pub const SECURITY_LEVEL_ENCRYPT: &str = "Encrypt";
/// URLs skipped by the signing interceptor.
pub const EXCLUDED_FROM_SIGNING: [&str; 2] = ["poi/getPoiTabs", "https://tsapi.amap.com"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
}

impl HttpMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// A recovered endpoint definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub name: &'static str,
    pub path: &'static str,
    pub method: HttpMethod,
    pub encrypted: bool,
}

impl Endpoint {
    #[must_use]
    pub const fn get(name: &'static str, path: &'static str) -> Self {
        Self {
            name,
            path,
            method: HttpMethod::Get,
            encrypted: false,
        }
    }

    #[must_use]
    pub const fn post(name: &'static str, path: &'static str) -> Self {
        Self {
            name,
            path,
            method: HttpMethod::Post,
            encrypted: false,
        }
    }

    #[must_use]
    pub const fn post_encrypted(name: &'static str, path: &'static str) -> Self {
        Self {
            name,
            path,
            method: HttpMethod::Post,
            encrypted: true,
        }
    }
}

/// Every endpoint recovered from `IPoiAPIService` and `IUserApiService`.
pub mod endpoints {
    use super::Endpoint;

    // POI / camera / route
    pub const CAL_PATH_AREA_SEC: Endpoint =
        Endpoint::post_encrypted("calPathAreaSec", "v2/poi/calPathByAreaSec");
    pub const CAL_PATH_AVOID_SEC: Endpoint =
        Endpoint::post_encrypted("calPathAvoidSec", "v11/poi/calPathAvoidSec");
    pub const CAL_PATH_QUICK: Endpoint = Endpoint::post("calPathQuick", "v1/poi/calPathQuick");
    pub const LOAD_CAMERA_TAB_TYPE: Endpoint =
        Endpoint::get("loadCameraTabType", "v2/poi/getTabType");
    pub const LOAD_CAMERA_TAB_TYPE_GROUP: Endpoint =
        Endpoint::get("loadCameraTabTypeGroup", "v2/poi/getTabTypeGroup");
    pub const LOAD_CITY_TAB_LIST: Endpoint = Endpoint::get("loadCityTabList", "v2/poi/getTab");
    pub const LOAD_POI_DETAIL_SEC: Endpoint =
        Endpoint::post_encrypted("loadPoiDetailSec", "v1/poi/poiDetailSec");
    pub const LOAD_POI_SHOP_DETAIL_SEC: Endpoint =
        Endpoint::post_encrypted("loadPoiShopDetailSec", "v1/poi/shopDetailSec");
    pub const LOAD_TRAFFIC_CAMERA_DETAIL_SEC: Endpoint =
        Endpoint::post_encrypted("loadTrafficCameraDetailSec", "v1/poi/trafficEyesDetailSec");
    pub const LOAD_TRAFFIC_TICKET_LIST_SEC: Endpoint =
        Endpoint::post_encrypted("loadTrafficTicketListSec", "v1/poi/trafficTicketListSec");
    pub const MAP_SHOW_AREA_SEC: Endpoint =
        Endpoint::post_encrypted("mapShowAreaSec", "v1/poi/mapShowAreaSec");
    pub const SEARCH_POI_SEC: Endpoint =
        Endpoint::post_encrypted("searchPoiSec", "v1/poi/searchPoiSec");
    pub const SEARCH_RULE_POI_SEC: Endpoint =
        Endpoint::post_encrypted("searchRulePoiSec", "v1/poi/searchRulePoiSec");
    pub const SEARCH_SHOP_SEC: Endpoint =
        Endpoint::post_encrypted("searchShopSec", "v1/poi/searchShopSec");
    pub const ADD_TRAFFIC_EYES: Endpoint =
        Endpoint::post("addTrafficEyes", "v2/poi/addTrafficEyes");
    pub const ADD_EYES_CHECK: Endpoint = Endpoint::post("addEyesCheck", "v2/poi/addEyesCheck");
    pub const UPDATE_TRAFFIC_EYES: Endpoint =
        Endpoint::post("updateTrafficEyes", "v1/poi/updateTrafficEyes");
    pub const ADD_USER_AVOID: Endpoint = Endpoint::post("addUserAvoid", "v1/poi/addUserAvoid");
    pub const UPDATE_USER_AVOID: Endpoint =
        Endpoint::post("updateUserAvoid", "v1/poi/updateUserAvoid");
    pub const DELETE_USER_AVOID: Endpoint =
        Endpoint::post("deleteUserAvoid", "v1/poi/deleteUserAvoid");
    pub const GET_USER_AVOID_LIST: Endpoint =
        Endpoint::get("getUserAvoidList", "v1/poi/user/userAvoidList");
    pub const GET_USER_AVOID_BY_MAP: Endpoint =
        Endpoint::get("getUserAvoidByMap", "v1/poi/getUserAvoidByMap");
    pub const GET_USER_AVOID_DETAIL: Endpoint =
        Endpoint::get("getUserAvoidDetail", "v1/poi/userAvoidDetail");
    pub const ADD_AVOID_WHITE: Endpoint = Endpoint::post("addAvoidWhite", "v1/poi/addAvoidWhite");
    pub const UPDATE_AVOID_WHITE: Endpoint =
        Endpoint::post("updateAvoidWhite", "v1/poi/updateAvoidWhite");
    pub const DELETE_AVOID_WHITE: Endpoint =
        Endpoint::post("deleteAvoidWhite", "v1/poi/deleteAvoidWhite");
    pub const GET_AVOID_WHITE_LIST: Endpoint =
        Endpoint::get("getAvoidWhiteList", "v1/poi/user/avoidWhiteList");
    pub const USER_EYES_LIST: Endpoint = Endpoint::get("userEyesList", "v2/poi/user/eyesList");
    pub const USER_SHOP_LIST: Endpoint = Endpoint::get("userShopList", "v2/poi/user/shopList");
    pub const USER_GAS_LIST: Endpoint = Endpoint::get("userGasList", "v2/poi/user/gasList");
    pub const GET_NAVIGATION_STATUS: Endpoint =
        Endpoint::get("getNavigationStatus", "v2/poi/getNavigationStatus");
    pub const GET_ALL_CITY_TAB: Endpoint = Endpoint::get("getAllCityTab", "v2/poi/getAllCityTab");
    pub const GET_TAB_TYPE_MAP_ICON: Endpoint =
        Endpoint::get("getTabTypeMapIcon", "v2/poi/getTabTypeMapIcon");
    pub const RECENT_TICKETS_STATUS: Endpoint =
        Endpoint::post("recentTicketsStatus", "v1/poi/recentTicketsStatus");
    pub const NEARBY_SHOP_SEARCH: Endpoint =
        Endpoint::post("nearbyShopSearch", "v1/poi/nearbyShopSearch");
    pub const NEARBY_SHOP_MAP_SEARCH: Endpoint =
        Endpoint::post("nearbyShopMapSearch", "v1/poi/nearbyShopMapSearch");

    // User / vehicle / VIP
    pub const LOGIN: Endpoint = Endpoint::post("login", "v4/user/login");
    pub const REGISTER: Endpoint = Endpoint::post("register", "v2/user/register");
    pub const LOGOUT: Endpoint = Endpoint::post("logout", "v4/user/logout");
    pub const GET_USER_INFO_SEC: Endpoint =
        Endpoint::post_encrypted("getUserInfoSec", "v1/user/infoSec");
    pub const GET_USER_CAR_LIST_SEC: Endpoint =
        Endpoint::post_encrypted("getUserCarListSec", "v1/user/userCarListSec");
    pub const ADD_USER_CAR: Endpoint = Endpoint::post("addUserCar", "v1/user/addUserCar");
    pub const UPDATE_USER_CAR: Endpoint = Endpoint::post("updateUserCar", "v1/user/updateUserCar");
    pub const DEL_USER_CAR: Endpoint = Endpoint::post("delUserCar", "v1/user/delUserCar");
    pub const SET_DEFAULT_CAR: Endpoint = Endpoint::post("setDefaultCar", "v1/user/setDefaultCar");
    pub const VISITOR_TOKEN: Endpoint = Endpoint::get("visitorToken", "v3/user/vistorToken");
    pub const FAST_LOGIN_TOKEN: Endpoint =
        Endpoint::get("fastLoginToken", "v1/user/fastLoginToken");
    pub const GET_LEVEL_INFO: Endpoint = Endpoint::get("getLevelInfo", "v1/user/levelInfo");
    pub const BUY_VIP_PRODUCT: Endpoint = Endpoint::post("buyVipProduct", "v2/mem/buy/product");
    pub const GET_VIP_PRODUCTS: Endpoint =
        Endpoint::post("getVipProducts", "v3/mem/query/product/list");
    pub const BLACK_LIST: Endpoint = Endpoint::get("blackList", "v1/user/blackList");
}

/// All endpoints in the recovered client, for JNI/UI lookup.
pub const ALL_ENDPOINTS: &[Endpoint] = &[
    endpoints::CAL_PATH_AREA_SEC,
    endpoints::CAL_PATH_AVOID_SEC,
    endpoints::CAL_PATH_QUICK,
    endpoints::LOAD_CAMERA_TAB_TYPE,
    endpoints::LOAD_CAMERA_TAB_TYPE_GROUP,
    endpoints::LOAD_CITY_TAB_LIST,
    endpoints::LOAD_POI_DETAIL_SEC,
    endpoints::LOAD_POI_SHOP_DETAIL_SEC,
    endpoints::LOAD_TRAFFIC_CAMERA_DETAIL_SEC,
    endpoints::LOAD_TRAFFIC_TICKET_LIST_SEC,
    endpoints::MAP_SHOW_AREA_SEC,
    endpoints::SEARCH_POI_SEC,
    endpoints::SEARCH_RULE_POI_SEC,
    endpoints::SEARCH_SHOP_SEC,
    endpoints::ADD_TRAFFIC_EYES,
    endpoints::ADD_EYES_CHECK,
    endpoints::UPDATE_TRAFFIC_EYES,
    endpoints::ADD_USER_AVOID,
    endpoints::UPDATE_USER_AVOID,
    endpoints::DELETE_USER_AVOID,
    endpoints::GET_USER_AVOID_LIST,
    endpoints::GET_USER_AVOID_BY_MAP,
    endpoints::GET_USER_AVOID_DETAIL,
    endpoints::ADD_AVOID_WHITE,
    endpoints::UPDATE_AVOID_WHITE,
    endpoints::DELETE_AVOID_WHITE,
    endpoints::GET_AVOID_WHITE_LIST,
    endpoints::USER_EYES_LIST,
    endpoints::USER_SHOP_LIST,
    endpoints::USER_GAS_LIST,
    endpoints::GET_NAVIGATION_STATUS,
    endpoints::GET_ALL_CITY_TAB,
    endpoints::GET_TAB_TYPE_MAP_ICON,
    endpoints::RECENT_TICKETS_STATUS,
    endpoints::NEARBY_SHOP_SEARCH,
    endpoints::NEARBY_SHOP_MAP_SEARCH,
    endpoints::LOGIN,
    endpoints::REGISTER,
    endpoints::LOGOUT,
    endpoints::GET_USER_INFO_SEC,
    endpoints::GET_USER_CAR_LIST_SEC,
    endpoints::ADD_USER_CAR,
    endpoints::UPDATE_USER_CAR,
    endpoints::DEL_USER_CAR,
    endpoints::SET_DEFAULT_CAR,
    endpoints::VISITOR_TOKEN,
    endpoints::FAST_LOGIN_TOKEN,
    endpoints::GET_LEVEL_INFO,
    endpoints::BUY_VIP_PRODUCT,
    endpoints::GET_VIP_PRODUCTS,
    endpoints::BLACK_LIST,
];

/// Look up an endpoint by its recovered Java method name.
#[must_use]
pub fn endpoint_by_name(name: &str) -> Option<Endpoint> {
    ALL_ENDPOINTS
        .iter()
        .copied()
        .find(|endpoint| endpoint.name == name)
}

#[must_use]
fn is_excluded_from_signing(path: &str) -> bool {
    EXCLUDED_FROM_SIGNING
        .iter()
        .any(|excluded| path.contains(excluded))
}

/// Headers added by `AppendHeaderParamIntercepter`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderInit {
    pub token: Option<String>,
    pub device_id: Option<String>,
    pub safe_env_token: Option<String>,
}

#[must_use]
pub fn base_headers(init: &HeaderInit) -> Vec<(String, String)> {
    let mut headers = vec![
        (HEADER_VERSION.to_owned(), crypto::APP_VERSION.to_owned()),
        (HEADER_PLATFORM.to_owned(), crypto::PLATFORM.to_owned()),
    ];
    if let Some(token) = init.token.as_deref().filter(|value| !value.is_empty()) {
        headers.push(("token".to_owned(), token.to_owned()));
    }
    if let Some(device_id) = init.device_id.as_deref().filter(|value| !value.is_empty()) {
        headers.push(("deviceId".to_owned(), device_id.to_owned()));
    }
    if let Some(token) = init
        .safe_env_token
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        headers.push(("safe-env-token".to_owned(), token.to_owned()));
    }
    headers
}

/// A fully prepared HTTP request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Serialised JSON body, present for POST.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

impl SignedRequest {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Percent-encode a query component (RFC 3986 unreserved set).
#[must_use]
pub fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(*byte));
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// Build a signed request for an endpoint.
///
/// `query` is used for GET; `body` for POST. For encrypted endpoints the
/// `Security-Level: Encrypt` header is set and the signed body is left for
/// [`SecurityClient`] to encrypt.
pub fn build_request(
    endpoint: &Endpoint,
    base_url: &str,
    headers: &[(String, String)],
    query: &BTreeMap<String, String>,
    body: Option<Value>,
    timestamp_ms: i64,
) -> Result<SignedRequest> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/{}", endpoint.path.trim_start_matches('/'));
    let mut headers = headers.to_vec();
    if endpoint.encrypted {
        headers.push((
            HEADER_SECURITY_LEVEL.to_owned(),
            SECURITY_LEVEL_ENCRYPT.to_owned(),
        ));
    }
    if is_excluded_from_signing(endpoint.path) {
        return Ok(SignedRequest {
            method: endpoint.method,
            url: finish_get_url(&url, query, None),
            headers,
            body: body.map(|value| value.to_string()),
        });
    }

    match endpoint.method {
        HttpMethod::Get => {
            let mut signable: Map<String, Value> = query
                .iter()
                .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                .collect();
            signable.insert(
                "timeStamp".to_owned(),
                Value::String(timestamp_ms.to_string()),
            );
            let sign = crypto::md5_hex(&crypto::canonical_string(&signable));
            let mut with_timestamp = query.clone();
            with_timestamp.insert("timeStamp".to_owned(), timestamp_ms.to_string());
            Ok(SignedRequest {
                method: HttpMethod::Get,
                url: finish_get_url(&url, &with_timestamp, Some(&sign)),
                headers,
                body: None,
            })
        }
        HttpMethod::Post => {
            let body = body.unwrap_or_else(|| Value::Object(Map::new()));
            let (signed, _sign) = sign_json_body(&body, timestamp_ms)?;
            headers.push((
                "Content-Type".to_owned(),
                "application/json; charset=utf-8".to_owned(),
            ));
            Ok(SignedRequest {
                method: HttpMethod::Post,
                url,
                headers,
                body: Some(signed.to_string()),
            })
        }
    }
}

fn finish_get_url(base: &str, query: &BTreeMap<String, String>, sign: Option<&str>) -> String {
    let mut url = base.to_owned();
    let mut first = !base.contains('?');
    let append = |key: &str, value: &str, url: &mut String, first: &mut bool| {
        url.push(if *first { '?' } else { '&' });
        *first = false;
        url.push_str(&percent_encode(key));
        url.push('=');
        url.push_str(&percent_encode(value));
    };
    for (key, value) in query {
        append(key, value, &mut url, &mut first);
    }
    if let Some(sign) = sign {
        append("sign", sign, &mut url, &mut first);
    }
    url
}

/// Raw response from a [`Transport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    #[must_use]
    pub fn json(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
        }
    }

    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone())
            .map_err(|error| MotuError::Transport(format!("response is not UTF-8: {error}")))
    }
}

/// Minimal transport port so tests and the JNI layer can use different clients.
pub trait Transport {
    fn execute(&self, request: &SignedRequest) -> Result<HttpResponse>;
}

/// Decode a plain (unencrypted) `ApiResultResponse<T>`.
pub fn decode_envelope<T: DeserializeOwned>(response: &HttpResponse) -> Result<ApiEnvelope<T>> {
    if !response.is_success() {
        return Err(MotuError::Api(format!("HTTP {}", response.status)));
    }
    Ok(serde_json::from_slice(&response.body)?)
}

/// Blocking `reqwest` transport, enabled with the `http` feature.
#[cfg(feature = "http")]
#[derive(Debug, Clone, Default)]
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

#[cfg(feature = "http")]
impl ReqwestTransport {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|error| MotuError::Transport(error.to_string()))?;
        Ok(Self { client })
    }
}

#[cfg(feature = "http")]
impl Transport for ReqwestTransport {
    fn execute(&self, request: &SignedRequest) -> Result<HttpResponse> {
        let method = match request.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
        };
        let mut builder = self.client.request(method, &request.url);
        for (key, value) in &request.headers {
            builder = builder.header(key, value);
        }
        if let Some(body) = &request.body {
            builder = builder.body(body.clone());
        }
        let response = builder
            .send()
            .map_err(|error| MotuError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .map_err(|error| MotuError::Transport(error.to_string()))?
            .to_vec();
        Ok(HttpResponse { status, body })
    }
}

/// Client for the encrypted protocol (`Security-Level: Encrypt`).
#[derive(Debug, Clone)]
pub struct SecurityClient {
    config: SecurityConfig,
    aad: Aad,
}

impl SecurityClient {
    #[must_use]
    pub const fn new(config: SecurityConfig, aad: Aad) -> Self {
        Self { config, aad }
    }

    #[must_use]
    pub const fn config(&self) -> &SecurityConfig {
        &self.config
    }

    /// Fetch `/api/security/public-key` (no signature required).
    pub fn fetch_config<T: Transport>(
        transport: &T,
        base_url: &str,
        headers: &[(String, String)],
        now_ms: i64,
    ) -> Result<Self> {
        let url = format!(
            "{}/{}",
            base_url.trim_end_matches('/'),
            PUBLIC_KEY_PATH.trim_start_matches('/')
        );
        let request = SignedRequest {
            method: HttpMethod::Get,
            url,
            headers: headers.to_vec(),
            body: None,
        };
        let response = transport.execute(&request)?;
        if !response.is_success() {
            return Err(MotuError::Api(format!(
                "public-key bootstrap returned HTTP {}",
                response.status
            )));
        }
        let value: Value = serde_json::from_slice(&response.body)?;
        let data = value
            .get("data")
            .ok_or_else(|| MotuError::Api("public-key response has no data".to_owned()))?;
        let read = |name: &str| {
            data.get(name)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| MotuError::Api(format!("public-key response missing {name}")))
        };
        let config = SecurityConfig {
            key_id: read("keyId")?,
            public_key: read("publicKey")?,
            salt: read("salt")?,
            fetched_at_ms: now_ms,
            ttl_ms: crypto::SECURITY_CONFIG_TTL_MS,
        };
        Ok(Self::new(config, Aad::SaltUtf8))
    }

    /// POST a JSON body to an encrypted endpoint and return the decrypted body.
    ///
    /// The plaintext that gets encrypted is the signed body (with `timeStamp`
    /// and `sign`), matching the original interceptor order.
    pub fn post_encrypted<T: Transport>(
        &self,
        transport: &T,
        endpoint: &Endpoint,
        base_url: &str,
        headers: &[(String, String)],
        body: &Value,
        now: (i64, i64),
    ) -> Result<Vec<u8>> {
        if !endpoint.encrypted {
            return Err(MotuError::Api(format!(
                "{} is not an encrypted endpoint",
                endpoint.name
            )));
        }
        let (timestamp_millis, timestamp_seconds) = now;
        let (signed, _sign) = sign_json_body(body, timestamp_millis)?;
        let plaintext = signed.to_string();
        let nonce = crypto::generate_nonce();
        let (envelope, key) = build_envelope_start(
            &self.config,
            &plaintext,
            timestamp_seconds,
            &nonce,
            &self.aad,
        )?;
        let request = SignedRequest {
            method: HttpMethod::Post,
            url: format!(
                "{}/{}",
                base_url.trim_end_matches('/'),
                endpoint.path.trim_start_matches('/')
            ),
            headers: {
                let mut headers = headers.to_vec();
                headers.push((
                    "Content-Type".to_owned(),
                    "application/json; charset=utf-8".to_owned(),
                ));
                headers
            },
            body: Some(serde_json::to_string(&envelope)?),
        };
        let response = transport.execute(&request)?;
        if !response.is_success() {
            return Err(MotuError::Api(format!("HTTP {}", response.status)));
        }
        let value: Value = serde_json::from_slice(&response.body)?;
        if value.get("data").is_some() && value.get("sign").is_some() {
            let security: SecurityResponse = serde_json::from_value(value)?;
            let plaintext = open_response(&security, &key, &self.config.salt, &self.aad)?;
            Ok(plaintext.into_bytes())
        } else {
            Ok(response.body)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Aad, aes_gcm_decrypt, encrypt_md5, hmac_sha256_b64};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use rsa::Oaep;
    use rsa::RsaPrivateKey;
    use rsa::pkcs8::EncodePublicKey;
    use std::sync::{Arc, Mutex};

    #[test]
    fn get_request_is_signed_and_percent_encoded() {
        let headers = base_headers(&HeaderInit {
            token: Some("t".to_owned()),
            device_id: Some("d".to_owned()),
            safe_env_token: None,
        });
        let mut query = BTreeMap::new();
        query.insert("cityCode".to_owned(), "310000".to_owned());
        let request = build_request(
            &endpoints::LOAD_CITY_TAB_LIST,
            BASE_URL,
            &headers,
            &query,
            None,
            1_700_000_000_000,
        )
        .unwrap();
        assert_eq!(request.method, HttpMethod::Get);
        assert!(
            request
                .url
                .starts_with("https://motu.motumap.com/v2/poi/getTab?")
        );
        assert!(request.url.contains("cityCode=310000"));
        assert!(request.url.contains("timeStamp=1700000000000"));
        assert!(request.url.contains("&sign="));
        assert_eq!(request.header("version"), Some("4.1.9"));
        assert_eq!(request.header("platForm"), Some("2"));
    }

    #[test]
    fn excluded_endpoint_is_not_signed() {
        let endpoint = Endpoint::get("excluded", "poi/getPoiTabs");
        let request = build_request(&endpoint, BASE_URL, &[], &BTreeMap::new(), None, 0).unwrap();
        assert!(!request.url.contains("sign="));
        assert!(!request.url.contains("timeStamp="));
    }

    #[test]
    fn post_request_carries_signed_json() {
        let request = build_request(
            &endpoints::CAL_PATH_AVOID_SEC,
            BASE_URL,
            &[],
            &BTreeMap::new(),
            Some(serde_json::json!({"plateVal": "沪A12345"})),
            1_700_000_000_000,
        )
        .unwrap();
        assert_eq!(request.header(HEADER_SECURITY_LEVEL), Some("Encrypt"));
        let body: Value = serde_json::from_str(request.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["plateVal"], "沪A12345");
        assert!(body["sign"].is_string());
        assert_eq!(body["timeStamp"], "1700000000000");
    }

    #[test]
    fn plain_envelope_decodes() {
        let response = HttpResponse::json(200, r#"{"code":0,"data":{"id":1}}"#);
        let decoded: ApiEnvelope<Value> = decode_envelope(&response).unwrap();
        assert!(decoded.is_success());
        assert_eq!(decoded.data.unwrap()["id"], 1);
    }

    /// Fake server that decrypts the envelope, echoes `{"ok":true}` encrypted,
    /// and signs the response with the request key.
    struct FakeServer {
        private: RsaPrivateKey,
        salt: String,
        observed: Arc<Mutex<Vec<SignedRequest>>>,
    }

    impl FakeServer {
        fn new() -> Self {
            let mut rng = rand::thread_rng();
            Self {
                private: RsaPrivateKey::new(&mut rng, 2048).unwrap(),
                salt: "server-salt".to_owned(),
                observed: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn public_spki_b64(&self) -> String {
            let spki = self.private.to_public_key().to_public_key_der().unwrap();
            BASE64.encode(spki.as_bytes())
        }
    }

    impl Transport for FakeServer {
        fn execute(&self, request: &SignedRequest) -> Result<HttpResponse> {
            self.observed.lock().unwrap().push(request.clone());
            if request.url.ends_with(PUBLIC_KEY_PATH) {
                let body = serde_json::json!({
                    "code": 0,
                    "data": {
                        "keyId": "kid-1",
                        "publicKey": self.public_spki_b64(),
                        "salt": self.salt,
                    }
                });
                return Ok(HttpResponse::json(200, body.to_string()));
            }
            let envelope: Value = serde_json::from_str(request.body.as_deref().unwrap())?;
            let key_b64 = envelope["encryptedKey"].as_str().unwrap();
            let encrypted_key = BASE64.decode(key_b64).unwrap();
            let key = self
                .private
                .decrypt(
                    Oaep::new_with_mgf_hash::<sha2::Sha256, sha1::Sha1>(),
                    &encrypted_key,
                )
                .unwrap();
            let key_b64 = String::from_utf8(key).unwrap();
            let iv = envelope["iv"].as_str().unwrap();
            let data = envelope["data"].as_str().unwrap();
            let plaintext = aes_gcm_decrypt(data, &key_b64, iv, &self.salt, &Aad::SaltUtf8)?;
            assert!(plaintext.contains("\"sign\""));

            let response_data = aes_gcm_encrypt_for_test("{\"code\":0}", &key_b64, iv, &self.salt);
            let timestamp = 1_700_000_000_i64;
            let nonce = "resp-nonce";
            let sign = hmac_sha256_b64(&format!("{timestamp}\n{nonce}\n{response_data}"), &key_b64)
                .unwrap();
            let body = serde_json::json!({
                "timestamp": timestamp,
                "nonce": nonce,
                "iv": iv,
                "data": response_data,
                "sign": sign,
            });
            Ok(HttpResponse::json(200, body.to_string()))
        }
    }

    fn aes_gcm_encrypt_for_test(
        plaintext: &str,
        key_b64: &str,
        iv_b64: &str,
        salt: &str,
    ) -> String {
        crate::crypto::aes_gcm_encrypt(plaintext, key_b64, iv_b64, salt, &Aad::SaltUtf8).unwrap()
    }

    #[test]
    fn encrypted_round_trip_against_fake_server() {
        let server = FakeServer::new();
        let headers = base_headers(&HeaderInit::default());
        let client =
            SecurityClient::fetch_config(&server, BASE_URL, &headers, 1_700_000_000_000).unwrap();
        assert_eq!(client.config().key_id, "kid-1");

        let plaintext = client
            .post_encrypted(
                &server,
                &endpoints::LOAD_POI_DETAIL_SEC,
                BASE_URL,
                &headers,
                &serde_json::json!({"id": 42}),
                (1_700_000_000_000, 1_700_000_000),
            )
            .unwrap();
        assert_eq!(String::from_utf8(plaintext).unwrap(), "{\"code\":0}");
        assert_eq!(server.observed.lock().unwrap().len(), 2);
    }

    #[test]
    fn signature_matches_manual_canonical_string() {
        let mut params = Map::new();
        params.insert("b".to_owned(), Value::String("2".to_owned()));
        params.insert("A".to_owned(), Value::String("1".to_owned()));
        let expected = encrypt_md5(&format!("A=1&b=2&key={}", crypto::SIGN_SECRET));
        assert_eq!(
            crypto::encrypt_md5(&crypto::canonical_string(&params)),
            expected
        );
    }

    #[test]
    fn public_config_rejects_incomplete_payload() {
        struct Incomplete;
        impl Transport for Incomplete {
            fn execute(&self, _request: &SignedRequest) -> Result<HttpResponse> {
                Ok(HttpResponse::json(
                    200,
                    r#"{"code":0,"data":{"keyId":"k"}}"#,
                ))
            }
        }
        assert!(SecurityClient::fetch_config(&Incomplete, BASE_URL, &[], 0).is_err());
    }

    #[test]
    fn percent_encoding_handles_chinese_and_spaces() {
        assert_eq!(percent_encode("沪"), "%E6%B2%AA");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("-_.~"), "-_.~");
    }
}
