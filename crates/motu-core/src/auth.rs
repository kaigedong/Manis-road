//! Login, logout and session-aware request helpers.
//!
//! The password is hashed in Rust with the Java-compatible lowercase
//! `MD5Utils.md5PasswordTo32` rule, then the request goes through the normal
//! signing interceptor (uppercase native MD5).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    // Parse the envelope as `Value` first so a shape mismatch can be reported
    // with a redacted payload instead of a bare serde line/column error.
    let envelope: ApiEnvelope<Value> = decode_envelope(&response)?;
    if !envelope.is_success() {
        let code = envelope.code;
        let message = envelope
            .message
            .unwrap_or_else(|| format!("login failed with code {code}"));
        return Err(MotuError::Server(message));
    }
    let data = envelope
        .data
        .ok_or_else(|| MotuError::Api("login response has no data".to_owned()))?;
    let user: LoginUser = serde_json::from_value(data.clone()).map_err(|error| {
        MotuError::Api(format!(
            "cannot parse login payload: {error}; payload={}",
            redact_payload(&data)
        ))
    })?;
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

/// Serialise a payload for an error message, masking credentials and PII.
fn redact_payload(data: &Value) -> String {
    let mut value = data.clone();
    if let Some(object) = value.as_object_mut() {
        for key in [
            "token",
            "refreshToken",
            "pwd",
            "phone",
            "realName",
            "nickName",
            "contactWay",
            "hldSn",
        ] {
            if object.contains_key(key) {
                object.insert(key.to_owned(), Value::String("***".to_owned()));
            }
        }
    }
    let text = serde_json::to_string(&value).unwrap_or_default();
    text.chars().take(1500).collect()
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
    fn login_accepts_formatted_dates_in_timestamps() {
        let transport = FakeTransport::new(
            r#"{"code":0,"message":"ok","data":{"token":"T","userId":7,"phone":"13800000000",
                "memEndTime":"2026-05-09","memStartTime":"2025-05-09 10:00:00",
                "createTime":"2024-01-02"}}"#,
        );
        let outcome = login(
            &transport,
            "https://example.test/",
            "device-1",
            "",
            "13800000000",
            "secret",
            1_700_000_000_000,
        )
        .unwrap();
        assert_eq!(outcome.user.mem_end_time, Some(1_778_256_000_000));
        assert_eq!(outcome.session.mem_end_time_ms, Some(1_778_256_000_000));
        assert_eq!(outcome.user.mem_start_time, Some(1_746_756_000_000));
        assert_eq!(outcome.user.create_time, Some(1_704_124_800_000));
    }

    #[test]
    fn login_accepts_the_live_payload_shape_with_nullable_fields() {
        // Trimmed copy of a real `v4/user/login` response: nullable counters,
        // a date-only `createTime`, and a `refreshToken`.
        let transport = FakeTransport::new(
            r#"{"code":0,"message":"ok","data":{
                "address":"","createTime":"2026-05-09","cruiseCount":3,"expCount":3,
                "fansCount":null,"followCount":null,"hasPwd":true,
                "isPro":0,"memEndTime":null,"memStartTime":null,
                "nickName":"motu","phone":"13800000000","photo":"",
                "refreshToken":"refresh-token","sex":null,"token":"T",
                "type":1,"userId":527965,"weChatBind":false}}"#,
        );
        let outcome = login(
            &transport,
            "https://example.test/",
            "device-1",
            "",
            "13800000000",
            "secret",
            1_700_000_000_000,
        )
        .unwrap();
        assert_eq!(outcome.user.user_id, Some(527_965));
        assert_eq!(outcome.user.create_time, Some(1_778_256_000_000));
        assert_eq!(outcome.user.sex, None);
        assert_eq!(outcome.user.exp_count, 3);
        assert_eq!(outcome.user.refresh_token.as_deref(), Some("refresh-token"));
    }

    #[test]
    fn login_reports_redacted_payload_when_a_field_cannot_be_parsed() {
        let transport = FakeTransport::new(
            r#"{"code":0,"message":"ok","data":{"token":"secret-token","phone":"13800000000","memEndTime":"someday"}}"#,
        );
        let error = login(
            &transport,
            "https://example.test/",
            "",
            "",
            "13800000000",
            "secret",
            1_700_000_000_000,
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("memEndTime"));
        assert!(message.contains("someday"));
        assert!(!message.contains("secret-token"));
        assert!(!message.contains("13800000000"));
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
