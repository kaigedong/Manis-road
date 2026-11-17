//! Wire and domain models recovered from `com.motu.api.*`.
//!
//! Field names mirror the Gson payloads so a request can be serialised and
//! signed without a second model layer. Optional fields are omitted when
//! absent, matching Gson's default null handling.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::geo::LatLng;

/// `com.motu.api.base.ApiResultResponse`: code 0 means success.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiEnvelope<T> {
    pub code: i32,
    pub message: Option<String>,
    pub data: Option<T>,
}

impl<T> ApiEnvelope<T> {
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.code == 0
    }
}

/// `com.motu.api.user.response.UserInfoLoginResult`.
///
/// The server omits many fields depending on the account, so every field is
/// optional on the wire. `isPro` is an integer in the original response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginUser {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub nick_name: String,
    #[serde(default)]
    pub real_name: String,
    #[serde(default)]
    pub photo: String,
    #[serde(default, deserialize_with = "null_i32")]
    pub is_pro: i32,
    #[serde(default, deserialize_with = "crate::wire_time::option_millis")]
    pub mem_start_time: Option<i64>,
    #[serde(default, deserialize_with = "crate::wire_time::option_millis")]
    pub mem_end_time: Option<i64>,
    #[serde(default, deserialize_with = "null_bool")]
    pub has_pwd: bool,
    #[serde(default, deserialize_with = "null_i32")]
    pub exp_count: i32,
    #[serde(default, deserialize_with = "null_i32")]
    pub cruise_count: i32,
    #[serde(default, deserialize_with = "crate::wire_time::option_millis")]
    pub create_time: Option<i64>,
    /// Nullable on the wire: accounts without a profile return `sex: null`.
    #[serde(default)]
    pub sex: Option<i32>,
    #[serde(default, deserialize_with = "null_i32")]
    pub r#type: i32,
    /// Refresh credential returned by `v4/user/login`; not persisted yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

fn null_i32<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<i32>::deserialize(deserializer)?.unwrap_or_default())
}

fn null_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<bool>::deserialize(deserializer)?.unwrap_or_default())
}

/// Cursor page returned by list endpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorPage<T> {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub pre_cursor: Option<String>,
    #[serde(default)]
    pub cdn_prefix: Option<String>,
    #[serde(default)]
    pub total: Option<i64>,
    #[serde(default = "Vec::new")]
    pub list: Vec<T>,
}

/// Camera categories from `com.motu.api.poi.CameraTabType`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CameraTabType {
    Unknown = 0,
    RestrictedEntry = 1,
    EmergencyLane = 2,
    NonMotorLane = 3,
    RedLight = 4,
    Speeding = 5,
    BusLane = 6,
    LaneViolation = 7,
    ReverseDriving = 8,
    TailNumberRestriction = 9,
    NoHelmet = 10,
    Noise = 11,
    OnSiteTicket = 12,
    LinePressing = 13,
    ImproperLighting = 14,
}

impl CameraTabType {
    /// Java's `fromValue` falls back to `UNKNOWN` for unrecognised ids.
    #[must_use]
    pub const fn from_id(id: u8) -> Self {
        match id {
            1 => Self::RestrictedEntry,
            2 => Self::EmergencyLane,
            3 => Self::NonMotorLane,
            4 => Self::RedLight,
            5 => Self::Speeding,
            6 => Self::BusLane,
            7 => Self::LaneViolation,
            8 => Self::ReverseDriving,
            9 => Self::TailNumberRestriction,
            10 => Self::NoHelmet,
            11 => Self::Noise,
            12 => Self::OnSiteTicket,
            13 => Self::LinePressing,
            14 => Self::ImproperLighting,
            _ => Self::Unknown,
        }
    }

    #[must_use]
    pub const fn id(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn label_zh(self) -> &'static str {
        match self {
            Self::Unknown => "未知类型",
            Self::RestrictedEntry => "闯限行",
            Self::EmergencyLane => "走应急车道",
            Self::NonMotorLane => "走非机动车道",
            Self::RedLight => "闯红灯",
            Self::Speeding => "超速",
            Self::BusLane => "公交车道",
            Self::LaneViolation => "未按车道行驶",
            Self::ReverseDriving => "逆行",
            Self::TailNumberRestriction => "尾号限行",
            Self::NoHelmet => "不带头盔",
            Self::Noise => "噪音",
            Self::OnSiteTicket => "现场罚单",
            Self::LinePressing => "压线",
            Self::ImproperLighting => "违规用灯",
        }
    }
}

impl fmt::Display for CameraTabType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label_zh())
    }
}

impl Serialize for CameraTabType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.id())
    }
}

impl<'de> Deserialize<'de> for CameraTabType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = u8::deserialize(deserializer)?;
        Ok(Self::from_id(raw))
    }
}

/// A camera supplied by the caller's own dataset.
///
/// `member_by_string` is Motu's stable reminder key; when empty the engine
/// derives `lat,lon:type` exactly like `CameraScanEngine.getReminderKey`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficCamera {
    pub id: String,
    pub position: LatLng,
    #[serde(default)]
    pub tab_types: Vec<CameraTabType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_by_string: Option<String>,
    #[serde(default)]
    pub new_flag: bool,
    #[serde(default)]
    pub white: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_m: Option<f64>,
}

impl TrafficCamera {
    /// Stable identity for local caching: the server `memberByString` when it
    /// is present, otherwise the synthetic `lat,lon:type` id.
    #[must_use]
    pub fn cache_key(&self) -> String {
        self.member_by_string
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map_or_else(|| self.id.clone(), str::to_owned)
    }

    #[must_use]
    pub fn new(
        id: impl Into<String>,
        position: LatLng,
        tab_types: impl IntoIterator<Item = CameraTabType>,
    ) -> Self {
        Self {
            id: id.into(),
            position,
            tab_types: tab_types.into_iter().collect(),
            member: None,
            member_by_string: None,
            new_flag: false,
            white: false,
            distance_m: None,
        }
    }

    /// First tab type wins, matching `CameraScanEngine.getCameraType`.
    #[must_use]
    pub fn primary_type(&self) -> CameraTabType {
        self.tab_types
            .first()
            .copied()
            .unwrap_or(CameraTabType::Unknown)
    }

    #[must_use]
    pub fn stable_key(&self) -> Option<&str> {
        self.member_by_string
            .as_deref()
            .filter(|value| !value.is_empty())
    }
}

/// One encoded route step from `com.motu.api.poi.entity.PolylineEntity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PolylineStep {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polyline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polyline_encode: Option<String>,
    /// Deprecated in the recovered model; retained for older payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city_codes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ad_codes: Option<Vec<String>>,
}

impl PolylineStep {
    #[must_use]
    pub fn encoded(value: impl Into<String>) -> Self {
        Self {
            polyline_encode: Some(value.into()),
            ..Self::default()
        }
    }
}

/// `com.motu.api.poi.body.PathAvoidBody`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PathAvoidRequest {
    #[serde(default)]
    pub gd_path_steps: Vec<PolylineStep>,
    /// 1 = cross-city / no plate comparison, 2 = same-city plate comparison.
    #[serde(default)]
    pub r#type: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displacement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plate_val: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_city_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_types: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(default)]
    pub loop_type: i32,
    #[serde(default)]
    pub elevated_type: i32,
    #[serde(default)]
    pub restriction_type: i32,
    #[serde(default)]
    pub capture_type: i32,
    #[serde(default)]
    pub user_avoid_type: i32,
    #[serde(default)]
    pub avoid_white_type: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_avoid_area_ids: Option<Vec<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uk_id: Option<String>,
}

impl PathAvoidRequest {
    /// Constructor logic shared by both recovered Java constructors.
    #[must_use]
    pub fn infer_trip_type(
        plate: Option<&str>,
        city_code: Option<&str>,
        to_city_code: Option<&str>,
    ) -> i32 {
        match (plate, city_code, to_city_code) {
            (Some(plate), Some(city), Some(to))
                if !plate.is_empty() && !city.is_empty() && !to.is_empty() && city == to =>
            {
                2
            }
            _ => 1,
        }
    }

    /// Fills `type` from the same-city rule.
    pub fn apply_trip_type(&mut self) {
        self.r#type = Self::infer_trip_type(
            self.plate_val.as_deref(),
            self.city_code.as_deref(),
            self.to_city_code.as_deref(),
        );
    }
}

/// `com.motu.api.poi.response.PathAvoidResult`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PathAvoidResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avoid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pos: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_pass_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_pass_pos: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_pass_city_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_fail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub biz_type: Option<i32>,
    #[serde(default)]
    pub has_avoid_white: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub already_cal_path: Option<Vec<PolylineStep>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avoid_area_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avoid_type: Option<String>,
}

/// `com.motu.api.poi.body.PathAreaBody` (start/end quick area route).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PathAreaRequest {
    pub start_pos: String,
    pub end_pos: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plate_val: Option<String>,
}

/// One avoid/white area from `MapAreaResult.MotuMapAreaEntity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotuMapArea {
    pub id: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub pos_encode: Option<String>,
    #[serde(default)]
    pub shape_type: Option<String>,
}

impl MotuMapArea {
    #[must_use]
    pub fn is_polygon(&self) -> bool {
        self.shape_type.as_deref() == Some("polygon")
    }
}

/// Alias used by the route module for decoded areas.
pub type MapArea = MotuMapArea;

/// `com.motu.api.poi.response.MapAreaResult`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MapAreaResult {
    #[serde(default)]
    pub list: Vec<MotuMapArea>,
    #[serde(default)]
    pub version: i64,
}

/// Convenience conversion for coordinate entities.
impl From<LatLng> for (f64, f64) {
    fn from(value: LatLng) -> Self {
        (value.latitude, value.longitude)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_tab_type_serialises_as_number() {
        let json = serde_json::to_string(&CameraTabType::RestrictedEntry).unwrap();
        assert_eq!(json, "1");
        let parsed: CameraTabType = serde_json::from_str("14").unwrap();
        assert_eq!(parsed, CameraTabType::ImproperLighting);
        let unknown: CameraTabType = serde_json::from_str("99").unwrap();
        assert_eq!(unknown, CameraTabType::Unknown);
    }

    #[test]
    fn camera_labels_match_recovered_enum() {
        assert_eq!(CameraTabType::RestrictedEntry.label_zh(), "闯限行");
        assert_eq!(CameraTabType::ImproperLighting.id(), 14);
        assert_eq!(
            CameraTabType::from_id(9),
            CameraTabType::TailNumberRestriction
        );
    }

    #[test]
    fn trip_type_is_two_only_for_same_city_with_plate() {
        assert_eq!(
            PathAvoidRequest::infer_trip_type(Some("沪A12345"), Some("310000"), Some("310000")),
            2
        );
        assert_eq!(
            PathAvoidRequest::infer_trip_type(Some("沪A12345"), Some("310000"), Some("320000")),
            1
        );
        assert_eq!(
            PathAvoidRequest::infer_trip_type(None, Some("310000"), Some("310000")),
            1
        );
    }

    #[test]
    fn avoid_result_parses_wire_payload() {
        let raw = r#"{
            "avoid": "1",
            "lastPos": "121.4,31.2",
            "noPassId": 8,
            "hasAvoidWhite": true,
            "alreadyCalPath": [{"polylineEncode": "abc"}],
            "avoidAreaId": 3
        }"#;
        let parsed: PathAvoidResult = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.no_pass_id, Some(8));
        assert!(parsed.has_avoid_white);
        assert_eq!(
            parsed.already_cal_path.unwrap()[0]
                .polyline_encode
                .as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn camera_primary_type_is_first_tab() {
        let camera = TrafficCamera::new(
            "c1",
            LatLng::new(31.0, 121.0),
            [CameraTabType::Speeding, CameraTabType::RedLight],
        );
        assert_eq!(camera.primary_type(), CameraTabType::Speeding);
    }
}
