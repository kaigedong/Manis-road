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

use std::{
    path::Path,
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jdouble, jint, jstring};
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
use motu_pack::pmtiles::TileSet;
use motu_routing::{
    CameraDeterrent, Candidate, ReplanPolicy, ReplanReason, RoadGraph, cameras_within,
    evaluate_after, route_overlap,
};
use serde_json::{Map, Value, json};

/// Graph loaded on demand; Android keeps `graph.bin` in its files directory.
struct LoadedGraph {
    path: String,
    graph: RoadGraph,
}

/// Cached `tiles.pmtiles` archive; the app asks for one tile per map frame.
/// Cached traffic prior; `routeWithContext` reads it on every plan/re-plan.
struct LoadedPrior {
    path: String,
    prior: motu_routing::TrafficPrior,
}

struct LoadedTiles {
    path: String,
    tiles: TileSet,
}

#[derive(Default)]
struct FfiState {
    engine: CameraScanEngine,
    store: Option<FileStore>,
    core: CoreState,
    graph: Option<LoadedGraph>,
    tiles: Option<LoadedTiles>,
    prior: Option<LoadedPrior>,
}

fn prior_for<'a>(
    state: &'a mut FfiState,
    path: &str,
) -> Result<&'a motu_routing::TrafficPrior, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("prior path is empty".to_owned());
    }
    if state.prior.as_ref().is_some_and(|loaded| loaded.path == path) {
        return Ok(&state.prior.as_ref().expect("checked above").prior);
    }
    let prior = motu_routing::TrafficPrior::load(Path::new(path))
        .map_err(|error| error.to_string())?;
    state.prior = Some(LoadedPrior {
        path: path.to_owned(),
        prior,
    });
    Ok(&state.prior.as_ref().expect("just stored").prior)
}

fn tiles_for<'a>(state: &'a mut FfiState, path: &str) -> Result<&'a TileSet, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("pmtiles path is empty".to_owned());
    }
    if state.tiles.as_ref().is_some_and(|loaded| loaded.path == path) {
        return Ok(&state.tiles.as_ref().expect("checked above").tiles);
    }
    let tiles = TileSet::open(Path::new(path)).map_err(|error| error.to_string())?;
    state.tiles = Some(LoadedTiles {
        path: path.to_owned(),
        tiles,
    });
    Ok(&state.tiles.as_ref().expect("just stored").tiles)
}

fn graph_for<'a>(state: &'a mut FfiState, path: &str) -> Result<&'a RoadGraph, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("graph path is empty".to_owned());
    }
    if state.graph.as_ref().is_some_and(|loaded| loaded.path == path) {
        return Ok(&state
            .graph
            .as_ref()
            .expect("checked above")
            .graph);
    }
    let graph = RoadGraph::load(Path::new(path)).map_err(|error| error.to_string())?;
    state.graph = Some(LoadedGraph {
        path: path.to_owned(),
        graph,
    });
    Ok(&state.graph.as_ref().expect("just loaded").graph)
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

/// `nativeLoadGraph(graphPath)`: load (and cache) the offline road graph.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeLoadGraph(
    mut env: JNIEnv,
    _class: JClass,
    path: JString,
) -> jstring {
    let result: Result<String, String> = (|| {
        let path = string_arg(&mut env, &path)?;
        let info = with_state(|state| {
            let graph = graph_for(state, &path)?;
            Ok(json!({
                "path": path,
                "nodes": graph.node_count(),
                "edges": graph.edge_count(),
                "bounds": graph.bounds(),
            }))
        })?;
        Ok(ok(info))
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeRoute(graphPath, originLat, originLon, destinationLat, destinationLon)`.
///
/// Fully offline: loads `graph.bin` once, then A* in memory.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeRoute(
    mut env: JNIEnv,
    _class: JClass,
    path: JString,
    origin_lat: jdouble,
    origin_lon: jdouble,
    destination_lat: jdouble,
    destination_lon: jdouble,
) -> jstring {
    let result: Result<String, String> = (|| {
        let path = string_arg(&mut env, &path)?;
        let origin = LatLng::new(origin_lat, origin_lon);
        let destination = LatLng::new(destination_lat, destination_lon);
        let plan = with_state(|state| {
            let graph = graph_for(state, &path)?;
            graph
                .route(origin, destination)
                .map_err(|error| error.to_string())
        })?;
        Ok(ok(
            serde_json::to_value(plan).map_err(|error| error.to_string())?
        ))
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeRouteWithContext(graphPath, oLat, oLon, dLat, dLon, priorPath, hour, avoidJson)`.
///
/// The full three-layer model: static profile + time-of-day prior + banned-area
/// constraint. `priorPath` may be empty (no time dependence); `avoidJson` is a
/// list of `[[lat, lon], ...]` polygons and may be empty.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeRouteWithContext(
    mut env: JNIEnv,
    _class: JClass,
    graph_path: JString,
    origin_lat: jdouble,
    origin_lon: jdouble,
    destination_lat: jdouble,
    destination_lon: jdouble,
    prior_path: JString,
    hour: jint,
    avoid_json: JString,
) -> jstring {
    let result: Result<String, String> = (|| {
        let graph_path = string_arg(&mut env, &graph_path)?;
        let prior_path = string_arg(&mut env, &prior_path)?;
        let avoid_json = string_arg(&mut env, &avoid_json)?;
        let origin = LatLng::new(origin_lat, origin_lon);
        let destination = LatLng::new(destination_lat, destination_lon);
        let hour = u8::try_from(hour).map_err(|_| format!("bad hour {hour}"))?;
        if hour > 23 {
            return Err(format!("hour must be 0..=23, got {hour}"));
        }
        let avoid: Vec<Vec<LatLng>> = if avoid_json.trim().is_empty() {
            Vec::new()
        } else {
            parse_json(&avoid_json, "avoid")?
        };

        with_state(|state| {
            let profile = motu_routing::RouteProfile::motorcycle_default();
            // 先验很小，克隆一份再借图，避免同时可变借用 state。
            let prior_owned = if prior_path.trim().is_empty() {
                None
            } else {
                Some(prior_for(state, &prior_path)?.clone())
            };
            let context = motu_routing::RouteContext {
                prior: prior_owned.as_ref(),
                hour,
                avoid: &avoid,
            };
            let graph = graph_for(state, &graph_path)?;
            let (plan, metrics) = graph
                .route_report_with_context(origin, destination, &profile, None, &context)
                .map_err(|error| error.to_string())?;
            let source = prior_owned.as_ref().map_or("none", |prior| match prior.source {
                motu_routing::PriorSource::Measured => "measured",
                motu_routing::PriorSource::Assumption => "assumption",
            });
            Ok(json!({
                "plan": plan,
                "metrics": metrics,
                "priorSource": source,
                "hour": hour,
                "avoidAreas": avoid.len(),
            }))
        })
        .map(|value| ok(value))
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// Build a route that penalises nodes near the supplied WGS-84 cameras.
fn route_avoiding_json(
    graph_path: &str,
    origin: LatLng,
    destination: LatLng,
    cameras_json: &str,
    radius_m: f64,
    _max_ratio: f64,
    _max_minutes: f64,
) -> Result<Value, String> {
    let cameras: Vec<TrafficCamera> = parse_json(cameras_json, "cameras")?;
    let points: Vec<LatLng> = cameras.iter().map(|camera| camera.position).collect();
    let radius = radius_m.clamp(20.0, 300.0);
    with_state(|state| {
        let graph = graph_for(state, graph_path)?;
        let baseline = graph
            .route(origin, destination)
            .map_err(|error| error.to_string())?;
        let before = cameras_within(&baseline.points, &points, radius);
        if before == 0 {
            return Ok(json!({
                "plan": baseline,
                "avoidance": {
                    "applied": false,
                    "rejected": false,
                    "reason": format!("本路线 {radius:.0} m 内没有已知点位"),
                    "camerasBefore": 0,
                    "camerasAfter": 0,
                    "radiusM": radius,
                    "detourRatio": 0.0,
                    "extraSeconds": 0.0,
                }
            }));
        }
        // Hard avoidance first; if the graph is disconnected, fall back to a
        // very high time penalty. Never return a route that cannot reduce the
        // known hits.
        let hard = CameraDeterrent::build_hard(graph, &points, radius);
        let candidate = match graph.route_with(origin, destination, Some(&hard)) {
            Ok(candidate) => candidate,
            Err(_) => {
                let soft = CameraDeterrent::build(graph, &points, radius, 50.0);
                match graph.route_with(origin, destination, Some(&soft)) {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        return Err(format!(
                            "强制避开电子眼后无可行路线（{error}）；可减小半径或取消勾选"
                        ));
                    }
                }
            }
        };
        let after = cameras_within(&candidate.points, &points, radius);
        if after >= before {
            return Err(format!(
                "强制避开电子眼后仍有 {after} 个已知点位，无法减少；可减小半径或取消勾选"
            ));
        }
        let extra = candidate.duration_s - baseline.duration_s;
        let ratio = if baseline.duration_s > 0.0 {
            extra / baseline.duration_s
        } else {
            0.0
        };
        let reason = (after > 0).then(|| {
            format!("无法完全避开，仍有 {after} 个已知点位")
        });
        Ok(json!({
            "plan": candidate,
            "avoidance": {
                "applied": true,
                "rejected": false,
                "reason": reason,
                "camerasBefore": before,
                "camerasAfter": after,
                "radiusM": radius,
                "detourRatio": ratio,
                "extraSeconds": extra,
            }
        }))
    })
}

/// `nativeRouteAvoiding(graphPath, oLat, oLon, dLat, dLon, camerasJson, radiusM, maxRatio, maxMinutes)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeRouteAvoiding(
    mut env: JNIEnv,
    _class: JClass,
    path: JString,
    origin_lat: jdouble,
    origin_lon: jdouble,
    destination_lat: jdouble,
    destination_lon: jdouble,
    cameras_json: JString,
    radius_m: jdouble,
    max_ratio: jdouble,
    max_minutes: jdouble,
) -> jstring {
    let result: Result<String, String> = (|| {
        let path = string_arg(&mut env, &path)?;
        let cameras = string_arg(&mut env, &cameras_json)?;
        let value = route_avoiding_json(
            &path,
            LatLng::new(origin_lat, origin_lon),
            LatLng::new(destination_lat, destination_lon),
            &cameras,
            radius_m,
            max_ratio,
            max_minutes,
        )?;
        Ok(ok(value))
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

/// `nativeDeviation(pointsJson, lat, lon)`: how far a position is from a route.
///
/// Used by the on-device re-planning check: "off-route by more than 50 m" has to
/// be measured against the polyline, not against the previous waypoint.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeDeviation(
    mut env: JNIEnv,
    _class: JClass,
    points_json: JString,
    latitude: jdouble,
    longitude: jdouble,
) -> jstring {
    let result: Result<String, String> = (|| {
        let text = string_arg(&mut env, &points_json)?;
        let points: Vec<LatLng> = parse_json(&text, "points")?;
        let position = LatLng::new(latitude, longitude);
        let distance = motu_core::geo::distance_to_polyline_m(position, &points)
            .ok_or_else(|| "route has fewer than two points".to_owned())?;
        Ok(ok(json!({ "distanceM": distance })))
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeReplanDecision(deviationM, improvement, overlap, msSinceSwitch, hasCandidate)`.
///
/// `msSinceSwitch` is negative when the controller never switched before.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeReplanDecision(
    mut env: JNIEnv,
    _class: JClass,
    deviation_m: jdouble,
    improvement: jdouble,
    overlap: jdouble,
    ms_since_switch: jdouble,
    has_candidate: jboolean,
) -> jstring {
    // 这个决策不会失败：参数都是数值，没有可返回的错误分支。
    let policy = ReplanPolicy::default();
    let since_switch = if ms_since_switch >= 0.0 {
        Duration::try_from_secs_f64(ms_since_switch / 1000.0).ok()
    } else {
        None
    };
    let candidate = (has_candidate != 0).then_some(Candidate {
        improvement,
        overlap,
    });
    let decision = evaluate_after(&policy, since_switch, deviation_m, candidate);
    let reason = match decision {
        Some(ReplanReason::Deviation) => "deviation",
        Some(ReplanReason::BetterRoute) => "betterRoute",
        None => "keep",
    };
    jstring_out(
        &mut env,
        ok(json!({
            "replan": decision.is_some(),
            "reason": reason,
            "deviationThresholdM": policy.deviation_m,
            "improvementThreshold": policy.improvement,
            "maxOverlap": policy.max_overlap,
            "minIntervalMs": policy.min_interval.as_millis(),
        })),
    )
}

/// `nativeRouteOverlap(aPointsJson, bPointsJson)`: share of route A within 50 m of B.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeRouteOverlap(
    mut env: JNIEnv,
    _class: JClass,
    a_points_json: JString,
    b_points_json: JString,
) -> jstring {
    let result: Result<String, String> = (|| {
        let a_text = string_arg(&mut env, &a_points_json)?;
        let b_text = string_arg(&mut env, &b_points_json)?;
        let a: Vec<LatLng> = parse_json(&a_text, "pointsA")?;
        let b: Vec<LatLng> = parse_json(&b_text, "pointsB")?;
        Ok(ok(json!({ "overlap": route_overlap(&a, &b, 50.0) })))
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeRouteFlicker(referenceJson, freshJson, lat, lon)`: how much the route
/// really changed after re-planning at `(lat, lon)`.
///
/// Comparing a re-planned route against the *whole* previous route is wrong when
/// the new route starts mid-way: the part already ridden is gone, so overlap
/// collapses for reasons that have nothing to do with flicker. The honest
/// comparison is against the remaining tail, starting at the nearest vertex.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeRouteFlicker(
    mut env: JNIEnv,
    _class: JClass,
    reference_json: JString,
    fresh_json: JString,
    latitude: jdouble,
    longitude: jdouble,
) -> jstring {
    let result: Result<String, String> = (|| {
        let reference_text = string_arg(&mut env, &reference_json)?;
        let fresh_text = string_arg(&mut env, &fresh_json)?;
        let reference: Vec<LatLng> = parse_json(&reference_text, "reference")?;
        let fresh: Vec<LatLng> = parse_json(&fresh_text, "fresh")?;
        let position = LatLng::new(latitude, longitude);
        let nearest = reference
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let da = motu_routing::graph::haversine_m(position.latitude, position.longitude, a.latitude, a.longitude);
                let db = motu_routing::graph::haversine_m(position.latitude, position.longitude, b.latitude, b.longitude);
                da.total_cmp(&db)
            })
            .map(|(index, _)| index)
            .ok_or_else(|| "reference route is empty".to_owned())?;
        let tail = &reference[nearest..];
        let overlap = if tail.len() >= 2 {
            route_overlap(&fresh, tail, 50.0)
        } else {
            1.0
        };
        Ok(ok(json!({ "overlap": overlap, "tailIndex": nearest, "tailPoints": tail.len() })))
    })();
    jstring_out(&mut env, result.unwrap_or_else(err))
}

/// `nativeTile(pmtilesPath, z, x, y)`: raw (gzip) tile bytes, or `null` when the
/// archive has no such tile. The Java side serves these over loopback so
/// `MapLibre` can consume them as a normal vector source.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_manisroad_app_core_MotuCore_nativeTile<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
    zoom: jint,
    x: jint,
    y: jint,
) -> jni::sys::jbyteArray {
    let result: Result<Option<Vec<u8>>, String> = (|| {
        let path = string_arg(&mut env, &path)?;
        let zoom = u8::try_from(zoom).map_err(|_| format!("bad zoom {zoom}"))?;
        let x = u32::try_from(x).map_err(|_| format!("bad x {x}"))?;
        let y = u32::try_from(y).map_err(|_| format!("bad y {y}"))?;
        with_state(|state| {
            let tiles = tiles_for(state, &path)?;
            let tile = tiles.tile(zoom, x, y).map_err(|error| error.to_string())?;
            Ok(tile.map(<[u8]>::to_vec))
        })
    })();
    match result {
        Ok(Some(bytes)) => match env.byte_array_from_slice(&bytes) {
            Ok(array) => array.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Ok(None) => std::ptr::null_mut(),
        Err(error) => {
            let _ = env.throw_new("java/lang/IllegalStateException", error);
            std::ptr::null_mut()
        }
    }
}

#[cfg(test)]
mod tests {
    use motu_routing::{GraphNode, RoadGraph};

    #[test]
    fn graph_cache_loads_and_reuses_a_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("graph.bin");
        let graph = RoadGraph {
            nodes: vec![GraphNode {
                id: 1,
                lat: 31.2304,
                lon: 121.4737,
            }],
            signals: Vec::new(),
            ways: Vec::new(),
            offsets: vec![0, 0],
            edges: Vec::new(),
        };
        graph.save(&path).expect("save");

        let mut state = FfiState::default();
        let text = path.to_str().expect("path");
        assert_eq!(graph_for(&mut state, text).expect("load").node_count(), 1);
        // Second call must reuse the cached graph rather than reloading.
        assert_eq!(graph_for(&mut state, text).expect("cached").edge_count(), 0);
    }

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
