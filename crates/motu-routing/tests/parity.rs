//! Parity harness behaviour: pass, fail and unroutable cases.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    reason = "grid indexes and metre<->degree conversions are tiny by construction"
)]

use motu_core::LatLng;
use motu_routing::graph::{Edge, GraphNode, RoadGraph, WayInfo, haversine_m};
use motu_routing::parity::{MAX_DETOUR_PCT, compare};
use motu_routing::{Baseline, BaselineRoute, RouteContext, RouteProfile};

const SIDE: usize = 6;
const STEP_DEG: f64 = 0.001;

fn position(row: usize, col: usize) -> LatLng {
    LatLng::new(
        31.2300 + row as f64 * STEP_DEG,
        121.4700 + col as f64 * STEP_DEG,
    )
}

fn grid() -> RoadGraph {
    let mut nodes = Vec::new();
    for row in 0..SIDE {
        for col in 0..SIDE {
            let point = position(row, col);
            nodes.push(GraphNode {
                id: (row * SIDE + col) as i64,
                lat: point.latitude,
                lon: point.longitude,
            });
        }
    }
    let ways = vec![WayInfo {
        name: "网格路".to_owned(),
        highway: "residential".to_owned(),
        ..WayInfo::default()
    }];
    let mut adjacency: Vec<Vec<Edge>> = vec![Vec::new(); nodes.len()];
    let mut push = |from: usize, to: usize| {
        let a = position(from / SIDE, from % SIDE);
        let b = position(to / SIDE, to % SIDE);
        let distance = haversine_m(a.latitude, a.longitude, b.latitude, b.longitude) as f32;
        adjacency[from].push(Edge {
            to: to as u32,
            distance_m: distance.max(1.0),
            speed_kmh: 30,
            way: 0,
        });
    };
    for row in 0..SIDE {
        for col in 0..SIDE {
            let here = row * SIDE + col;
            if col + 1 < SIDE {
                push(here, here + 1);
                push(here + 1, here);
            }
            if row + 1 < SIDE {
                push(here, here + SIDE);
                push(here + SIDE, here);
            }
        }
    }
    let mut offsets = vec![0_u64; nodes.len() + 1];
    let mut edges = Vec::new();
    for (index, list) in adjacency.iter().enumerate() {
        offsets[index + 1] = offsets[index] + list.len() as u64;
        edges.extend_from_slice(list);
    }
    RoadGraph {
        nodes,
        signals: Vec::new(),
        ways,
        offsets,
        edges,
    }
}

fn baseline_with(distance_m: f64) -> Baseline {
    Baseline {
        source: "calPath".to_owned(),
        generated_at: None,
        routes: vec![BaselineRoute {
            name: "网格对角".to_owned(),
            origin: [31.2300, 121.4700],
            destination: [31.2350, 121.4750],
            distance_m,
            duration_s: 600.0,
            points: vec![[31.2300, 121.4700], [31.2350, 121.4750]],
        }],
    }
}

fn our_distance(graph: &RoadGraph) -> f64 {
    graph
        .route(
            LatLng::new(31.2300, 121.4700),
            LatLng::new(31.2350, 121.4750),
        )
        .expect("route")
        .distance_m
}

#[test]
fn within_budget_passes() {
    let graph = grid();
    let ours = our_distance(&graph);
    // Baseline 5 % shorter than ours → detour ≈ +5.3 %.
    let baseline = baseline_with(ours * 0.95);
    let report = compare(
        &graph,
        &baseline,
        &RouteProfile::motorcycle_default(),
        &RouteContext::default(),
    )
    .expect("compare");
    assert!(report.ok(), "should pass: {:?}", report.routes);
    assert_eq!(report.passed, 1);
    assert!(report.median_detour_pct < MAX_DETOUR_PCT);
}

#[test]
fn over_budget_fails_with_a_reason() {
    let graph = grid();
    let ours = our_distance(&graph);
    // Baseline half of ours → detour +100 %.
    let baseline = baseline_with(ours * 0.5);
    let report = compare(
        &graph,
        &baseline,
        &RouteProfile::motorcycle_default(),
        &RouteContext::default(),
    )
    .expect("compare");
    assert!(!report.ok());
    assert_eq!(report.failed, 1);
    let failure = report.routes[0].failures.first().expect("reason");
    assert!(failure.contains("绕行"), "reason: {failure}");
    assert!(report.routes[0].overlap.is_some(), "overlap is reported");
}

#[test]
fn unroutable_route_is_reported_not_ignored() {
    let graph = grid();
    let mut baseline = baseline_with(1000.0);
    baseline.routes.push(BaselineRoute {
        name: "图外".to_owned(),
        origin: [40.0, 116.0],
        destination: [40.1, 116.1],
        distance_m: 1000.0,
        duration_s: 60.0,
        points: Vec::new(),
    });
    let report = compare(
        &graph,
        &baseline,
        &RouteProfile::motorcycle_default(),
        &RouteContext::default(),
    )
    .expect("compare");
    assert_eq!(report.unroutable, vec!["图外".to_owned()]);
    assert!(!report.ok(), "an unroutable baseline must fail the run");
}
