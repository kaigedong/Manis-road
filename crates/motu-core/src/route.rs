//! Route planning boundary and camera/avoid-area audit.
//!
//! Motu delegates real road routing to the proprietary `AMap` SDK
//! (`calPathAvoidSec` returns the `AMap` geometry). That engine cannot be
//! reproduced here, so this module defines a [`RoutePlanner`] port and a
//! deterministic offline [`StraightLinePlanner`] for tests and fallback.
//!
//! The core owns the part Motu implements itself: auditing an externally
//! supplied polyline against avoid areas and cameras, and decoding the
//! polyline formats used in `PathAvoidResult.alreadyCalPath`.

use serde::{Deserialize, Serialize};

use crate::error::{MotuError, Result};
use crate::geo::{
    LatLng, distance_m, distance_to_polyline_m, point_in_polygon, polyline_intersects_polygon,
};
use crate::model::{CameraTabType, TrafficCamera};

/// Why an area is being avoided.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvoidAreaKind {
    /// User-added avoid area (`userAvoidType`).
    User = 0,
    /// Platform/global avoid area.
    Platform = 1,
    /// White list: passing through is explicitly allowed.
    White = 2,
}

/// A polygon (or polyline) avoid area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvoidArea {
    pub id: i64,
    #[serde(default)]
    pub name: Option<String>,
    /// Closed polygon in order; the first point need not be repeated.
    pub boundary: Vec<LatLng>,
    #[serde(default = "default_area_kind")]
    pub kind: AvoidAreaKind,
}

const fn default_area_kind() -> AvoidAreaKind {
    AvoidAreaKind::User
}

impl AvoidArea {
    #[must_use]
    pub fn polygon(id: i64, boundary: Vec<LatLng>) -> Self {
        Self {
            id,
            name: None,
            boundary,
            kind: AvoidAreaKind::User,
        }
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.boundary.len() >= 3 && self.boundary.iter().all(LatLng::is_valid)
    }
}

/// Input for a planning request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutePlanRequest {
    pub origin: LatLng,
    pub destination: LatLng,
    #[serde(default)]
    pub waypoints: Vec<LatLng>,
    #[serde(default)]
    pub avoid_areas: Vec<AvoidArea>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vehicle_plate: Option<String>,
}

/// Geometry returned by a planner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutePlan {
    pub points: Vec<LatLng>,
    pub distance_m: f64,
    pub duration_s: f64,
    /// True when the planner could not honour every avoid area.
    #[serde(default)]
    pub degraded: bool,
}

impl RoutePlan {
    #[must_use]
    pub fn crosses(&self, area: &AvoidArea) -> bool {
        if !area.is_valid() {
            return false;
        }
        self.points
            .iter()
            .any(|point| point_in_polygon(*point, &area.boundary))
            || area.boundary.iter().any(|corner| {
                distance_to_polyline_m(*corner, &self.points).is_some_and(|d| d < 1.0)
            })
    }
}

/// Port for a real routing engine (`AMap` or another provider).
pub trait RoutePlanner {
    fn plan(&self, request: &RoutePlanRequest) -> Result<RoutePlan>;
}

/// Deterministic offline planner: straight segments through the waypoints.
///
/// It does not avoid areas; set `degraded` when the requested route crosses an
/// avoid area so the UI can say the result is only a fallback.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StraightLinePlanner {
    /// Assumed average speed for the duration estimate.
    pub speed_kmh: f64,
}

impl Default for StraightLinePlanner {
    fn default() -> Self {
        Self { speed_kmh: 30.0 }
    }
}

impl RoutePlanner for StraightLinePlanner {
    fn plan(&self, request: &RoutePlanRequest) -> Result<RoutePlan> {
        if !request.origin.is_valid() || !request.destination.is_valid() {
            return Err(MotuError::Route(
                "origin and destination must be valid coordinates".to_owned(),
            ));
        }
        let mut points = vec![request.origin];
        points.extend(request.waypoints.iter().copied());
        points.push(request.destination);
        if points.iter().any(|point| !point.is_valid()) {
            return Err(MotuError::Route(
                "waypoint is not a valid coordinate".to_owned(),
            ));
        }
        let distance: f64 = points
            .windows(2)
            .map(|pair| distance_m(pair[0], pair[1]))
            .sum();
        let speed_ms = (self.speed_kmh.max(0.1)) / 3.6;
        let degraded = request.avoid_areas.iter().any(|area| {
            area.kind != AvoidAreaKind::White
                && (points
                    .iter()
                    .any(|point| point_in_polygon(*point, &area.boundary))
                    || area.boundary.iter().any(|corner| {
                        distance_to_polyline_m(*corner, &points).is_some_and(|d| d < 1.0)
                    }))
        });
        Ok(RoutePlan {
            points,
            distance_m: distance,
            duration_s: distance / speed_ms,
            degraded,
        })
    }
}

/// How a camera relates to the audited route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraHit {
    pub camera_id: String,
    pub camera_type: CameraTabType,
    pub distance_to_route_m: f64,
    /// Index of the nearest route vertex.
    pub nearest_point_index: usize,
}

/// An avoid area crossed by the audited route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvoidHit {
    pub area_id: i64,
    pub kind: AvoidAreaKind,
    /// True when a route vertex falls inside the polygon.
    pub inside: bool,
    /// True when a route segment crosses a polygon edge.
    pub intersects: bool,
    /// Minimum distance from the area boundary to the route.
    pub distance_m: f64,
}

/// Result of auditing a finished route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteAudit {
    pub avoid_hits: Vec<AvoidHit>,
    pub camera_hits: Vec<CameraHit>,
}

impl RouteAudit {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.avoid_hits.is_empty() && self.camera_hits.is_empty()
    }

    /// True when the route enters an avoid area that is not white-listed.
    #[must_use]
    pub fn has_blocking_avoid(&self) -> bool {
        self.avoid_hits
            .iter()
            .any(|hit| (hit.inside || hit.intersects) && hit.kind != AvoidAreaKind::White)
    }
}

/// Audit a route polyline against avoid areas and cameras.
///
/// `camera_buffer_m` is how close a camera must be to the route to be reported.
pub fn audit_route(
    points: &[LatLng],
    avoid_areas: &[AvoidArea],
    cameras: &[TrafficCamera],
    camera_buffer_m: f64,
) -> Result<RouteAudit> {
    if points.iter().any(|point| !point.is_valid()) {
        return Err(MotuError::Route(
            "route contains an invalid coordinate".to_owned(),
        ));
    }
    if !camera_buffer_m.is_finite() || camera_buffer_m < 0.0 {
        return Err(MotuError::Route(
            "camera buffer must be a non-negative finite number".to_owned(),
        ));
    }

    let mut avoid_hits = Vec::new();
    for area in avoid_areas {
        if !area.is_valid() {
            continue;
        }
        let inside = points
            .iter()
            .any(|point| point_in_polygon(*point, &area.boundary));
        let intersects = if points.len() >= 2 {
            polyline_intersects_polygon(points, &area.boundary)
        } else {
            false
        };
        let distance = area
            .boundary
            .iter()
            .filter_map(|corner| distance_to_polyline_m(*corner, points))
            .fold(f64::INFINITY, f64::min);
        // A hit means the route enters/crosses the area, not merely passes
        // nearby: a 0.5 m tolerance covers rounding on the boundary.
        if inside || intersects || distance <= 0.5 {
            avoid_hits.push(AvoidHit {
                area_id: area.id,
                kind: area.kind,
                inside,
                intersects,
                distance_m: if distance.is_finite() { distance } else { 0.0 },
            });
        }
    }

    let camera_hits = if points.len() < 2 {
        Vec::new()
    } else {
        cameras
            .iter()
            .filter(|camera| camera.position.is_valid())
            .filter_map(|camera| {
                let distance = distance_to_polyline_m(camera.position, points)?;
                if distance > camera_buffer_m {
                    return None;
                }
                let nearest_point_index = points
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        distance_m(**a, camera.position)
                            .total_cmp(&distance_m(**b, camera.position))
                    })
                    .map_or(0, |(index, _)| index);
                Some(CameraHit {
                    camera_id: camera.id.clone(),
                    camera_type: camera.primary_type(),
                    distance_to_route_m: distance,
                    nearest_point_index,
                })
            })
            .collect()
    };

    Ok(RouteAudit {
        avoid_hits,
        camera_hits,
    })
}

/// Parse `AMap`'s simple `lon,lat;lon,lat` coordinate string.
pub fn parse_amap_positions(value: &str) -> Result<Vec<LatLng>> {
    let mut points = Vec::new();
    for pair in value.split(';').filter(|part| !part.is_empty()) {
        let (lon, lat) = pair
            .split_once(',')
            .ok_or_else(|| MotuError::Route(format!("invalid position pair: {pair}")))?;
        let longitude: f64 = lon
            .trim()
            .parse()
            .map_err(|_| MotuError::Route(format!("invalid longitude: {lon}")))?;
        let latitude: f64 = lat
            .trim()
            .parse()
            .map_err(|_| MotuError::Route(format!("invalid latitude: {lat}")))?;
        let point = LatLng::new(latitude, longitude);
        if !point.is_valid() {
            return Err(MotuError::Route(format!("position out of range: {pair}")));
        }
        points.push(point);
    }
    Ok(points)
}

/// Encode a Google/`AMap`-style polyline (precision `1e5`).
#[must_use]
pub fn encode_polyline(points: &[LatLng]) -> String {
    encode_polyline_scaled(points, 1e5)
}

/// Encode with an explicit coordinate scale; `AMap` map areas use `1e6`.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "polyline coordinates are scaled and fit i64"
)]
pub fn encode_polyline_scaled(points: &[LatLng], scale: f64) -> String {
    let mut output = String::new();
    let mut last_lat = 0_i64;
    let mut last_lon = 0_i64;
    for point in points {
        let lat = (point.latitude * scale).round() as i64;
        let lon = (point.longitude * scale).round() as i64;
        encode_signed(lat - last_lat, &mut output);
        encode_signed(lon - last_lon, &mut output);
        last_lat = lat;
        last_lon = lon;
    }
    output
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "polyline chunks are masked to 0..=63 before adding 63"
)]
fn encode_signed(value: i64, output: &mut String) {
    let mut v = value << 1;
    if value < 0 {
        v = !v;
    }
    while v >= 0x20 {
        output.push(char::from_u32(((0x20 | (v & 0x1f)) + 63) as u32).unwrap_or('?'));
        v >>= 5;
    }
    output.push(char::from_u32((v + 63) as u32).unwrap_or('?'));
}

/// Decode a Google/`AMap`-style polyline (precision `1e5`).
pub fn decode_polyline(value: &str) -> Result<Vec<LatLng>> {
    decode_polyline_scaled(value, 1e5)
}

/// Decode a polyline whose coordinates were scaled by `scale`.
///
/// `AMap`'s `PolylineUtil` uses `1e6` for `posEncode` (map areas), while the
/// route/Google format uses `1e5`; mixing them yields out-of-range latitudes.
#[allow(
    clippy::cast_precision_loss,
    reason = "polyline precision is fixed at 1e-5/1e-6 degrees"
)]
pub fn decode_polyline_scaled(value: &str, scale: f64) -> Result<Vec<LatLng>> {
    let bytes = value.as_bytes();
    let mut index = 0_usize;
    let mut lat = 0_i64;
    let mut lon = 0_i64;
    let mut points = Vec::new();
    while index < bytes.len() {
        lat += decode_signed(bytes, &mut index)?;
        lon += decode_signed(bytes, &mut index)?;
        let point = LatLng::new(lat as f64 / scale, lon as f64 / scale);
        if !point.is_valid() {
            return Err(MotuError::Route(
                "decoded coordinate out of range".to_owned(),
            ));
        }
        points.push(point);
    }
    Ok(points)
}

fn decode_signed(bytes: &[u8], index: &mut usize) -> Result<i64> {
    let mut result = 0_i64;
    let mut shift = 0_u32;
    loop {
        let byte = *bytes
            .get(*index)
            .ok_or_else(|| MotuError::Route("truncated polyline".to_owned()))?;
        *index += 1;
        if byte < 63 {
            return Err(MotuError::Route("invalid polyline character".to_owned()));
        }
        result |= i64::from((byte - 63) & 0x1f) << shift;
        shift += 5;
        if (byte - 63) < 0x20 {
            break;
        }
        if shift > 60 {
            return Err(MotuError::Route("polyline value overflow".to_owned()));
        }
    }
    Ok(if result & 1 != 0 {
        !(result >> 1)
    } else {
        result >> 1
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CameraTabType;

    fn square() -> AvoidArea {
        AvoidArea::polygon(
            1,
            vec![
                LatLng::new(31.0, 121.0),
                LatLng::new(31.0, 121.02),
                LatLng::new(31.02, 121.02),
                LatLng::new(31.02, 121.0),
            ],
        )
    }

    #[test]
    fn straight_line_planner_measures_distance() {
        let planner = StraightLinePlanner::default();
        let request = RoutePlanRequest {
            origin: LatLng::new(31.0, 121.0),
            destination: LatLng::new(31.01, 121.0),
            waypoints: Vec::new(),
            avoid_areas: Vec::new(),
            vehicle_plate: None,
        };
        let plan = planner.plan(&request).unwrap();
        assert_eq!(plan.points.len(), 2);
        assert!((1_100.0..1_120.0).contains(&plan.distance_m));
        assert!(!plan.degraded);
    }

    #[test]
    fn planner_flags_crossed_avoid_area() {
        let planner = StraightLinePlanner::default();
        let request = RoutePlanRequest {
            origin: LatLng::new(31.0, 120.99),
            destination: LatLng::new(31.0, 121.03),
            waypoints: Vec::new(),
            avoid_areas: vec![square()],
            vehicle_plate: None,
        };
        let plan = planner.plan(&request).unwrap();
        assert!(plan.degraded);
        assert!(plan.crosses(&square()));
    }

    #[test]
    fn audit_reports_cameras_near_route() {
        let points = vec![
            LatLng::new(31.0, 121.0),
            LatLng::new(31.0, 121.01),
            LatLng::new(31.0, 121.02),
        ];
        let near = TrafficCamera::new(
            "near",
            LatLng::new(31.000_1, 121.01),
            [CameraTabType::Speeding],
        );
        let far = TrafficCamera::new("far", LatLng::new(31.01, 121.01), [CameraTabType::Speeding]);
        let audit = audit_route(&points, &[], &[near, far], 50.0).unwrap();
        assert_eq!(audit.camera_hits.len(), 1);
        assert_eq!(audit.camera_hits[0].camera_id, "near");
        assert_eq!(audit.camera_hits[0].camera_type, CameraTabType::Speeding);
    }

    #[test]
    fn audit_detects_avoid_area_entry() {
        let points = vec![LatLng::new(31.01, 120.99), LatLng::new(31.01, 121.03)];
        let audit = audit_route(&points, &[square()], &[], 50.0).unwrap();
        assert!(audit.has_blocking_avoid());
    }

    #[test]
    fn white_area_is_not_blocking() {
        let mut area = square();
        area.kind = AvoidAreaKind::White;
        let points = vec![LatLng::new(31.01, 120.99), LatLng::new(31.01, 121.03)];
        let audit = audit_route(&points, &[area], &[], 50.0).unwrap();
        assert!(!audit.has_blocking_avoid());
    }

    #[test]
    fn amap_positions_parse_lon_then_lat() {
        let points = parse_amap_positions("121.4737,31.2304;121.4998,31.2397").unwrap();
        assert_eq!(points.len(), 2);
        assert!((points[0].latitude - 31.230_4).abs() < 1e-9);
        assert!((points[0].longitude - 121.473_7).abs() < 1e-9);
        assert!(parse_amap_positions("bad").is_err());
    }

    #[test]
    fn polyline_round_trips() {
        let points = vec![
            LatLng::new(31.230_4, 121.473_7),
            LatLng::new(31.239_7, 121.499_8),
            LatLng::new(31.2, 121.5),
        ];
        let encoded = encode_polyline(&points);
        let decoded = decode_polyline(&encoded).unwrap();
        assert_eq!(decoded, points);
    }

    #[test]
    fn invalid_route_input_is_rejected() {
        assert!(audit_route(&[LatLng::new(99.0, 0.0)], &[], &[], 10.0).is_err());
        assert!(audit_route(&[LatLng::new(31.0, 121.0)], &[], &[], -1.0).is_err());
    }
}
