//! Login, logout and session-aware request helpers.
//!
//! The password is hashed in Rust with the Java-compatible lowercase
//! `MD5Utils.md5PasswordTo32` rule, then the request goes through the normal
//! signing interceptor (uppercase native MD5).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::api::{HeaderInit, Transport, base_headers, build_request, decode_envelope, endpoints};
use crate::crypto::{md5_hex, safe_env_token};
use crate::error::{MotuError, Result};
use crate::model::{ApiEnvelope, LoginUser};
use crate::store::AuthSession;

/// Successful password login, carrying both the persisted session and the
/// full server user payload for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginOutcome {
    pub session: AuthSession,
    pub user: LoginUser,
}

/// `POST v4/user/login` with the password hashed exactly like `MD5Utils`.
pub fn login<T: Transport>(
    transport: &T,
    base_url: &str,
    device_id: &str,
    signature_md5: &str,
    phone: &str,
    password: &str,
    now_ms: i64,
) -> Result<LoginOutcome> {
    let phone = phone.trim();
    if phone.is_empty() || password.is_empty() {
        return Err(MotuError::Api("phone and password are required".to_owned()));
    }

    let headers = login_headers(device_id, signature_md5, now_ms);
    let body = serde_json::json!({
        "phone": phone,
        "pwd": md5_hex(password),
        "type": 0,
    });
    let request = build_request(
        &endpoints::LOGIN,
        base_url,
        &headers,
        &BTreeMap::new(),
        Some(body),
        now_ms,
    )?;
    let response = transport.execute(&request)?;
    let envelope: ApiEnvelope<LoginUser> = decode_envelope(&response)?;
    if !envelope.is_success() {
        let code = envelope.code;
        let message = envelope
            .message
            .unwrap_or_else(|| format!("login failed with code {code}"));
        return Err(MotuError::Server(message));
    }
    let user = envelope
        .data
        .ok_or_else(|| MotuError::Api("login response has no data".to_owned()))?;
    if user.token.is_empty() {
        return Err(MotuError::Api("login response has no token".to_owned()));
    }
    let session = AuthSession::from_login(&user, now_ms);
    Ok(LoginOutcome { session, user })
}

/// Best-effort `POST v4/user/logout`. Local session clearing is the caller's
/// responsibility, so a failed network call does not keep a stale token.
pub fn logout<T: Transport>(
    transport: &T,
    base_url: &str,
    session: &AuthSession,
    now_ms: i64,
) -> Result<()> {
    if !session.is_logged_in() {
        return Ok(());
    }
    let headers = base_headers(&HeaderInit {
        token: Some(session.token.clone()),
        device_id: None,
        safe_env_token: None,
    });
    let request = build_request(
        &endpoints::LOGOUT,
        base_url,
        &headers,
        &BTreeMap::new(),
        None,
        now_ms,
    )?;
    let response = transport.execute(&request)?;
    let envelope: ApiEnvelope<serde_json::Value> = decode_envelope(&response)?;
    if !envelope.is_success() {
        return Err(MotuError::Server(
            envelope
                .message
                .unwrap_or_else(|| "logout failed".to_owned()),
        ));
    }
    Ok(())
}

fn login_headers(device_id: &str, signature_md5: &str, now_ms: i64) -> Vec<(String, String)> {
    let device_id = device_id.trim();
    let safe_env_token = if device_id.is_empty() || signature_md5.trim().is_empty() {
        None
    } else {
        let seconds = u64::try_from(now_ms / 1000).unwrap_or(0);
        Some(safe_env_token(device_id, seconds, signature_md5))
    };
    base_headers(&HeaderInit {
        token: None,
        device_id: (!device_id.is_empty()).then(|| device_id.to_owned()),
        safe_env_token,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::api::{HttpResponse, SignedRequest};

    #[derive(Clone)]
    struct FakeTransport {
        response: String,
        observed: Arc<Mutex<Vec<SignedRequest>>>,
    }

    impl FakeTransport {
        fn new(response: impl Into<String>) -> Self {
            Self {
                response: response.into(),
                observed: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl Transport for FakeTransport {
        fn execute(&self, request: &SignedRequest) -> Result<HttpResponse> {
            self.observed.lock().unwrap().push(request.clone());
            Ok(HttpResponse::json(200, self.response.clone()))
        }
    }

    #[test]
    fn login_hashes_password_and_signs_with_uppercase_md5() {
        let transport = FakeTransport::new(
            r#"{"code":0,"message":"ok","data":{"token":"T","userId":7,"phone":"13800000000","nickName":"motu","isPro":1,"memEndTime":1800000000000}}"#,
        );
        let outcome = login(
            &transport,
            "https://example.test/",
            "device-1",
            "CFFFCB2B9E968B9619C749EE1FBF19D7",
            "13800000000",
            "secret",
            1_700_000_000_000,
        )
        .unwrap();
        assert_eq!(outcome.session.token, "T");
        assert_eq!(outcome.session.user_id, Some(7));
        assert!(outcome.session.is_pro);
        assert_eq!(outcome.session.mem_end_time_ms, Some(1_800_000_000_000));

        let observed = transport.observed.lock().unwrap();
        let request = &observed[0];
        let body: serde_json::Value =
            serde_json::from_str(request.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["phone"], "13800000000");
        assert_eq!(body["pwd"], md5_hex("secret"));
        assert_eq!(body["type"], 0);
        assert_eq!(body["timeStamp"], "1700000000000");
        assert_eq!(
            body["sign"].as_str().unwrap(),
            body["sign"].as_str().unwrap().to_ascii_uppercase()
        );
        assert!(
            request
                .headers
                .iter()
                .any(|(key, value)| key == "deviceId" && value == "device-1")
        );
    }

    #[test]
    fn login_surfaces_server_error_message() {
        let transport = FakeTransport::new(r#"{"code":20106,"message":"sign非法","data":null}"#);
        let error = login(
            &transport,
            "https://example.test/",
            "",
            "",
            "10000000000",
            "secret",
            1_700_000_000_000,
        )
        .unwrap_err();
        assert!(error.to_string().contains("sign非法"));
    }

    #[test]
    fn logout_requires_a_token_and_sends_it_in_header() {
        let transport = FakeTransport::new(r#"{"code":0,"message":"ok","data":null}"#);
        let session = AuthSession {
            token: "T".to_owned(),
            ..AuthSession::default()
        };
        logout(
            &transport,
            "https://example.test/",
            &session,
            1_700_000_000_000,
        )
        .unwrap();
        let observed = transport.observed.lock().unwrap();
        assert!(
            observed[0]
                .headers
                .iter()
                .any(|(key, value)| key == "token" && value == "T")
        );
    }
}
