//! A* routing over the offline graph, with simple navigation steps.

use std::{cmp::Ordering, collections::BinaryHeap};

use motu_core::LatLng;
use serde::{Deserialize, Serialize};

use crate::avoid::CameraDeterrent;
use crate::error::{Result, RoutingError};
use crate::graph::{RoadGraph, haversine_m};
use crate::metrics::{RouteMetrics, measure_edges};
use crate::profile::RouteProfile;
use crate::traffic::TrafficPrior;

/// How far a coordinate may be from the graph and still snap to it.
const MAX_SNAP_M: f64 = 600.0;

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

/// Everything about *when* and *under which preferences* a route is planned.
///
/// Kept separate from [`RouteProfile`] because the profile is static (how much
/// this rider dislikes arterials) while this is situational (is it rush hour).
#[derive(Debug, Clone, Copy, Default)]
pub struct RouteContext<'a> {
    /// Time-of-day multipliers; `None` means no time dependence.
    pub prior: Option<&'a TrafficPrior>,
    /// Local hour, 0..=23. Only read when `prior` is set.
    pub hour: u8,
    /// Camera and restricted-zone risk. Zones raise the cost of the roads inside
    /// them; they never make a road impassable — riding through is the point.
    pub risk: Option<&'a crate::risk::RiskModel>,
}

#[derive(Debug, Clone, Copy)]
struct QueueItem {
    estimate: f64,
    /// Recorded cost to reach `index`, used to discard stale queue entries.
    cost: f64,
    /// Edge index, not node index: the search runs over directed edges so that
    /// junction cost can depend on how the rider arrived.
    index: u32,
}

impl PartialEq for QueueItem {
    fn eq(&self, other: &Self) -> bool {
        self.estimate == other.estimate && self.index == other.index
    }
}

impl Eq for QueueItem {}

impl Ord for QueueItem {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse: lowest estimate has the highest priority.
        other
            .estimate
            .total_cmp(&self.estimate)
            .then_with(|| other.index.cmp(&self.index))
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
    pub fn route(&self, origin: LatLng, destination: LatLng) -> Result<RoutePlan> {
        self.route_with(origin, destination, None)
    }

    /// Like [`RoadGraph::route`], but optionally penalises nodes near cameras.
    pub fn route_with(
        &self,
        origin: LatLng,
        destination: LatLng,
        deterrent: Option<&CameraDeterrent>,
    ) -> Result<RoutePlan> {
        self.route_report(
            origin,
            destination,
            &RouteProfile::motorcycle_default(),
            deterrent,
        )
        .map(|(plan, _)| plan)
    }

    /// Plan with an explicit cost profile.
    pub fn route_with_profile(
        &self,
        origin: LatLng,
        destination: LatLng,
        profile: &RouteProfile,
        deterrent: Option<&CameraDeterrent>,
    ) -> Result<RoutePlan> {
        self.route_report(origin, destination, profile, deterrent)
            .map(|(plan, _)| plan)
    }

    /// Plan a route and measure its road-class composition in the same pass.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "edge indexes fit the address space"
    )]
    pub fn route_report(
        &self,
        origin: LatLng,
        destination: LatLng,
        profile: &RouteProfile,
        deterrent: Option<&CameraDeterrent>,
    ) -> Result<(RoutePlan, RouteMetrics)> {
        self.route_report_with_context(
            origin,
            destination,
            profile,
            deterrent,
            &RouteContext::default(),
        )
    }

    /// Plan and measure with a time-of-day prior.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "edge indexes fit the address space"
    )]
    pub fn route_report_with_context(
        &self,
        origin: LatLng,
        destination: LatLng,
        profile: &RouteProfile,
        deterrent: Option<&CameraDeterrent>,
        context: &RouteContext<'_>,
    ) -> Result<(RoutePlan, RouteMetrics)> {
        if !origin.is_valid() || !destination.is_valid() {
            return Err(RoutingError::InvalidCoordinate);
        }
        if self.nodes.is_empty() {
            return Err(RoutingError::GraphMissing);
        }
        let start = self.snap(origin).ok_or(RoutingError::SnapFailed)?;
        let goal = self.snap(destination).ok_or(RoutingError::SnapFailed)?;

        let route_edges = if start.node == goal.node {
            Vec::new()
        } else {
            self.search_edges(start.node, goal.node, profile, deterrent, context)?
        };

        let mut points: Vec<LatLng> = Vec::with_capacity(route_edges.len() + 1);
        points.push(self.nodes[start.node as usize].position());
        for &index in &route_edges {
            points.push(self.nodes[self.edges[index as usize].to as usize].position());
        }

        let mut distance_m = route_edges
            .iter()
            .map(|index| f64::from(self.edges[*index as usize].distance_m))
            .sum::<f64>();
        let mut duration_s = route_edges
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
        if points.len() == 1 {
            points.push(destination);
        }

        let point_offset = u32::from(start.distance_m > 1.0);
        let mut steps = build_steps(self, &route_edges, start.node, distance_m, point_offset);
        if steps.is_empty() {
            steps.push(RouteStep {
                direction: "arrive".to_owned(),
                instruction: "到达终点".to_owned(),
                road: String::new(),
                distance_m: 0.0,
                start_index: point_offset,
                end_index: point_offset,
            });
        }

        let mut metrics = measure_edges(self, &route_edges);
        if let Some(risk) = context.risk {
            let (count, share) =
                crate::risk::route_risk_report(&points, &risk.cameras, &risk.zones, 30.0);
            let km = metrics.distance_m / 1000.0;
            metrics.cameras_on_route = Some(count);
            #[allow(
                clippy::cast_precision_loss,
                reason = "camera count per route is a small integer"
            )]
            let count_f = count as f64;
            metrics.cameras_per_km = if km > 0.0 { Some(count_f / km) } else { None };
            metrics.restricted_share = Some(share);
        }
        metrics.distance_m = distance_m;
        metrics.duration_s = duration_s;

        Ok((
            RoutePlan {
                points,
                distance_m,
                duration_s,
                steps,
            },
            metrics,
        ))
    }


    /// A* over directed edges.
    ///
    /// Edges, not nodes, are the search states: the cost of leaving a junction
    /// depends on the road the rider arrived on (turn penalty), which a
    /// node-only search cannot express.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::too_many_lines,
        reason = "one linear pass over the adjacency plus the A* loop"
    )]
    fn search_edges(
        &self,
        start_node: u32,
        goal_node: u32,
        profile: &RouteProfile,
        deterrent: Option<&CameraDeterrent>,
        context: &RouteContext<'_>,
    ) -> Result<Vec<u64>> {
        let edge_count = self.edges.len();
        let max_speed = self
            .edges
            .iter()
            .map(|edge| edge.speed_kmh)
            .max()
            .unwrap_or(80)
            .max(80);
        let destination_point = self.nodes[goal_node as usize].position();

        let mut base_cost = vec![0_f64; edge_count];
        let mut bearing_deg = vec![0_f32; edge_count];
        for node in 0..self.nodes.len() {
            let from = self.nodes[node].position();
            for index in self.offsets[node] as usize..self.offsets[node + 1] as usize {
                let edge = self.edges[index];
                let highway = self
                    .ways
                    .get(edge.way as usize)
                    .map_or("", |way| way.highway.as_str());
                let traffic = context
                    .prior
                    .map_or(1.0, |prior| prior.factor(context.hour, highway));
                let way = self.ways.get(edge.way as usize);
                let static_factor = profile.way_factor(
                    highway,
                    way.map_or(0, |info| info.lanes),
                    way.map_or(crate::graph::Surface::Unknown, |info| info.surface),
                );
                let mut cost = edge.duration_s() * static_factor * traffic;
                if let Some(risk) = context.risk {
                    cost += risk.edge_penalty_s(from, self.nodes[edge.to as usize].position(), cost);
                }
                base_cost[index] = cost;
                bearing_deg[index] = bearing(from, self.nodes[edge.to as usize].position()) as f32;
            }
        }

        let mut best = vec![f64::INFINITY; edge_count];
        let mut came = vec![u32::MAX; edge_count];
        let mut heap = BinaryHeap::new();

        for index in self.offsets[start_node as usize] as usize
            ..self.offsets[start_node as usize + 1] as usize
        {
            let edge = self.edges[index];
            if deterrent.is_some_and(|d| d.blocks_edge(index)) {
                continue;
            }
            let camera = deterrent.map_or(0.0, |d| f64::from(d.penalty(edge.to)));
            if camera.is_infinite() {
                continue;
            }
            // The first edge can also arrive at a light; charge it here too.
            let signal_penalty = if self.signals.get(edge.to as usize).copied() == Some(true) {
                profile.signal_penalty_s
            } else {
                0.0
            };
            let candidate = base_cost[index] * (1.0 + camera) + signal_penalty;
            best[index] = candidate;
            let estimate = candidate
                + heuristic(
                    self.nodes[edge.to as usize].position(),
                    destination_point,
                    max_speed,
                );
            heap.push(QueueItem {
                estimate,
                cost: candidate,
                index: u32::try_from(index).unwrap_or(u32::MAX),
            });
        }

        let mut last_edge: Option<usize> = None;
        while let Some(item) = heap.pop() {
            let current = item.index as usize;
            let recorded = best[current];
            if item.cost > recorded + 1e-3 {
                continue;
            }
            let edge = self.edges[current];
            if edge.to == goal_node {
                last_edge = Some(current);
                break;
            }
            let turn_from = f64::from(bearing_deg[current]);
            for next in self.offsets[edge.to as usize] as usize
                ..self.offsets[edge.to as usize + 1] as usize
            {
                let next_edge = self.edges[next];
                if deterrent.is_some_and(|d| d.blocks_edge(next)) {
                    continue;
                }
                let camera = deterrent.map_or(0.0, |d| f64::from(d.penalty(next_edge.to)));
                if camera.is_infinite() {
                    continue;
                }
                let angle = normalize_degrees(f64::from(bearing_deg[next]) - turn_from).abs();
                let signal_penalty = if self.signals.get(next_edge.to as usize).copied() == Some(true)
                {
                    profile.signal_penalty_s
                } else {
                    0.0
                };
                let cost = recorded
                    + base_cost[next] * (1.0 + camera)
                    + profile.turns.for_angle(angle)
                    + signal_penalty;
                if cost + 1e-6 >= best[next] {
                    continue;
                }
                best[next] = cost;
                came[next] = u32::try_from(current).unwrap_or(u32::MAX);
                let estimate = cost
                    + heuristic(
                        self.nodes[next_edge.to as usize].position(),
                        destination_point,
                        max_speed,
                    );
                heap.push(QueueItem {
                    estimate,
                    cost,
                    index: u32::try_from(next).unwrap_or(u32::MAX),
                });
            }
        }

        let Some(mut cursor) = last_edge else {
            return Err(RoutingError::NoPath);
        };
        let mut edges = vec![cursor as u64];
        while came[cursor] != u32::MAX {
            cursor = came[cursor] as usize;
            edges.push(cursor as u64);
        }
        edges.reverse();
        Ok(edges)
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

fn heuristic(from: LatLng, to: LatLng, max_speed_kmh: u16) -> f64 {
    haversine_m(from.latitude, from.longitude, to.latitude, to.longitude)
        / (f64::from(max_speed_kmh).max(1.0) / 3.6)
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

pub(crate) fn bearing(from: LatLng, to: LatLng) -> f64 {
    let phi1 = from.latitude.to_radians();
    let phi2 = to.latitude.to_radians();
    let delta_lambda = (to.longitude - from.longitude).to_radians();
    let y = delta_lambda.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * delta_lambda.cos();
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

pub(crate) fn normalize_degrees(value: f64) -> f64 {
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
            WayInfo { name: "南北路".to_owned(), highway: "primary".to_owned() , ..WayInfo::default() },
            WayInfo { name: "东西路".to_owned(), highway: "primary".to_owned() , ..WayInfo::default() },
            WayInfo { name: "南路".to_owned(), highway: "secondary".to_owned() , ..WayInfo::default() },
            WayInfo { name: "西路".to_owned(), highway: "secondary".to_owned() , ..WayInfo::default() },
        ];
        let edges = vec![
            Edge { to: 1, distance_m: 445.0, speed_kmh: 40, way: 0 },
            Edge { to: 2, distance_m: 571.0, speed_kmh: 40, way: 1 },
            Edge { to: 3, distance_m: 445.0, speed_kmh: 40, way: 2 },
            Edge { to: 0, distance_m: 571.0, speed_kmh: 40, way: 3 },
        ];
        RoadGraph {
            nodes,
            signals: Vec::new(),
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
