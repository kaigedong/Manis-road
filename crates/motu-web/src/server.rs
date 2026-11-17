//! Local HTTP API and static UI server.
//!
//! Every expensive read goes through the SQLite cache first: camera points are
//! served from disk after the first sync, camera details are reused until their
//! TTL expires, and images are fetched at most once. The upstream Motu service
//! is only contacted on a cache miss or an explicit refresh.

use std::{
    io::Read,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Query, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use motu_routing::graph::haversine_m;
use motu_routing::{CameraDeterrent, RoadGraph, RoutingError, cameras_within};
use motu_core::api::ReqwestTransport;
use motu_core::{
    BASE_URL, CameraBounds, CameraCache, CameraDetail, CachedDetail, CoreState, FileStore,
    HeaderInit, LatLng, MapAreaSnapshot, OfficialCameraQuery, PlateColor, auth, base_headers,
    default_tab_types, gcj02_to_wgs84, load_camera_detail, load_map_areas, safe_env_token,
    sync_official_cameras, wgs84_to_gcj02,
};
use serde_json::{Value, json};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer, trace::TraceLayer};

use crate::{
    config::WebConfig,
    error::ApiError,
    model::{
        AreaView, AreasResponse, AvoidanceInfo, BasemapQuery, CameraQuery, CameraSnapshot,
        DetailRequest, DetailResponse, ImageQuery, LoginRequest, NearbyQuery, RefreshQuery,
        RouteComparison, RouteRequest, RouteResponse, SyncRequest, WebCamera,
    },
};

/// Hard cap for one cached image; ticket photos are small JPEGs.
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub store: FileStore,
    pub config: Arc<WebConfig>,
    /// Loaded offline road graph; `None` disables `/api/route`.
    pub graph: Option<Arc<RoadGraph>>,
}

pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/config", get(config_view))
        .route("/api/state", get(state_view))
        .route("/api/cameras", get(cameras))
        .route("/api/cameras/nearby", get(nearby_cameras))
        .route("/api/basemap", get(basemap))
        .route("/api/export/cameras", get(export_cameras))
        .route("/api/export/details", get(export_details))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/sync", post(sync))
        .route("/api/detail", post(detail))
        .route("/api/areas", get(areas))
        .route("/api/route", post(route))
        .route("/api/cache", get(cache_status))
        .route("/api/cache/clear", post(cache_clear))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .with_state(Arc::clone(&state));

    // Images keep their own cache headers, so they are mounted separately from
    // the `no-store` API layer.
    let images = Router::new()
        .route("/api/image", get(image))
        .with_state(state);

    Router::new()
        .merge(api)
        .merge(images)
        .fallback_service(ServeDir::new(static_dir()).append_index_html_on_directories(true))
        // The debugger is edited live; never let a stale app.js/style.css hide
        // a new feature behind the browser cache.
        .layer(middleware::from_fn(no_cache_static))
        .layer(TraceLayer::new_for_http())
}

async fn no_cache_static(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    if path == "/"
        || path.ends_with(".html")
        || path.ends_with(".js")
        || path.ends_with(".css")
    {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, must-revalidate"),
        );
    }
    response
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
        "signatureSource": state.config.signature_source.as_str(),
        "cacheTtlSecs": state.config.cache_ttl_secs,
        "syncTtlSecs": state.config.sync_ttl_secs,
        "offline": state.config.offline,
        "graph": graph_view(state.graph.as_deref()),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn state_view(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let state = Arc::clone(&state);
    blocking(move || {
        let core = load_core(&state)?;
        let snapshot = camera_snapshot(&core, &state.config, None)?;
        let stats = open_cache(&state.config)
            .and_then(|cache| {
                cache
                    .stats()
                    .map_err(|error| ApiError::internal(format!("cannot read cache stats: {error}")))
            })
            .unwrap_or_default();
        Ok(Json(state_payload(&core, &state.config, &snapshot, &stats)))
    })
    .await
}

async fn cameras(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CameraQuery>,
) -> Result<Json<CameraSnapshot>, ApiError> {
    let bounds = query
        .bbox
        .as_deref()
        .and_then(parse_bbox)
        .and_then(wgs_bounds_to_gcj);
    let state = Arc::clone(&state);
    blocking(move || {
        let core = load_core(&state)?;
        camera_snapshot(&core, &state.config, bounds).map(Json)
    })
    .await
}

async fn export_cameras(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let state = Arc::clone(&state);
    blocking(move || {
        let cache = open_cache(&state.config)?;
        let rows = cache
            .cameras()
            .map_err(|error| ApiError::internal(format!("cannot read camera cache: {error}")))?;
        let cameras: Vec<Value> = rows
            .iter()
            .map(|row| {
                // Android has no GCJ-02 map SDK; export WGS-84 so GNSS
                // distances and the schematic canvas are both correct.
                let mut camera = row.camera.clone();
                camera.position = gcj02_to_wgs84(camera.position);
                json!({
                    "id": camera.id,
                    "position": camera.position,
                    "tabTypes": camera.tab_types.iter().map(|tab| tab.id()).collect::<Vec<_>>(),
                    "member": camera.member,
                    "memberByString": camera.member_by_string,
                    "newFlag": camera.new_flag,
                    "white": camera.white,
                })
            })
            .collect();
        Ok(Json(json!({
            "generatedAtMs": now_ms(),
            "count": cameras.len(),
            "cameras": cameras,
        })))
    })
    .await
}

async fn export_details(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let state = Arc::clone(&state);
    blocking(move || {
        let cache = open_cache(&state.config)?;
        let rows = cache
            .all_details()
            .map_err(|error| ApiError::internal(format!("cannot read detail cache: {error}")))?;
        let mut details = serde_json::Map::new();
        for (wire_id, cached) in &rows {
            details.insert(
                wire_id.clone(),
                serde_json::to_value(&cached.detail)
                    .map_err(|error| ApiError::internal(error.to_string()))?,
            );
        }
        Ok(Json(json!({
            "generatedAtMs": now_ms(),
            "count": details.len(),
            "details": details,
        })))
    })
    .await
}

async fn basemap(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BasemapQuery>,
) -> Result<Json<Value>, ApiError> {
    let Some(bounds) = parse_bbox(&query.bbox) else {
        return Err(ApiError::bad_request("bbox 需要 south,west,north,east"));
    };
    let Some(graph) = state.graph.clone() else {
        return Ok(Json(json!({
            "segments": [],
            "truncated": false,
            "graph": false,
            "zoom": query.zoom.unwrap_or(12),
        })));
    };
    let zoom = query.zoom.unwrap_or(12);
    // Low zooms only need the arterial network; high zooms include everything.
    let max_level: u8 = match zoom {
        0..=12 => 1,
        13..=14 => 2,
        _ => 3,
    };
    blocking(move || {
        let (segments, truncated) = graph.basemap_segments(
            bounds.south,
            bounds.west,
            bounds.north,
            bounds.east,
            max_level,
            8_000,
        );
        Ok(Json(json!({
            "segments": segments,
            "truncated": truncated,
            "graph": true,
            "zoom": zoom,
            "maxLevel": max_level,
        })))
    })
    .await
}

async fn nearby_cameras(
    State(state): State<Arc<AppState>>,
    Query(query): Query<NearbyQuery>,
) -> Result<Json<Value>, ApiError> {
    let point = LatLng::new(query.lat, query.lon);
    if !point.is_valid() {
        return Err(ApiError::bad_request("坐标无效"));
    }
    let radius = query.radius.unwrap_or(1000.0).clamp(50.0, 5000.0);
    let state = Arc::clone(&state);
    blocking(move || {
        let gcj = wgs84_to_gcj02(point);
        let delta = radius / 111_000.0 * 1.4;
        let bounds = CameraBounds::new(
            gcj.latitude - delta,
            gcj.longitude - delta,
            gcj.latitude + delta,
            gcj.longitude + delta,
        );
        let cache = open_cache(&state.config)?;
        let rows = cache
            .cameras_in(bounds)
            .map_err(|error| ApiError::internal(format!("cannot read camera cache: {error}")))?;
        let mut cameras: Vec<(f64, WebCamera)> = rows
            .iter()
            .map(|row| {
                let mut web = WebCamera::from_official(&row.camera);
                let distance = haversine_m(
                    point.latitude,
                    point.longitude,
                    web.position.latitude,
                    web.position.longitude,
                );
                web.distance_m = Some(distance);
                (distance, web)
            })
            .filter(|(distance, _)| *distance <= radius)
            .collect();
        cameras.sort_by(|left, right| {
            left.0
                .partial_cmp(&right.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        cameras.truncate(30);
        Ok(Json(json!({
            "cameras": cameras.into_iter().map(|(_, camera)| camera).collect::<Vec<_>>(),
            "radius": radius,
        })))
    })
    .await
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

    ensure_online(&state.config)?;
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
        // The cache is keyed by this device; logging in does not discard it.
        let snapshot = camera_snapshot(&core, &state.config, None)?;
        let stats = cache_stats(&state.config)?;
        Ok(Json(json!({
            "loggedIn": true,
            "user": session_user(&core),
            "state": state_payload(&core, &state.config, &snapshot, &stats),
        })))
    })
    .await
}

async fn logout(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    ensure_online(&state.config)?;
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
        let snapshot = camera_snapshot(&core, &state.config, None)?;
        let stats = cache_stats(&state.config)?;
        Ok(Json(json!({
            "loggedIn": false,
            "warning": warning,
            "state": state_payload(&core, &state.config, &snapshot, &stats),
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
    let viewport = request_viewport(&request).and_then(wgs_bounds_to_gcj);

    ensure_online(&state.config)?;
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
        let now = now_ms();
        let mut cache = open_cache(&state.config)?;

        // Cache-first: if a previous sync already covered this viewport and it
        // is still fresh, answer locally without touching Motu.
        if !request.force {
            if let Some(bounds) = viewport {
                let covered = cache
                    .region_covering(bounds)
                    .map_err(|error| {
                        ApiError::internal(format!("cannot read sync regions: {error}"))
                    })?;
                if let Some(region) = covered {
                    if is_fresh(region.synced_at_ms, now, sync_ttl_ms(&state.config)) {
                        let rows = cache.cameras_in(bounds).map_err(|error| {
                            ApiError::internal(format!("cannot read camera cache: {error}"))
                        })?;
                        let snapshot = snapshot_from_rows(rows, &city);
                        let stats = cache.stats().map_err(|error| {
                            ApiError::internal(format!("cannot read cache stats: {error}"))
                        })?;
                        return Ok(Json(json!({
                            "snapshot": snapshot,
                            "synced": 0,
                            "skipped": true,
                            "syncedAtMs": region.synced_at_ms,
                            "state": state_payload(&core, &state.config, &snapshot, &stats),
                        })));
                    }
                }
            }
        }

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
            cruise: request.cruise,
            source: 1,
        };
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
        let synced = sync.cameras.len();
        cache
            .upsert_cameras(&sync.cameras, &city, sync.fetched_at_ms)
            .map_err(|error| ApiError::internal(format!("cannot write camera cache: {error}")))?;
        if let Some(region) = region_bounds(&sync.cameras) {
            cache
                .record_region(&city, region, sync.fetched_at_ms)
                .map_err(|error| {
                    ApiError::internal(format!("cannot write sync region: {error}"))
                })?;
        }
        let rows = match viewport {
            Some(bounds) => cache.cameras_in(bounds),
            None => cache.cameras(),
        }
        .map_err(|error| ApiError::internal(format!("cannot read camera cache: {error}")))?;
        let snapshot = snapshot_from_rows(rows, &city);
        let stats = cache
            .stats()
            .map_err(|error| ApiError::internal(format!("cannot read cache stats: {error}")))?;
        let core = state
            .store
            .update(|state| state.settings.city_code = city)
            .map_err(|error| ApiError::internal(format!("cannot save city code: {error}")))?;
        Ok(Json(json!({
            "snapshot": snapshot,
            "synced": synced,
            "skipped": false,
            "syncedAtMs": null,
            "state": state_payload(&core, &state.config, &snapshot, &stats),
        })))
    })
    .await
}

async fn detail(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DetailRequest>,
) -> Result<Json<DetailResponse>, ApiError> {
    let camera_id = request.camera_id.trim().to_owned();
    if camera_id.is_empty() {
        return Err(ApiError::bad_request("cameraId 不能为空"));
    }

    let state = Arc::clone(&state);
    blocking(move || {
        let core = load_core(&state)?;
        let ttl_ms = cache_ttl_ms(&state.config);
        let now = now_ms();

        // Cache-first: a fresh payload is returned without touching the
        // network, and it does not require a live session.
        let cached = open_cache(&state.config)?
            .detail(&camera_id)
            .map_err(|error| ApiError::internal(format!("cannot read detail cache: {error}")))?;
        if let Some(cached) = &cached {
            if state.config.offline || (!request.refresh && is_fresh(cached.fetched_at_ms, now, ttl_ms))
            {
                let warning = state
                    .config
                    .offline
                    .then(|| "离线模式：详情来自本地缓存".to_owned());
                return Ok(Json(detail_response(cached, true, ttl_ms, warning)));
            }
        }
        ensure_online(&state.config)?;

        let fetched = fetch_detail(&state, &core, &camera_id, now);
        match fetched {
            Ok(detail) => {
                let mut cache = open_cache(&state.config)?;
                cache
                    .put_detail(&camera_id, &detail, now)
                    .map_err(|error| {
                        ApiError::internal(format!("cannot write detail cache: {error}"))
                    })?;
                let cached = CachedDetail {
                    detail,
                    fetched_at_ms: now,
                };
                Ok(Json(detail_response(&cached, false, ttl_ms, None)))
            }
            Err(error) => {
                // Stale data beats an empty panel, as long as we say so.
                if let Some(cached) = cached {
                    return Ok(Json(detail_response(
                        &cached,
                        true,
                        ttl_ms,
                        Some(error.to_string()),
                    )));
                }
                Err(error)
            }
        }
    })
    .await
}

async fn areas(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RefreshQuery>,
) -> Result<Json<AreasResponse>, ApiError> {
    let state = Arc::clone(&state);
    blocking(move || {
        let core = load_core(&state)?;
        let ttl_ms = cache_ttl_ms(&state.config);
        let now = now_ms();
        let mut cache = open_cache(&state.config)?;
        let cached = cache
            .areas()
            .map_err(|error| ApiError::internal(format!("cannot read area cache: {error}")))?;
        if let Some(snapshot) = &cached {
            if state.config.offline || (!query.refresh && is_fresh(snapshot.fetched_at_ms, now, ttl_ms))
            {
                let warning = state
                    .config
                    .offline
                    .then(|| "离线模式：区域图层来自本地缓存".to_owned());
                return Ok(Json(areas_response(snapshot, true, ttl_ms, warning)));
            }
        }
        ensure_online(&state.config)?;

        let headers = auth_headers(&core, &state.config, now);
        let transport = ReqwestTransport::new()
            .map_err(|error| ApiError::upstream(format!("cannot create HTTP client: {error}")))?;
        match load_map_areas(
            &transport,
            &base_url(&core, &state.config),
            &headers,
            now,
        ) {
            Ok(snapshot) => {
                cache.put_areas(&snapshot).map_err(|error| {
                    ApiError::internal(format!("cannot write area cache: {error}"))
                })?;
                Ok(Json(areas_response(&snapshot, false, ttl_ms, None)))
            }
            Err(error) => {
                if let Some(snapshot) = cached {
                    return Ok(Json(areas_response(
                        &snapshot,
                        true,
                        ttl_ms,
                        Some(error.to_string()),
                    )));
                }
                Err(ApiError::from(error))
            }
        }
    })
    .await
}

fn areas_response(
    snapshot: &MapAreaSnapshot,
    from_cache: bool,
    ttl_ms: i64,
    warning: Option<String>,
) -> AreasResponse {
    AreasResponse {
        version: snapshot.version,
        fetched_at_ms: snapshot.fetched_at_ms,
        expires_at_ms: (ttl_ms > 0).then(|| snapshot.fetched_at_ms.saturating_add(ttl_ms)),
        cached: from_cache,
        warning,
        areas: snapshot
            .areas
            .iter()
            .map(|area| AreaView {
                id: area.id,
                name: area.name.clone(),
                shape_type: area.shape_type.clone(),
                // The official layer is `GCJ-02`, same as camera points.
                points: area.points.iter().copied().map(gcj02_to_wgs84).collect(),
            })
            .collect(),
    }
}

async fn route(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RouteRequest>,
) -> Result<Json<RouteResponse>, ApiError> {
    let Some(graph) = state.graph.clone() else {
        return Err(ApiError::unavailable(
            "离线路网尚未构建：先运行 motu-graph build 生成 graph.bin",
        ));
    };
    let radius = request.camera_radius_m.unwrap_or(60.0).clamp(20.0, 300.0);

    let state = Arc::clone(&state);
    blocking(move || {
        let baseline = graph
            .route(request.origin, request.destination)
            .map_err(route_error)?;
        if !request.avoid_cameras {
            return Ok(Json(RouteResponse {
                plan: baseline,
                baseline: None,
                avoidance: None,
            }));
        }

        let cameras: Vec<LatLng> = {
            let cache = open_cache(&state.config)?;
            let rows = cache.cameras().map_err(|error| {
                ApiError::internal(format!("cannot read camera cache: {error}"))
            })?;
            rows.iter()
                .map(|row| gcj02_to_wgs84(row.camera.position))
                .collect()
        };
        if cameras.is_empty() {
            let comparison = RouteComparison {
                distance_m: baseline.distance_m,
                duration_s: baseline.duration_s,
                camera_hits: 0,
            };
            return Ok(Json(RouteResponse {
                plan: baseline,
                baseline: Some(comparison),
                avoidance: Some(AvoidanceInfo {
                    applied: false,
                    rejected: false,
                    reason: Some("没有缓存的电子眼点位".to_owned()),
                    cameras_before: 0,
                    cameras_after: 0,
                    radius_m: radius,
                    detour_ratio: 0.0,
                    extra_seconds: 0.0,
                }),
            }));
        }

        let before = cameras_within(&baseline.points, &cameras, radius);
        let comparator = RouteComparison {
            distance_m: baseline.distance_m,
            duration_s: baseline.duration_s,
            camera_hits: before,
        };
        if before == 0 {
            return Ok(Json(RouteResponse {
                plan: baseline,
                baseline: Some(comparator),
                avoidance: Some(AvoidanceInfo {
                    applied: false,
                    rejected: false,
                    reason: Some(format!("本路线 {radius:.0} m 内没有已知点位")),
                    cameras_before: 0,
                    cameras_after: 0,
                    radius_m: radius,
                    detour_ratio: 0.0,
                    extra_seconds: 0.0,
                }),
            }));
        }

        // First try hard avoidance (camera road segments are removed). If the
        // network gets disconnected, fall back to a very high time penalty so
        // the route still avoids as many cameras as possible. A route that
        // cannot reduce the known hits is reported as an error, never silently
        // replaced by the original road.
        let hard = CameraDeterrent::build_hard(&graph, &cameras, radius);
        let candidate = match graph.route_with(request.origin, request.destination, Some(&hard)) {
            Ok(candidate) => candidate,
            Err(_) => {
                let soft = CameraDeterrent::build(&graph, &cameras, radius, 50.0);
                match graph.route_with(request.origin, request.destination, Some(&soft)) {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        return Err(ApiError::unprocessable(format!(
                            "强制避开电子眼后无可行路线（{error}）；可减小半径或取消勾选"
                        )));
                    }
                }
            }
        };
        let after = cameras_within(&candidate.points, &cameras, radius);
        if after >= before {
            return Err(ApiError::unprocessable(format!(
                "强制避开电子眼后仍有 {after} 个已知点位，无法减少；可减小半径或取消勾选"
            )));
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
        Ok(Json(RouteResponse {
            plan: candidate,
            baseline: Some(comparator),
            avoidance: Some(AvoidanceInfo {
                applied: true,
                rejected: false,
                reason,
                cameras_before: before,
                cameras_after: after,
                radius_m: radius,
                detour_ratio: ratio,
                extra_seconds: extra,
            }),
        }))
    })
    .await
}

fn route_error(error: RoutingError) -> ApiError {
    let message = error.to_string();
    match error {
        RoutingError::InvalidCoordinate | RoutingError::SnapFailed | RoutingError::NoPath => {
            ApiError::bad_request(message)
        }
        RoutingError::GraphMissing => ApiError::unavailable(message),
        _ => ApiError::internal(message),
    }
}

fn ensure_online(config: &WebConfig) -> Result<(), ApiError> {
    if config.offline {
        return Err(ApiError::unavailable(
            "离线模式已开启：本机不会访问任何外部网络",
        ));
    }
    Ok(())
}

fn graph_view(graph: Option<&RoadGraph>) -> Value {
    match graph {
        None => json!({ "loaded": false }),
        Some(graph) => {
            let bounds = graph.bounds().map(|(south, west, north, east)| {
                json!({
                    "south": south,
                    "west": west,
                    "north": north,
                    "east": east,
                })
            });
            json!({
                "loaded": true,
                "nodes": graph.node_count(),
                "edges": graph.edge_count(),
                "bounds": bounds,
            })
        }
    }
}

fn fetch_detail(
    state: &AppState,
    core: &CoreState,
    camera_id: &str,
    now: i64,
) -> Result<CameraDetail, ApiError> {
    require_session(core)?;
    let headers = auth_headers(core, &state.config, now);
    let transport = ReqwestTransport::new()
        .map_err(|error| ApiError::upstream(format!("cannot create HTTP client: {error}")))?;
    load_camera_detail(
        &transport,
        &base_url(core, &state.config),
        &headers,
        camera_id,
        now,
    )
    .map_err(ApiError::from)
}

async fn cache_status(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let state = Arc::clone(&state);
    blocking(move || {
        let stats = cache_stats(&state.config)?;
        Ok(Json(json!({
            "stats": stats,
            "ttlSecs": state.config.cache_ttl_secs,
            "path": cache_path(&state.config).display().to_string(),
        })))
    })
    .await
}

async fn cache_clear(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let state = Arc::clone(&state);
    blocking(move || {
        let mut cache = open_cache(&state.config)?;
        cache
            .clear()
            .map_err(|error| ApiError::internal(format!("cannot clear cache: {error}")))?;
        let stats = cache_stats(&state.config)?;
        Ok(Json(json!({
            "cleared": true,
            "stats": stats,
        })))
    })
    .await
}

async fn image(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ImageQuery>,
) -> Result<Response, ApiError> {
    let url = query.url.trim().to_owned();
    if !is_fetchable_image_url(&url) {
        return Err(ApiError::bad_request("图片地址必须是 http(s) URL"));
    }

    let state = Arc::clone(&state);
    blocking(move || {
        // Signed OSS URLs rotate their query string on every detail refresh;
        // the object path (plus resize/watermark parameters) identifies the
        // actual image.
        let key = image_cache_key(&url);
        let mut cache = open_cache(&state.config)?;
        if let Some(cached) = cache
            .image(&key)
            .map_err(|error| ApiError::internal(format!("cannot read image cache: {error}")))?
        {
            return Ok(image_response(&cached.content_type, cached.bytes));
        }
        ensure_online(&state.config)?;
        let (content_type, bytes) = download_image(&url)?;
        cache
            .put_image(&key, &content_type, &bytes, now_ms())
            .map_err(|error| ApiError::internal(format!("cannot write image cache: {error}")))?;
        Ok(image_response(&content_type, bytes))
    })
    .await
}

/// Stable image cache key: scheme, host, path and content-affecting query
/// parameters. Volatile OSS authorization parameters (`Expires`,
/// `OSSAccessKeyId`, `Signature`, `security-token`) are dropped so a re-signed
/// URL for the same photo still hits the cache.
fn image_cache_key(url: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return url.to_owned();
    };
    let mut key = String::new();
    key.push_str(parsed.scheme());
    key.push_str("://");
    key.push_str(parsed.host_str().unwrap_or_default());
    if let Some(port) = parsed.port() {
        key.push(':');
        key.push_str(&port.to_string());
    }
    key.push_str(parsed.path());

    let mut content: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(name, _)| {
            matches!(
                name.as_ref(),
                "x-oss-process" | "imageMogr2" | "imageView2" | "w" | "h"
            )
        })
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    content.sort();
    if !content.is_empty() {
        key.push('?');
        for (index, (name, value)) in content.iter().enumerate() {
            if index > 0 {
                key.push('&');
            }
            key.push_str(name);
            key.push('=');
            key.push_str(value);
        }
    }
    key
}

fn image_response(content_type: &str, bytes: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type.to_owned()),
            (
                header::CACHE_CONTROL,
                "public, max-age=604800, immutable".to_owned(),
            ),
        ],
        bytes,
    )
        .into_response()
}

fn download_image(url: &str) -> Result<(String, Vec<u8>), ApiError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(format!("manis-motu-web/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| ApiError::upstream(format!("cannot create HTTP client: {error}")))?;
    let response = client
        .get(url)
        .send()
        .map_err(|error| ApiError::upstream(format!("image download failed: {error}")))?;
    if !response.status().is_success() {
        return Err(ApiError::upstream(format!(
            "image download failed with HTTP {}",
            response.status()
        )));
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_owned())
        .unwrap_or_default();
    if !content_type.starts_with("image/") {
        return Err(ApiError::upstream(format!(
            "unexpected content type: {content_type}"
        )));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| ApiError::upstream(format!("image read failed: {error}")))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(ApiError::upstream("image is larger than 8 MiB"));
    }
    Ok((content_type, bytes))
}

fn is_fetchable_image_url(url: &str) -> bool {
    if url.len() > 2048 {
        return false;
    }
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") {
        return false;
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(address) = bare.parse::<std::net::IpAddr>() {
        let blocked = match address {
            std::net::IpAddr::V4(address) => {
                address.is_loopback()
                    || address.is_private()
                    || address.is_link_local()
                    || address.is_unspecified()
            }
            std::net::IpAddr::V6(address) => {
                address.is_loopback() || address.is_unspecified()
            }
        };
        if blocked {
            return false;
        }
    }
    true
}

fn state_payload(
    core: &CoreState,
    config: &WebConfig,
    snapshot: &CameraSnapshot,
    stats: &motu_core::CacheStats,
) -> Value {
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
        "offline": config.offline,
        "cameraSource": snapshot.source,
        "cameraCount": stats.cameras,
        "fetchedAtMs": snapshot.fetched_at_ms,
        "viewportCount": snapshot.cameras.len(),
        "cache": stats,
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

fn cache_path(config: &WebConfig) -> PathBuf {
    config.data_dir.join("cache.sqlite")
}

fn open_cache(config: &WebConfig) -> Result<CameraCache, ApiError> {
    CameraCache::open(&cache_path(config))
        .map_err(|error| ApiError::internal(format!("cannot open camera cache: {error}")))
}

fn cache_stats(config: &WebConfig) -> Result<motu_core::CacheStats, ApiError> {
    open_cache(config)?
        .stats()
        .map_err(|error| ApiError::internal(format!("cannot read cache stats: {error}")))
}

fn sync_ttl_ms(config: &WebConfig) -> i64 {
    i64::try_from(config.sync_ttl_secs.saturating_mul(1000)).unwrap_or(i64::MAX)
}

/// Padding added around the extent of a sync result, in degrees (~500 m).
const REGION_PAD: f64 = 0.005;

fn request_viewport(request: &SyncRequest) -> Option<CameraBounds> {
    let (Some(south), Some(west), Some(north), Some(east)) = (
        request.south,
        request.west,
        request.north,
        request.east,
    ) else {
        return None;
    };
    let bounds = CameraBounds::new(south, west, north, east);
    bounds.is_valid().then_some(bounds)
}

/// Extent of the points the server returned (`GCJ-02`), padded so a viewport
/// at the edge still counts as covered.
fn region_bounds(cameras: &[motu_core::TrafficCamera]) -> Option<CameraBounds> {
    let mut iter = cameras
        .iter()
        .map(|camera| camera.position)
        .filter(LatLng::is_valid);
    let first = iter.next()?;
    let (mut south, mut west, mut north, mut east) = (
        first.latitude,
        first.longitude,
        first.latitude,
        first.longitude,
    );
    for position in iter {
        south = south.min(position.latitude);
        north = north.max(position.latitude);
        west = west.min(position.longitude);
        east = east.max(position.longitude);
    }
    let bounds = CameraBounds::new(
        south - REGION_PAD,
        west - REGION_PAD,
        north + REGION_PAD,
        east + REGION_PAD,
    );
    bounds.is_valid().then_some(bounds)
}

fn cache_ttl_ms(config: &WebConfig) -> i64 {
    i64::try_from(config.cache_ttl_secs.saturating_mul(1000)).unwrap_or(i64::MAX)
}

fn is_fresh(fetched_at_ms: i64, now_ms: i64, ttl_ms: i64) -> bool {
    // `0` means "never expire": ticket data is historical and only refreshed
    // explicitly.
    ttl_ms <= 0 || now_ms.saturating_sub(fetched_at_ms) < ttl_ms
}

fn detail_response(
    cached: &CachedDetail,
    from_cache: bool,
    ttl_ms: i64,
    warning: Option<String>,
) -> DetailResponse {
    let mut detail = cached.detail.clone();
    // The cache keeps `GCJ-02`; the browser draws on WGS-84 tiles.
    if let Some(position) = detail.position {
        detail.position = Some(gcj02_to_wgs84(position));
    }
    DetailResponse {
        detail,
        cached: from_cache,
        fetched_at_ms: cached.fetched_at_ms,
        expires_at_ms: (ttl_ms > 0).then(|| cached.fetched_at_ms.saturating_add(ttl_ms)),
        warning,
    }
}

fn camera_snapshot(
    core: &CoreState,
    config: &WebConfig,
    bounds: Option<CameraBounds>,
) -> Result<CameraSnapshot, ApiError> {
    let cache = open_cache(config)?;
    let stats = cache
        .stats()
        .map_err(|error| ApiError::internal(format!("cannot read cache stats: {error}")))?;
    if stats.cameras == 0 {
        return Ok(CameraSnapshot::demo(core.settings.city_code.clone()));
    }
    let rows = match bounds {
        Some(bounds) => cache.cameras_in(bounds),
        None => cache.cameras(),
    }
    .map_err(|error| ApiError::internal(format!("cannot read camera cache: {error}")))?;
    Ok(snapshot_from_rows(rows, &core.settings.city_code))
}

fn snapshot_from_rows(
    rows: Vec<motu_core::CachedCamera>,
    fallback_city: &str,
) -> CameraSnapshot {
    let fetched_at_ms = rows.iter().map(|row| row.last_synced_ms).max();
    let city_code = rows
        .iter()
        .map(|row| row.city_code.trim())
        .find(|city| !city.is_empty())
        .map_or_else(|| fallback_city.to_owned(), str::to_owned);
    CameraSnapshot {
        source: "cache".to_owned(),
        city_code,
        fetched_at_ms,
        cameras: rows
            .iter()
            .map(|row| WebCamera::from_official(&row.camera))
            .collect(),
    }
}

/// Parse `south,west,north,east` from the Leaflet map bounds (WGS-84).
fn parse_bbox(value: &str) -> Option<CameraBounds> {
    let mut parts = value.split(',').map(|part| part.trim().parse::<f64>());
    let south = parts.next()?.ok()?;
    let west = parts.next()?.ok()?;
    let north = parts.next()?.ok()?;
    let east = parts.next()?.ok()?;
    if parts.next().is_some() {
        return None;
    }
    let bounds = CameraBounds::new(south, west, north, east);
    bounds.is_valid().then_some(bounds)
}

/// Convert WGS-84 map bounds to the cache's `GCJ-02` frame, with a small pad so
/// markers near the edge are not clipped by the datum shift.
fn wgs_bounds_to_gcj(bounds: CameraBounds) -> Option<CameraBounds> {
    let corners = [
        wgs84_to_gcj02(LatLng::new(bounds.south, bounds.west)),
        wgs84_to_gcj02(LatLng::new(bounds.south, bounds.east)),
        wgs84_to_gcj02(LatLng::new(bounds.north, bounds.west)),
        wgs84_to_gcj02(LatLng::new(bounds.north, bounds.east)),
    ];
    let south = corners
        .iter()
        .map(|point| point.latitude)
        .fold(f64::INFINITY, f64::min)
        - 0.002;
    let north = corners
        .iter()
        .map(|point| point.latitude)
        .fold(f64::NEG_INFINITY, f64::max)
        + 0.002;
    let west = corners
        .iter()
        .map(|point| point.longitude)
        .fold(f64::INFINITY, f64::min)
        - 0.002;
    let east = corners
        .iter()
        .map(|point| point.longitude)
        .fold(f64::NEG_INFINITY, f64::max)
        + 0.002;
    let expanded = CameraBounds::new(south, west, north, east);
    expanded.is_valid().then_some(expanded)
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
    use crate::config::SignatureSource;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use motu_core::{CameraTabType, TrafficCamera};
    use tower::ServiceExt;

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = FileStore::new(directory.path());
        store.ensure_root().expect("store root");
        store.ensure_identity().expect("identity");
        let config = WebConfig {
            bind: "127.0.0.1:0".parse().expect("socket address"),
            data_dir: directory.path().to_path_buf(),
            api_base_url: String::new(),
            signature_md5: String::new(),
            signature_source: SignatureSource::None,
            cache_ttl_secs: 3600,
            sync_ttl_secs: 3600,
            offline: false,
            graph_path: PathBuf::from("unused"),
            tile_url: "https://tile.openstreetmap.org/{z}/{x}/{y}.png".to_owned(),
            tile_attribution: "test".to_owned(),
        };
        let state = Arc::new(AppState {
            store,
            config: Arc::new(config),
            graph: None,
        });
        (directory, state)
    }

    async fn body_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        serde_json::from_slice(&body).expect("json")
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
        let value = body_json(response).await;
        assert_eq!(value["source"], "demo");
        assert!(!value["cameras"].as_array().expect("cameras").is_empty());
    }

    #[tokio::test]
    async fn cameras_are_served_from_the_cache_after_a_sync() {
        let (directory, state) = test_state();
        let mut cache = CameraCache::open(&directory.path().join("cache.sqlite")).expect("cache");
        let mut camera = TrafficCamera::new(
            "31.23,121.47:4",
            LatLng::new(31.23, 121.47),
            [CameraTabType::RedLight],
        );
        camera.member_by_string = Some("wire-1".to_owned());
        cache
            .upsert_cameras(&[camera], "021", 1_000)
            .expect("insert");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/cameras")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("cameras");
        assert_eq!(response.status(), StatusCode::OK);
        let value = body_json(response).await;
        assert_eq!(value["source"], "cache");
        assert_eq!(value["fetchedAtMs"], 1_000);
        assert_eq!(value["cameras"].as_array().expect("cameras").len(), 1);
        assert_eq!(value["cameras"][0]["official"], true);
    }

    #[tokio::test]
    async fn demo_cameras_are_never_official() {
        let (_directory, state) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/cameras")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("cameras");
        let value = body_json(response).await;
        assert!(
            value["cameras"]
                .as_array()
                .expect("cameras")
                .iter()
                .all(|camera| camera["official"] == false)
        );
    }

    #[tokio::test]
    async fn detail_requires_login_when_the_cache_is_empty() {
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
    async fn cached_detail_is_returned_without_login() {
        let (directory, state) = test_state();
        let mut cache = CameraCache::open(&directory.path().join("cache.sqlite")).expect("cache");
        let detail = CameraDetail {
            camera_id: "camera-1".to_owned(),
            title: "电子摄像头".to_owned(),
            detail: "由东向西抓拍".to_owned(),
            address: "上海市某路口".to_owned(),
            position: Some(LatLng::new(31.230_4, 121.473_7)),
            types: Vec::new(),
            rules: Vec::new(),
            tickets_count: 0,
            praise_count: 4,
            comment_count: 1,
            white: false,
            avoid: false,
            avoid_type: 0,
            contact_person: String::new(),
            contact_way: String::new(),
            user_photo: String::new(),
            photo: String::new(),
            create_time: None,
            avoid_id: None,
            avoid_white_id: None,
            tickets: Vec::new(),
        };
        cache.put_detail("camera-1", &detail, now_ms()).expect("put");

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
        assert_eq!(response.status(), StatusCode::OK);
        let value = body_json(response).await;
        assert_eq!(value["cached"], true);
        assert_eq!(value["title"], "电子摄像头");
        assert_eq!(value["praiseCount"], 4);
        // Cached GCJ-02 position is converted for the WGS-84 map.
        let latitude = value["position"]["latitude"].as_f64().expect("latitude");
        assert!((latitude - 31.230_4).abs() > 0.001);
    }

    #[tokio::test]
    async fn expired_detail_falls_back_after_a_failed_refresh() {
        let (directory, state) = test_state();
        let mut cache = CameraCache::open(&directory.path().join("cache.sqlite")).expect("cache");
        let detail = CameraDetail {
            camera_id: "camera-1".to_owned(),
            title: "旧缓存".to_owned(),
            detail: String::new(),
            address: String::new(),
            position: None,
            types: Vec::new(),
            rules: Vec::new(),
            tickets_count: 0,
            praise_count: 0,
            comment_count: 0,
            white: false,
            avoid: false,
            avoid_type: 0,
            contact_person: String::new(),
            contact_way: String::new(),
            user_photo: String::new(),
            photo: String::new(),
            create_time: None,
            avoid_id: None,
            avoid_white_id: None,
            tickets: Vec::new(),
        };
        // Older than the 1 h TTL in `test_state`, and no session to refresh with.
        cache
            .put_detail("camera-1", &detail, now_ms() - 7_200_000)
            .expect("put");

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
        assert_eq!(response.status(), StatusCode::OK);
        let value = body_json(response).await;
        assert_eq!(value["cached"], true);
        assert_eq!(value["title"], "旧缓存");
        assert!(value["warning"].as_str().is_some_and(|text| !text.is_empty()));
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

    #[tokio::test]
    async fn cached_areas_are_served_without_login() {
        let (directory, state) = test_state();
        let mut cache = CameraCache::open(&directory.path().join("cache.sqlite")).expect("cache");
        let snapshot = MapAreaSnapshot {
            areas: vec![motu_core::MapAreaShape {
                id: 1,
                name: "外牌限行".to_owned(),
                shape_type: motu_core::areas::SHAPE_POLYGON.to_owned(),
                points: vec![LatLng::new(31.20, 121.40), LatLng::new(31.30, 121.50)],
            }],
            version: 4,
            fetched_at_ms: now_ms(),
        };
        cache.put_areas(&snapshot).expect("put");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/areas")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("areas");
        assert_eq!(response.status(), StatusCode::OK);
        let value = body_json(response).await;
        assert_eq!(value["cached"], true);
        assert_eq!(value["version"], 4);
        assert_eq!(value["areas"][0]["name"], "外牌限行");
        assert_eq!(value["areas"][0]["shapeType"], "polygon");
        let latitude = value["areas"][0]["points"][0]["latitude"]
            .as_f64()
            .expect("latitude");
        assert!((latitude - 31.20).abs() > 0.001, "GCJ-02 converted to WGS-84");
    }

    #[tokio::test]
    async fn exported_cameras_are_wgs84_for_android() {
        let (directory, state) = test_state();
        let mut cache = CameraCache::open(&directory.path().join("cache.sqlite")).expect("cache");
        let camera = TrafficCamera::new(
            "31.23,121.47:4",
            LatLng::new(31.23, 121.47),
            [CameraTabType::RedLight],
        );
        cache
            .upsert_cameras(&[camera], "021", 1_000)
            .expect("insert");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/export/cameras")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("export");
        assert_eq!(response.status(), StatusCode::OK);
        let value = body_json(response).await;
        assert_eq!(value["count"], 1);
        let latitude = value["cameras"][0]["position"]["latitude"]
            .as_f64()
            .expect("latitude");
        assert!(
            (latitude - 31.23).abs() > 0.001,
            "export must use WGS-84, not the cached GCJ-02 position"
        );
    }

    #[tokio::test]
    async fn exported_details_keep_tickets() {
        let (directory, state) = test_state();
        let mut cache = CameraCache::open(&directory.path().join("cache.sqlite")).expect("cache");
        let detail = CameraDetail {
            camera_id: "25245".to_owned(),
            title: "东向西".to_owned(),
            detail: "由东向西抓拍".to_owned(),
            address: "某路口".to_owned(),
            position: None,
            types: Vec::new(),
            rules: Vec::new(),
            tickets_count: 1,
            praise_count: 0,
            comment_count: 0,
            white: false,
            avoid: false,
            avoid_type: 0,
            contact_person: String::new(),
            contact_way: String::new(),
            user_photo: String::new(),
            photo: String::new(),
            create_time: None,
            avoid_id: None,
            avoid_white_id: None,
            tickets: Vec::new(),
        };
        cache.put_detail("25245", &detail, 1).expect("put");
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/export/details")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("export");
        let value = body_json(response).await;
        assert_eq!(value["count"], 1);
        assert_eq!(value["details"]["25245"]["title"], "东向西");
        assert_eq!(value["details"]["25245"]["ticketsCount"], 1);
    }

    #[tokio::test]
    async fn nearby_cameras_read_the_cache() {
        let (directory, state) = test_state();
        let mut cache = CameraCache::open(&directory.path().join("cache.sqlite")).expect("cache");
        let camera = TrafficCamera::new(
            "31.23,121.47:4",
            LatLng::new(31.23, 121.47),
            [CameraTabType::RedLight],
        );
        cache
            .upsert_cameras(&[camera], "021", 1_000)
            .expect("insert");
        let point = gcj02_to_wgs84(LatLng::new(31.23, 121.47));

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/cameras/nearby?lat={}&lon={}&radius=800",
                        point.latitude, point.longitude
                    ))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("nearby");
        assert_eq!(response.status(), StatusCode::OK);
        let value = body_json(response).await;
        let cameras = value["cameras"].as_array().expect("cameras");
        assert_eq!(cameras.len(), 1);
        assert!(
            cameras[0]["distanceM"].as_f64().expect("distance") < 20.0,
            "the camera should be almost on the query point"
        );
    }

    #[tokio::test]
    async fn camera_avoidance_can_change_the_route() {
        use motu_routing::graph::Edge;
        use motu_routing::{GraphNode, RoadGraph, WayInfo};
        let nodes = vec![
            GraphNode { id: 0, lat: 31.2300, lon: 121.4700 },
            GraphNode { id: 1, lat: 31.2320, lon: 121.4720 },
            GraphNode { id: 2, lat: 31.2340, lon: 121.4700 },
            GraphNode { id: 3, lat: 31.2340, lon: 121.4720 },
            GraphNode { id: 4, lat: 31.2310, lon: 121.4710 },
        ];
        let graph = RoadGraph {
            nodes,
            ways: vec![WayInfo { name: "测试路".to_owned(), highway: "primary".to_owned() }],
            offsets: vec![0, 2, 4, 6, 8, 10],
            edges: vec![
                Edge { to: 4, distance_m: 150.0, speed_kmh: 30, way: 0 },
                Edge { to: 2, distance_m: 150.0, speed_kmh: 30, way: 0 },
                Edge { to: 4, distance_m: 150.0, speed_kmh: 30, way: 0 },
                Edge { to: 3, distance_m: 150.0, speed_kmh: 30, way: 0 },
                Edge { to: 0, distance_m: 150.0, speed_kmh: 30, way: 0 },
                Edge { to: 3, distance_m: 150.0, speed_kmh: 30, way: 0 },
                Edge { to: 2, distance_m: 150.0, speed_kmh: 30, way: 0 },
                Edge { to: 1, distance_m: 150.0, speed_kmh: 30, way: 0 },
                Edge { to: 0, distance_m: 150.0, speed_kmh: 30, way: 0 },
                Edge { to: 1, distance_m: 150.0, speed_kmh: 30, way: 0 },
            ],
        };
        let camera_wgs = LatLng::new(31.2310, 121.4710);
        let (directory, base_state) = test_state();
        let mut cache = CameraCache::open(&directory.path().join("cache.sqlite")).expect("cache");
        cache
            .upsert_cameras(
                &[TrafficCamera::new(
                    "cam-4",
                    wgs84_to_gcj02(camera_wgs),
                    [CameraTabType::Speeding],
                )],
                "021",
                1_000,
            )
            .expect("insert");

        let state = Arc::new(AppState {
            store: FileStore::new(directory.path()),
            config: Arc::clone(&base_state.config),
            graph: Some(Arc::new(graph)),
        });
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/route")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"origin":{"latitude":31.23,"longitude":121.47},"destination":{"latitude":31.232,"longitude":121.472},"avoidCameras":true,"cameraRadiusM":30,"maxDetourRatio":1.0}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("route");
        assert_eq!(response.status(), StatusCode::OK);
        let value = body_json(response).await;
        assert_eq!(value["avoidance"]["applied"], true);
        assert!(value["avoidance"]["camerasBefore"].as_u64().unwrap_or(0) >= 1);
        assert_eq!(value["avoidance"]["camerasAfter"], 0);
        assert!(value["distanceM"].as_f64().unwrap_or(0.0) > 300.0);
    }

    #[tokio::test]
    async fn route_without_a_graph_is_unavailable() {
        let (_directory, state) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/route")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"origin":{"latitude":31.23,"longitude":121.47},"destination":{"latitude":31.24,"longitude":121.48}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("route");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn offline_mode_rejects_upstream_calls() {
        let (directory, state) = test_state();
        let mut config = (*state.config).clone();
        config.offline = true;
        let offline_state = Arc::new(AppState {
            store: FileStore::new(directory.path()),
            config: Arc::new(config),
            graph: None,
        });
        let response = router(offline_state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"phone":"1","password":"x"}"#))
                    .expect("request"),
            )
            .await
            .expect("login");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn cache_endpoints_report_and_clear() {
        let (directory, state) = test_state();
        let mut cache = CameraCache::open(&directory.path().join("cache.sqlite")).expect("cache");
        cache
            .upsert_cameras(
                &[TrafficCamera::new(
                    "31.23,121.47:4",
                    LatLng::new(31.23, 121.47),
                    [CameraTabType::RedLight],
                )],
                "021",
                5,
            )
            .expect("insert");

        let app = router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/cache")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("cache");
        let value = body_json(response).await;
        assert_eq!(value["stats"]["cameras"], 1);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/cache/clear")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("clear");
        let value = body_json(response).await;
        assert_eq!(value["stats"]["cameras"], 0);
    }

    #[test]
    fn image_url_validation_rejects_unsafe_targets() {
        assert!(is_fetchable_image_url("https://cdn.example/a.jpg"));
        assert!(!is_fetchable_image_url("file:///etc/passwd"));
        assert!(!is_fetchable_image_url("http://localhost:8787/api/state"));
        assert!(!is_fetchable_image_url("http://127.0.0.1/secret"));
        assert!(!is_fetchable_image_url("http://192.168.1.1/a.jpg"));
    }

    #[test]
    fn image_cache_key_ignores_rotating_signature_parameters() {
        let first = image_cache_key(
            "https://trafficeyes.oss-cn-beijing.aliyuncs.com/a/b.jpg?Expires=1&OSSAccessKeyId=K&Signature=S1&security-token=T1&x-oss-process=image%2Fresize%2Cw_1200",
        );
        let second = image_cache_key(
            "https://trafficeyes.oss-cn-beijing.aliyuncs.com/a/b.jpg?Expires=999&OSSAccessKeyId=K&Signature=S2&security-token=T2&x-oss-process=image%2Fresize%2Cw_1200",
        );
        assert_eq!(first, second);
        assert!(first.contains("x-oss-process="));
        assert!(!first.contains("Signature"));

        let thumbnail = image_cache_key(
            "https://trafficeyes.oss-cn-beijing.aliyuncs.com/a/b.jpg?x-oss-process=image%2Fresize%2Cw_300",
        );
        assert_ne!(first, thumbnail);
        assert_eq!(
            image_cache_key("http://motuserver.oss-cn-beijing.aliyuncs.com/u/1.png?x=1"),
            "http://motuserver.oss-cn-beijing.aliyuncs.com/u/1.png"
        );
    }

    #[test]
    fn bbox_round_trips_between_wgs_and_gcj() {
        let bounds = parse_bbox("31.0,121.0,31.5,121.8").expect("bbox");
        let gcj = wgs_bounds_to_gcj(bounds).expect("gcj");
        assert!(gcj.south < gcj.north);
        assert!(gcj.west < gcj.east);
        // The datum shift is small but non-zero in Shanghai.
        assert!((gcj.west - bounds.west).abs() > 0.001);
        assert!(parse_bbox("1,2,3").is_none());
        assert!(parse_bbox("91,0,92,1").is_none());
    }

    #[test]
    fn ttl_freshness_handles_never_expiring_and_expiry() {
        assert!(is_fresh(1_000, 1_500, 1_000));
        assert!(!is_fresh(1_000, 2_500, 1_000));
        // Zero means "keep forever", not "always refresh".
        assert!(is_fresh(1_000, 10_000_000_000, 0));
    }
}
