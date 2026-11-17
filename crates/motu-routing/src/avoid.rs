//! Camera-aware routing: penalise road nodes near known camera points.
//!
//! The client still plans on its local graph; cameras only change the cost of
//! edges whose target node is within a radius of a known point. A separate
//! fastest route is computed for comparison, and the caller decides whether the
//! detour is worth it.

use std::collections::HashMap;

use motu_core::LatLng;
use motu_core::geo::distance_to_polyline_m;

use crate::graph::{RoadGraph, haversine_m};

/// Extra travel time applied to nodes near a camera: `1.0` doubles the edge
/// cost, `4.0` makes it five times as expensive.
pub struct CameraDeterrent {
    penalties: Vec<f32>,
    blocked_edges: Vec<bool>,
}

impl CameraDeterrent {
    /// Build per-node penalties for every camera within `radius_m`.
    ///
    /// A small grid keeps the build near-linear in the number of nodes even for
    /// the full Shanghai graph.
    /// Hard avoidance: block the road segments that pass within `radius_m` of
    /// a camera. Blocking edges (rather than nodes) keeps intersections and the
    /// rest of the network connected.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "offsets are bounded by the in-memory edge count"
    )]
    pub fn build_hard(graph: &RoadGraph, cameras: &[LatLng], radius_m: f64) -> Self {
        let mut blocked_edges = vec![false; graph.edge_count()];
        if cameras.is_empty() || radius_m <= 0.0 {
            return Self {
                penalties: vec![0.0; graph.node_count()],
                blocked_edges,
            };
        }
        let cells = node_cells(graph);
        let lat_delta = radius_m / 111_320.0;
        for camera in cameras {
            if !camera.is_valid() {
                continue;
            }
            let cos_lat = camera.latitude.to_radians().cos().abs().max(0.05);
            let lon_delta = radius_m / (111_320.0 * cos_lat);
            let (min_i, min_j) = cell_key(camera.latitude - lat_delta, camera.longitude - lon_delta);
            let (max_i, max_j) = cell_key(camera.latitude + lat_delta, camera.longitude + lon_delta);
            for i in min_i..=max_i {
                for j in min_j..=max_j {
                    let Some(nodes) = cells.get(&(i, j)) else {
                        continue;
                    };
                    for &node_index in nodes {
                        let node = &graph.nodes[node_index as usize];
                        if haversine_m(camera.latitude, camera.longitude, node.lat, node.lon)
                            > radius_m + 1.0
                        {
                            continue;
                        }
                        let start = graph.offsets[node_index as usize] as usize;
                        let end = graph.offsets[node_index as usize + 1] as usize;
                        for (local_index, edge) in graph.edges[start..end].iter().enumerate() {
                            let target = &graph.nodes[edge.to as usize];
                            let distance = distance_to_polyline_m(
                                *camera,
                                &[node.position(), target.position()],
                            )
                            .unwrap_or(f64::MAX);
                            if distance <= radius_m {
                                blocked_edges[start + local_index] = true;
                            }
                        }
                    }
                }
            }
        }
        Self {
            penalties: vec![0.0; graph.node_count()],
            blocked_edges,
        }
    }

    #[must_use]
    pub fn build(
        graph: &RoadGraph,
        cameras: &[LatLng],
        radius_m: f64,
        penalty: f32,
    ) -> Self {
        let mut penalties = vec![0.0_f32; graph.node_count()];
        let blocked_edges = vec![false; graph.edge_count()];
        if cameras.is_empty() || radius_m <= 0.0 || penalty <= 0.0 {
            return Self {
                penalties,
                blocked_edges,
            };
        }
        let cells = node_cells(graph);

        let lat_delta = radius_m / 111_320.0;
        for camera in cameras {
            if !camera.is_valid() {
                continue;
            }
            let cos_lat = camera.latitude.to_radians().cos().abs().max(0.05);
            let lon_delta = radius_m / (111_320.0 * cos_lat);
            let (min_i, min_j) = cell_key(camera.latitude - lat_delta, camera.longitude - lon_delta);
            let (max_i, max_j) = cell_key(camera.latitude + lat_delta, camera.longitude + lon_delta);
            for i in min_i..=max_i {
                for j in min_j..=max_j {
                    let Some(nodes) = cells.get(&(i, j)) else {
                        continue;
                    };
                    for &index in nodes {
                        let node = &graph.nodes[index as usize];
                        if haversine_m(camera.latitude, camera.longitude, node.lat, node.lon)
                            <= radius_m
                        {
                            penalties[index as usize] = penalties[index as usize].max(penalty);
                        }
                    }
                }
            }
        }
        Self {
            penalties,
            blocked_edges,
        }
    }

    #[must_use]
    pub fn blocks_edge(&self, edge_index: usize) -> bool {
        self.blocked_edges
            .get(edge_index)
            .copied()
            .unwrap_or(false)
    }

    #[must_use]
    pub fn blocked_edges(&self) -> usize {
        self.blocked_edges.iter().filter(|blocked| **blocked).count()
    }

    #[must_use]
    pub fn penalty(&self, node: u32) -> f32 {
        self.penalties
            .get(node as usize)
            .copied()
            .unwrap_or_default()
    }

    #[must_use]
    pub fn affected_nodes(&self) -> usize {
        self.penalties.iter().filter(|penalty| **penalty > 0.0).count()
    }
}

fn node_cells(graph: &RoadGraph) -> HashMap<(i32, i32), Vec<u32>> {
    let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        let Ok(index) = u32::try_from(index) else {
            continue;
        };
        cells
            .entry(cell_key(node.lat, node.lon))
            .or_default()
            .push(index);
    }
    cells
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "cell keys are bounded by the valid coordinate range"
)]
fn cell_key(latitude: f64, longitude: f64) -> (i32, i32) {
    const CELL_DEGREES: f64 = 0.002;
    (
        (latitude / CELL_DEGREES).floor() as i32,
        (longitude / CELL_DEGREES).floor() as i32,
    )
}

/// How many cameras lie within `radius_m` of the route polyline.
#[must_use]
pub fn cameras_within(points: &[LatLng], cameras: &[LatLng], radius_m: f64) -> usize {
    cameras
        .iter()
        .filter(|camera| {
            distance_to_polyline_m(**camera, points).is_some_and(|distance| distance <= radius_m)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, GraphNode, WayInfo};

    fn graph() -> RoadGraph {
        // Direct path 0 -> 4 -> 1 (camera at node 4) and a detour 0 -> 2 -> 3 -> 1.
        let nodes = vec![
            GraphNode { id: 0, lat: 31.2300, lon: 121.4700 },
            GraphNode { id: 1, lat: 31.2320, lon: 121.4720 },
            GraphNode { id: 2, lat: 31.2340, lon: 121.4700 },
            GraphNode { id: 3, lat: 31.2340, lon: 121.4720 },
            GraphNode { id: 4, lat: 31.2310, lon: 121.4710 },
        ];
        let ways = vec![WayInfo { name: "demo".to_owned(), highway: "primary".to_owned() , ..WayInfo::default() }];
        let edges = vec![
            Edge { to: 4, distance_m: 150.0, speed_kmh: 30, way: 0 },
            Edge { to: 2, distance_m: 700.0, speed_kmh: 30, way: 0 },
            Edge { to: 4, distance_m: 150.0, speed_kmh: 30, way: 0 },
            Edge { to: 3, distance_m: 700.0, speed_kmh: 30, way: 0 },
            Edge { to: 0, distance_m: 700.0, speed_kmh: 30, way: 0 },
            Edge { to: 3, distance_m: 200.0, speed_kmh: 30, way: 0 },
            Edge { to: 2, distance_m: 200.0, speed_kmh: 30, way: 0 },
            Edge { to: 1, distance_m: 700.0, speed_kmh: 30, way: 0 },
            Edge { to: 0, distance_m: 150.0, speed_kmh: 30, way: 0 },
            Edge { to: 1, distance_m: 150.0, speed_kmh: 30, way: 0 },
        ];
        RoadGraph {
            signals: Vec::new(),
            nodes,
            ways,
            offsets: vec![0, 2, 4, 6, 8, 10],
            edges,
        }
    }

    #[test]
    fn penalties_only_cover_nearby_nodes() {
        let graph = graph();
        let camera = graph.nodes[4].position();
        let deterrent = CameraDeterrent::build(&graph, &[camera], 30.0, 2.0);
        assert_eq!(deterrent.affected_nodes(), 1);
        assert!(deterrent.penalty(4) > 0.0);
        assert_eq!(deterrent.penalty(2), 0.0);
    }

    #[test]
    fn hard_avoidance_blocks_the_direct_road() {
        let graph = graph();
        let origin = graph.nodes[0].position();
        let destination = graph.nodes[1].position();
        let camera = graph.nodes[4].position();
        let deterrent = CameraDeterrent::build_hard(&graph, &[camera], 30.0);
        let avoiding = graph
            .route_with(origin, destination, Some(&deterrent))
            .expect("detour exists");
        assert_eq!(cameras_within(&avoiding.points, &[camera], 20.0), 0);
        assert!(avoiding.distance_m > 300.0);
    }

    #[test]
    fn hard_avoidance_reports_no_route_when_blocked() {
        let mut graph = graph();
        // Remove the detour edges, leaving only the camera road.
        graph.offsets = vec![0, 1, 2, 2, 2, 2];
        graph.edges = vec![
            Edge { to: 4, distance_m: 150.0, speed_kmh: 30, way: 0 },
            Edge { to: 1, distance_m: 150.0, speed_kmh: 30, way: 0 },
        ];
        let origin = graph.nodes[0].position();
        let destination = graph.nodes[1].position();
        let camera = graph.nodes[4].position();
        let deterrent = CameraDeterrent::build_hard(&graph, &[camera], 30.0);
        assert!(graph
            .route_with(origin, destination, Some(&deterrent))
            .is_err());
    }

    #[test]
    fn camera_penalty_can_choose_the_detour() {
        let graph = graph();
        let origin = graph.nodes[0].position();
        let destination = graph.nodes[1].position();
        let camera = graph.nodes[4].position();
        let baseline = graph.route(origin, destination).expect("baseline");
        let deterrent = CameraDeterrent::build(&graph, &[camera], 30.0, 10.0);
        let avoiding = graph
            .route_with(origin, destination, Some(&deterrent))
            .expect("avoiding");
        assert!(baseline.distance_m < avoiding.distance_m, "detour is longer");
        assert_eq!(cameras_within(&baseline.points, &[camera], 20.0), 1);
        assert_eq!(cameras_within(&avoiding.points, &[camera], 20.0), 0);
    }

    #[test]
    fn camera_near_route_is_counted() {
        let graph = graph();
        let points = vec![graph.nodes[0].position(), graph.nodes[4].position()];
        assert_eq!(cameras_within(&points, &[graph.nodes[4].position()], 20.0), 1);
        assert_eq!(cameras_within(&points, &[graph.nodes[2].position()], 20.0), 0);
    }
}
