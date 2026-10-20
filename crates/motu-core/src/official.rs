//! Official camera-point synchronization for a signed-in account.
//!
//! The recovered `MotuMapViewModel.searchCameras` calls
//! `v1/poi/searchRulePoiSec` with `CameraSearchRuleBody` and renders the
//! returned `TrafficCamerasPoiResponse` list. This module reproduces the
//! request and wire parsing only. The FFI keeps the result in the in-memory
//! scan engine; it is deliberately not written to the JSON store.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{SecurityClient, Transport, endpoints};
use crate::error::{MotuError, Result};
use crate::geo::LatLng;
use crate::model::{ApiEnvelope, CameraTabType, TrafficCamera};

/// Default category ids from `GeneralCacheHolder.getCityTabType()`.
pub const DEFAULT_TAB_TYPES: [u8; 15] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 14, 15, 16];

/// Parameters recovered from `CameraSearchRuleBody`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficialCameraQuery {
    pub position: LatLng,
    #[serde(default)]
    pub city_code: String,
    #[serde(default)]
    pub to_city_code: String,
    #[serde(default = "default_tab_types")]
    pub tab_types: Vec<u8>,
    #[serde(default)]
    pub plate_color: String,
    #[serde(default)]
    pub plate_val: String,
    /// `true` sets `model: "CRUISE"`; the normal map layer leaves it off.
    #[serde(default)]
    pub cruise: bool,
    #[serde(default = "default_source")]
    pub source: i32,
}

const fn default_source() -> i32 {
    1
}

/// The result returned to the UI. `cameras` uses the core [`TrafficCamera`]
/// model, whose reminder key still prefers `memberByString` like the original
/// scan engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficialCameraSync {
    pub cameras: Vec<TrafficCamera>,
    pub city_code: String,
    pub fetched_at_ms: i64,
}

/// Aggregated data shown after tapping a camera marker.
///
/// The original client issues `trafficEyesDetailSec` and
/// `trafficTicketListSec` in parallel and merges both payloads into one bottom
/// sheet. This type keeps the same shape but flattens the nested rule lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraDetail {
    pub camera_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<LatLng>,
    #[serde(default)]
    pub types: Vec<CameraDetailType>,
    #[serde(default)]
    pub rules: Vec<CameraRule>,
    #[serde(default)]
    pub tickets_count: i64,
    #[serde(default)]
    pub praise_count: i64,
    #[serde(default)]
    pub comment_count: i64,
    #[serde(default)]
    pub white: bool,
    #[serde(default)]
    pub avoid: bool,
    #[serde(default)]
    pub avoid_type: i32,
    #[serde(default)]
    pub tickets: Vec<CameraTicket>,
}

/// One `eyePoiTabs` entry: the camera category and its explanatory remark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraDetailType {
    pub tab_type: CameraTabType,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub remark: String,
}

/// One flattened `ruleInfoList` entry, e.g. "外地车牌 / 工作日 / 07:00-20:00".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraRule {
    #[serde(default)]
    pub plate_name: String,
    #[serde(default)]
    pub day_type_names: Vec<String>,
    #[serde(default)]
    pub tail: Vec<i32>,
    #[serde(default)]
    pub time: String,
}

/// One ticket returned by `trafficTicketListSec`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraTicket {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub remark: String,
    #[serde(default)]
    pub photo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_date: Option<i64>,
    #[serde(default)]
    pub contact_person: String,
    #[serde(default)]
    pub user_photo: String,
}

#[must_use]
pub fn default_tab_types() -> Vec<u8> {
    DEFAULT_TAB_TYPES.to_vec()
}

impl Default for OfficialCameraQuery {
    fn default() -> Self {
        Self {
            position: LatLng::new(0.0, 0.0),
            city_code: "010".to_owned(),
            to_city_code: "010".to_owned(),
            tab_types: default_tab_types(),
            plate_color: "YELLOW".to_owned(),
            plate_val: String::new(),
            cruise: false,
            source: 1,
        }
    }
}

/// Build the JSON body exactly as Gson serialises `CameraSearchRuleBody`.
///
/// Gson omits null fields, so `displacement`, `time` and non-cruise `model`
/// are absent. The recovered caller appends a trailing comma to `tabTypes`;
/// the server accepted that form in the live probe.
#[must_use]
pub fn camera_search_body(query: &OfficialCameraQuery) -> Value {
    let city = if query.city_code.trim().is_empty() {
        "010"
    } else {
        query.city_code.trim()
    };
    let to_city = if query.to_city_code.trim().is_empty() {
        city
    } else {
        query.to_city_code.trim()
    };
    let query_type = if city.is_empty() || to_city.is_empty() {
        "1"
    } else {
        "2"
    };
    let mut body = json!({
        "lon": query.position.longitude,
        "lat": query.position.latitude,
        "cityCode": city,
        "toCityCode": to_city,
        "type": query_type,
        "tabTypes": tab_types_string(&query.tab_types),
        "color": query.plate_color.trim(),
        "plateVal": query.plate_val.trim(),
        "source": query.source,
    });
    if query.cruise {
        body["model"] = Value::String("CRUISE".to_owned());
    }
    body
}

fn tab_types_string(ids: &[u8]) -> String {
    use std::fmt::Write as _;

    let ids = if ids.is_empty() {
        DEFAULT_TAB_TYPES.as_slice()
    } else {
        ids
    };
    let mut out = String::new();
    for id in ids {
        let _ = write!(out, "{id},");
    }
    out
}

/// Fetch the security config, then query official camera points.
///
/// `headers` must include the signed-in `token`; the endpoint returns
/// `code: 20105` when the token is missing or invalid.
pub fn sync_official_cameras<T: Transport>(
    transport: &T,
    base_url: &str,
    headers: &[(String, String)],
    query: &OfficialCameraQuery,
    now_ms: i64,
) -> Result<OfficialCameraSync> {
    if !query.position.is_valid() {
        return Err(MotuError::InvalidCoordinate(
            "official camera search position is not a valid latitude/longitude".to_owned(),
        ));
    }
    let client = SecurityClient::fetch_config(transport, base_url, headers, now_ms)?;
    let cameras = search_official_cameras(
        transport,
        &client,
        base_url,
        headers,
        query,
        (now_ms, now_ms / 1000),
    )?;
    Ok(OfficialCameraSync {
        cameras,
        city_code: camera_search_body(query)["cityCode"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        fetched_at_ms: now_ms,
    })
}

/// Load one camera's detail and ticket list.
///
/// Both requests use the same encrypted body (`trafficEyesId`) and the same
/// security config, matching the original client's `Observable.zip`. The
/// returned detail always owns its ticket list; an empty list means the server
/// returned no tickets.
pub fn load_camera_detail<T: Transport>(
    transport: &T,
    base_url: &str,
    headers: &[(String, String)],
    camera_id: &str,
    now_ms: i64,
) -> Result<CameraDetail> {
    let camera_id = camera_id.trim();
    if camera_id.is_empty() {
        return Err(MotuError::Api("trafficEyesId must not be empty".to_owned()));
    }
    let client = SecurityClient::fetch_config(transport, base_url, headers, now_ms)?;
    let body = json!({ "trafficEyesId": camera_id });
    let now = (now_ms, now_ms / 1000);
    let detail = client.post_encrypted(
        transport,
        &endpoints::LOAD_TRAFFIC_CAMERA_DETAIL_SEC,
        base_url,
        headers,
        &body,
        now,
    )?;
    let tickets = client.post_encrypted(
        transport,
        &endpoints::LOAD_TRAFFIC_TICKET_LIST_SEC,
        base_url,
        headers,
        &body,
        now,
    )?;
    let mut detail = parse_camera_detail(&detail, camera_id)?;
    detail.tickets = parse_ticket_list(&tickets)?;
    Ok(detail)
}

/// Query `v1/poi/searchRulePoiSec` with an existing security config.
pub fn search_official_cameras<T: Transport>(
    transport: &T,
    client: &SecurityClient,
    base_url: &str,
    headers: &[(String, String)],
    query: &OfficialCameraQuery,
    now: (i64, i64),
) -> Result<Vec<TrafficCamera>> {
    let body = camera_search_body(query);
    let plaintext = client.post_encrypted(
        transport,
        &endpoints::SEARCH_RULE_POI_SEC,
        base_url,
        headers,
        &body,
        now,
    )?;
    parse_camera_response(&plaintext)
}

fn parse_camera_response(plaintext: &[u8]) -> Result<Vec<TrafficCamera>> {
    let envelope: ApiEnvelope<Vec<CameraPoiWire>> = serde_json::from_slice(plaintext)?;
    if !envelope.is_success() {
        let code = envelope.code;
        let message = envelope
            .message
            .unwrap_or_else(|| format!("official camera search failed with code {code}"));
        return Err(MotuError::Server(message));
    }
    Ok(envelope
        .data
        .unwrap_or_default()
        .into_iter()
        .filter_map(CameraPoiWire::into_camera)
        .collect())
}

fn parse_camera_detail(plaintext: &[u8], requested_id: &str) -> Result<CameraDetail> {
    let envelope: ApiEnvelope<CameraDetailWire> = serde_json::from_slice(plaintext)?;
    if !envelope.is_success() {
        let code = envelope.code;
        let message = envelope
            .message
            .unwrap_or_else(|| format!("camera detail failed with code {code}"));
        return Err(MotuError::Server(message));
    }
    let wire = envelope
        .data
        .ok_or_else(|| MotuError::Server("camera detail response has no data".to_owned()))?;
    Ok(wire.into_detail(requested_id))
}

fn parse_ticket_list(plaintext: &[u8]) -> Result<Vec<CameraTicket>> {
    let envelope: ApiEnvelope<TicketListWire> = serde_json::from_slice(plaintext)?;
    if !envelope.is_success() {
        let code = envelope.code;
        let message = envelope
            .message
            .unwrap_or_else(|| format!("camera ticket list failed with code {code}"));
        return Err(MotuError::Server(message));
    }
    Ok(envelope
        .data
        .and_then(|data| data.tickets)
        .unwrap_or_default()
        .into_iter()
        .map(TicketWire::into_ticket)
        .collect())
}

fn clean_text(value: Option<String>) -> String {
    value.map_or_else(String::new, |text| text.trim().to_owned())
}

fn parse_position(value: &str) -> Option<LatLng> {
    let mut parts = value.split(',');
    let longitude = parts.next()?.trim().parse::<f64>().ok()?;
    let latitude = parts.next()?.trim().parse::<f64>().ok()?;
    let position = LatLng::new(latitude, longitude);
    position.is_valid().then_some(position)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CameraDetailWire {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    tittle: Option<String>,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    address: Option<String>,
    #[serde(default)]
    pos: Option<String>,
    #[serde(default)]
    eye_poi_tabs: Option<Vec<CameraTypeWire>>,
    #[serde(default)]
    rule_info_list: Option<Vec<Vec<RuleInfoWire>>>,
    #[serde(default)]
    tickets_count: Option<StringOrNumber>,
    #[serde(default)]
    praise_count: Option<i64>,
    #[serde(default)]
    comment_count: Option<i64>,
    #[serde(default)]
    white: Option<BoolOrInt>,
    #[serde(default)]
    avoid: Option<BoolOrInt>,
    #[serde(default)]
    avoid_type: Option<i32>,
}

impl CameraDetailWire {
    fn into_detail(self, requested_id: &str) -> CameraDetail {
        let types = self
            .eye_poi_tabs
            .unwrap_or_default()
            .into_iter()
            .map(CameraTypeWire::into_type)
            .collect();
        let rules = self
            .rule_info_list
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .map(RuleInfoWire::into_rule)
            .collect();
        CameraDetail {
            camera_id: self
                .id
                .map_or_else(|| requested_id.to_owned(), |id| id.to_string()),
            title: clean_text(self.tittle),
            detail: clean_text(self.detail),
            address: clean_text(self.address),
            position: self.pos.as_deref().and_then(parse_position),
            types,
            rules,
            tickets_count: self.tickets_count.map_or(0, |value| value.to_i64()),
            praise_count: self.praise_count.unwrap_or(0),
            comment_count: self.comment_count.unwrap_or(0),
            white: self.white.is_some_and(|value| value.is_true()),
            avoid: self.avoid.is_some_and(|value| value.is_true()),
            avoid_type: self.avoid_type.unwrap_or(0),
            tickets: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CameraTypeWire {
    #[serde(default)]
    tab_type: Option<i32>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    remark: Option<String>,
}

impl CameraTypeWire {
    fn into_type(self) -> CameraDetailType {
        let tab_type = self
            .tab_type
            .and_then(|id| u8::try_from(id).ok())
            .map_or(CameraTabType::Unknown, CameraTabType::from_id);
        let name = clean_text(self.name);
        CameraDetailType {
            tab_type,
            name: if name.is_empty() {
                tab_type.label_zh().to_owned()
            } else {
                name
            },
            remark: clean_text(self.remark),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuleInfoWire {
    #[serde(default)]
    plate: Option<PlateWire>,
    #[serde(default)]
    day_type_name: Option<Vec<String>>,
    #[serde(default)]
    tail: Option<Vec<i32>>,
    #[serde(default)]
    time: Option<String>,
}

impl RuleInfoWire {
    fn into_rule(self) -> CameraRule {
        CameraRule {
            plate_name: clean_text(self.plate.and_then(|plate| plate.name)),
            day_type_names: self
                .day_type_name
                .unwrap_or_default()
                .into_iter()
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty())
                .collect(),
            tail: self.tail.unwrap_or_default(),
            time: clean_text(self.time),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PlateWire {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TicketListWire {
    #[serde(default)]
    tickets: Option<Vec<TicketWire>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TicketWire {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    tittle: Option<String>,
    #[serde(default)]
    remark: Option<String>,
    #[serde(default)]
    photo: Option<String>,
    #[serde(default)]
    create_date: Option<i64>,
    #[serde(default)]
    contact_person: Option<String>,
    #[serde(default)]
    user_photo: Option<String>,
}

impl TicketWire {
    fn into_ticket(self) -> CameraTicket {
        CameraTicket {
            id: self.id.unwrap_or(0),
            title: clean_text(self.tittle),
            remark: clean_text(self.remark),
            photo: clean_text(self.photo),
            create_date: self.create_date,
            contact_person: clean_text(self.contact_person),
            user_photo: clean_text(self.user_photo),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StringOrNumber {
    String(String),
    Number(i64),
}

impl StringOrNumber {
    fn to_i64(&self) -> i64 {
        match self {
            Self::String(value) => value.trim().parse().unwrap_or(0),
            Self::Number(value) => *value,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CameraPoiWire {
    #[serde(default)]
    coordinate: Option<WireCoordinate>,
    #[serde(default)]
    distance: Option<f64>,
    #[serde(default)]
    member: Option<String>,
    #[serde(default)]
    member_by_string: Option<String>,
    #[serde(default)]
    new_flag: Option<BoolOrInt>,
    #[serde(default)]
    tab_type: Option<Vec<i32>>,
    #[serde(default)]
    white: Option<BoolOrInt>,
}

#[derive(Debug, Deserialize)]
struct WireCoordinate {
    #[serde(default)]
    latitude: Option<f64>,
    #[serde(default)]
    longitude: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BoolOrInt {
    Bool(bool),
    Int(i32),
}

impl BoolOrInt {
    const fn is_true(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            Self::Int(value) => *value != 0,
        }
    }
}

impl CameraPoiWire {
    fn into_camera(self) -> Option<TrafficCamera> {
        let coordinate = self.coordinate?;
        let position = LatLng::new(coordinate.latitude?, coordinate.longitude?);
        if !position.is_valid() {
            return None;
        }
        let tab_types: Vec<CameraTabType> = self
            .tab_type
            .unwrap_or_default()
            .into_iter()
            .filter_map(|id| u8::try_from(id).ok())
            .map(CameraTabType::from_id)
            .collect();
        let primary = tab_types.first().copied().unwrap_or(CameraTabType::Unknown);
        Some(TrafficCamera {
            id: format!(
                "{:.6},{:.6}:{}",
                position.latitude,
                position.longitude,
                primary.id()
            ),
            position,
            tab_types,
            member: self.member.filter(|value| !value.trim().is_empty()),
            member_by_string: self
                .member_by_string
                .filter(|value| !value.trim().is_empty()),
            new_flag: self.new_flag.is_some_and(|value| value.is_true()),
            white: self.white.is_some_and(|value| value.is_true()),
            distance_m: self.distance,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_uses_default_tab_types_and_trailing_comma() {
        let body = camera_search_body(&OfficialCameraQuery {
            position: LatLng::new(31.2304, 121.4737),
            city_code: "021".to_owned(),
            to_city_code: "021".to_owned(),
            ..OfficialCameraQuery::default()
        });
        assert_eq!(body["lat"], 31.2304);
        assert_eq!(body["lon"], 121.4737);
        assert_eq!(body["cityCode"], "021");
        assert_eq!(body["type"], "2");
        assert_eq!(body["tabTypes"], "1,2,3,4,5,6,7,8,9,10,11,13,14,15,16,");
        assert_eq!(body["color"], "YELLOW");
        assert_eq!(body["source"], 1);
        assert!(body.get("model").is_none());
    }

    #[test]
    fn empty_city_falls_back_to_beijing_and_cruise_sets_model() {
        let body = camera_search_body(&OfficialCameraQuery {
            position: LatLng::new(39.9, 116.4),
            city_code: String::new(),
            to_city_code: String::new(),
            cruise: true,
            ..OfficialCameraQuery::default()
        });
        assert_eq!(body["cityCode"], "010");
        assert_eq!(body["toCityCode"], "010");
        assert_eq!(body["model"], "CRUISE");
    }

    #[test]
    fn parses_camera_list_and_skips_invalid_coordinates() {
        let plaintext = br#"{"code":0,"message":"ok","data":[
            {"coordinate":{"latitude":31.23,"longitude":121.47},"distance":12.5,
             "member":"m","memberByString":"camera-7","newFlag":1,
             "tabType":[5,7],"white":0},
            {"coordinate":null,"tabType":[1]},
            {"coordinate":{"latitude":91.0,"longitude":0.0},"tabType":[1]}
        ]}"#;
        let cameras = parse_camera_response(plaintext).expect("valid response");
        assert_eq!(cameras.len(), 1);
        let camera = &cameras[0];
        assert_eq!(camera.position, LatLng::new(31.23, 121.47));
        assert_eq!(
            camera.tab_types,
            vec![CameraTabType::Speeding, CameraTabType::LaneViolation]
        );
        assert_eq!(camera.member.as_deref(), Some("m"));
        assert_eq!(camera.member_by_string.as_deref(), Some("camera-7"));
        assert!(camera.new_flag);
        assert!(!camera.white);
        assert_eq!(camera.distance_m, Some(12.5));
    }

    #[test]
    fn surfaces_token_business_error() {
        let plaintext =
            r#"{"message":"token校验异常：access_token非法 Token无效","code":20105,"data":null}"#
                .as_bytes();
        let error = parse_camera_response(plaintext).expect_err("must fail");
        assert!(error.to_string().contains("token校验异常"));
    }

    #[test]
    fn parses_camera_detail_and_flattens_rules() {
        let plaintext = r#"{"code":0,"message":"ok","data":{
            "id":123,"tittle":"电子摄像头","detail":"由东向西抓拍",
            "address":"上海市某路口","pos":"121.4737,31.2304",
            "eyePoiTabs":[
                {"tabType":1,"name":"闯限行","remark":"外地车牌"},
                {"tabType":5,"name":"超速","remark":""}
            ],
            "ruleInfoList":[
                [{"plate":{"name":"外地车牌"},"dayTypeName":["工作日"],"tail":[1,2],"time":"07:00-20:00"}],
                [{"plate":null,"dayTypeName":["周末"],"time":"09:00-18:00"}]
            ],
            "ticketsCount":"3","praiseCount":5,"commentCount":2,
            "white":0,"avoid":1,"avoidType":2
        }}"#
        .as_bytes();
        let detail = parse_camera_detail(plaintext, "fallback-id").expect("valid detail");
        assert_eq!(detail.camera_id, "123");
        assert_eq!(detail.title, "电子摄像头");
        assert_eq!(detail.detail, "由东向西抓拍");
        assert_eq!(detail.address, "上海市某路口");
        assert_eq!(detail.position, Some(LatLng::new(31.2304, 121.4737)));
        assert_eq!(detail.types.len(), 2);
        assert_eq!(detail.types[0].tab_type, CameraTabType::RestrictedEntry);
        assert_eq!(detail.types[0].name, "闯限行");
        assert_eq!(detail.types[0].remark, "外地车牌");
        assert_eq!(detail.rules.len(), 2);
        assert_eq!(detail.rules[0].plate_name, "外地车牌");
        assert_eq!(detail.rules[0].day_type_names, vec!["工作日"]);
        assert_eq!(detail.rules[0].tail, vec![1, 2]);
        assert_eq!(detail.rules[0].time, "07:00-20:00");
        assert_eq!(detail.rules[1].plate_name, "");
        assert_eq!(detail.tickets_count, 3);
        assert_eq!(detail.praise_count, 5);
        assert_eq!(detail.comment_count, 2);
        assert!(!detail.white);
        assert!(detail.avoid);
        assert_eq!(detail.avoid_type, 2);
        assert_eq!(detail.tickets.len(), 0);
    }

    #[test]
    fn parses_camera_ticket_list() {
        let plaintext = r#"{"code":0,"message":"ok","data":{"cursor":0,"tickets":[
            {"id":7,"tittle":"违章罚单","remark":"未按车道行驶","photo":"https://example/photo.jpg",
             "createDate":1720000000000,"contactPerson":"车友","userPhoto":"https://example/avatar.jpg"},
            {"id":8,"tittle":"","remark":"","photo":"","createDate":null}
        ]}}"#
        .as_bytes();
        let tickets = parse_ticket_list(plaintext).expect("valid tickets");
        assert_eq!(tickets.len(), 2);
        assert_eq!(tickets[0].id, 7);
        assert_eq!(tickets[0].title, "违章罚单");
        assert_eq!(tickets[0].remark, "未按车道行驶");
        assert_eq!(tickets[0].create_date, Some(1_720_000_000_000));
        assert_eq!(tickets[0].contact_person, "车友");
        assert_eq!(tickets[1].id, 8);
        assert_eq!(tickets[1].create_date, None);
    }

    #[test]
    fn camera_detail_without_data_is_an_error() {
        let error =
            parse_camera_detail(br#"{"code":0,"data":null}"#, "camera-1").expect_err("must fail");
        assert!(error.to_string().contains("no data"));
    }
}
