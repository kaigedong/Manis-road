//! Route quality metrics.
//!
//! The app's acceptance criteria are neighbourhood statistics of a route, not a
//! feeling: how much of it runs on arterials, how much on quiet streets, how
//! many turns it takes. Measuring them here keeps the numbers identical between
//! the CLI, the tests and the Android UI.

use serde::{Deserialize, Serialize};

use crate::graph::RoadGraph;
use crate::route::{bearing, normalize_degrees};

/// One road class and how far the route used it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassDistance {
    pub highway: String,
    pub distance_m: f64,
}

/// Measured properties of one route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteMetrics {
    pub distance_m: f64,
    pub duration_s: f64,
    /// `trunk` + `primary` distance, and its share of the whole route.
    pub arterial_distance_m: f64,
    pub arterial_share: f64,
    /// `tertiary` and smaller: what a motorcycle rider actually wants.
    pub side_street_distance_m: f64,
    pub side_street_share: f64,
    /// Signalised junctions per kilometre.
    ///
    /// `None` means the graph carries no signal data yet — reporting `0.0`
    /// would silently satisfy the acceptance threshold, which is exactly the
    /// kind of fake pass the goal forbids.
    pub signals_per_km: Option<f64>,
    pub turn_count: usize,
    pub u_turn_count: usize,
    pub classes: Vec<ClassDistance>,
}

impl RouteMetrics {
    /// Merge the per-class distances into a stable, descending order.
    fn from_parts(
        distance_m: f64,
        duration_s: f64,
        classes: Vec<ClassDistance>,
        turn_count: usize,
        u_turn_count: usize,
    ) -> Self {
        let arterial_distance_m: f64 = classes
            .iter()
            .filter(|entry| {
                matches!(
                    entry.highway.as_str(),
                    "trunk" | "trunk_link" | "primary" | "primary_link"
                )
            })
            .map(|entry| entry.distance_m)
            .sum();
        let side_street_distance_m: f64 = classes
            .iter()
            .filter(|entry| {
                matches!(
                    entry.highway.as_str(),
                    "tertiary"
                        | "tertiary_link"
                        | "residential"
                        | "unclassified"
                        | "living_street"
                        | "service"
                )
            })
            .map(|entry| entry.distance_m)
            .sum();
        let total = distance_m.max(1.0);
        let mut classes = classes;
        classes.sort_by(|a, b| {
            b.distance_m
                .partial_cmp(&a.distance_m)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Self {
            distance_m,
            duration_s,
            arterial_share: arterial_distance_m / total,
            arterial_distance_m,
            side_street_share: side_street_distance_m / total,
            side_street_distance_m,
            signals_per_km: None,
            turn_count,
            u_turn_count,
            classes,
        }
    }
}

/// Measure a route given the graph and the ordered edge indexes it traverses.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "edge indexes and signal counts fit the address space"
)]
pub fn measure_edges(graph: &RoadGraph, edges: &[u64]) -> RouteMetrics {
    use std::collections::BTreeMap;

    let mut by_class: BTreeMap<&str, f64> = BTreeMap::new();
    let mut distance_m = 0.0;
    let mut duration_s = 0.0;
    let mut turn_count = 0;
    let mut u_turn_count = 0;
    let mut previous_bearing: Option<f64> = None;

    for &index in edges {
        let Some(edge) = graph.edges.get(index as usize) else {
            continue;
        };
        let highway = graph
            .ways
            .get(edge.way as usize)
            .map_or("unknown", |way| way.highway.as_str());
        *by_class.entry(highway).or_insert(0.0) += f64::from(edge.distance_m);
        distance_m += f64::from(edge.distance_m);
        duration_s += edge.duration_s();

        let from = graph.nodes.get(edge_from(graph, index) as usize);
        let to = graph.nodes.get(edge.to as usize);
        if let (Some(from), Some(to)) = (from, to) {
            let edge_bearing = bearing(from.position(), to.position());
            if let Some(previous) = previous_bearing {
                let angle = normalize_degrees(edge_bearing - previous).abs();
                if angle >= 25.0 {
                    turn_count += 1;
                }
                if angle >= 170.0 {
                    u_turn_count += 1;
                }
            }
            previous_bearing = Some(edge_bearing);
        }
    }

    let classes = by_class
        .into_iter()
        .map(|(highway, distance_m)| ClassDistance {
            highway: highway.to_owned(),
            distance_m,
        })
        .collect();

    let mut metrics =
        RouteMetrics::from_parts(distance_m, duration_s, classes, turn_count, u_turn_count);

    // Signal data exists only when the graph carries one flag per node.
    if graph.signals.len() == graph.nodes.len() && !graph.nodes.is_empty() {
        let signals = edges
            .iter()
            .filter(|index| {
                graph
                    .edges
                    .get(**index as usize)
                    .and_then(|edge| graph.signals.get(edge.to as usize))
                    .copied()
                    == Some(true)
            })
            .count();
        let km = (distance_m / 1000.0).max(0.001);
        metrics.signals_per_km = Some(signals as f64 / km);
    }
    metrics
}

/// The node an edge starts from.
///
/// Edges are stored in CSR order, so the owning node is the last one whose
/// offset range still covers `index`.
fn edge_from(graph: &RoadGraph, index: u64) -> u32 {
    let mut low = 0_usize;
    let mut high = graph.nodes.len();
    while low + 1 < high {
        let mid = low + (high - low) / 2;
        if graph.offsets[mid] <= index {
            low = mid;
        } else {
            high = mid;
        }
    }
    u32::try_from(low).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, GraphNode, WayInfo};

    fn graph() -> RoadGraph {
        // One straight road east: three nodes, two edges, both `primary`.
        let nodes = vec![
            GraphNode {
                id: 1,
                lat: 31.230,
                lon: 121.470,
            },
            GraphNode {
                id: 2,
                lat: 31.230,
                lon: 121.472,
            },
            GraphNode {
                id: 3,
                lat: 31.230,
                lon: 121.474,
            },
        ];
        let ways = vec![WayInfo {
            name: "大路".to_owned(),
            highway: "primary".to_owned(),
        }];
        let edges = vec![
            Edge {
                to: 1,
                distance_m: 200.0,
                speed_kmh: 40,
                way: 0,
            },
            Edge {
                to: 2,
                distance_m: 200.0,
                speed_kmh: 40,
                way: 0,
            },
        ];
        RoadGraph {
            signals: Vec::new(),
            nodes,
            ways,
            offsets: vec![0, 1, 2, 2],
            edges,
        }
    }

    #[test]
    fn measures_arterial_share() {
        let metrics = measure_edges(&graph(), &[0, 1]);
        assert!((metrics.distance_m - 400.0).abs() < 0.01);
        assert!((metrics.arterial_share - 1.0).abs() < 0.01);
        assert!(metrics.side_street_share < 0.01);
        assert_eq!(metrics.turn_count, 0, "straight run has no turns");
    }

    #[test]
    fn signals_are_unknown_not_zero() {
        let metrics = measure_edges(&graph(), &[0]);
        assert_eq!(
            metrics.signals_per_km, None,
            "a graph without signal data must not report 0 signals/km"
        );
    }

    #[test]
    fn edges_belong_to_the_right_node() {
        let graph = graph();
        assert_eq!(edge_from(&graph, 0), 0);
        assert_eq!(edge_from(&graph, 1), 1);
    }
}
