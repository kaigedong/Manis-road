//! Acceptance-style test for continuous re-planning.
//!
//! Target from the goal: five deliberate deviations each re-plan within 3 s, and
//! the route does not flicker. The ride here is simulated on a grid graph so the
//! test is deterministic and fast; the routing itself is the real one.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    reason = "grid indexes and metre<->degree conversions are tiny by construction"
)]

use std::time::{Duration, Instant};

use motu_core::LatLng;
use motu_routing::graph::{Edge, GraphNode, RoadGraph, WayInfo, haversine_m};
use motu_routing::replan::{Candidate, ReplanPolicy, ReplanReason, ReplanState, route_overlap};
use motu_routing::{RouteContext, RouteProfile};

const SIDE: usize = 6;
/// ~111 m per 0.001° of latitude.
const STEP_DEG: f64 = 0.001;

fn index(row: usize, col: usize) -> usize {
    row * SIDE + col
}

fn position(row: usize, col: usize) -> LatLng {
    LatLng::new(
        31.2300 + row as f64 * STEP_DEG,
        121.4700 + col as f64 * STEP_DEG,
    )
}

/// A `SIDE × SIDE` bidirectional grid, all residential.
fn grid() -> RoadGraph {
    let mut nodes = Vec::new();
    for row in 0..SIDE {
        for col in 0..SIDE {
            let point = position(row, col);
            nodes.push(GraphNode {
                id: index(row, col) as i64,
                lat: point.latitude,
                lon: point.longitude,
            });
        }
    }
    let ways = vec![WayInfo {
        name: "网格路".to_owned(),
        highway: "residential".to_owned(),
     ..WayInfo::default() }];

    // Build adjacency, then flatten into CSR.
    let mut adjacency: Vec<Vec<Edge>> = vec![Vec::new(); nodes.len()];
    let mut push = |from: usize, to: usize| {
        let a = position(from / SIDE, from % SIDE);
        let b = position(to / SIDE, to % SIDE);
        let distance = (haversine_m(a.latitude, a.longitude, b.latitude, b.longitude) as f32).max(1.0);
        adjacency[from].push(Edge {
            to: to as u32,
            distance_m: distance,
            speed_kmh: 30,
            way: 0,
        });
    };
    for row in 0..SIDE {
        for col in 0..SIDE {
            let here = index(row, col);
            if col + 1 < SIDE {
                push(here, index(row, col + 1));
                push(index(row, col + 1), here);
            }
            if row + 1 < SIDE {
                push(here, index(row + 1, col));
                push(index(row + 1, col), here);
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

fn offset(point: LatLng, metres: f64) -> LatLng {
    // Shift east; 0.001° of latitude ≈ 111 m, and longitude is close enough here.
    LatLng::new(
        point.latitude,
        point.longitude + metres / 111_000.0,
    )
}

#[test]
fn five_deviations_replan_fast_and_the_route_does_not_flicker() {
    let graph = grid();
    let profile = RouteProfile::motorcycle_default();
    let context = RouteContext::default();
    let destination = position(SIDE - 1, SIDE - 1);

    let policy = ReplanPolicy::default();
    let mut state = ReplanState::new();
    let base = Instant::now();

    let mut position_now = position(0, 0);
    let mut slowest = Duration::ZERO;
    let mut deviation_switches = 0;

    // Five deliberate deviations, 60 s apart so the cooldown never masks them.
    for step in 0..5 {
        let now = base + Duration::from_secs(60 * (step + 1));
        position_now = offset(position_now, 120.0);

        let started = Instant::now();
        let decision = state.evaluate(&policy, now, 120.0, None);
        assert_eq!(
            decision,
            Some(ReplanReason::Deviation),
            "deviation {step} must force a re-plan"
        );
        let (_plan, _metrics) = graph
            .route_report_with_context(position_now, destination, &profile, None, &context)
            .expect("re-plan must succeed");
        let elapsed = started.elapsed();
        slowest = slowest.max(elapsed);

        deviation_switches += 1;
    }

    // Normal riding with a near-identical candidate must not switch.
    let mut flicker = 0;
    for step in 0..20 {
        let now = base + Duration::from_secs(600 + 20 * step);
        let decision = state.evaluate(
            &policy,
            now,
            5.0,
            Some(Candidate {
                improvement: 0.20,
                overlap: 0.85, // covers most of the route → same road in practice
            }),
        );
        if decision.is_some() {
            flicker += 1;
        }
    }

    assert_eq!(deviation_switches, 5);
    assert!(
        flicker <= 1,
        "route flicker must stay ≤ 1, got {flicker}"
    );
    assert!(
        slowest < Duration::from_secs(3),
        "each re-plan must finish within 3 s, slowest was {slowest:?}"
    );
    assert_eq!(state.switches(), 5);
}

#[test]
fn overlap_distinguishes_a_real_detour_from_the_same_road() {
    // The same corridor nudged 30 m sideways is "the same route" for switching
    // purposes; a parallel street 300 m away is not.
    let corridor: Vec<LatLng> = (0..20)
        .map(|step| LatLng::new(31.2300, 121.4700 + f64::from(step) * 0.001))
        .collect();
    let nudged: Vec<LatLng> = corridor
        .iter()
        .map(|point| LatLng::new(point.latitude, point.longitude + 30.0 / 111_000.0))
        .collect();
    let parallel: Vec<LatLng> = corridor
        .iter()
        .map(|point| LatLng::new(point.latitude + 300.0 / 111_000.0, point.longitude))
        .collect();

    let nudged_overlap = route_overlap(&corridor, &nudged, 50.0);
    let parallel_overlap = route_overlap(&corridor, &parallel, 50.0);

    assert!(
        nudged_overlap > 0.9,
        "a 30 m nudge must read as the same route: {nudged_overlap}"
    );
    assert!(
        parallel_overlap < 0.1,
        "a parallel street 300 m away must not: {parallel_overlap}"
    );
    // And the switch threshold sits between them.
    let policy = ReplanPolicy::default();
    assert!(nudged_overlap > policy.max_overlap);
    assert!(parallel_overlap < policy.max_overlap);
}
