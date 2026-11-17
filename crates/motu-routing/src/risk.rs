#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "grid keys are degree cells and counts, far below f64's exact range"
)]

//! Camera and restricted-zone risk, priced in **seconds**.
//!
//! The product point is riding *through* restricted areas while being caught by
//! cameras as rarely as possible. So a restricted zone is not a wall: it makes
//! the road inside more expensive. Cameras are not a wall either: each one adds a
//! fixed number of seconds, decaying with distance.
//!
//! Everything here is additive to the router's cost, which is measured in
//! seconds — that is what makes "detour 800 m to skip two cameras" a comparable
//! question.

use std::collections::HashMap;

use motu_core::LatLng;
use motu_core::geo::point_in_polygon;

use crate::graph::{RoadGraph, haversine_m};

/// Seconds added for a camera that enforces restricted-zone / banned-plate rules.
pub const SECONDS_RESTRICTED: f64 = 60.0;
/// Seconds added for red-light, speeding, lane and line-pressing cameras.
pub const SECONDS_MOVING: f64 = 40.0;
/// Seconds added for the remaining categories (helmet, horn, lighting, …).
pub const SECONDS_OTHER: f64 = 20.0;
/// Risk decays to zero at this distance from the camera.
pub const DEFAULT_CAMERA_RADIUS_M: f64 = 40.0;
/// Extra cost multiplier for riding inside a restricted zone.
pub const DEFAULT_ZONE_FACTOR: f64 = 1.3;

/// One camera with its already-converted risk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraRisk {
    pub point: LatLng,
    pub seconds: f64,
    /// `tabTypes[0]` as returned by the official API, kept for reporting.
    pub tab_type: u8,
}

/// Map the official camera categories onto a risk weight.
///
/// `1 / 3 / 9` are restricted-entry, non-motor-lane and plate-restriction
/// cameras — for a legal motorcycle in Shanghai these are the ones that matter.
#[must_use]
pub fn seconds_for_tab_type(tab_type: u8) -> f64 {
    match tab_type {
        1 | 3 | 9 => SECONDS_RESTRICTED,
        4 | 5 | 6 | 7 | 8 | 13 => SECONDS_MOVING,
        _ => SECONDS_OTHER,
    }
}

/// Spatial index over cameras, queried once per edge.
#[derive(Debug, Clone)]
pub struct CameraIndex {
    cells: HashMap<(i32, i32), Vec<CameraRisk>>,
    radius_m: f64,
    count: usize,
}

impl CameraIndex {
    /// Grid cell size: one camera radius, so a query touches 9 cells at most.
    #[must_use]
    pub fn new(cameras: &[CameraRisk], radius_m: f64) -> Self {
        let radius_m = if radius_m > 1.0 {
            radius_m
        } else {
            DEFAULT_CAMERA_RADIUS_M
        };
        let cell_deg = radius_m / 111_320.0;
        let mut cells: HashMap<(i32, i32), Vec<CameraRisk>> = HashMap::new();
        let mut count = 0;
        for camera in cameras {
            // Zero-weight cameras stay in the index: they must still be counted
            // when we measure exposure without letting them steer the route.
            if !camera.point.is_valid() {
                continue;
            }
            cells
                .entry(Self::key(camera.point, cell_deg))
                .or_default()
                .push(*camera);
            count += 1;
        }
        Self {
            cells,
            radius_m,
            count,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.count
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[must_use]
    pub fn radius_m(&self) -> f64 {
        self.radius_m
    }

    fn key(point: LatLng, cell_deg: f64) -> (i32, i32) {
        (
            (point.latitude / cell_deg).floor() as i32,
            (point.longitude / cell_deg).floor() as i32,
        )
    }

    /// Seconds of risk for a segment, summed over nearby cameras with linear decay.
    #[must_use]
    pub fn penalty_s(&self, from: LatLng, to: LatLng) -> f64 {
        if self.cells.is_empty() {
            return 0.0;
        }
        let mid = LatLng::new(from.latitude.midpoint(to.latitude), from.longitude.midpoint(to.longitude));
        let cell_deg = self.radius_m / 111_320.0;
        let (i, j) = Self::key(mid, cell_deg);
        let mut total = 0.0;
        for di in -1..=1 {
            for dj in -1..=1 {
                let Some(bucket) = self.cells.get(&(i + di, j + dj)) else {
                    continue;
                };
                for camera in bucket {
                    let distance = distance_to_segment_m(camera.point, from, to);
                    if distance > self.radius_m {
                        continue;
                    }
                    total += camera.seconds * (1.0 - distance / self.radius_m);
                }
            }
        }
        total
    }
}

/// Restricted-zone polygons, priced as a multiplier instead of a barrier.
#[derive(Debug, Clone)]
pub struct ZoneIndex {
    zones: Vec<Vec<LatLng>>,
    factor: f64,
}

impl ZoneIndex {
    #[must_use]
    pub fn new(zones: &[Vec<LatLng>], factor: f64) -> Self {
        let factor = if factor.is_finite() && factor >= 1.0 {
            factor
        } else {
            DEFAULT_ZONE_FACTOR
        };
        Self {
            zones: zones
                .iter()
                .filter(|ring| ring.len() >= 3)
                .cloned()
                .collect(),
            factor,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.zones.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    #[must_use]
    pub fn factor(&self) -> f64 {
        self.factor
    }

    /// Cost multiplier at a point: `1.0` outside, [`ZoneIndex::factor`] inside.
    #[must_use]
    pub fn factor_at(&self, point: LatLng) -> f64 {
        if self.zones.is_empty() {
            return 1.0;
        }
        if self
            .zones
            .iter()
            .any(|ring| point_in_polygon(point, ring))
        {
            self.factor
        } else {
            1.0
        }
    }
}

/// Both risk layers together, so the router takes one optional parameter.
#[derive(Debug, Clone)]
pub struct RiskModel {
    pub cameras: CameraIndex,
    pub zones: ZoneIndex,
}

impl RiskModel {
    #[must_use]
    pub fn new(cameras: &[CameraRisk], radius_m: f64, zones: &[Vec<LatLng>], zone_factor: f64) -> Self {
        Self {
            cameras: CameraIndex::new(cameras, radius_m),
            zones: ZoneIndex::new(zones, zone_factor),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cameras.is_empty() && self.zones.is_empty()
    }

    /// Total extra cost (seconds) for one edge.
    #[must_use]
    pub fn edge_penalty_s(&self, from: LatLng, to: LatLng, base_seconds: f64) -> f64 {
        let zone_extra = if self.zones.is_empty() {
            0.0
        } else {
            // 采样端点与中点：只要有一段落在区内就按区内计价，否则短边穿越
            // 一个小区时中点可能刚好落在区外，代价会漏算。
            let mid = LatLng::new(
                from.latitude.midpoint(to.latitude),
                from.longitude.midpoint(to.longitude),
            );
            let factor = self
                .zones
                .factor_at(from)
                .max(self.zones.factor_at(mid))
                .max(self.zones.factor_at(to));
            base_seconds * (factor - 1.0)
        };
        zone_extra + self.cameras.penalty_s(from, to)
    }
}

/// Distance from `point` to segment `a–b`, metres.
fn distance_to_segment_m(point: LatLng, a: LatLng, b: LatLng) -> f64 {
    // Work in a local planar approximation: over one edge this is accurate to
    // well under a metre, and it avoids point-to-great-circle edge cases.
    let cos_lat = a.latitude.to_radians().cos().abs().max(0.05);
    let scale = 111_320.0;
    let px = (point.longitude - a.longitude) * cos_lat * scale;
    let py = (point.latitude - a.latitude) * scale;
    let bx = (b.longitude - a.longitude) * cos_lat * scale;
    let by = (b.latitude - a.latitude) * scale;
    let length_squared = bx * bx + by * by;
    if length_squared <= 1e-9 {
        return haversine_m(point.latitude, point.longitude, a.latitude, a.longitude);
    }
    let t = ((px * bx + py * by) / length_squared).clamp(0.0, 1.0);
    let dx = px - bx * t;
    let dy = py - by * t;
    dx.hypot(dy)
}

/// Count cameras within `radius_m` of a polyline, and share of length inside zones.
#[must_use]
pub fn route_risk_report(
    points: &[LatLng],
    index: &CameraIndex,
    zones: &ZoneIndex,
    radius_m: f64,
) -> (usize, f64) {
    if points.len() < 2 {
        return (0, 0.0);
    }
    // Count distinct cameras by index rather than per-edge hits.
    let mut hit: Vec<&CameraRisk> = Vec::new();
    let mut inside_m = 0.0;
    let mut total_m = 0.0;
    for pair in points.windows(2) {
        let [a, b] = [pair[0], pair[1]];
        let length = haversine_m(a.latitude, a.longitude, b.latitude, b.longitude);
        total_m += length;
        let mid = LatLng::new(a.latitude.midpoint(b.latitude), a.longitude.midpoint(b.longitude));
        if !zones.is_empty() && zones.factor_at(mid) > 1.0 {
            inside_m += length;
        }
        let cell_deg = index.radius_m() / 111_320.0;
        let (i, j) = CameraIndex::key(mid, cell_deg);
        for di in -1..=1 {
            for dj in -1..=1 {
                let Some(bucket) = index.cells.get(&(i + di, j + dj)) else {
                    continue;
                };
                for camera in bucket {
                    if distance_to_segment_m(camera.point, a, b) <= radius_m {
                        hit.push(camera);
                    }
                }
            }
        }
    }
    hit.sort_by(|left, right| {
        left.point
            .latitude
            .total_cmp(&right.point.latitude)
            .then(left.point.longitude.total_cmp(&right.point.longitude))
    });
    hit.dedup_by(|left, right| {
        (left.point.latitude - right.point.latitude).abs() < 1e-7
            && (left.point.longitude - right.point.longitude).abs() < 1e-7
    });
    let share = if total_m > 0.0 {
        inside_m / total_m
    } else {
        0.0
    };
    (hit.len(), share)
}

/// Build a [`CameraRisk`] list from raw `(point, tabType)` pairs.
#[must_use]
pub fn cameras_from_tab_types(entries: &[(LatLng, u8)]) -> Vec<CameraRisk> {
    entries
        .iter()
        .map(|(point, tab_type)| CameraRisk {
            point: *point,
            seconds: seconds_for_tab_type(*tab_type),
            tab_type: *tab_type,
        })
        .collect()
}

/// Convenience: highest edge speed in the graph, used by the A* heuristic.
#[must_use]
pub fn max_speed_kmh(graph: &RoadGraph) -> f64 {
    graph
        .edges
        .iter()
        .map(|edge| f64::from(edge.speed_kmh))
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(lat: f64, lon: f64) -> LatLng {
        LatLng::new(lat, lon)
    }

    #[test]
    fn restricted_cameras_cost_more_than_others() {
        assert!((seconds_for_tab_type(1) - SECONDS_RESTRICTED).abs() < 1e-9);
        assert!((seconds_for_tab_type(3) - SECONDS_RESTRICTED).abs() < 1e-9);
        assert!((seconds_for_tab_type(9) - SECONDS_RESTRICTED).abs() < 1e-9);
        assert!((seconds_for_tab_type(4) - SECONDS_MOVING).abs() < 1e-9);
        assert!((seconds_for_tab_type(5) - SECONDS_MOVING).abs() < 1e-9);
        assert!((seconds_for_tab_type(11) - SECONDS_OTHER).abs() < 1e-9);
    }

    #[test]
    fn risk_decays_with_distance_and_stops_at_the_radius() {
        let camera = CameraRisk {
            point: at(31.2300, 121.4700),
            seconds: SECONDS_RESTRICTED,
            tab_type: 1,
        };
        let index = CameraIndex::new(&[camera], 40.0);
        // Edge passing right through the camera.
        let close = index.penalty_s(at(31.2299, 121.4700), at(31.2301, 121.4700));
        assert!(close > 50.0, "expected nearly the full weight, got {close}");
        // Edge 30 m away: still some risk.
        let far = index.penalty_s(at(31.2299, 121.4703), at(31.2301, 121.4703));
        assert!(far > 0.0 && far < close, "expected decay, got {far}");
        // Edge outside the radius: nothing.
        let outside = index.penalty_s(at(31.2310, 121.4700), at(31.2312, 121.4700));
        assert!(outside.abs() < f64::EPSILON);
    }

    #[test]
    fn zones_multiply_cost_instead_of_blocking() {
        let ring = vec![
            at(31.2280, 121.4680),
            at(31.2280, 121.4780),
            at(31.2340, 121.4780),
            at(31.2340, 121.4680),
        ];
        let zones = ZoneIndex::new(&[ring], 1.3);
        assert!((zones.factor_at(at(31.2300, 121.4720)) - 1.3).abs() < 1e-9);
        assert!((zones.factor_at(at(31.2400, 121.4720)) - 1.0).abs() < 1e-9);
        let model = RiskModel::new(&[], 40.0, &[vec![
            at(31.2280, 121.4680),
            at(31.2280, 121.4780),
            at(31.2340, 121.4780),
            at(31.2340, 121.4680),
        ]], 1.3);
        // 100 s edge inside the zone costs 30 s more, and is still passable.
        let penalty = model.edge_penalty_s(at(31.2300, 121.4719), at(31.2300, 121.4721), 100.0);
        assert!((penalty - 30.0).abs() < 1.0, "got {penalty}");
    }

    #[test]
    fn segment_distance_handles_off_end_queries() {
        let a = at(31.2300, 121.4700);
        let b = at(31.2300, 121.4701);
        // Point beyond the end falls back to the endpoint distance.
        let far = at(31.2300, 121.4800);
        assert!(distance_to_segment_m(far, a, b) > 900.0);
    }
}
