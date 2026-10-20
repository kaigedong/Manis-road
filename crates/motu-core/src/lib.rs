//! Motorcycle-navigation core recovered from the Motu 4.1.9 Android build.
//!
//! The crate keeps transport, security, camera-alert and route-audit rules in
//! pure Rust so the Android layer only renders UI and forwards user actions.
//! Recovered behaviour is marked with the Java class it came from. Motu's
//! proprietary `AMap` routing and electronic-eye data are not bundled here: the
//! route module audits caller-supplied geometry and the camera module consumes
//! caller-supplied/local data.

// Public fallible APIs return [`MotuError`]; the variant names are the error
// documentation, so repeating a `# Errors` section on every function would add
// noise without information.
#![allow(clippy::missing_errors_doc)]

pub mod api;
pub mod auth;
pub mod camera;
pub mod crypto;
pub mod error;
pub mod geo;
pub mod model;
pub mod official;
pub mod route;
pub mod store;
pub mod vehicle;

pub use api::{
    ALL_ENDPOINTS, BASE_URL, Endpoint, HeaderInit, HttpMethod, HttpResponse, SecurityClient,
    SignedRequest, Transport, base_headers, build_request, decode_envelope, endpoint_by_name,
    endpoints,
};
pub use auth::{LoginOutcome, login, logout};
pub use camera::{CameraScanEngine, NaviLocation, ScanConfig, ScanResult};
pub use crypto::{
    Aad, SecurityConfig, SecurityEnvelope, SecurityResponse, encrypt_md5, encrypt_md5_bytes,
    md5_hex, safe_env_token, sign_json_body, sign_params, signing_certificate_md5,
};
pub use error::MotuError;
pub use geo::LatLng;
pub use model::{
    ApiEnvelope, CameraTabType, LoginUser, MapArea, MapAreaResult, MotuMapArea, PathAreaRequest,
    PathAvoidRequest, PathAvoidResult, PolylineStep, TrafficCamera,
};
pub use official::{
    CameraDetail, CameraDetailType, CameraRule, CameraTicket, OfficialCameraQuery,
    OfficialCameraSync, camera_search_body, default_tab_types, load_camera_detail,
    sync_official_cameras,
};
pub use route::{
    AvoidArea, AvoidAreaKind, AvoidHit, CameraHit, RouteAudit, RoutePlan, RoutePlanRequest,
    RoutePlanner, StraightLinePlanner, audit_route, decode_polyline, encode_polyline,
    parse_amap_positions,
};
pub use store::{AuthSession, CoreState, FileStore, Settings};
pub use vehicle::{
    PlateColor, PowerType, TransType, Vehicle, VehicleValidation, normalize_plate, parse_plate,
};
