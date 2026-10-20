//! JNI bridge for the Android UI.
//!
//! Every export returns a JSON envelope (`{"ok":true,"data":...}` or
//! `{"ok":false,"error":"..."}`) so the Java layer never needs to parse Rust
//! errors or catch native exceptions. The UI class is
//! `com.manisroad.app.core.MotuCore`.
//!
//! JNI exports necessarily use `#[unsafe(no_mangle)]` and raw Java strings;
//! the unsafe surface is confined to this crate.

#![allow(unsafe_code)]

use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jdouble, jstring};
#[cfg(feature = "http")]
use motu_core::api::ReqwestTransport;
use motu_core::{
    BASE_URL, CameraScanEngine, CoreState, FileStore, HeaderInit, LatLng, NaviLocation,
    OfficialCameraQuery, PlateColor, RoutePlanRequest, ScanConfig, SignedRequest,
    StraightLinePlanner, TrafficCamera, Vehicle, audit_route, base_headers, build_request,
    decode_polyline, default_tab_types, encode_polyline, endpoint_by_name, load_camera_detail,
    login as auth_login, logout as auth_logout, parse_amap_positions, safe_env_token,
    signing_certificate_md5, sync_official_cameras,
};
use serde_json::{Map, Value, json};

#[derive(Default)]
struct FfiState {
    engine: CameraScanEngine,
    store: Option<FileStore>,
    core: CoreState,
}

static STATE: OnceLock<Mutex<FfiState>> = OnceLock::new();

fn with_state<T>(f: impl FnOnce(&mut FfiState) -> Result<T, String>) -> Result<T, String> {
    let lock = STATE.get_or_init(|| Mutex::new(FfiState::default()));
    let mut guard = lock
        .lock()
        .map_err(|_| "native state lock is poisoned".to_owned())?;
    f(&mut guard)
}

#[allow(clippy::needless_pass_by_value)]
fn ok(data: Value) -> String {
    json!({"ok": true, "data": data}).to_string()
}

fn ok_empty() -> String {
    json!({"ok": true}).to_string()
}

#[allow(clippy::needless_pass_by_value)]
fn err(message: impl ToString) -> String {
    json!({"ok": false, "error": message.to_string()}).to_string()
}

fn string_arg(env: &mut JNIEnv, value: &JString) -> Result<String, String> {
    env.get_string(value)
        .map(Into::into)
        .map_err(|error| format!("invalid Java string: {error}"))
}

fn byte_array_arg(env: &mut JNIEnv, value: &JByteArray) -> Result<Vec<u8>, String> {
    if value.as_raw().is_null() {
        return Ok(Vec::new());
    }
    env.convert_byte_array(value)
        .map_err(|error| format!("invalid Java byte array: {error}"))
}

fn jstring_out(env: &mut JNIEnv, text: String) -> jstring {
    env.new_string(text)
        .map_or(std::ptr::null_mut(), JString::into_raw)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

fn parse_json<T: serde::de::DeserializeOwned>(text: &str, label: &str) -> Result<T, String> {
    serde_json::from_str(text).map_err(|error| format!("invalid {label} JSON: {error}"))
}

fn query_from_json(text: &str) -> Result<std::collections::BTreeMap<String, String>, String> {
    if text.trim().is_empty() || text.trim() == "null" {
        return Ok(std::collections::BTreeMap::new());
    }
    let map: Map<String, Value> = parse_json(text, "query")?;
    let mut query = std::collections::BTreeMap::new();
    for (key, value) in map {
        let text = match value {
            Value::Null => continue,
            Value::String(text) => text,
            Value::Bool(flag) => flag.to_string(),
            Value::Number(number) => number.to_string(),
            Value::Array(_) | Value::Object(_) => value.to_string(),
        };
        query.insert(key, text);
    }
    Ok(query)
}

fn body_from_json(text: &str) -> Result<Option<Value>, String> {
    if text.trim().is_empty() || text.trim() == "null" {
        return Ok(None);
    }
    Ok(Some(parse_json(text, "body")?))
}

fn apply_settings(state: &mut FfiState) {
    let mut config = ScanConfig::default();
    if let Some(override_m) = state.core.settings.camera_radius_override_m {
        if override_m.is_finite() && override_m >= 1.0 {
            config.ordinary_radius_m = override_m;
            config.restricted_radius_m = override_m.max(config.restricted_radius_m);
        }
    }
    state.engine = CameraScanEngine::new(config);
}

/// `nativeInit(dataDir)`: open the JSON store and restore settings/vehicles.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeInit(
    mut env: JNIEnv,
    _class: JClass,
    data_dir: JString,
) -> jstring {
    let result = string_arg(&mut env, &data_dir).and_then(|path| {
        let store = FileStore::new(path);
        store.ensure_root().map_err(|error| error.to_string())?;
        let core = store.ensure_identity().map_err(|error| error.to_string())?;
        with_state(|state| {
            state.store = Some(store);
            state.core = core;
            apply_settings(state);
            Ok(ok_empty())
        })
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeLoadState()`: return the persisted [`CoreState`].
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeLoadState(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let result = with_state(|state| {
        serde_json::to_value(&state.core)
            .map(ok)
            .map_err(|error| error.to_string())
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeSaveState(json)`: validate and persist a [`CoreState`].
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeSaveState(
    mut env: JNIEnv,
    _class: JClass,
    json: JString,
) -> jstring {
    let result = string_arg(&mut env, &json).and_then(|text| {
        let core: CoreState = parse_json(&text, "state")?;
        for vehicle in &core.vehicles {
            vehicle
                .validate()
                .require_valid()
                .map_err(|error| error.to_string())?;
        }
        with_state(|state| {
            let store = state.store.as_ref().ok_or("nativeInit was not called")?;
            store.save(&core).map_err(|error| error.to_string())?;
            state.core = core;
            apply_settings(state);
            Ok(ok_empty())
        })
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeDefaultVehicle()`: default vehicle or `null`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeDefaultVehicle(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let result = with_state(|state| {
        serde_json::to_value(state.core.default_vehicle())
            .map(ok)
            .map_err(|error| error.to_string())
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeUpdateCameras(json)`: replace the scanner's camera set.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeUpdateCameras(
    mut env: JNIEnv,
    _class: JClass,
    json: JString,
) -> jstring {
    let result = string_arg(&mut env, &json).and_then(|text| {
        let cameras: Vec<TrafficCamera> = parse_json(&text, "cameras")?;
        with_state(|state| {
            state.engine.update_cameras(&cameras);
            Ok(ok(json!(state.engine.cameras().len())))
        })
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeScan(lat, lon, bearing, allow)`: return a `ScanResult` or `null`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeScan(
    mut env: JNIEnv,
    _class: JClass,
    latitude: jdouble,
    longitude: jdouble,
    bearing: jdouble,
    allow_scan: jboolean,
) -> jstring {
    let result = with_state(|state| {
        let location = NaviLocation::new(LatLng::new(latitude, longitude), bearing);
        let allow = allow_scan != 0 && state.core.settings.camera_alerts_enabled;
        let result = state.engine.scan(Some(&location), allow);
        serde_json::to_value(result)
            .map(ok)
            .map_err(|error| error.to_string())
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativePause()`: clear the scan gate/reminders, keep cameras.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativePause(
    _env: JNIEnv,
    _class: JClass,
) {
    let _ = with_state(|state| {
        state.engine.pause();
        Ok(())
    });
}

/// `nativeResetScan()`: clear cameras, gate and reminders.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeResetScan(
    _env: JNIEnv,
    _class: JClass,
) {
    let _ = with_state(|state| {
        state.engine.reset();
        Ok(())
    });
}

/// `nativeBuildRequest(endpoint, queryJson, bodyJson, headersJson, timestampMs)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeBuildRequest(
    mut env: JNIEnv,
    _class: JClass,
    endpoint_name: JString,
    query_json: JString,
    body_json: JString,
    headers_json: JString,
    timestamp_ms: jni::sys::jlong,
) -> jstring {
    let result: Result<String, String> = (|| {
        let name = string_arg(&mut env, &endpoint_name)?;
        let query_text = string_arg(&mut env, &query_json)?;
        let body_text = string_arg(&mut env, &body_json)?;
        let headers_text = string_arg(&mut env, &headers_json)?;

        let endpoint =
            endpoint_by_name(&name).ok_or_else(|| format!("unknown endpoint: {name}"))?;
        let query = query_from_json(&query_text)?;
        let body = body_from_json(&body_text)?;
        let init: HeaderInit = if headers_text.trim().is_empty() {
            HeaderInit::default()
        } else {
            parse_json(&headers_text, "headers")?
        };
        let headers = base_headers(&init);
        let timestamp = if timestamp_ms == 0 {
            now_ms()
        } else {
            timestamp_ms
        };
        let request: SignedRequest =
            build_request(&endpoint, BASE_URL, &headers, &query, body, timestamp)
                .map_err(|error| error.to_string())?;
        Ok(ok(
            serde_json::to_value(request).map_err(|error| error.to_string())?
        ))
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeSafeEnvToken(deviceId, seconds, signingCertificate)`.
///
/// Java supplies the APK signing certificate DER bytes; the uppercase MD5
/// fingerprint and the token are computed here.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeSafeEnvToken(
    mut env: JNIEnv,
    _class: JClass,
    device_id: JString,
    seconds: jni::sys::jlong,
    signing_certificate: JByteArray,
) -> jstring {
    let result: Result<String, String> = (|| {
        let device_id = string_arg(&mut env, &device_id)?;
        let certificate = byte_array_arg(&mut env, &signing_certificate)?;
        let signature = signing_certificate_md5(&certificate);
        let seconds = u64::try_from(seconds).map_err(|_| "seconds must be non-negative")?;
        Ok(ok(json!(safe_env_token(&device_id, seconds, &signature))))
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeVehicleValidate(vehicleJson)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeVehicleValidate(
    mut env: JNIEnv,
    _class: JClass,
    json: JString,
) -> jstring {
    let result = string_arg(&mut env, &json).and_then(|text| {
        let vehicle: Vehicle = parse_json(&text, "vehicle")?;
        serde_json::to_value(vehicle.validate())
            .map(ok)
            .map_err(|error| error.to_string())
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeRouteAudit(pointsJson, areasJson, camerasJson, bufferMeters)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeRouteAudit(
    mut env: JNIEnv,
    _class: JClass,
    points_json: JString,
    areas_json: JString,
    cameras_json: JString,
    buffer_m: jdouble,
) -> jstring {
    let result: Result<String, String> = (|| {
        let points_text = string_arg(&mut env, &points_json)?;
        let areas_text = string_arg(&mut env, &areas_json)?;
        let cameras_text = string_arg(&mut env, &cameras_json)?;
        let points: Vec<LatLng> = parse_json(&points_text, "route points")?;
        let areas: Vec<motu_core::AvoidArea> = parse_json(&areas_text, "avoid areas")?;
        let cameras: Vec<TrafficCamera> = parse_json(&cameras_text, "cameras")?;
        let audit =
            audit_route(&points, &areas, &cameras, buffer_m).map_err(|error| error.to_string())?;
        Ok(ok(
            serde_json::to_value(audit).map_err(|error| error.to_string())?
        ))
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativePlanStraightLine(requestJson)`: offline fallback route.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativePlanStraightLine(
    mut env: JNIEnv,
    _class: JClass,
    json: JString,
) -> jstring {
    let result = string_arg(&mut env, &json).and_then(|text| {
        let request: RoutePlanRequest = parse_json(&text, "route request")?;
        let plan = motu_core::RoutePlanner::plan(&StraightLinePlanner::default(), &request)
            .map_err(|error| error.to_string())?;
        Ok(ok(
            serde_json::to_value(plan).map_err(|error| error.to_string())?
        ))
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeEncodePolyline(pointsJson)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeEncodePolyline(
    mut env: JNIEnv,
    _class: JClass,
    json: JString,
) -> jstring {
    let result = string_arg(&mut env, &json).and_then(|text| {
        let points: Vec<LatLng> = parse_json(&text, "route points")?;
        Ok(ok(json!(encode_polyline(&points))))
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeDecodePolyline(value)` or `nativeParseAmapPositions(value)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeDecodePolyline(
    mut env: JNIEnv,
    _class: JClass,
    value: JString,
) -> jstring {
    let result = string_arg(&mut env, &value).and_then(|text| {
        let points = decode_polyline(&text).map_err(|error| error.to_string())?;
        Ok(ok(
            serde_json::to_value(points).map_err(|error| error.to_string())?
        ))
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeParseAmapPositions(value)`: `lon,lat;lon,lat`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeParseAmapPositions(
    mut env: JNIEnv,
    _class: JClass,
    value: JString,
) -> jstring {
    let result = string_arg(&mut env, &value).and_then(|text| {
        let points = parse_amap_positions(&text).map_err(|error| error.to_string())?;
        Ok(ok(
            serde_json::to_value(points).map_err(|error| error.to_string())?
        ))
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeEndpoints()`: endpoint metadata for diagnostics.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeEndpoints(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let endpoints: Vec<Value> = motu_core::ALL_ENDPOINTS
        .iter()
        .map(|endpoint| {
            json!({
                "name": endpoint.name,
                "path": endpoint.path,
                "method": endpoint.method.as_str(),
                "encrypted": endpoint.encrypted,
            })
        })
        .collect();
    jstring_out(&mut env, ok(Value::Array(endpoints)))
}

/// `nativeConstants()`: recovered constants displayed in diagnostics.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeConstants(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    jstring_out(
        &mut env,
        ok(json!({
            "baseUrl": BASE_URL,
            "appVersion": motu_core::crypto::APP_VERSION,
            "platform": motu_core::crypto::PLATFORM,
            "scanIntervalM": 100,
            "forwardConeDeg": 7.5,
            "ordinaryRadiusM": 300,
            "restrictedRadiusM": 500,
        })),
    )
}

/// `nativeLogin(phone, password, signingCertificate)`: official `v4/user/login`.
///
/// Java supplies the APK signing certificate DER bytes; the Rust core derives
/// the uppercase MD5 fingerprint. The call blocks on the network, so Java must
/// run it off the UI thread. The returned session is persisted before the call
/// returns.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeLogin(
    mut env: JNIEnv,
    _class: JClass,
    phone: JString,
    password: JString,
    signing_certificate: JByteArray,
) -> jstring {
    let result = (|| -> Result<String, String> {
        let phone = string_arg(&mut env, &phone)?;
        let password = string_arg(&mut env, &password)?;
        let certificate = byte_array_arg(&mut env, &signing_certificate)?;
        let signature_md5 = signing_certificate_md5(&certificate);
        let (store, core, base_url) = with_state(|state| {
            let store = state
                .store
                .clone()
                .ok_or_else(|| "core is not initialised".to_owned())?;
            let core = store.ensure_identity().map_err(|error| error.to_string())?;
            let base_url = if core.settings.api_base_url.trim().is_empty() {
                BASE_URL.to_owned()
            } else {
                core.settings.api_base_url.trim().to_owned()
            };
            Ok((store, core, base_url))
        })?;
        #[cfg(feature = "http")]
        let outcome = {
            let transport = ReqwestTransport::new().map_err(|error| error.to_string())?;
            auth_login(
                &transport,
                &base_url,
                &core.device_id,
                &signature_md5,
                &phone,
                &password,
                now_ms(),
            )
            .map_err(|error| error.to_string())?
        };
        #[cfg(not(feature = "http"))]
        let outcome = {
            let _ = (&core, &phone, &password, &signature_md5, &base_url);
            return Err("networking is not enabled in this build".to_owned());
        };
        let saved = store
            .save_session(outcome.session.clone())
            .map_err(|error| error.to_string())?;
        with_state(|state| {
            state.core = saved;
            Ok(())
        })?;
        serde_json::to_value(&outcome)
            .map(ok)
            .map_err(|error| error.to_string())
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeSession()`: return the persisted session, or an empty object.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeSession(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let result = with_state(|state| {
        serde_json::to_value(&state.core.session)
            .map(ok)
            .map_err(|error| error.to_string())
    });
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeDeviceId()`: stable per-install identifier.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeDeviceId(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let result = with_state(|state| Ok(ok(json!(state.core.device_id))));
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeLogout()`: best-effort server logout, then clear the local session.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeLogout(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let result = (|| -> Result<String, String> {
        let (store, core, base_url) = with_state(|state| {
            let store = state
                .store
                .clone()
                .ok_or_else(|| "core is not initialised".to_owned())?;
            let core = store.load().map_err(|error| error.to_string())?;
            let base_url = if core.settings.api_base_url.trim().is_empty() {
                BASE_URL.to_owned()
            } else {
                core.settings.api_base_url.trim().to_owned()
            };
            Ok((store, core, base_url))
        })?;
        let mut server_error: Option<String> = None;
        #[cfg(feature = "http")]
        {
            if core.session.is_logged_in() {
                match ReqwestTransport::new() {
                    Ok(transport) => {
                        if let Err(error) =
                            auth_logout(&transport, &base_url, &core.session, now_ms())
                        {
                            server_error = Some(error.to_string());
                        }
                    }
                    Err(error) => server_error = Some(error.to_string()),
                }
            }
        }
        #[cfg(not(feature = "http"))]
        {
            let _ = (&core, &base_url);
        }
        let saved = store.clear_session().map_err(|error| error.to_string())?;
        with_state(|state| {
            state.core = saved;
            Ok(())
        })?;
        if let Some(error) = server_error {
            return Err(error);
        }
        Ok(ok_empty())
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeSyncOfficialCameras(lat, lon, cityCode, signingCertificate)`.
///
/// Requires a signed-in session. Java supplies the APK signing certificate DER
/// bytes; the Rust core derives the uppercase MD5 fingerprint. The official
/// camera list is kept in the in-memory scan engine and returned to the UI; it
/// is never written to the JSON store. Network access blocks, so Java must call
/// this off the UI thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeSyncOfficialCameras(
    mut env: JNIEnv,
    _class: JClass,
    latitude: jdouble,
    longitude: jdouble,
    city_code: JString,
    signing_certificate: JByteArray,
) -> jstring {
    let result = (|| -> Result<String, String> {
        let city_code = string_arg(&mut env, &city_code)?;
        let certificate = byte_array_arg(&mut env, &signing_certificate)?;
        let signature_md5 = signing_certificate_md5(&certificate);
        let (core, base_url) = with_state(|state| {
            let core = state
                .store
                .as_ref()
                .ok_or_else(|| "core is not initialised".to_owned())?
                .load()
                .map_err(|error| error.to_string())?;
            let base_url = if core.settings.api_base_url.trim().is_empty() {
                BASE_URL.to_owned()
            } else {
                core.settings.api_base_url.trim().to_owned()
            };
            Ok((core, base_url))
        })?;
        if !core.session.is_logged_in() {
            return Err("official data requires a signed-in account".to_owned());
        }

        let city = city_code.trim();
        let city = if city.is_empty() { "010" } else { city };
        let vehicle = core.default_vehicle();
        let plate_color = vehicle.map_or("YELLOW", |vehicle| {
            let color = PlateColor::from_id(vehicle.plate_color).as_wire_str();
            if color.is_empty() { "YELLOW" } else { color }
        });
        let plate_val = vehicle.map_or_else(String::new, |vehicle| vehicle.number_plate.clone());
        let query = OfficialCameraQuery {
            position: LatLng::new(latitude, longitude),
            city_code: city.to_owned(),
            to_city_code: city.to_owned(),
            tab_types: default_tab_types(),
            plate_color: plate_color.to_owned(),
            plate_val,
            cruise: false,
            source: 1,
        };

        let now = now_ms();
        let device_id = core.device_id.trim();
        let env_token = if device_id.is_empty() || signature_md5.trim().is_empty() {
            None
        } else {
            Some(safe_env_token(
                device_id,
                u64::try_from(now / 1000).unwrap_or(0),
                &signature_md5,
            ))
        };
        let headers = base_headers(&HeaderInit {
            token: Some(core.session.token.clone()),
            device_id: (!device_id.is_empty()).then(|| device_id.to_owned()),
            safe_env_token: env_token,
        });

        #[cfg(feature = "http")]
        {
            let transport = ReqwestTransport::new().map_err(|error| error.to_string())?;
            let sync = sync_official_cameras(&transport, &base_url, &headers, &query, now)
                .map_err(|error| error.to_string())?;
            with_state(|state| {
                state.engine.pause();
                state.engine.update_cameras(&sync.cameras);
                Ok(())
            })?;
            serde_json::to_value(&sync)
                .map(ok)
                .map_err(|error| error.to_string())
        }
        #[cfg(not(feature = "http"))]
        {
            let _ = (&base_url, &headers, &query, now);
            Err("networking is not enabled in this build".to_owned())
        }
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeLoadCameraDetail(cameraId, signingCertificate)`.
///
/// Requires a signed-in session. The Rust core calls the encrypted
/// `trafficEyesDetailSec` and `trafficTicketListSec` endpoints, merges the
/// payloads and returns them to the UI. Nothing is persisted. Network access
/// blocks, so Java must call this off the UI thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeLoadCameraDetail(
    mut env: JNIEnv,
    _class: JClass,
    camera_id: JString,
    signing_certificate: JByteArray,
) -> jstring {
    let result = (|| -> Result<String, String> {
        let camera_id = string_arg(&mut env, &camera_id)?;
        let certificate = byte_array_arg(&mut env, &signing_certificate)?;
        let signature_md5 = signing_certificate_md5(&certificate);
        let (core, base_url) = with_state(|state| {
            let core = state
                .store
                .as_ref()
                .ok_or_else(|| "core is not initialised".to_owned())?
                .load()
                .map_err(|error| error.to_string())?;
            let base_url = if core.settings.api_base_url.trim().is_empty() {
                BASE_URL.to_owned()
            } else {
                core.settings.api_base_url.trim().to_owned()
            };
            Ok((core, base_url))
        })?;
        if !core.session.is_logged_in() {
            return Err("official data requires a signed-in account".to_owned());
        }
        if camera_id.trim().is_empty() {
            return Err("trafficEyesId must not be empty".to_owned());
        }

        let now = now_ms();
        let device_id = core.device_id.trim();
        let env_token = if device_id.is_empty() || signature_md5.trim().is_empty() {
            None
        } else {
            Some(safe_env_token(
                device_id,
                u64::try_from(now / 1000).unwrap_or(0),
                &signature_md5,
            ))
        };
        let headers = base_headers(&HeaderInit {
            token: Some(core.session.token.clone()),
            device_id: (!device_id.is_empty()).then(|| device_id.to_owned()),
            safe_env_token: env_token,
        });

        #[cfg(feature = "http")]
        {
            let transport = ReqwestTransport::new().map_err(|error| error.to_string())?;
            let detail = load_camera_detail(&transport, &base_url, &headers, &camera_id, now)
                .map_err(|error| error.to_string())?;
            serde_json::to_value(detail)
                .map(ok)
                .map_err(|error| error.to_string())
        }
        #[cfg(not(feature = "http"))]
        {
            let _ = (&base_url, &headers, &camera_id);
            Err("networking is not enabled in this build".to_owned())
        }
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).expect("bridge output must be JSON")
    }

    #[test]
    fn envelopes_are_valid_json() {
        let success = parse(&ok(json!({"value": 7})));
        assert_eq!(success["ok"], Value::Bool(true));
        assert_eq!(success["data"]["value"], json!(7));

        let failure = parse(&err("boom"));
        assert_eq!(failure["ok"], Value::Bool(false));
        assert_eq!(failure["error"], json!("boom"));

        let empty = parse(&ok_empty());
        assert_eq!(empty["ok"], Value::Bool(true));
        assert!(empty.get("data").is_none());
    }

    #[test]
    fn query_skips_nulls_and_coerces_scalars() {
        let query =
            query_from_json(r#"{"text":"a","flag":true,"count":3,"list":[1,2],"none":null}"#)
                .unwrap();
        assert_eq!(query.get("text").map(String::as_str), Some("a"));
        assert_eq!(query.get("flag").map(String::as_str), Some("true"));
        assert_eq!(query.get("count").map(String::as_str), Some("3"));
        assert_eq!(query.get("list").map(String::as_str), Some("[1,2]"));
        assert!(!query.contains_key("none"));
    }

    #[test]
    fn empty_query_and_body_are_accepted() {
        assert!(query_from_json("").unwrap().is_empty());
        assert!(query_from_json("null").unwrap().is_empty());
        assert!(body_from_json("").unwrap().is_none());
        assert!(body_from_json("null").unwrap().is_none());
        assert_eq!(body_from_json(r#"{"a":1}"#).unwrap(), Some(json!({"a": 1})));
    }

    #[test]
    fn malformed_payloads_are_reported() {
        let error = query_from_json("{oops").unwrap_err();
        assert!(error.contains("invalid query JSON"), "{error}");
        let error = body_from_json("[1").unwrap_err();
        assert!(error.contains("invalid body JSON"), "{error}");
    }

    #[test]
    fn timestamps_are_positive() {
        assert!(now_ms() > 0);
    }
}
