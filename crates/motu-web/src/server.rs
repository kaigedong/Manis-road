//! Local HTTP API and static UI server.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, header},
    routing::{get, post},
};
use motu_core::api::ReqwestTransport;
use motu_core::{
    BASE_URL, CameraDetail, CoreState, FileStore, HeaderInit, LatLng, OfficialCameraQuery,
    PlateColor, auth, base_headers, default_tab_types, gcj02_to_wgs84, load_camera_detail,
    safe_env_token, sync_official_cameras, wgs84_to_gcj02,
};
use serde_json::{Value, json};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer, trace::TraceLayer};

use crate::{
    config::WebConfig,
    error::ApiError,
    model::{CameraSnapshot, DetailRequest, LoginRequest, SyncRequest, WebCamera},
};

#[derive(Clone)]
pub struct AppState {
    pub store: FileStore,
    pub config: Arc<WebConfig>,
    pub cameras: Arc<Mutex<CameraSnapshot>>,
}

pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/config", get(config_view))
        .route("/api/state", get(state_view))
        .route("/api/cameras", get(cameras))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/sync", post(sync))
        .route("/api/detail", post(detail))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .with_state(state);

    Router::new()
        .merge(api)
        .fallback_service(ServeDir::new(static_dir()).append_index_html_on_directories(true))
        .layer(TraceLayer::new_for_http())
}

async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "service": "motu-web",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn config_view(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "mapCenter": LatLng::new(31.230_4, 121.473_7),
        "maxZoom": 19,
        "tileUrl": state.config.tile_url,
        "tileAttribution": state.config.tile_attribution,
        "signatureConfigured": !state.config.signature_md5.is_empty(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn state_view(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let state = Arc::clone(&state);
    blocking(move || {
        let core = load_core(&state)?;
        let snapshot = snapshot(&state)?;
        Ok(Json(state_payload(&core, &state.config, &snapshot)))
    })
    .await
}

async fn cameras(State(state): State<Arc<AppState>>) -> Result<Json<CameraSnapshot>, ApiError> {
    let state = Arc::clone(&state);
    blocking(move || snapshot(&state).map(Json)).await
}

async fn login(
    State(state): State<Arc<AppState>>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<Value>, ApiError> {
    let phone = request.phone.trim().to_owned();
    let password = request.password;
    if phone.is_empty() || password.is_empty() {
        return Err(ApiError::bad_request("手机号和密码不能为空"));
    }

    let state = Arc::clone(&state);
    blocking(move || {
        let core = state.store.ensure_identity().map_err(|error| {
            ApiError::internal(format!("cannot initialise local store: {error}"))
        })?;
        let base_url = base_url(&core, &state.config);
        let transport = ReqwestTransport::new()
            .map_err(|error| ApiError::upstream(format!("cannot create HTTP client: {error}")))?;
        let now = now_ms();
        let outcome = auth::login(
            &transport,
            &base_url,
            &core.device_id,
            &state.config.signature_md5,
            &phone,
            &password,
            now,
        )
        .map_err(ApiError::from)?;
        let core = state
            .store
            .save_session(outcome.session)
            .map_err(|error| ApiError::internal(format!("cannot save login session: {error}")))?;
        let snapshot = reset_camera_cache(&state, &core)?;
        Ok(Json(json!({
            "loggedIn": true,
            "user": session_user(&core),
            "state": state_payload(&core, &state.config, &snapshot),
        })))
    })
    .await
}

async fn logout(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let state = Arc::clone(&state);
    blocking(move || {
        let core = load_core(&state)?;
        let mut warning = None;
        if core.session.is_logged_in() {
            let now = now_ms();
            match ReqwestTransport::new() {
                Ok(transport) => {
                    if let Err(error) = auth::logout(
                        &transport,
                        &base_url(&core, &state.config),
                        &core.session,
                        now,
                    ) {
                        warning = Some(error.to_string());
                    }
                }
                Err(error) => warning = Some(error.to_string()),
            }
        }
        let core = state
            .store
            .clear_session()
            .map_err(|error| ApiError::internal(format!("cannot clear login session: {error}")))?;
        let snapshot = reset_camera_cache(&state, &core)?;
        Ok(Json(json!({
            "loggedIn": false,
            "warning": warning,
            "state": state_payload(&core, &state.config, &snapshot),
        })))
    })
    .await
}

async fn sync(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SyncRequest>,
) -> Result<Json<Value>, ApiError> {
    let map_center = LatLng::new(request.latitude, request.longitude);
    if !map_center.is_valid() {
        return Err(ApiError::bad_request("地图中心坐标无效"));
    }

    let state = Arc::clone(&state);
    blocking(move || {
        let core = load_core(&state)?;
        require_session(&core)?;
        let city = request
            .city_code
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                let configured = core.settings.city_code.trim();
                if configured.is_empty() {
                    "010".to_owned()
                } else {
                    configured.to_owned()
                }
            });
        let vehicle = core.default_vehicle();
        let plate_color = vehicle.map_or_else(
            || "YELLOW".to_owned(),
            |vehicle| {
                let wire = PlateColor::from_id(vehicle.plate_color).as_wire_str();
                if wire.is_empty() {
                    "YELLOW".to_owned()
                } else {
                    wire.to_owned()
                }
            },
        );
        let plate_val = vehicle.map_or_else(String::new, |vehicle| vehicle.number_plate.clone());
        let query = OfficialCameraQuery {
            position: wgs84_to_gcj02(map_center),
            city_code: city.clone(),
            to_city_code: city.clone(),
            tab_types: default_tab_types(),
            plate_color,
            plate_val,
            cruise: false,
            source: 1,
        };
        let now = now_ms();
        let headers = auth_headers(&core, &state.config, now);
        let transport = ReqwestTransport::new()
            .map_err(|error| ApiError::upstream(format!("cannot create HTTP client: {error}")))?;
        let sync = sync_official_cameras(
            &transport,
            &base_url(&core, &state.config),
            &headers,
            &query,
            now,
        )
        .map_err(ApiError::from)?;
        let snapshot = CameraSnapshot {
            source: "official".to_owned(),
            city_code: if sync.city_code.trim().is_empty() {
                city.clone()
            } else {
                sync.city_code
            },
            fetched_at_ms: Some(sync.fetched_at_ms),
            cameras: sync.cameras.iter().map(WebCamera::from_official).collect(),
        };
        let core = state
            .store
            .update(|state| state.settings.city_code = city)
            .map_err(|error| ApiError::internal(format!("cannot save city code: {error}")))?;
        {
            let mut cache = state
                .cameras
                .lock()
                .map_err(|_| ApiError::internal("camera cache lock is poisoned"))?;
            *cache = snapshot.clone();
        }
        Ok(Json(json!({
            "snapshot": snapshot,
            "state": state_payload(&core, &state.config, &snapshot),
        })))
    })
    .await
}

async fn detail(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DetailRequest>,
) -> Result<Json<CameraDetail>, ApiError> {
    let camera_id = request.camera_id.trim().to_owned();
    if camera_id.is_empty() {
        return Err(ApiError::bad_request("cameraId 不能为空"));
    }

    let state = Arc::clone(&state);
    blocking(move || {
        let core = load_core(&state)?;
        require_session(&core)?;
        let now = now_ms();
        let headers = auth_headers(&core, &state.config, now);
        let transport = ReqwestTransport::new()
            .map_err(|error| ApiError::upstream(format!("cannot create HTTP client: {error}")))?;
        let mut detail = load_camera_detail(
            &transport,
            &base_url(&core, &state.config),
            &headers,
            &camera_id,
            now,
        )
        .map_err(ApiError::from)?;
        if let Some(position) = detail.position {
            detail.position = Some(gcj02_to_wgs84(position));
        }
        Ok(Json(detail))
    })
    .await
}

fn state_payload(core: &CoreState, config: &WebConfig, snapshot: &CameraSnapshot) -> Value {
    json!({
        "loggedIn": core.session.is_logged_in(),
        "user": if core.session.is_logged_in() {
            session_user(core)
        } else {
            Value::Null
        },
        "cityCode": core.settings.city_code,
        "apiBaseUrl": base_url(core, config),
        "signatureConfigured": !config.signature_md5.is_empty(),
        "cameraSource": snapshot.source,
        "cameraCount": snapshot.cameras.len(),
        "fetchedAtMs": snapshot.fetched_at_ms,
    })
}

fn session_user(core: &CoreState) -> Value {
    let session = &core.session;
    json!({
        "phone": session.phone,
        "nickName": session.nick_name,
        "realName": session.real_name,
        "photo": session.photo,
        "isPro": session.is_pro,
        "memEndTimeMs": session.mem_end_time_ms,
    })
}

fn snapshot(state: &AppState) -> Result<CameraSnapshot, ApiError> {
    state
        .cameras
        .lock()
        .map(|cache| cache.clone())
        .map_err(|_| ApiError::internal("camera cache lock is poisoned"))
}

fn reset_camera_cache(state: &AppState, core: &CoreState) -> Result<CameraSnapshot, ApiError> {
    let snapshot = CameraSnapshot::demo(core.settings.city_code.clone());
    {
        let mut cache = state
            .cameras
            .lock()
            .map_err(|_| ApiError::internal("camera cache lock is poisoned"))?;
        *cache = snapshot.clone();
    }
    Ok(snapshot)
}

fn load_core(state: &AppState) -> Result<CoreState, ApiError> {
    state
        .store
        .load()
        .map_err(|error| ApiError::internal(format!("cannot read local state: {error}")))
}

fn require_session(core: &CoreState) -> Result<(), ApiError> {
    if core.session.is_logged_in() {
        Ok(())
    } else {
        Err(ApiError::unauthorized("请先登录 Motu 账号"))
    }
}

fn base_url(core: &CoreState, config: &WebConfig) -> String {
    if !config.api_base_url.trim().is_empty() {
        config.api_base_url.trim().to_owned()
    } else if !core.settings.api_base_url.trim().is_empty() {
        core.settings.api_base_url.trim().to_owned()
    } else {
        BASE_URL.to_owned()
    }
}

fn auth_headers(core: &CoreState, config: &WebConfig, now_ms: i64) -> Vec<(String, String)> {
    let device_id = core.device_id.trim();
    let env_token = if device_id.is_empty() || config.signature_md5.trim().is_empty() {
        None
    } else {
        Some(safe_env_token(
            device_id,
            u64::try_from(now_ms / 1000).unwrap_or(0),
            &config.signature_md5,
        ))
    };
    base_headers(&HeaderInit {
        token: Some(core.session.token.clone()),
        device_id: (!device_id.is_empty()).then(|| device_id.to_owned()),
        safe_env_token: env_token,
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

async fn blocking<T, F>(work: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ApiError::internal(format!("background task failed: {error}")))?
}

fn static_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = FileStore::new(directory.path());
        store.ensure_root().expect("store root");
        let core = store.ensure_identity().expect("identity");
        let config = WebConfig {
            bind: "127.0.0.1:0".parse().expect("socket address"),
            data_dir: directory.path().to_path_buf(),
            api_base_url: String::new(),
            signature_md5: String::new(),
            tile_url: "https://tile.openstreetmap.org/{z}/{x}/{y}.png".to_owned(),
            tile_attribution: "test".to_owned(),
        };
        let snapshot = CameraSnapshot::demo(core.settings.city_code.clone());
        let state = Arc::new(AppState {
            store,
            config: Arc::new(config),
            cameras: Arc::new(Mutex::new(snapshot)),
        });
        (directory, state)
    }

    #[tokio::test]
    async fn health_and_demo_cameras_work() {
        let (_directory, state) = test_state();
        let app = router(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("health");
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/cameras")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("cameras");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        let value: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(value["source"], "demo");
        assert!(!value["cameras"].as_array().expect("cameras").is_empty());
    }

    #[tokio::test]
    async fn detail_requires_login() {
        let (_directory, state) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/detail")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"cameraId":"camera-1"}"#))
                    .expect("request"),
            )
            .await
            .expect("detail");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_rejects_empty_credentials() {
        let (_directory, state) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"phone":"","password":""}"#))
                    .expect("request"),
            )
            .await
            .expect("login");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn sync_rejects_invalid_coordinates() {
        let (_directory, state) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sync")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"latitude":999,"longitude":0}"#))
                    .expect("request"),
            )
            .await
            .expect("sync");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
