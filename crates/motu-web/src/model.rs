//! JSON shapes exchanged with the browser.
//!
//! The browser only renders these structures. Coordinates crossing the API
//! boundary are WGS-84 so they line up with OpenStreetMap tiles; the original
//! `GCJ-02` position is retained as `sourcePosition` for debugging.

use motu_core::{CameraTabType, LatLng, TrafficCamera, wgs84_to_gcj02};
use serde::{Deserialize, Serialize};

/// A map marker plus the stable identifier used to request official details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebCamera {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_by_string: Option<String>,
    /// WGS-84 position for Leaflet / OpenStreetMap.
    pub position: LatLng,
    /// Original `GCJ-02` position returned by Motu, kept for debugging.
    pub source_position: LatLng,
    pub tab_types: Vec<u8>,
    pub tab_labels: Vec<String>,
    pub primary_type: u8,
    pub primary_label: String,
    pub new_flag: bool,
    pub white: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_m: Option<f64>,
    /// `false` marks a bundled demo marker rather than server data.
    pub official: bool,
}

impl WebCamera {
    /// Convert one official `TrafficCamera` into the browser-friendly shape.
    #[must_use]
    pub fn from_official(camera: &TrafficCamera) -> Self {
        let primary = camera.primary_type();
        Self {
            id: camera.id.clone(),
            member: camera.member.clone(),
            member_by_string: camera.member_by_string.clone(),
            position: motu_core::gcj02_to_wgs84(camera.position),
            source_position: camera.position,
            tab_types: camera.tab_types.iter().map(|tab| tab.id()).collect(),
            tab_labels: camera
                .tab_types
                .iter()
                .map(|tab| tab.label_zh().to_owned())
                .collect(),
            primary_type: primary.id(),
            primary_label: primary.label_zh().to_owned(),
            new_flag: camera.new_flag,
            white: camera.white,
            distance_m: camera.distance_m,
            official: true,
        }
    }

    fn demo(id: &str, position: LatLng, tab_types: &[CameraTabType]) -> Self {
        let primary = tab_types.first().copied().unwrap_or(CameraTabType::Unknown);
        Self {
            id: id.to_owned(),
            member: None,
            member_by_string: None,
            position,
            source_position: wgs84_to_gcj02(position),
            tab_types: tab_types.iter().map(|tab| tab.id()).collect(),
            tab_labels: tab_types
                .iter()
                .map(|tab| tab.label_zh().to_owned())
                .collect(),
            primary_type: primary.id(),
            primary_label: primary.label_zh().to_owned(),
            new_flag: false,
            white: false,
            distance_m: None,
            official: false,
        }
    }
}

/// In-memory camera cache. Official rows are never written to disk.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraSnapshot {
    /// `demo` before the first sync, then `official`.
    pub source: String,
    pub city_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at_ms: Option<i64>,
    pub cameras: Vec<WebCamera>,
}

impl CameraSnapshot {
    #[must_use]
    pub fn demo(city_code: impl Into<String>) -> Self {
        Self {
            source: "demo".to_owned(),
            city_code: city_code.into(),
            fetched_at_ms: None,
            cameras: demo_cameras(),
        }
    }
}

/// A small Shanghai fixture so the map is useful before signing in.
///
/// These are deliberately invented positions. The UI marks them as demo data
/// and refuses to request official details for them.
#[must_use]
pub fn demo_cameras() -> Vec<WebCamera> {
    vec![
        WebCamera::demo(
            "demo-people-square",
            LatLng::new(31.230_4, 121.473_7),
            &[CameraTabType::RedLight, CameraTabType::LinePressing],
        ),
        WebCamera::demo(
            "demo-lujiazui",
            LatLng::new(31.239_7, 121.499_8),
            &[CameraTabType::Speeding],
        ),
        WebCamera::demo(
            "demo-xujiahui",
            LatLng::new(31.195_0, 121.437_0),
            &[CameraTabType::LaneViolation],
        ),
        WebCamera::demo(
            "demo-jingan",
            LatLng::new(31.223_0, 121.445_0),
            &[CameraTabType::TailNumberRestriction],
        ),
        WebCamera::demo(
            "demo-hongqiao",
            LatLng::new(31.197_0, 121.336_0),
            &[CameraTabType::RestrictedEntry],
        ),
        WebCamera::demo(
            "demo-wujiaochang",
            LatLng::new(31.300_0, 121.514_0),
            &[CameraTabType::BusLane],
        ),
        WebCamera::demo(
            "demo-expo",
            LatLng::new(31.187_0, 121.490_0),
            &[CameraTabType::NonMotorLane, CameraTabType::NoHelmet],
        ),
        WebCamera::demo(
            "demo-north-bund",
            LatLng::new(31.253_0, 121.494_0),
            &[CameraTabType::EmergencyLane],
        ),
    ]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub phone: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRequest {
    /// WGS-84 latitude from the current Leaflet center.
    pub latitude: f64,
    /// WGS-84 longitude from the current Leaflet center.
    pub longitude: f64,
    #[serde(default)]
    pub city_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailRequest {
    pub camera_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_markers_are_labeled_as_demo() {
        let cameras = demo_cameras();
        assert!(!cameras.is_empty());
        assert!(cameras.iter().all(|camera| !camera.official));
        assert!(cameras.iter().all(|camera| camera.position.is_valid()));
    }

    #[test]
    fn official_camera_is_converted_to_wgs84() {
        let gcj = LatLng::new(31.234_8, 121.480_2);
        let camera = TrafficCamera::new(
            "camera-1",
            gcj,
            [CameraTabType::RedLight, CameraTabType::Speeding],
        );
        let web = WebCamera::from_official(&camera);
        assert_eq!(web.id, "camera-1");
        assert_eq!(web.source_position, gcj);
        assert_eq!(web.tab_types, vec![4, 5]);
        assert_eq!(web.primary_label, "闯红灯");
        assert!(web.official);
        assert!(
            (web.position.latitude - gcj.latitude).abs() > 0.001,
            "GCJ and WGS coordinates should differ in Shanghai"
        );
    }
}
