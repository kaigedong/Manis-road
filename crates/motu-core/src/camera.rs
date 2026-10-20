//! Forward-looking camera alert engine, ported from
//! `com.motu.motumap.navi.camera.CameraScanEngine`.
//!
//! Recovered constants:
//! - scan at most every 100 m;
//! - alert radius 300 m, or 500 m for `RESTRICTED_ENTRY`;
//! - 7.5° forward cone;
//! - one alert per camera type, with same-type cameras within 50 m suppressed;
//! - reminders are released after receding 25 m or turning more than 60°.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::geo::{LatLng, bearing_deg, delta_angle_deg, distance_m, within_degrees};
use crate::model::{CameraTabType, TrafficCamera};

/// Current navigation fix. `bearing_deg` is the device heading in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NaviLocation {
    pub coord: LatLng,
    pub bearing_deg: f64,
}

impl NaviLocation {
    #[must_use]
    pub fn new(coord: LatLng, bearing_deg: f64) -> Self {
        Self {
            coord,
            bearing_deg: bearing_deg.rem_euclid(360.0),
        }
    }
}

/// Tunable copy of the recovered constants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanConfig {
    pub scan_interval_m: f64,
    pub forward_cone_deg: f64,
    pub cluster_radius_m: f64,
    pub release_recede_m: f64,
    pub release_turn_deg: f64,
    pub ordinary_radius_m: f64,
    pub restricted_radius_m: f64,
    pub bounding_box_deg: f64,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            scan_interval_m: 100.0,
            forward_cone_deg: 7.5,
            cluster_radius_m: 50.0,
            release_recede_m: 25.0,
            release_turn_deg: 60.0,
            ordinary_radius_m: 300.0,
            restricted_radius_m: 500.0,
            bounding_box_deg: 0.05,
        }
    }
}

impl ScanConfig {
    /// Reject configurations that would make the engine behave unpredictably.
    pub fn validate(&self) -> Result<(), String> {
        let positive = [
            ("scanIntervalM", self.scan_interval_m),
            ("forwardConeDeg", self.forward_cone_deg),
            ("clusterRadiusM", self.cluster_radius_m),
            ("releaseRecedeM", self.release_recede_m),
            ("releaseTurnDeg", self.release_turn_deg),
            ("ordinaryRadiusM", self.ordinary_radius_m),
            ("restrictedRadiusM", self.restricted_radius_m),
            ("boundingBoxDeg", self.bounding_box_deg),
        ];
        for (name, value) in positive {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!("{name} must be a positive finite number"));
            }
        }
        if self.forward_cone_deg > 180.0 {
            return Err("forwardConeDeg must not exceed 180".to_owned());
        }
        if self.restricted_radius_m < self.ordinary_radius_m {
            return Err("restrictedRadiusM must be >= ordinaryRadiusM".to_owned());
        }
        Ok(())
    }
}

/// Result returned for one accepted scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub cameras: Vec<TrafficCamera>,
    pub camera_types: Vec<CameraTabType>,
    pub primary_distance_m: u32,
    /// True when every alerted type is within 50 m of the nearest one.
    /// A single camera is trivially clustered, matching the Java expression
    /// `(farthest - nearest) <= 50`.
    pub clustered: bool,
}

#[derive(Debug, Clone)]
struct ReminderState {
    position: LatLng,
    last_distance_m: f64,
}

#[derive(Debug, Clone)]
struct CameraCandidate {
    camera_index: usize,
    position: LatLng,
    camera_type: CameraTabType,
    distance_m: f64,
}

/// Stateful camera scanner. One instance per navigation session.
#[derive(Debug, Clone)]
pub struct CameraScanEngine {
    config: ScanConfig,
    last_scan_position: Option<LatLng>,
    cameras: Vec<TrafficCamera>,
    reminded: HashMap<String, ReminderState>,
}

impl Default for CameraScanEngine {
    fn default() -> Self {
        Self::new(ScanConfig::default())
    }
}

impl CameraScanEngine {
    #[must_use]
    pub fn new(config: ScanConfig) -> Self {
        Self {
            config,
            last_scan_position: None,
            cameras: Vec::new(),
            reminded: HashMap::new(),
        }
    }

    #[must_use]
    pub const fn config(&self) -> &ScanConfig {
        &self.config
    }

    #[must_use]
    pub fn cameras(&self) -> &[TrafficCamera] {
        &self.cameras
    }

    #[must_use]
    pub fn last_scan_position(&self) -> Option<LatLng> {
        self.last_scan_position
    }

    #[must_use]
    pub fn reminded_count(&self) -> usize {
        self.reminded.len()
    }

    /// Replace the camera set. Cameras without a valid coordinate are ignored,
    /// as in `updateCameras`.
    pub fn update_cameras(&mut self, cameras: &[TrafficCamera]) {
        self.cameras = cameras
            .iter()
            .filter(|camera| camera.position.is_valid())
            .cloned()
            .collect();
    }

    /// Clear the gate and active reminders but keep the loaded cameras.
    pub fn pause(&mut self) {
        self.last_scan_position = None;
        self.reminded.clear();
    }

    /// Full reset, including loaded cameras.
    pub fn reset(&mut self) {
        self.last_scan_position = None;
        self.cameras.clear();
        self.reminded.clear();
    }

    #[must_use]
    pub fn should_scan(&self, position: LatLng) -> bool {
        self.last_scan_position
            .is_none_or(|last| distance_m(last, position) >= self.config.scan_interval_m)
    }

    /// Run one navigation tick.
    ///
    /// `allow_scan` mirrors the Java boolean (for example, false while the
    /// navigation view is not visible). Passed cameras are released even when
    /// a new scan is not allowed.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the recovered Java code rounds a float distance to an int"
    )]
    pub fn scan(
        &mut self,
        location: Option<&NaviLocation>,
        allow_scan: bool,
    ) -> Option<ScanResult> {
        let location = location?;
        let position = location.coord;
        if !position.is_valid() || !location.bearing_deg.is_finite() {
            return None;
        }

        self.release_passed_cameras(position, location.bearing_deg);

        if !allow_scan || !self.should_scan(position) {
            return None;
        }
        self.last_scan_position = Some(position);

        if self.cameras.is_empty() {
            return None;
        }

        let mut candidates = self.find_forward_candidates(position, location.bearing_deg);
        if candidates.is_empty() {
            return None;
        }
        candidates.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));

        // Deduplicate by primary type, keeping the nearest camera of each type.
        let mut type_order: Vec<CameraTabType> = Vec::new();
        let mut nearest_by_type: HashMap<CameraTabType, usize> = HashMap::new();
        for (index, candidate) in candidates.iter().enumerate() {
            if nearest_by_type.contains_key(&candidate.camera_type) {
                continue;
            }
            nearest_by_type.insert(candidate.camera_type, index);
            type_order.push(candidate.camera_type);
        }

        self.mark_reminded_cameras(position, &candidates, &nearest_by_type);

        let primary_distance = candidates[0].distance_m;
        let farthest = type_order
            .iter()
            .filter_map(|kind| nearest_by_type.get(kind))
            .map(|index| candidates[*index].distance_m)
            .fold(primary_distance, f64::max);

        let cameras = type_order
            .iter()
            .filter_map(|kind| nearest_by_type.get(kind))
            .map(|index| self.cameras[candidates[*index].camera_index].clone())
            .collect();

        Some(ScanResult {
            cameras,
            camera_types: type_order,
            primary_distance_m: primary_distance.round().max(1.0) as u32,
            clustered: (farthest - primary_distance) <= self.config.cluster_radius_m,
        })
    }

    fn find_forward_candidates(&self, position: LatLng, bearing: f64) -> Vec<CameraCandidate> {
        let mut candidates = Vec::new();
        for (camera_index, camera) in self.cameras.iter().enumerate() {
            if !within_degrees(camera.position, position, self.config.bounding_box_deg) {
                continue;
            }
            let key = self.reminder_key(camera);
            if key.is_empty() || self.reminded.contains_key(&key) {
                continue;
            }
            let camera_type = camera.primary_type();
            let distance = distance_m(position, camera.position);
            let radius = self.reminder_radius_m(camera_type);
            if distance > radius {
                continue;
            }
            let camera_bearing = bearing_deg(position, camera.position);
            if delta_angle_deg(camera_bearing, bearing) > self.config.forward_cone_deg {
                continue;
            }
            candidates.push(CameraCandidate {
                camera_index,
                position: camera.position,
                camera_type,
                distance_m: distance,
            });
        }
        candidates
    }

    fn mark_reminded_cameras(
        &mut self,
        position: LatLng,
        candidates: &[CameraCandidate],
        nearest_by_type: &HashMap<CameraTabType, usize>,
    ) {
        for index in nearest_by_type.values() {
            let chosen = &candidates[*index];
            self.mark_reminded(position, chosen);

            // Suppress the same camera type within the cluster radius, even if
            // it was not the nearest member of that type.
            for other in candidates {
                if other.camera_index == chosen.camera_index
                    || other.camera_type != chosen.camera_type
                {
                    continue;
                }
                if distance_m(chosen.position, other.position) <= self.config.cluster_radius_m {
                    self.mark_reminded(position, other);
                }
            }
        }
    }

    fn mark_reminded(&mut self, position: LatLng, candidate: &CameraCandidate) {
        let key = self.reminder_key(&self.cameras[candidate.camera_index]);
        if key.is_empty() {
            return;
        }
        self.reminded.insert(
            key,
            ReminderState {
                position: candidate.position,
                last_distance_m: distance_m(position, candidate.position),
            },
        );
    }

    fn release_passed_cameras(&mut self, position: LatLng, bearing: f64) {
        self.reminded.retain(|_, state| {
            let distance = distance_m(position, state.position);
            let receded = distance > state.last_distance_m + self.config.release_recede_m;
            let turned = delta_angle_deg(bearing_deg(position, state.position), bearing)
                > self.config.release_turn_deg;
            if receded || turned {
                false
            } else {
                state.last_distance_m = distance;
                true
            }
        });
    }

    #[must_use]
    pub fn reminder_radius_m(&self, kind: CameraTabType) -> f64 {
        if kind == CameraTabType::RestrictedEntry {
            self.config.restricted_radius_m
        } else {
            self.config.ordinary_radius_m
        }
    }

    /// `memberByString` when present, otherwise `lat,lon:typeId`.
    #[must_use]
    pub fn reminder_key(&self, camera: &TrafficCamera) -> String {
        if let Some(key) = camera.stable_key() {
            return key.to_owned();
        }
        format!(
            "{},{}:{}",
            java_double(camera.position.latitude),
            java_double(camera.position.longitude),
            camera.primary_type().id()
        )
    }
}

/// Format a double the way Java's `Double.toString` does for ordinary
/// coordinate values (at least one fractional digit).
fn java_double(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CameraTabType::{RedLight, RestrictedEntry, Speeding};

    /// Rough metre offsets at the test latitude.
    fn north(meters: f64) -> LatLng {
        LatLng::new(31.0 + meters / 111_320.0, 121.0)
    }

    fn camera(id: &str, at: LatLng, kind: CameraTabType) -> TrafficCamera {
        TrafficCamera::new(id, at, [kind])
    }

    fn loc(at: LatLng, bearing: f64) -> NaviLocation {
        NaviLocation::new(at, bearing)
    }

    #[test]
    fn alerts_on_camera_directly_ahead() {
        let mut engine = CameraScanEngine::default();
        engine.update_cameras(&[camera("a", north(150.0), Speeding)]);
        let result = engine
            .scan(Some(&loc(LatLng::new(31.0, 121.0), 0.0)), true)
            .expect("camera ahead should alert");
        assert_eq!(result.cameras.len(), 1);
        assert_eq!(result.camera_types, vec![Speeding]);
        assert!((140..=160).contains(&result.primary_distance_m));
        // The Java expression is `(farthest - nearest) <= 50`, so one camera
        // is reported as clustered.
        assert!(result.clustered);
    }

    #[test]
    fn ignores_camera_behind() {
        let mut engine = CameraScanEngine::default();
        engine.update_cameras(&[camera("a", north(-150.0), Speeding)]);
        assert!(
            engine
                .scan(Some(&loc(LatLng::new(31.0, 121.0), 0.0)), true)
                .is_none()
        );
    }

    #[test]
    fn forward_cone_is_seven_point_five_degrees() {
        let mut engine = CameraScanEngine::default();
        // ~100 m north, ~8.7 m east => ~5 degrees.
        let five_deg = LatLng::new(31.0 + 100.0 / 111_320.0, 121.0 + 8.7 / 95_000.0);
        engine.update_cameras(&[camera("a", five_deg, Speeding)]);
        assert!(
            engine
                .scan(Some(&loc(LatLng::new(31.0, 121.0), 0.0)), true)
                .is_some()
        );

        let mut engine = CameraScanEngine::default();
        // ~17.6 m east => ~10 degrees.
        let ten_deg = LatLng::new(31.0 + 100.0 / 111_320.0, 121.0 + 17.6 / 95_000.0);
        engine.update_cameras(&[camera("a", ten_deg, Speeding)]);
        assert!(
            engine
                .scan(Some(&loc(LatLng::new(31.0, 121.0), 0.0)), true)
                .is_none()
        );
    }

    #[test]
    fn restricted_entry_uses_five_hundred_metre_radius() {
        let mut engine = CameraScanEngine::default();
        engine.update_cameras(&[camera("restricted", north(400.0), RestrictedEntry)]);
        assert!(
            engine
                .scan(Some(&loc(LatLng::new(31.0, 121.0), 0.0)), true)
                .is_some()
        );

        let mut engine = CameraScanEngine::default();
        engine.update_cameras(&[camera("ordinary", north(400.0), Speeding)]);
        assert!(
            engine
                .scan(Some(&loc(LatLng::new(31.0, 121.0), 0.0)), true)
                .is_none()
        );
    }

    #[test]
    fn scan_gate_is_one_hundred_metres() {
        let mut engine = CameraScanEngine::default();
        // First tick establishes the 100 m gate even with no cameras loaded.
        assert!(engine.scan(Some(&loc(north(0.0), 0.0)), true).is_none());
        engine.update_cameras(&[camera("a", north(400.0), RestrictedEntry)]);

        // 50 m later: too soon.
        assert!(engine.scan(Some(&loc(north(50.0), 0.0)), true).is_none());
        // 120 m later: accepted and the camera is still ahead.
        assert!(engine.scan(Some(&loc(north(120.0), 0.0)), true).is_some());
    }

    #[test]
    fn clusters_two_types_within_fifty_metres() {
        let mut engine = CameraScanEngine::default();
        engine.update_cameras(&[
            camera("speed", north(120.0), Speeding),
            camera("light", north(150.0), RedLight),
        ]);
        let result = engine
            .scan(Some(&loc(LatLng::new(31.0, 121.0), 0.0)), true)
            .unwrap();
        assert_eq!(result.cameras.len(), 2);
        assert_eq!(result.camera_types, vec![Speeding, RedLight]);
        assert!(result.clustered);
    }

    #[test]
    fn deduplicates_same_type_and_suppresses_nearby_cluster() {
        let mut engine = CameraScanEngine::default();
        engine.update_cameras(&[
            camera("near", north(100.0), Speeding),
            camera("far", north(130.0), Speeding),
        ]);
        let result = engine
            .scan(Some(&loc(LatLng::new(31.0, 121.0), 0.0)), true)
            .unwrap();
        assert_eq!(result.cameras.len(), 1);
        assert_eq!(result.cameras[0].id, "near");
        // Both are remembered because they are within 50 m of each other.
        assert_eq!(engine.reminded_count(), 2);
    }

    #[test]
    fn releases_reminder_after_receding_twenty_five_metres() {
        let mut engine = CameraScanEngine::default();
        engine.update_cameras(&[camera("a", north(100.0), Speeding)]);
        assert!(engine.scan(Some(&loc(north(0.0), 0.0)), true).is_some());
        assert_eq!(engine.reminded_count(), 1);

        // Move away by 30 m while still facing the camera; release runs even
        // when scanning is not allowed.
        let _ = engine.scan(Some(&loc(north(-30.0), 0.0)), false);
        assert_eq!(engine.reminded_count(), 0);
    }

    #[test]
    fn releases_reminder_after_sharp_turn() {
        let mut engine = CameraScanEngine::default();
        engine.update_cameras(&[camera("a", north(100.0), Speeding)]);
        assert!(engine.scan(Some(&loc(north(0.0), 0.0)), true).is_some());
        assert_eq!(engine.reminded_count(), 1);

        // Still close, but now driving south: the camera is behind.
        let _ = engine.scan(Some(&loc(north(10.0), 180.0)), false);
        assert_eq!(engine.reminded_count(), 0);
    }

    #[test]
    fn pause_keeps_cameras_but_clears_gate_and_reminders() {
        let mut engine = CameraScanEngine::default();
        engine.update_cameras(&[camera("a", north(100.0), Speeding)]);
        assert!(engine.scan(Some(&loc(north(0.0), 0.0)), true).is_some());
        engine.pause();
        assert_eq!(engine.reminded_count(), 0);
        assert!(engine.last_scan_position().is_none());
        assert_eq!(engine.cameras().len(), 1);

        engine.reset();
        assert_eq!(engine.cameras().len(), 0);
    }

    #[test]
    fn derived_reminder_key_uses_type() {
        let engine = CameraScanEngine::default();
        let speed = TrafficCamera::new("a", LatLng::new(31.0, 121.0), [Speeding]);
        let light = TrafficCamera::new("b", LatLng::new(31.0, 121.0), [RedLight]);
        assert_eq!(engine.reminder_key(&speed), "31.0,121.0:5");
        assert_eq!(engine.reminder_key(&light), "31.0,121.0:4");
    }

    #[test]
    fn invalid_camera_coordinates_are_dropped() {
        let mut engine = CameraScanEngine::default();
        engine.update_cameras(&[
            TrafficCamera::new("bad", LatLng::new(f64::NAN, 121.0), [Speeding]),
            camera("good", north(100.0), Speeding),
        ]);
        assert_eq!(engine.cameras().len(), 1);
    }

    #[test]
    fn scan_without_fix_or_permission_is_none() {
        let mut engine = CameraScanEngine::default();
        engine.update_cameras(&[camera("a", north(100.0), Speeding)]);
        assert!(engine.scan(None, true).is_none());
        assert!(engine.scan(Some(&loc(north(0.0), 0.0)), false).is_none());
    }

    #[test]
    fn config_validation_rejects_nonsense() {
        assert!(ScanConfig::default().validate().is_ok());
        let config = ScanConfig {
            forward_cone_deg: 0.0,
            ..ScanConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
