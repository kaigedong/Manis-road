//! Build a [`RoadGraph`] from an Overpass JSON extract.
//!
//! Query shape used during preparation:
//!
//! ```text
//! [out:json][timeout:900];
//! way["highway"](south,west,north,east);
//! out body; >; out skel qt;
//! ```
//!
//! The profile follows Shanghai motorcycle rules at a coarse level: no
//! expressways (`motorway`), respect `oneway`, skip `access`/`motorcycle` denials
//! and pedestrian-only ways. Turn restrictions are not modelled yet.

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{Result, RoutingError};
use crate::graph::{Edge, GraphNode, RoadGraph, WayInfo, haversine_m};

/// Ways we are willing to route motorcycles on.
const ALLOWED_HIGHWAYS: &[&str] = &[
    "trunk",
    "trunk_link",
    "primary",
    "primary_link",
    "secondary",
    "secondary_link",
    "tertiary",
    "tertiary_link",
    "unclassified",
    "residential",
    "living_street",
    "service",
    "road",
];

/// Counters useful when preparing a graph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OsmBuildReport {
    pub nodes_total: usize,
    pub nodes_used: usize,
    pub ways_total: usize,
    pub ways_used: usize,
    pub edges: usize,
}

/// Parse Overpass JSON and build the directed motorcycle graph.
#[allow(
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    reason = "graph distances and speeds are bounded by road geometry"
)]
pub fn build_from_osm_json(bytes: &[u8]) -> Result<(RoadGraph, OsmBuildReport)> {
    let root: OsmRoot =
        serde_json::from_slice(bytes).map_err(|error| RoutingError::Osm(error.to_string()))?;

    let mut coordinates: HashMap<i64, (f64, f64)> = HashMap::new();
    let mut signal_nodes: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut raw_ways: Vec<(Vec<i64>, HashMap<String, String>)> = Vec::new();
    for element in root.elements {
        match element {
            Element::Node {
                id,
                lat,
                lon,
                tags,
            } => {
                coordinates.insert(id, (lat, lon));
                if is_signal(tags.get("highway").map_or("", String::as_str)) {
                    signal_nodes.insert(id);
                }
            }
            Element::Way { nodes, tags, .. } => raw_ways.push((nodes, tags)),
            Element::Relation {} => {}
        }
    }

    // Keep only ways with a routable profile and at least two known nodes.
    let mut candidates = Vec::new();
    for (nodes, tags) in &raw_ways {
        let Some(profile) = way_profile(tags) else {
            continue;
        };
        let usable: Vec<i64> = nodes
            .iter()
            .copied()
            .filter(|id| coordinates.contains_key(id))
            .collect();
        if usable.len() < 2 {
            continue;
        }
        candidates.push((usable, profile, way_info(tags)));
    }

    // Assign dense indexes to the nodes the candidates use.
    let mut node_index: HashMap<i64, u32> = HashMap::new();
    let mut nodes: Vec<GraphNode> = Vec::new();
    for (usable, _, _) in &candidates {
        for id in usable {
            if node_index.contains_key(id) {
                continue;
            }
            let (lat, lon) = coordinates[id];
            let index = u32::try_from(nodes.len())
                .map_err(|_| RoutingError::Osm("too many nodes".to_owned()))?;
            node_index.insert(*id, index);
            nodes.push(GraphNode { id: *id, lat, lon });
        }
    }

    let mut signals: Vec<bool> = vec![false; nodes.len()];
    for (id, index) in &node_index {
        if signal_nodes.contains(id) {
            signals[*index as usize] = true;
        }
    }

    let mut ways: Vec<WayInfo> = Vec::with_capacity(candidates.len());
    let mut adjacency: Vec<Vec<Edge>> = vec![Vec::new(); nodes.len()];
    for (usable, profile, info) in candidates {
        let way_index = u32::try_from(ways.len())
            .map_err(|_| RoutingError::Osm("too many ways".to_owned()))?;
        ways.push(info);
        for pair in usable.windows(2) {
            let (Some(&from_id), Some(&to_id)) = (node_index.get(&pair[0]), node_index.get(&pair[1]))
            else {
                continue;
            };
            if from_id == to_id {
                continue;
            }
            let from = nodes[from_id as usize];
            let to = nodes[to_id as usize];
            let distance = haversine_m(from.lat, from.lon, to.lat, to.lon);
            if distance < 0.5 {
                continue;
            }
            let distance = distance as f32;
            if profile.oneway >= 0 {
                adjacency[from_id as usize].push(Edge {
                    to: to_id,
                    distance_m: distance,
                    speed_kmh: profile.speed_kmh,
                    way: way_index,
                });
            }
            if profile.oneway <= 0 {
                adjacency[to_id as usize].push(Edge {
                    to: from_id,
                    distance_m: distance,
                    speed_kmh: profile.speed_kmh,
                    way: way_index,
                });
            }
        }
    }

    let mut offsets = vec![0_u64; nodes.len() + 1];
    let mut edges = Vec::new();
    for (index, list) in adjacency.iter().enumerate() {
        offsets[index + 1] = offsets[index] + list.len() as u64;
        edges.extend_from_slice(list);
    }

    let report = OsmBuildReport {
        nodes_total: coordinates.len(),
        nodes_used: nodes.len(),
        ways_total: raw_ways.len(),
        ways_used: ways.len(),
        edges: edges.len(),
    };
    Ok((
        RoadGraph {
            nodes,
            signals,
            ways,
            offsets,
            edges,
        },
        report,
    ))
}

#[derive(Debug, Clone, Copy)]
struct WayProfile {
    speed_kmh: u16,
    /// `1` forward only, `-1` backward only, `0` both ways.
    oneway: i8,
}

/// `highway=traffic_signals` is the only node tag we route on. `crossing` is a
/// zebra or a push-button crossing, not a light, and would over-count.
fn is_signal(highway: &str) -> bool {
    highway == "traffic_signals"
}

fn way_info(tags: &HashMap<String, String>) -> WayInfo {
    WayInfo {
        name: tags.get("name").cloned().unwrap_or_default(),
        highway: tags.get("highway").cloned().unwrap_or_default(),
    }
}

fn way_profile(tags: &HashMap<String, String>) -> Option<WayProfile> {
    let highway = tags.get("highway")?.trim();
    if !ALLOWED_HIGHWAYS.contains(&highway) {
        return None;
    }
    if tags.get("area").is_some_and(|value| value == "yes") {
        return None;
    }

    // Shanghai elevated roads are usually `trunk` + `bridge` without a
    // `motorcycle` tag (OSM data gap), and motorcycles are banned on them.
    // The same applies to most Huangpu river bridges. Name keywords catch the
    // rest; tunnels that explicitly allow motorcycles keep their tag.
    let name = tags.get("name").map_or("", String::as_str);
    let elevated_name = ["高架", "快速路", "高速"]
        .iter()
        .any(|keyword| name.contains(keyword));
    let bridge = tags
        .get("bridge")
        .is_some_and(|value| value != "no");
    let tunnel = tags
        .get("tunnel")
        .is_some_and(|value| value != "no");
    let layer = tags
        .get("layer")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0);
    let elevated_structure = bridge
        && layer >= 2.0
        && matches!(
            highway,
            "trunk" | "trunk_link" | "motorway" | "motorway_link"
        );
    if elevated_name || elevated_structure {
        return None;
    }

    let motorcycle_yes = tags
        .get("motorcycle")
        .is_some_and(|value| matches!(value.as_str(), "yes" | "designated"));
    if !motorcycle_yes {
        let denied = ["motorcycle", "motor_vehicle", "vehicle", "access"]
            .iter()
            .any(|key| {
                tags.get(*key)
                    .is_some_and(|value| matches!(value.as_str(), "no" | "private"))
            });
        if denied {
            return None;
        }
        // Shanghai bans motorcycles from almost every Huangpu crossing; OSM
        // only tags the exception (复兴东路隧道下层 has `motorcycle=designated`).
        // River crossings are named 隧道/大桥, so require an explicit permit.
        let named_crossing = (name.contains("隧道") || name.contains("大桥"))
            && (bridge || tunnel);
        if named_crossing {
            return None;
        }
    }

    let oneway = match tags.get("oneway").map(String::as_str) {
        Some("yes" | "true" | "1") => 1,
        Some("-1" | "reverse") => -1,
        _ if tags.get("junction").is_some_and(|value| value == "roundabout") => 1,
        _ => 0,
    };

    Some(WayProfile {
        speed_kmh: speed_kmh(highway, tags),
        oneway,
    })
}

fn speed_kmh(highway: &str, tags: &HashMap<String, String>) -> u16 {
    tags.get("maxspeed")
        .and_then(|value| parse_maxspeed(value))
        .unwrap_or_else(|| default_speed(highway))
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the accepted range is 5..=140 km/h"
)]
fn parse_maxspeed(value: &str) -> Option<u16> {
    let first = value.split(';').next()?.trim();
    let (number, is_mph) = first
        .strip_suffix("mph")
        .map_or((first.trim_end_matches("km/h").trim(), false), |value| {
            (value.trim(), true)
        });
    let speed = number.parse::<f64>().ok()?;
    let kmh = if is_mph { speed * 1.609_344 } else { speed };
    (5.0..=140.0)
        .contains(&kmh)
        .then(|| kmh.round() as u16)
}

const fn default_speed(highway: &str) -> u16 {
    match highway.as_bytes() {
        b"trunk" => 50,
        b"primary" => 45,
        b"trunk_link" | b"secondary" => 40,
        b"primary_link" | b"tertiary" => 35,
        b"secondary_link" | b"unclassified" | b"road" => 30,
        b"living_street" | b"service" => 15,
        // tertiary_link, residential and unknown classes.
        _ => 25,
    }
}

#[derive(Debug, Deserialize)]
struct OsmRoot {
    #[serde(default)]
    elements: Vec<Element>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Element {
    #[serde(rename = "node")]
    Node {
        id: i64,
        #[serde(default)]
        lat: f64,
        #[serde(default)]
        lon: f64,
        #[serde(default)]
        tags: HashMap<String, String>,
    },
    #[serde(rename = "way")]
    Way {
        #[serde(default)]
        nodes: Vec<i64>,
        #[serde(default)]
        tags: HashMap<String, String>,
    },
    #[serde(rename = "relation")]
    Relation {},
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::needless_pass_by_value, reason = "tests build a value inline")]
    fn json(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).expect("json")
    }

    #[test]
    fn builds_a_bidirectional_residential_way() {
        let data = json(serde_json::json!({
            "elements": [
                {"type": "node", "id": 1, "lat": 31.2300, "lon": 121.4700},
                {"type": "node", "id": 2, "lat": 31.2310, "lon": 121.4710},
                {"type": "way", "id": 10, "nodes": [1, 2],
                 "tags": {"highway": "residential", "name": "测试路"}}
            ]
        }));
        let (graph, report) = build_from_osm_json(&data).expect("build");
        assert_eq!(report.ways_used, 1);
        assert_eq!(report.nodes_used, 2);
        assert_eq!(graph.edge_count(), 2, "residential is bidirectional");
        assert_eq!(graph.ways[0].name, "测试路");
        assert!((graph.edges[0].distance_m - 130.0).abs() < 30.0);
    }

    #[test]
    fn skips_expressways_and_denied_ways() {
        let data = json(serde_json::json!({
            "elements": [
                {"type": "node", "id": 1, "lat": 31.23, "lon": 121.47},
                {"type": "node", "id": 2, "lat": 31.24, "lon": 121.48},
                {"type": "way", "id": 1, "nodes": [1, 2], "tags": {"highway": "motorway"}},
                {"type": "way", "id": 2, "nodes": [1, 2], "tags": {"highway": "primary", "motorcycle": "no"}},
                {"type": "way", "id": 3, "nodes": [1, 2], "tags": {"highway": "footway"}}
            ]
        }));
        let (graph, report) = build_from_osm_json(&data).expect("build");
        assert_eq!(report.ways_used, 0);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn marks_signalised_nodes_only() {
        let data = json(serde_json::json!({
            "elements": [
                {"type": "node", "id": 1, "lat": 31.2300, "lon": 121.4700},
                {"type": "node", "id": 2, "lat": 31.2310, "lon": 121.4710,
                 "tags": {"highway": "traffic_signals"}},
                {"type": "node", "id": 3, "lat": 31.2320, "lon": 121.4720,
                 "tags": {"highway": "crossing"}},
                {"type": "way", "id": 10, "nodes": [1, 2, 3],
                 "tags": {"highway": "residential"}}
            ]
        }));
        let (graph, _) = build_from_osm_json(&data).expect("build");
        assert_eq!(
            graph.signals.len(),
            graph.nodes.len(),
            "every node needs a flag to make the data complete"
        );
        let flagged: Vec<i64> = graph
            .nodes
            .iter()
            .zip(&graph.signals)
            .filter(|(_, flag)| **flag)
            .map(|(node, _)| node.id)
            .collect();
        assert_eq!(
            flagged,
            vec![2],
            "traffic_signals counts; a zebra crossing does not"
        );
    }

    #[test]
    fn respects_oneway_and_maxspeed() {
        let data = json(serde_json::json!({
            "elements": [
                {"type": "node", "id": 1, "lat": 31.23, "lon": 121.47},
                {"type": "node", "id": 2, "lat": 31.24, "lon": 121.48},
                {"type": "way", "id": 1, "nodes": [1, 2],
                 "tags": {"highway": "primary", "oneway": "-1", "maxspeed": "60"}}
            ]
        }));
        let (graph, _) = build_from_osm_json(&data).expect("build");
        assert_eq!(graph.edge_count(), 1);
        assert_eq!(graph.edges[0].speed_kmh, 60);
        assert_eq!(graph.edges[0].to, 0, "reverse-only way points back");
    }

    #[test]
    fn skips_elevated_roads_and_trunk_bridges() {
        let data = json(serde_json::json!({
            "elements": [
                {"type": "node", "id": 1, "lat": 31.23, "lon": 121.47},
                {"type": "node", "id": 2, "lat": 31.24, "lon": 121.48},
                {"type": "way", "id": 1, "nodes": [1, 2],
                 "tags": {"highway": "trunk", "name": "延安高架路"}},
                {"type": "way", "id": 2, "nodes": [1, 2],
                 "tags": {"highway": "trunk", "bridge": "yes", "name": "南浦大桥"}},
                {"type": "way", "id": 3, "nodes": [1, 2],
                 "tags": {"highway": "secondary", "tunnel": "yes", "name": "复兴东路隧道 (下层)",
                          "motorcycle": "designated"}},
                {"type": "way", "id": 4, "nodes": [1, 2],
                 "tags": {"highway": "secondary", "tunnel": "yes", "name": "复兴东路隧道 (上层)",
                          "motorcycle": "no"}},
                {"type": "way", "id": 5, "nodes": [1, 2],
                 "tags": {"highway": "secondary", "tunnel": "yes", "name": "延安东路隧道"}}
            ]
        }));
        let (graph, report) = build_from_osm_json(&data).expect("build");
        assert_eq!(report.ways_used, 1, "only the motorcycle tunnel survives");
        assert_eq!(graph.ways[0].name, "复兴东路隧道 (下层)");
    }

    #[test]
    fn parses_maxspeed_variants() {
        assert_eq!(parse_maxspeed("30"), Some(30));
        assert_eq!(parse_maxspeed("50 km/h"), Some(50));
        assert_eq!(parse_maxspeed("30 mph"), Some(48));
        assert_eq!(parse_maxspeed("signals"), None);
        assert_eq!(parse_maxspeed("0"), None);
    }
}
