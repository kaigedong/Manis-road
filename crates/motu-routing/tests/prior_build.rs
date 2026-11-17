//! Measured prior from recorded traces: factors, sample thresholds, clamping.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "test coordinates and small counts are exact in f64"
)]

use motu_core::LatLng;
use motu_routing::graph::{Edge, GraphNode, RoadGraph, WayInfo, haversine_m};
use motu_routing::prior_build::{MIN_SAMPLES_PER_CELL, TracePoint, build_prior};
use motu_routing::traffic::{ClassFactors, PriorSource, TimeBucket};

const SIDE: usize = 4;
const STEP_DEG: f64 = 0.001;

fn position(row: usize, col: usize) -> LatLng {
    LatLng::new(
        31.2300 + row as f64 * STEP_DEG,
        121.4700 + col as f64 * STEP_DEG,
    )
}

/// A north-south chain of residential edges at 30 km/h.
fn road() -> RoadGraph {
    let nodes: Vec<GraphNode> = (0..SIDE)
        .map(|row| {
            let point = position(row, 0);
            GraphNode {
                id: row as i64,
                lat: point.latitude,
                lon: point.longitude,
            }
        })
        .collect();
    let ways = vec![WayInfo {
        name: "测试路".to_owned(),
        highway: "residential".to_owned(),
        lanes: 2,
        ..WayInfo::default()
    }];
    let mut edges = Vec::new();
    for row in 0..SIDE - 1 {
        let a = position(row, 0);
        let b = position(row + 1, 0);
        let distance = haversine_m(a.latitude, a.longitude, b.latitude, b.longitude) as f32;
        edges.push(Edge {
            to: (row + 1) as u32,
            distance_m: distance,
            speed_kmh: 30,
            way: 0,
        });
    }
    let mut offsets = vec![0_u64; SIDE];
    for node in 0..SIDE - 1 {
        offsets[node + 1] = offsets[node] + 1;
    }
    RoadGraph {
        nodes,
        signals: Vec::new(),
        edges,
        ways,
        offsets,
    }
}

fn bucket() -> TimeBucket {
    TimeBucket {
        label: "早高峰".to_owned(),
        start_hour: 7,
        end_hour: 10,
        factors: ClassFactors::default(),
    }
}

/// Fixes moving north at `speed_mps`, one every two seconds, starting 08:00 local
/// (00:00 UTC), for `count` hops.
fn trace(speed_mps: f64, count: usize) -> Vec<TracePoint> {
    let mut points = Vec::new();
    // 落在 UTC 午夜 → 本地 08:00，正好在"早高峰"桶里。
    let start = (1_700_000_000_i64 / 86_400) * 86_400;
    let mut travelled = 0.0_f64;
    // 1 degree of latitude ≈ 110_574 m at this latitude.
    let degrees_per_metre = 1.0 / 110_574.0;
    for index in 0..=count {
        points.push(TracePoint {
            t: start + index as i64 * 2,
            lat: 31.2300 + travelled * degrees_per_metre,
            lon: 121.4700,
            speed_mps: Some(speed_mps),
        });
        travelled += speed_mps * 2.0;
    }
    points
}

#[test]
fn slow_traffic_becomes_a_factor_above_one() {
    let graph = road();
    // Free flow is 30 km/h; the ride happened at 15 km/h → factor 2.0.
    let points = trace(15.0 / 3.6, 30);
    let built = build_prior(&graph, &points, &[bucket()], 5);
    assert_eq!(built.prior.source, PriorSource::Measured);
    let factors = built.prior.buckets[0].factors;
    assert!(
        (factors.residential - 2.0).abs() < 0.35,
        "expected ~2.0, got {}",
        factors.residential
    );
    assert!(built.report.usable_segments >= 10, "segments: {}", built.report.usable_segments);
}

#[test]
fn free_flowing_traffic_stays_near_one() {
    let graph = road();
    let points = trace(30.0 / 3.6, 30);
    let built = build_prior(&graph, &points, &[bucket()], 5);
    let factor = built.prior.buckets[0].factors.residential;
    assert!((factor - 1.0).abs() < 0.2, "expected ~1.0, got {factor}");
}

#[test]
fn too_few_samples_leave_the_bucket_at_one() {
    let graph = road();
    let points = trace(15.0 / 3.6, 3); // far fewer than MIN_SAMPLES_PER_CELL
    let built = build_prior(&graph, &points, &[bucket()], MIN_SAMPLES_PER_CELL);
    assert_eq!(
        built.prior.buckets[0].factors.residential, 1.0,
        "an under-sampled cell must not invent a factor"
    );
    assert!(!built.report.skipped_cells.is_empty());
}

#[test]
fn a_hop_with_a_huge_time_gap_is_ignored() {
    let graph = road();
    let mut points = trace(15.0 / 3.6, 10);
    for (index, point) in points.iter_mut().enumerate() {
        // 每个点相隔一小时：远超 MAX_GAP_SECONDS。
        point.t = (1_700_000_000_i64 / 86_400) * 86_400 + index as i64 * 3600;
    }
    let built = build_prior(&graph, &points, &[bucket()], 1);
    assert_eq!(built.report.usable_segments, 0);
    assert_eq!(built.prior.buckets[0].factors.residential, 1.0);
}
