//! Geographic helpers matching the formulas used by Motu's camera engine.
//!
//! `distance_m` follows the spherical haversine form used by
//! `AMapUtils.calculateLineDistance` (mean Earth radius `6_378_137` m). It is a
//! navigation approximation, not a survey-grade geodesic.

use serde::{Deserialize, Serialize};

/// `WGS-84` / `GCJ-02` style latitude-longitude pair.
///
/// The core never converts coordinate systems implicitly. Motu works with
/// `AMap` (`GCJ-02`) coordinates, so callers must pass and store one system
/// consistently.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatLng {
    pub latitude: f64,
    pub longitude: f64,
}

impl LatLng {
    #[must_use]
    pub const fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.latitude.is_finite()
            && self.longitude.is_finite()
            && (-90.0..=90.0).contains(&self.latitude)
            && (-180.0..=180.0).contains(&self.longitude)
    }
}

const EARTH_RADIUS_M: f64 = 6_378_137.0;

/// Great-circle distance in metres.
#[must_use]
pub fn distance_m(a: LatLng, b: LatLng) -> f64 {
    let lat1 = a.latitude.to_radians();
    let lat2 = b.latitude.to_radians();
    let delta_lat = (b.latitude - a.latitude).to_radians();
    let delta_lon = (b.longitude - a.longitude).to_radians();
    let h =
        (delta_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (delta_lon / 2.0).sin().powi(2);
    2.0 * h.sqrt().asin() * EARTH_RADIUS_M
}

/// Initial bearing from `a` to `b`, normalised to `[0, 360)`.
///
/// Port of `CameraScanEngine.calculateBearing`.
#[must_use]
pub fn bearing_deg(a: LatLng, b: LatLng) -> f64 {
    let lat1 = a.latitude.to_radians();
    let lon1 = a.longitude.to_radians();
    let lat2 = b.latitude.to_radians();
    let delta_lon = b.longitude.to_radians() - lon1;
    let y = delta_lon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * delta_lon.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// Smallest absolute difference between two bearings, in `[0, 180]`.
///
/// Port of `CameraScanEngine.calculateDeltaAngle`.
#[must_use]
pub fn delta_angle_deg(a: f64, b: f64) -> f64 {
    (((a - b) + 540.0) % 360.0 - 180.0).abs()
}

/// Distance from a point to a line segment, in metres.
///
/// Uses a local equirectangular projection; accurate enough for the short
/// distances (hundreds of metres) used by route and camera audits.
#[must_use]
pub fn distance_to_segment_m(point: LatLng, start: LatLng, end: LatLng) -> f64 {
    let lat0 = point.latitude.to_radians();
    let scale_x = EARTH_RADIUS_M * lat0.cos() * std::f64::consts::PI / 180.0;
    let scale_y = EARTH_RADIUS_M * std::f64::consts::PI / 180.0;

    let px = (point.longitude - start.longitude) * scale_x;
    let py = (point.latitude - start.latitude) * scale_y;
    let ex = (end.longitude - start.longitude) * scale_x;
    let ey = (end.latitude - start.latitude) * scale_y;

    let len_sq = ex * ex + ey * ey;
    if len_sq == 0.0 {
        return (px * px + py * py).sqrt();
    }
    let t = ((px * ex + py * ey) / len_sq).clamp(0.0, 1.0);
    let dx = px - t * ex;
    let dy = py - t * ey;
    (dx * dx + dy * dy).sqrt()
}

/// Shortest distance from a point to a polyline, in metres.
#[must_use]
pub fn distance_to_polyline_m(point: LatLng, polyline: &[LatLng]) -> Option<f64> {
    match polyline {
        [] => None,
        [only] => Some(distance_m(point, *only)),
        _ => polyline
            .windows(2)
            .map(|pair| distance_to_segment_m(point, pair[0], pair[1]))
            .reduce(f64::min),
    }
}

/// Ray-casting point-in-polygon test. The polygon is treated as closed; the
/// first point does not need to be repeated.
#[must_use]
pub fn point_in_polygon(point: LatLng, polygon: &[LatLng]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let pi = polygon[i];
        let pj = polygon[j];
        let intersects = (pi.latitude > point.latitude) != (pj.latitude > point.latitude)
            && point.longitude
                < (pj.longitude - pi.longitude) * (point.latitude - pi.latitude)
                    / (pj.latitude - pi.latitude)
                    + pi.longitude;
        if intersects {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Test whether two line segments intersect (including touching endpoints).
#[must_use]
pub fn segments_intersect(a: LatLng, b: LatLng, c: LatLng, d: LatLng) -> bool {
    const EPS: f64 = 1e-12;
    let o1 = orientation(a, b, c);
    let o2 = orientation(a, b, d);
    let o3 = orientation(c, d, a);
    let o4 = orientation(c, d, b);

    if ((o1 > EPS && o2 < -EPS) || (o1 < -EPS && o2 > EPS))
        && ((o3 > EPS && o4 < -EPS) || (o3 < -EPS && o4 > EPS))
    {
        return true;
    }
    (o1.abs() <= EPS && on_segment(a, b, c))
        || (o2.abs() <= EPS && on_segment(a, b, d))
        || (o3.abs() <= EPS && on_segment(c, d, a))
        || (o4.abs() <= EPS && on_segment(c, d, b))
}

fn orientation(a: LatLng, b: LatLng, c: LatLng) -> f64 {
    (b.longitude - a.longitude) * (c.latitude - a.latitude)
        - (b.latitude - a.latitude) * (c.longitude - a.longitude)
}

fn on_segment(a: LatLng, b: LatLng, point: LatLng) -> bool {
    point.longitude >= a.longitude.min(b.longitude)
        && point.longitude <= a.longitude.max(b.longitude)
        && point.latitude >= a.latitude.min(b.latitude)
        && point.latitude <= a.latitude.max(b.latitude)
}

/// True when any segment of `polyline` crosses any edge of `polygon`, or when
/// a polyline vertex lies inside the polygon.
#[must_use]
pub fn polyline_intersects_polygon(polyline: &[LatLng], polygon: &[LatLng]) -> bool {
    if polygon.len() < 3 || polyline.is_empty() {
        return false;
    }
    if polyline
        .iter()
        .any(|point| point_in_polygon(*point, polygon))
    {
        return true;
    }
    if polyline.len() == 1 {
        return false;
    }
    let edges = polygon
        .iter()
        .copied()
        .zip(polygon.iter().copied().cycle().skip(1))
        .take(polygon.len());
    for pair in polyline.windows(2) {
        for (c, d) in edges.clone() {
            if segments_intersect(pair[0], pair[1], c, d) {
                return true;
            }
        }
    }
    false
}

/// Bounding box helper used by the camera scan's fast rejection step.
#[must_use]
pub fn within_degrees(a: LatLng, b: LatLng, degrees: f64) -> bool {
    (a.latitude - b.latitude).abs() <= degrees && (a.longitude - b.longitude).abs() <= degrees
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people_square() -> LatLng {
        LatLng::new(31.230_4, 121.473_7)
    }

    fn bund() -> LatLng {
        LatLng::new(31.239_7, 121.499_8)
    }

    #[test]
    fn distance_is_in_the_expected_range() {
        let d = distance_m(people_square(), bund());
        assert!((2_300.0..2_700.0).contains(&d), "distance was {d}");
    }

    #[test]
    fn distance_is_symmetric() {
        let a = people_square();
        let b = bund();
        assert!((distance_m(a, b) - distance_m(b, a)).abs() < 0.001);
    }

    #[test]
    fn bearing_east_is_ninety_degrees() {
        let a = LatLng::new(31.0, 121.0);
        let b = LatLng::new(31.0, 121.01);
        assert!((bearing_deg(a, b) - 90.0).abs() < 0.01);
    }

    #[test]
    fn bearing_north_is_zero() {
        let a = LatLng::new(31.0, 121.0);
        let b = LatLng::new(31.01, 121.0);
        assert!(bearing_deg(a, b) < 0.01 || bearing_deg(a, b) > 359.99);
    }

    #[test]
    fn delta_angle_wraps() {
        assert!((delta_angle_deg(355.0, 5.0) - 10.0).abs() < 0.000_1);
        assert!((delta_angle_deg(0.0, 180.0) - 180.0).abs() < 0.000_1);
        assert!((delta_angle_deg(90.0, 90.0)).abs() < 0.000_1);
    }

    #[test]
    fn segment_distance_uses_perpendicular_foot() {
        let point = LatLng::new(31.000_1, 121.0);
        let d = distance_to_segment_m(
            point,
            LatLng::new(31.0, 120.999),
            LatLng::new(31.0, 121.001),
        );
        assert!((d - 11.0).abs() < 2.0, "distance was {d}");
    }

    #[test]
    fn polygon_contains_center() {
        let square = [
            LatLng::new(31.0, 121.0),
            LatLng::new(31.0, 121.1),
            LatLng::new(31.1, 121.1),
            LatLng::new(31.1, 121.0),
        ];
        assert!(point_in_polygon(LatLng::new(31.05, 121.05), &square));
        assert!(!point_in_polygon(LatLng::new(31.2, 121.05), &square));
    }

    #[test]
    fn segment_intersection_detects_crossing() {
        let a = LatLng::new(31.0, 121.0);
        let b = LatLng::new(31.0, 121.1);
        let c = LatLng::new(30.9, 121.05);
        let d = LatLng::new(31.1, 121.05);
        assert!(segments_intersect(a, b, c, d));
        assert!(!segments_intersect(
            a,
            b,
            LatLng::new(31.2, 121.05),
            LatLng::new(31.3, 121.05)
        ));
    }

    #[test]
    fn polyline_polygon_crossing_is_detected() {
        let square = [
            LatLng::new(31.0, 121.0),
            LatLng::new(31.0, 121.02),
            LatLng::new(31.02, 121.02),
            LatLng::new(31.02, 121.0),
        ];
        let crossing = [LatLng::new(31.01, 120.99), LatLng::new(31.01, 121.03)];
        let outside = [LatLng::new(31.05, 120.99), LatLng::new(31.05, 121.03)];
        assert!(polyline_intersects_polygon(&crossing, &square));
        assert!(!polyline_intersects_polygon(&outside, &square));
    }

    #[test]
    fn invalid_coordinates_are_rejected() {
        assert!(!LatLng::new(f64::NAN, 0.0).is_valid());
        assert!(!LatLng::new(91.0, 0.0).is_valid());
        assert!(!LatLng::new(0.0, 181.0).is_valid());
        assert!(people_square().is_valid());
    }
}
