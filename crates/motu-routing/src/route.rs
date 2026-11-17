//! A* routing over the offline graph, with simple navigation steps.

use std::{cmp::Ordering, collections::BinaryHeap};

use motu_core::LatLng;
use serde::{Deserialize, Serialize};

use crate::avoid::CameraDeterrent;
use crate::error::{Result, RoutingError};
use crate::graph::{RoadGraph, haversine_m};

/// How far a coordinate may be from the graph and still snap to it.
const MAX_SNAP_M: f64 = 600.0;
/// Upper bound used by the A* heuristic; the fastest road is below this.
const HEURISTIC_SPEED_KMH: f64 = 80.0;

/// Result of snapping a coordinate to the nearest graph node.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snap {
    pub node: u32,
    pub distance_m: f64,
}

/// One user-facing navigation instruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteStep {
    /// `start`, `straight`, `left`, `right`, `slight-left`, `slight-right`,
    /// `uturn`, `arrive`.
    pub direction: String,
    pub instruction: String,
    pub road: String,
    pub distance_m: f64,
    /// First point index of this leg in [`RoutePlan::points`].
    #[serde(default)]
    pub start_index: u32,
    /// Last point index of this leg in [`RoutePlan::points`].
    #[serde(default)]
    pub end_index: u32,
}

/// A planned route in WGS-84.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutePlan {
    pub points: Vec<LatLng>,
    pub distance_m: f64,
    pub duration_s: f64,
    pub steps: Vec<RouteStep>,
}

#[derive(Debug, Clone, Copy)]
struct QueueItem {
    estimate: f64,
    node: u32,
}

impl PartialEq for QueueItem {
    fn eq(&self, other: &Self) -> bool {
        self.estimate == other.estimate && self.node == other.node
    }
}

impl Eq for QueueItem {}

impl Ord for QueueItem {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse: lowest estimate has the highest priority.
        other
            .estimate
            .total_cmp(&self.estimate)
            .then_with(|| other.node.cmp(&self.node))
    }
}

impl PartialOrd for QueueItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl RoadGraph {
    /// Snap a coordinate to the nearest graph node.
    #[must_use]
    pub fn snap(&self, point: LatLng) -> Option<Snap> {
        let (node, distance_m) = self.nearest_node(point)?;
        (distance_m <= MAX_SNAP_M).then_some(Snap { node, distance_m })
    }

    /// Plan a motorcycle route between two WGS-84 coordinates.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "edge indexes fit the address space"
    )]
    pub fn route(&self, origin: LatLng, destination: LatLng) -> Result<RoutePlan> {
        self.route_with(origin, destination, None)
    }

    /// Like [`RoadGraph::route`], but optionally penalises nodes near cameras.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "edge indexes fit the address space"
    )]
    pub fn route_with(
        &self,
        origin: LatLng,
        destination: LatLng,
        deterrent: Option<&CameraDeterrent>,
    ) -> Result<RoutePlan> {
        if !origin.is_valid() || !destination.is_valid() {
            return Err(RoutingError::InvalidCoordinate);
        }
        if self.nodes.is_empty() {
            return Err(RoutingError::GraphMissing);
        }
        let start = self.snap(origin).ok_or(RoutingError::SnapFailed)?;
        let goal = self.snap(destination).ok_or(RoutingError::SnapFailed)?;

        let destination_node = &self.nodes[goal.node as usize];
        let destination_point = destination_node.position();
        let start_node = &self.nodes[start.node as usize];

        let mut best = vec![f64::INFINITY; self.nodes.len()];
        let mut came: Vec<Option<(u32, u64)>> = vec![None; self.nodes.len()];
        let mut heap = BinaryHeap::new();
        best[start.node as usize] = 0.0;
        heap.push(QueueItem {
            estimate: heuristic(start_node.position(), destination_point),
            node: start.node,
        });

        while let Some(item) = heap.pop() {
            if item.node == goal.node {
                break;
            }
            let current = item.node as usize;
            if !item.estimate.is_finite() {
                continue;
            }
            let base = best[current];
            let offset = self.offsets[current];
            for (index, edge) in self.neighbors(item.node).iter().enumerate() {
                let next = edge.to as usize;
                let global_edge = offset as usize + index;
                if deterrent.is_some_and(|d| d.blocks_edge(global_edge)) {
                    continue;
                }
                let camera_penalty = deterrent.map_or(0.0, |d| f64::from(d.penalty(edge.to)));
                if camera_penalty.is_infinite() {
                    // Hard avoidance: never enter a camera zone.
                    continue;
                }
                let candidate = base + edge.duration_s() * (1.0 + camera_penalty);
                if candidate + 1e-9 >= best[next] {
                    continue;
                }
                best[next] = candidate;
                came[next] = Some((item.node, offset + index as u64));
                let estimate =
                    candidate + heuristic(self.nodes[next].position(), destination_point);
                heap.push(QueueItem {
                    estimate,
                    node: edge.to,
                });
            }
        }

        if !best[goal.node as usize].is_finite() {
            return Err(RoutingError::NoPath);
        }

        // Walk backwards, collecting nodes and the edges between them.
        let mut nodes_reversed = vec![goal.node];
        let mut edges_reversed = Vec::new();
        let mut cursor = goal.node;
        while let Some((previous, edge_index)) = came[cursor as usize] {
            edges_reversed.push(edge_index);
            nodes_reversed.push(previous);
            cursor = previous;
        }
        nodes_reversed.reverse();
        edges_reversed.reverse();

        let mut points: Vec<LatLng> = nodes_reversed
            .iter()
            .map(|node| self.nodes[*node as usize].position())
            .collect();
        let mut distance_m = edges_reversed
            .iter()
            .map(|index| f64::from(self.edges[*index as usize].distance_m))
            .sum::<f64>();
        let mut duration_s = edges_reversed
            .iter()
            .map(|index| self.edges[*index as usize].duration_s())
            .sum::<f64>();

        // Include the short connectors from the exact GPS points.
        if start.distance_m > 1.0 {
            distance_m += start.distance_m;
            duration_s += start.distance_m / (25.0 / 3.6);
            points.insert(0, origin);
        }
        if goal.distance_m > 1.0 {
            distance_m += goal.distance_m;
            duration_s += goal.distance_m / (25.0 / 3.6);
            points.push(destination);
        }

        let point_offset = u32::from(start.distance_m > 1.0);
        let steps = build_steps(self, &edges_reversed, start.node, distance_m, point_offset);

        Ok(RoutePlan {
            points,
            distance_m,
            duration_s,
            steps,
        })
    }
}

struct Leg {
    way: u32,
    road: String,
    distance_m: f64,
    entry_bearing: f64,
    exit_bearing: f64,
    start_index: u32,
    end_index: u32,
}

fn heuristic(from: LatLng, to: LatLng) -> f64 {
    haversine_m(from.latitude, from.longitude, to.latitude, to.longitude)
        / (HEURISTIC_SPEED_KMH / 3.6)
}

#[allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    reason = "one linear pass over the legs; indexes fit the address space"
)]
fn build_steps(
    graph: &RoadGraph,
    edges: &[u64],
    start_node: u32,
    _total_distance_m: f64,
    point_offset: u32,
) -> Vec<RouteStep> {
    if edges.is_empty() {
        return Vec::new();
    }

    let mut legs: Vec<Leg> = Vec::new();
    let mut current = start_node;
    let mut point_cursor = point_offset;
    for &index in edges {
        let edge = graph.edges[index as usize];
        let from_index = point_cursor;
        let to_index = point_cursor + 1;
        let from = graph.nodes[current as usize].position();
        let to = graph.nodes[edge.to as usize].position();
        let edge_bearing = bearing(from, to);
        let road = graph
            .ways
            .get(edge.way as usize)
            .map(|info| info.name.trim().to_owned())
            .unwrap_or_default();
        let merge = legs.last().is_some_and(|leg| {
            leg.road == road
                && normalize_degrees(edge_bearing - leg.exit_bearing).abs() < 20.0
        });
        if merge {
            if let Some(leg) = legs.last_mut() {
                leg.distance_m += f64::from(edge.distance_m);
                leg.exit_bearing = edge_bearing;
                leg.way = edge.way;
                leg.end_index = to_index;
            }
        } else {
            legs.push(Leg {
                way: edge.way,
                road,
                distance_m: f64::from(edge.distance_m),
                entry_bearing: edge_bearing,
                exit_bearing: edge_bearing,
                start_index: from_index,
                end_index: to_index,
            });
        }
        point_cursor = to_index;
        current = edge.to;
    }

    let mut steps = Vec::with_capacity(legs.len() + 1);
    let mut previous_bearing: Option<f64> = None;
    for leg in legs {
        let road_label = if leg.road.is_empty() {
            "无名道路".to_owned()
        } else {
            leg.road
        };

        let (direction, instruction) = match previous_bearing {
            None => (
                "start".to_owned(),
                format!("出发，沿{road_label}行驶"),
            ),
            Some(previous) => {
                let delta = normalize_degrees(leg.entry_bearing - previous);
                let direction = turn_direction(delta);
                let verb = match direction {
                    "left" => "左转",
                    "right" => "右转",
                    "slight-left" => "靠左",
                    "slight-right" => "靠右",
                    "uturn" => "掉头",
                    _ => "直行",
                };
                let instruction = if direction == "straight" {
                    format!("沿{road_label}继续行驶")
                } else {
                    format!("{verb}进入{road_label}")
                };
                (direction.to_owned(), instruction)
            }
        };

        steps.push(RouteStep {
            direction,
            instruction,
            road: road_label,
            distance_m: leg.distance_m,
            start_index: leg.start_index,
            end_index: leg.end_index,
        });
        previous_bearing = Some(leg.exit_bearing);
    }

    steps.push(RouteStep {
        direction: "arrive".to_owned(),
        instruction: "到达终点".to_owned(),
        road: String::new(),
        distance_m: 0.0,
        start_index: point_cursor,
        end_index: point_cursor,
    });
    steps
}

fn bearing(from: LatLng, to: LatLng) -> f64 {
    let phi1 = from.latitude.to_radians();
    let phi2 = to.latitude.to_radians();
    let delta_lambda = (to.longitude - from.longitude).to_radians();
    let y = delta_lambda.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * delta_lambda.cos();
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

fn normalize_degrees(value: f64) -> f64 {
    (value + 540.0).rem_euclid(360.0) - 180.0
}

fn turn_direction(delta: f64) -> &'static str {
    let magnitude = delta.abs();
    if magnitude < 20.0 {
        "straight"
    } else if magnitude > 150.0 {
        "uturn"
    } else if magnitude <= 55.0 {
        if delta > 0.0 { "slight-right" } else { "slight-left" }
    } else if delta > 0.0 {
        "right"
    } else {
        "left"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, GraphNode, WayInfo};

    fn square() -> RoadGraph {
        // 0 -> 1 -> 2 -> 3 around a ~500 m square, plus the closing edge.
        let nodes = vec![
            GraphNode { id: 1, lat: 31.230, lon: 121.470 },
            GraphNode { id: 2, lat: 31.234, lon: 121.470 },
            GraphNode { id: 3, lat: 31.234, lon: 121.476 },
            GraphNode { id: 4, lat: 31.230, lon: 121.476 },
        ];
        let ways = vec![
            WayInfo { name: "南北路".to_owned(), highway: "primary".to_owned() },
            WayInfo { name: "东西路".to_owned(), highway: "primary".to_owned() },
            WayInfo { name: "南路".to_owned(), highway: "secondary".to_owned() },
            WayInfo { name: "西路".to_owned(), highway: "secondary".to_owned() },
        ];
        let edges = vec![
            Edge { to: 1, distance_m: 445.0, speed_kmh: 40, way: 0 },
            Edge { to: 2, distance_m: 571.0, speed_kmh: 40, way: 1 },
            Edge { to: 3, distance_m: 445.0, speed_kmh: 40, way: 2 },
            Edge { to: 0, distance_m: 571.0, speed_kmh: 40, way: 3 },
        ];
        RoadGraph {
            nodes,
            ways,
            offsets: vec![0, 1, 2, 3, 4],
            edges,
        }
    }

    #[test]
    fn routes_around_the_square() {
        let graph = square();
        let plan = graph
            .route(LatLng::new(31.230, 121.470), LatLng::new(31.230, 121.476))
            .expect("route");
        assert!(plan.distance_m > 1_400.0);
        assert!(plan.duration_s > 100.0);
        assert!(plan.points.len() >= 4);
        assert_eq!(plan.steps.first().map(|step| step.direction.as_str()), Some("start"));
        assert_eq!(plan.steps.last().map(|step| step.direction.as_str()), Some("arrive"));
        assert!(plan.steps.iter().any(|step| step.instruction.contains("左转") || step.instruction.contains("右转")));
        let last = plan.steps.last().expect("arrive");
        assert_eq!(last.end_index as usize, plan.points.len() - 1);
        assert!(
            plan.steps.windows(2).all(|pair| pair[0].end_index <= pair[1].end_index),
            "step ranges must be monotonic"
        );
    }

    #[test]
    fn rejects_far_away_points() {
        let graph = square();
        let error = graph
            .route(LatLng::new(40.0, 116.0), LatLng::new(31.230, 121.476))
            .expect_err("must fail");
        assert!(matches!(error, RoutingError::SnapFailed));
    }

    #[test]
    fn reports_no_path_when_disconnected() {
        let mut graph = square();
        graph.offsets = vec![0, 1, 1, 1, 1];
        graph.edges.truncate(1);
        let error = graph
            .route(LatLng::new(31.230, 121.470), LatLng::new(31.230, 121.476))
            .expect_err("must fail");
        assert!(matches!(error, RoutingError::NoPath));
    }

    #[test]
    fn turn_direction_thresholds() {
        assert_eq!(turn_direction(0.0), "straight");
        assert_eq!(turn_direction(30.0), "slight-right");
        assert_eq!(turn_direction(-30.0), "slight-left");
        assert_eq!(turn_direction(90.0), "right");
        assert_eq!(turn_direction(-90.0), "left");
        assert_eq!(turn_direction(179.0), "uturn");
    }
}
