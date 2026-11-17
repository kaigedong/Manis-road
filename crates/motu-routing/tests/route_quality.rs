//! Behavioural tests for the motorcycle cost profile.
//!
//! These lock in the two properties the goal is graded on: prefer quiet streets
//! when the detour is cheap, and do not zig-zag through junctions when a
//! straight line is available.

use motu_core::LatLng;
use motu_routing::graph::{Edge, GraphNode, RoadGraph, Surface, WayInfo};
use motu_routing::{ClassWeights, RouteProfile, TurnPenalties};

fn node(id: i64, lat: f64, lon: f64) -> GraphNode {
    GraphNode { id, lat, lon }
}

fn edge(to: u32, distance_m: f32, speed_kmh: u16, way: u32) -> Edge {
    Edge {
        to,
        distance_m,
        speed_kmh,
        way,
    }
}

fn neutral_profile() -> RouteProfile {
    RouteProfile {
        classes: ClassWeights {
            trunk: 1.0,
            primary: 1.0,
            secondary: 1.0,
            tertiary: 1.0,
            residential: 1.0,
            other: 1.0,
        },
        turns: TurnPenalties {
            straight_max_deg: 0.0,
            slight_max_deg: 0.0,
            turn_max_deg: 0.0,
            straight_s: 0.0,
            slight_s: 0.0,
            turn_s: 0.0,
            sharp_s: 0.0,
            u_turn_s: 0.0,
        },
        signal_penalty_s: 0.0,
        lane_step: 0.0,
        unpaved_factor: 1.0,
    }
}

/// A primary shortcut (1200 m) against a residential detour (2×1100 m).
///
/// Time alone prefers the arterial; the motorcycle profile must prefer the
/// quiet street even though it takes 76 s longer.
fn shortcut_vs_detour() -> RoadGraph {
    let nodes = vec![
        node(1, 31.2300, 121.4700), // A
        node(2, 31.2340, 121.4750), // C, the detour corner
        node(3, 31.2300, 121.4800), // D
    ];
    let ways = vec![
        WayInfo {
            name: "大路".to_owned(),
            highway: "primary".to_owned(),
         ..WayInfo::default() },
        WayInfo {
            name: "小路".to_owned(),
            highway: "residential".to_owned(),
         ..WayInfo::default() },
    ];
    let edges = vec![
        edge(2, 1200.0, 30, 0), // 0: A -> D, primary
        edge(1, 1100.0, 30, 1), // 1: A -> C, residential
        edge(2, 1100.0, 30, 1), // 2: C -> D, residential
    ];
    RoadGraph {
        nodes,
        signals: Vec::new(),
        ways,
        offsets: vec![0, 2, 3, 3],
        edges,
    }
}

/// Two equal-cost paths to the same point: one straight, one with a bend.
fn straight_vs_bend() -> RoadGraph {
    let nodes = vec![
        node(1, 31.2300, 121.4700), // A
        node(2, 31.2330, 121.4730), // bend point, north-east
        node(3, 31.2300, 121.4820), // D
        node(4, 31.2300, 121.4760), // straight point, due east
    ];
    let ways = vec![
        WayInfo {
            name: "直路".to_owned(),
            highway: "residential".to_owned(),
         ..WayInfo::default() },
        WayInfo {
            name: "弯路".to_owned(),
            highway: "residential".to_owned(),
         ..WayInfo::default() },
    ];
    let edges = vec![
        edge(3, 600.0, 30, 0), // 0: A -> straight point
        edge(1, 600.0, 30, 1), // 1: A -> bend point
        edge(2, 600.0, 30, 1), // 2: bend point -> D (sharp turn in)
        edge(2, 600.0, 30, 0), // 3: straight point -> D
    ];
    RoadGraph {
        nodes,
        signals: Vec::new(),
        ways,
        offsets: vec![0, 2, 3, 3, 4],
        edges,
    }
}

#[test]
fn prefers_the_quiet_street_over_the_arterial_shortcut() {
    let graph = shortcut_vs_detour();
    let origin = LatLng::new(31.2300, 121.4700);
    let destination = LatLng::new(31.2300, 121.4800);

    let (_, plain) = graph
        .route_report(origin, destination, &neutral_profile(), None)
        .expect("plain route");
    assert!(
        plain.arterial_share > 0.95,
        "without weights the arterial wins: {}",
        plain.arterial_share
    );

    let (_, motorcycle) = graph
        .route_report(
            origin,
            destination,
            &RouteProfile::motorcycle_default(),
            None,
        )
        .expect("motorcycle route");
    assert!(
        motorcycle.arterial_share < 0.05,
        "profile must avoid the arterial: {}",
        motorcycle.arterial_share
    );
    assert!(
        motorcycle.side_street_share > 0.95,
        "profile must use the quiet street: {}",
        motorcycle.side_street_share
    );
    assert!(
        motorcycle.distance_m > plain.distance_m,
        "the quiet route is allowed to be longer: {} vs {}",
        motorcycle.distance_m,
        plain.distance_m
    );
    assert!(
        motorcycle.duration_s > plain.duration_s,
        "and slower: {} vs {}",
        motorcycle.duration_s,
        plain.duration_s
    );
}

#[test]
fn turn_penalty_breaks_the_tie_towards_the_straight_route() {
    let graph = straight_vs_bend();
    let (plan, metrics) = graph
        .route_report(
            LatLng::new(31.2300, 121.4700),
            LatLng::new(31.2300, 121.4820),
            &RouteProfile::motorcycle_default(),
            None,
        )
        .expect("route");

    // The bend sits at 31.2330; the straight option keeps latitude at 31.2300.
    let stays_south = plan
        .points
        .iter()
        .all(|point| (point.latitude - 31.2300).abs() < 1e-6);
    assert!(
        stays_south,
        "the router should take the straight path, points: {:?}",
        plan.points
    );
    assert_eq!(metrics.turn_count, 0, "the straight path has no turns");
}

#[test]
fn metrics_do_not_claim_signal_coverage_the_graph_lacks() {
    let graph = shortcut_vs_detour();
    let (_, metrics) = graph
        .route_report(
            LatLng::new(31.2300, 121.4700),
            LatLng::new(31.2300, 121.4800),
            &RouteProfile::motorcycle_default(),
            None,
        )
        .expect("route");
    assert_eq!(
        metrics.signals_per_km, None,
        "a graph without signal nodes must report unknown, not zero"
    );
}

/// Two near-equal paths: the shorter one passes a signalised junction.
fn signal_vs_detour(south_signalled: bool) -> RoadGraph {
    let nodes = vec![
        node(1, 31.2300, 121.4700), // A
        node(2, 31.2300, 121.4750), // south midpoint (the light)
        node(3, 31.2300, 121.4800), // D
        node(4, 31.2320, 121.4750), // north midpoint
    ];
    let ways = vec![
        WayInfo {
            name: "南线".to_owned(),
            highway: "residential".to_owned(),
         ..WayInfo::default() },
        WayInfo {
            name: "北线".to_owned(),
            highway: "residential".to_owned(),
         ..WayInfo::default() },
    ];
    // CSR order: node 0 owns 0-1, node 1 owns 2, node 2 owns none, node 3 owns 3.
    let edges = vec![
        edge(1, 700.0, 30, 0),  // 0: A -> south midpoint
        edge(3, 725.0, 30, 1),  // 1: A -> north midpoint
        edge(2, 700.0, 30, 0),  // 2: south midpoint -> D
        edge(2, 725.0, 30, 1),  // 3: north midpoint -> D
    ];
    RoadGraph {
        nodes,
        signals: vec![false, south_signalled, false, false],
        ways,
        offsets: vec![0, 2, 3, 3, 4],
        edges,
    }
}

#[test]
fn a_signal_makes_the_shortcut_unattractive() {
    let origin = LatLng::new(31.2300, 121.4700);
    let destination = LatLng::new(31.2300, 121.4800);
    let profile = RouteProfile::motorcycle_default();

    // Without a light the shorter south line wins.
    let clear = signal_vs_detour(false);
    let (south_plan, _) = clear
        .route_report(origin, destination, &profile, None)
        .expect("route");
    assert!(
        south_plan
            .points
            .iter()
            .any(|point| (point.latitude - 31.2300).abs() < 1e-6 && (point.longitude - 121.4750).abs() < 1e-6),
        "the short south line should win when it is clear"
    );

    // Adding one light flips the choice to the slightly longer north line.
    let lit = signal_vs_detour(true);
    let (lit_plan, metrics) = lit
        .route_report(origin, destination, &profile, None)
        .expect("route");
    assert!(
        lit_plan
            .points
            .iter()
            .any(|point| (point.latitude - 31.2320).abs() < 1e-6),
        "the signalised shortcut must be avoided"
    );
    assert_eq!(
        metrics.signals_per_km,
        Some(0.0),
        "the chosen route crosses no light, and the graph knows about lights"
    );
}

/// Three corridors between A and D: a short gravel line, a slightly longer
/// paved residential line, and a wide arterial.
fn surface_vs_lanes() -> RoadGraph {
    let nodes = vec![
        node(1, 31.2300, 121.4700), // A
        node(2, 31.2300, 121.4730), // gravel midpoint
        node(3, 31.2315, 121.4740), // paved midpoint
        node(4, 31.2300, 121.4800), // D
        node(5, 31.2330, 121.4735), // wide arterial midpoint
    ];
    let ways = vec![
        WayInfo {
            name: "碎石小路".to_owned(),
            highway: "residential".to_owned(),
            lanes: 1,
            surface: Surface::Unpaved,
        },
        WayInfo {
            name: "铺装小路".to_owned(),
            highway: "residential".to_owned(),
            lanes: 1,
            surface: Surface::Paved,
        },
        WayInfo {
            name: "八车道大路".to_owned(),
            highway: "primary".to_owned(),
            lanes: 8,
            surface: Surface::Paved,
        },
    ];
    let edges = vec![
        edge(1, 600.0, 30, 0),  // 0: A -> gravel mid
        edge(2, 950.0, 30, 1),  // 1: A -> paved mid
        edge(4, 725.0, 30, 2),  // 2: A -> arterial mid
        edge(3, 725.0, 30, 0),  // 3: gravel mid -> D
        edge(3, 950.0, 30, 1),  // 4: paved mid -> D
        edge(3, 725.0, 30, 2),  // 5: arterial mid -> D
    ];
    RoadGraph {
        nodes,
        signals: Vec::new(),
        ways,
        offsets: vec![0, 3, 4, 5, 5, 6],
        edges,
    }
}

#[test]
fn surface_and_lane_factors_change_the_choice() {
    let graph = surface_vs_lanes();
    let origin = LatLng::new(31.2300, 121.4700);
    let destination = LatLng::new(31.2300, 121.4800);

    let passes_near = |plan: &motu_routing::RoutePlan, lat: f64| {
        plan.points
            .iter()
            .any(|point| (point.latitude - lat).abs() < 1e-6)
    };

    // Neutral profile: the shortest line wins, gravel or not.
    let (plain, _) = graph
        .route_report(origin, destination, &neutral_profile(), None)
        .expect("plain route");
    assert!(
        passes_near(&plain, 31.2300) && !passes_near(&plain, 31.2315),
        "without factors the short gravel line wins"
    );

    // Motorcycle profile: unpaved costs 1.6× and the eight-lane arterial 1.36×
    // on top of its class weight, so the slightly longer paved street wins.
    let (paved, _) = graph
        .route_report(origin, destination, &RouteProfile::motorcycle_default(), None)
        .expect("motorcycle route");
    assert!(
        passes_near(&paved, 31.2315),
        "the paved residential corridor must win, points: {:?}",
        paved.points
    );
}
