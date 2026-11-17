//! Restricted / restricted-entry map areas from `v1/poi/mapShowAreaSec`.
//!
//! The recovered client calls `IPoiAPIService.mapArea()` with no body and draws
//! the decoded polylines as `AMap` polygons/lines
//! (`MotuMapAreaSnapshot.fromResponse`). `posEncode` is the standard `AMap`
//! polyline encoding, in `GCJ-02`; callers convert to WGS-84 for Leaflet.
//!
//! The endpoint is encrypted but does not require a login (the live probe
//! accepted a minimal header set). Tokens are still forwarded when present so
//! the request looks the same as the app's.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api::{SecurityClient, Transport, endpoints};
use crate::error::{MotuError, Result};
use crate::geo::LatLng;
use crate::model::{ApiEnvelope, MapAreaResult};
use crate::route::decode_polyline_scaled;

/// `AMap`'s `PolylineUtil` encodes `posEncode` with six decimal places.
const AMAP_POLYLINE_SCALE: f64 = 1e6;

/// `SHAPE_TYPE_POLYGON` / `SHAPE_TYPE_LINESTRING`, lower-cased.
pub const SHAPE_POLYGON: &str = "polygon";
pub const SHAPE_LINESTRING: &str = "linestring";

/// One decoded map area, still in the server's `GCJ-02` frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapAreaShape {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    /// [`SHAPE_POLYGON`] or [`SHAPE_LINESTRING`].
    pub shape_type: String,
    pub points: Vec<LatLng>,
}

/// Whole area layer plus when it was fetched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapAreaSnapshot {
    pub areas: Vec<MapAreaShape>,
    #[serde(default)]
    pub version: i64,
    pub fetched_at_ms: i64,
}

impl MapAreaSnapshot {
    #[must_use]
    pub fn polygon_count(&self) -> usize {
        self.areas
            .iter()
            .filter(|area| area.shape_type == SHAPE_POLYGON)
            .count()
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.areas
            .iter()
            .filter(|area| area.shape_type == SHAPE_LINESTRING)
            .count()
    }
}

/// Fetch and decode the restricted-area layer.
///
/// `headers` normally come from [`crate::base_headers`]; a `token` is optional.
pub fn load_map_areas<T: Transport>(
    transport: &T,
    base_url: &str,
    headers: &[(String, String)],
    now_ms: i64,
) -> Result<MapAreaSnapshot> {
    let client = SecurityClient::fetch_config(transport, base_url, headers, now_ms)?;
    let plaintext = client.post_encrypted(
        transport,
        &endpoints::MAP_SHOW_AREA_SEC,
        base_url,
        headers,
        &json!({}),
        (now_ms, now_ms / 1000),
    )?;
    parse_map_areas(&plaintext, now_ms)
}

fn parse_map_areas(plaintext: &[u8], now_ms: i64) -> Result<MapAreaSnapshot> {
    let envelope: ApiEnvelope<MapAreaResult> = serde_json::from_slice(plaintext)?;
    if !envelope.is_success() {
        let code = envelope.code;
        let message = envelope
            .message
            .unwrap_or_else(|| format!("map area request failed with code {code}"));
        return Err(MotuError::Server(message));
    }
    let data = envelope.data.unwrap_or_default();
    let mut areas = Vec::new();
    for area in data.list {
        let Some(encoded) = area
            .pos_encode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        // A malformed shape must not discard the rest of the layer.
        let Ok(points) = decode_polyline_scaled(encoded, AMAP_POLYLINE_SCALE) else {
            continue;
        };
        if points.len() < 2 {
            continue;
        }
        let shape_type = area
            .shape_type
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if shape_type != SHAPE_POLYGON && shape_type != SHAPE_LINESTRING {
            continue;
        }
        areas.push(MapAreaShape {
            id: area.id,
            name: area.name.unwrap_or_default().trim().to_owned(),
            shape_type,
            points,
        });
    }
    Ok(MapAreaSnapshot {
        areas,
        version: data.version,
        fetched_at_ms: now_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::encode_polyline_scaled;

    fn polygon_points() -> Vec<LatLng> {
        vec![
            LatLng::new(31.20, 121.40),
            LatLng::new(31.20, 121.50),
            LatLng::new(31.30, 121.50),
            LatLng::new(31.30, 121.40),
        ]
    }

    fn line_points() -> Vec<LatLng> {
        vec![LatLng::new(31.22, 121.45), LatLng::new(31.24, 121.47)]
    }

    #[test]
    fn decodes_polygons_and_lines_and_skips_bad_shapes() {
        let response = json!({
            "code": 0,
            "message": "ok",
            "data": {
                "version": 7,
                "list": [
                    {"id": 1, "name": "外牌限行", "posEncode": encode_polyline_scaled(&polygon_points(), 1e6), "shapeType": "polygon"},
                    {"id": 2, "name": "禁摩路段", "posEncode": encode_polyline_scaled(&line_points(), 1e6), "shapeType": "linestring"},
                    {"id": 3, "name": "无坐标", "posEncode": "", "shapeType": "polygon"},
                    {"id": 4, "name": "坏数据", "posEncode": "!!", "shapeType": "polygon"},
                    {"id": 5, "name": "未知形状", "posEncode": encode_polyline_scaled(&line_points(), 1e6), "shapeType": "circle"}
                ]
            }
        });
        let snapshot =
            parse_map_areas(response.to_string().as_bytes(), 1_000).expect("parse");
        assert_eq!(snapshot.version, 7);
        assert_eq!(snapshot.fetched_at_ms, 1_000);
        assert_eq!(snapshot.areas.len(), 2);
        assert_eq!(snapshot.polygon_count(), 1);
        assert_eq!(snapshot.line_count(), 1);
        assert_eq!(snapshot.areas[0].name, "外牌限行");
        assert_eq!(snapshot.areas[0].points.len(), 4);
        assert!((snapshot.areas[0].points[0].latitude - 31.20).abs() < 1e-6);
    }

    #[test]
    fn surfaces_business_errors() {
        let response = r#"{"code":20105,"message":"token校验异常","data":null}"#.as_bytes();
        let error = parse_map_areas(response, 0).expect_err("must fail");
        assert!(error.to_string().contains("token校验异常"));
    }

    #[test]
    fn empty_layer_is_valid() {
        let snapshot =
            parse_map_areas(br#"{"code":0,"message":"ok","data":{"list":[],"version":1}}"#, 5)
                .expect("parse");
        assert_eq!(snapshot.areas.len(), 0);
    }
}
