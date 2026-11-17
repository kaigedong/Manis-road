#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    reason = "sample counts, second gaps and metre distances are tiny compared to f64's mantissa"
)]

//! Turn recorded rides into a measured traffic prior.
//!
//! The goal allows two sources for the time-of-day prior: a free traffic API, or
//! **traces recorded on this device**. This module is the second one: it takes
//! GPS fixes, works out how fast the rider actually moved on each road class at
//! each hour, and expresses that as `free-flow speed / observed speed` — the same
//! multiplier the router already applies.
//!
//! Two rules keep it honest:
//!
//! * a bucket without enough samples stays at `1.0` and is reported as skipped,
//!   never silently invented;
//! * factors are clamped, so a single red light at a junction cannot turn one
//!   ride into a "traffic jam" prior for the whole class.

use std::collections::BTreeMap;
use std::path::Path;

use motu_core::LatLng;
use serde::{Deserialize, Serialize};

use crate::error::{Result, RoutingError};
use crate::graph::{RoadGraph, haversine_m};
use crate::traffic::{ClassFactors, PriorSource, TimeBucket, TrafficPrior};

/// `(factor, observed km/h, free-flow km/h)` samples per cell and per bucket.
type Samples = Vec<(f64, f64, f64)>;

/// One GPS fix.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TracePoint {
    /// Unix seconds.
    pub t: i64,
    pub lat: f64,
    pub lon: f64,
    /// Reported speed, when the device provides one.
    #[serde(default)]
    pub speed_mps: Option<f64>,
}

/// A recording is only useful if the fixes are close enough together.
pub const MAX_GAP_SECONDS: i64 = 15;
/// Samples needed before a (class, hour) cell is trusted.
pub const MIN_SAMPLES_PER_CELL: usize = 20;
/// How far a fix may be from the road network.
pub const MAX_SNAP_M: f64 = 60.0;
/// Clamp so one bad sample cannot move a whole class.
pub const MIN_FACTOR: f64 = 0.7;
pub const MAX_FACTOR: f64 = 4.0;
/// Shanghai is UTC+8; traces carry absolute timestamps.
const HOUR_OFFSET: i64 = 8;

/// Per-hour, per-class sample counts and factors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriorBuildReport {
    pub points: usize,
    pub usable_segments: usize,
    /// Cells that reached [`MIN_SAMPLES_PER_CELL`].
    pub accepted_cells: Vec<CellReport>,
    /// Cells that were dropped, with the reason.
    pub skipped_cells: Vec<CellReport>,
    pub source: PriorSource,
    /// Per-hop verdicts, in trace order.
    pub hops: Vec<HopReport>,
}

/// One (hour, class) cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellReport {
    pub hour: u8,
    pub class: String,
    pub samples: usize,
    pub factor: f64,
    pub observed_kmh: f64,
    pub free_flow_kmh: f64,
}

impl PriorBuildReport {
    /// Markdown table for the data-prep log.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "# 实测时段先验\n");
        let _ = writeln!(
            out,
            "轨迹点 {}，可用路段 {}，接受 {} 个格子，丢弃 {} 个\n",
            self.points,
            self.usable_segments,
            self.accepted_cells.len(),
            self.skipped_cells.len()
        );
        let _ = writeln!(out, "| 小时 | 等级 | 样本 | 实测 km/h | 自由流 km/h | 系数 |");
        let _ = writeln!(out, "|---|---|---|---|---|---|");
        for cell in &self.accepted_cells {
            let _ = writeln!(
                out,
                "| {:.0} | {} | {} | {:.1} | {:.1} | {:.2} |",
                cell.hour, cell.class, cell.samples, cell.observed_kmh, cell.free_flow_kmh, cell.factor
            );
        }
        if !self.skipped_cells.is_empty() {
            let _ = writeln!(out, "\n被丢弃（样本不足，保持 1.0）：");
            for cell in &self.skipped_cells {
                let _ = writeln!(
                    out,
                    "- 小时 {} / {}：{} 个样本（需要 {}）",
                    cell.hour, cell.class, cell.samples, MIN_SAMPLES_PER_CELL
                );
            }
        }
        out
    }
}

/// Result of a build: the prior plus what was accepted.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltPrior {
    pub prior: TrafficPrior,
    pub report: PriorBuildReport,
}

/// Load a JSONL trace file.
///
/// # Errors
/// Returns [`RoutingError::Osm`] when the file cannot be read or a line is not a
/// valid trace point.
pub fn load_trace(path: &Path) -> Result<Vec<TracePoint>> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| RoutingError::Osm(format!("{}: {error}", path.display())))?;
    let mut points = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let point: TracePoint = serde_json::from_str(line).map_err(|error| {
            RoutingError::Osm(format!("{}:{}: {error}", path.display(), number + 1))
        })?;
        points.push(point);
    }
    if points.is_empty() {
        return Err(RoutingError::Osm(format!(
            "{}: no trace points",
            path.display()
        )));
    }
    Ok(points)
}

fn class_name(highway: &str) -> &'static str {
    match highway {
        "trunk" | "trunk_link" => "trunk",
        "primary" | "primary_link" => "primary",
        "secondary" | "secondary_link" => "secondary",
        "tertiary" | "tertiary_link" => "tertiary",
        "residential" | "unclassified" | "living_street" | "service" => "residential",
        _ => "other",
    }
}

fn set_factor(factors: &mut ClassFactors, class: &str, value: f64) {
    match class {
        "trunk" => factors.trunk = value,
        "primary" => factors.primary = value,
        "secondary" => factors.secondary = value,
        "tertiary" => factors.tertiary = value,
        "residential" => factors.residential = value,
        _ => factors.other = value,
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        values[middle - 1] + (values[middle] - values[middle - 1]) / 2.0
    } else {
        values[middle]
    }
}

/// Bearing from `a` to `b`, degrees clockwise from north.
fn bearing_deg(a: LatLng, b: LatLng) -> f64 {
    let phi1 = a.latitude.to_radians();
    let phi2 = b.latitude.to_radians();
    let delta_lon = (b.longitude - a.longitude).to_radians();
    let y = delta_lon.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * delta_lon.cos();
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

fn angle_between(a: f64, b: f64) -> f64 {
    let diff = (a - b).rem_euclid(360.0);
    if diff > 180.0 { 360.0 - diff } else { diff }
}

/// Why a hop was dropped. Surfaced by `motu-prior --explain` so a rider can see
/// whether their recording is useless or the matcher is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HopRejection {
    TimeGap,
    TooShort,
    SpeedOutOfRange,
    NoNode,
    TooFarFromRoad,
    NoMatchingDirection,
    LongerThanEdge,
}

/// One hop's verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HopReport {
    pub index: usize,
    pub observed_kmh: f64,
    pub class: Option<String>,
    pub rejection: Option<HopRejection>,
}

/// Find the road segment a trace hop happened on.
///
/// Snapping to the nearest *node* is not enough: at an intersection the rider may
/// leave on any of the outgoing edges. The edge whose direction matches the hop
/// wins, which is enough to attribute speed to the right class.
fn classify_hop_detailed(
    graph: &RoadGraph,
    from: TracePoint,
    to: TracePoint,
) -> std::result::Result<(String, f64, f64), HopRejection> {
    let mid = LatLng::new(from.lat.midpoint(to.lat), from.lon.midpoint(to.lon));
    let (node, distance) = graph.nearest_node(mid).ok_or(HopRejection::NoNode)?;
    if distance > MAX_SNAP_M {
        return Err(HopRejection::TooFarFromRoad);
    }
    let index = usize::try_from(node).map_err(|_| HopRejection::NoNode)?;
    let here = graph.nodes.get(index).ok_or(HopRejection::NoNode)?;
    let origin = LatLng::new(here.lat, here.lon);
    let hop_bearing = bearing_deg(
        LatLng::new(from.lat, from.lon),
        LatLng::new(to.lat, to.lon),
    );

    let first = usize::try_from(*graph.offsets.get(index).ok_or(HopRejection::NoNode)?)
        .map_err(|_| HopRejection::NoNode)?;
    let last = usize::try_from(*graph.offsets.get(index + 1).ok_or(HopRejection::NoNode)?)
        .map_err(|_| HopRejection::NoNode)?;
    let mut best: Option<(f64, &crate::graph::Edge)> = None;
    for edge in graph
        .edges
        .get(first..last)
        .ok_or(HopRejection::NoNode)?
    {
        let target = graph
            .nodes
            .get(usize::try_from(edge.to).map_err(|_| HopRejection::NoNode)?)
            .ok_or(HopRejection::NoNode)?;
        let edge_bearing = bearing_deg(origin, LatLng::new(target.lat, target.lon));
        let delta = angle_between(hop_bearing, edge_bearing);
        if delta > 60.0 {
            continue;
        }
        if best.is_none_or(|(current, _)| delta < current) {
            best = Some((delta, edge));
        }
    }
    let (_, edge) = best.ok_or(HopRejection::NoMatchingDirection)?;
    let way = graph
        .ways
        .get(usize::try_from(edge.way).map_err(|_| HopRejection::NoNode)?)
        .ok_or(HopRejection::NoNode)?;
    Ok((
        class_name(&way.highway).to_owned(),
        f64::from(edge.speed_kmh),
        f64::from(edge.distance_m),
    ))
}

/// Build a measured prior from traces.
///
/// `buckets` defines the time windows to fill; only cells inside those windows are
/// produced, so the output always covers the whole day.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one pass over the trace with the rejection bookkeeping kept together reads better than splitting"
)]
pub fn build_prior(
    graph: &RoadGraph,
    traces: &[TracePoint],
    buckets: &[TimeBucket],
    min_samples: usize,
) -> BuiltPrior {
    // (hour, class) -> (factor, observed km/h, free-flow km/h) samples
    let mut cells: BTreeMap<(u8, String), Samples> = BTreeMap::new();
    let mut usable_segments = 0_usize;
    let mut hops: Vec<HopReport> = Vec::new();

    for (index, pair) in traces.windows(2).enumerate() {
        let [from, to] = [pair[0], pair[1]];
        let gap = to.t - from.t;
        let distance_m = haversine_m(from.lat, from.lon, to.lat, to.lon);
        let observed_mps = to
            .speed_mps
            .filter(|speed| speed.is_finite() && *speed > 0.5)
            .unwrap_or(if gap > 0 {
                distance_m / gap as f64
            } else {
                0.0
            });
        let observed_kmh = observed_mps * 3.6;
        let mut rejection = None;
        if !(1..=MAX_GAP_SECONDS).contains(&gap) {
            rejection = Some(HopRejection::TimeGap);
        } else if distance_m < 1.0 {
            rejection = Some(HopRejection::TooShort);
        } else if !(2.0..=90.0).contains(&observed_kmh) {
            rejection = Some(HopRejection::SpeedOutOfRange);
        }
        let classified = if rejection.is_none() {
            Some(classify_hop_detailed(graph, from, to))
        } else {
            None
        };
        let (class, free_flow_kmh, edge_length_m) = match classified {
            None => {
                hops.push(HopReport {
                    index,
                    observed_kmh,
                    class: None,
                    rejection,
                });
                continue;
            }
            Some(Err(reason)) => {
                hops.push(HopReport {
                    index,
                    observed_kmh,
                    class: None,
                    rejection: Some(reason),
                });
                continue;
            }
            Some(Ok(found)) => found,
        };
        // A hop that covered far more ground than the edge cannot be trusted.
        if distance_m > edge_length_m.max(50.0) * 6.0 {
            hops.push(HopReport {
                index,
                observed_kmh,
                class: None,
                rejection: Some(HopRejection::LongerThanEdge),
            });
            continue;
        }
        hops.push(HopReport {
            index,
            observed_kmh,
            class: Some(class.clone()),
            rejection: None,
        });
        usable_segments += 1;
        let hour = u8::try_from((to.t + HOUR_OFFSET * 3600).rem_euclid(86_400) / 3600).unwrap_or(0);
        let factor = (free_flow_kmh / observed_kmh).clamp(MIN_FACTOR, MAX_FACTOR);
        cells
            .entry((hour, class))
            .or_default()
            .push((factor, observed_kmh, free_flow_kmh));
    }

    // Expected cells: every (class, hour) that a bucket covers.
    let classes = [
        "trunk",
        "primary",
        "secondary",
        "tertiary",
        "residential",
        "other",
    ];
    let mut accepted = Vec::new();
    let mut skipped = Vec::new();
    let mut by_bucket: BTreeMap<(u8, u8, String), Samples> = BTreeMap::new();

    for bucket in buckets {
        for hour in 0..24_u8 {
            if !bucket.contains(hour) {
                continue;
            }
            for class in classes {
                let key = (hour, class.to_owned());
                let samples = cells.get(&key).cloned().unwrap_or_default();
                let count = samples.len();
                if count < min_samples {
                    skipped.push(CellReport {
                        hour,
                        class: class.to_owned(),
                        samples: count,
                        factor: 1.0,
                        observed_kmh: 0.0,
                        free_flow_kmh: 0.0,
                    });
                    continue;
                }
                let mut factors: Vec<f64> = samples.iter().map(|(factor, _, _)| *factor).collect();
                let factor = median(&mut factors);
                let mean_observed = samples.iter().map(|(_, observed, _)| observed).sum::<f64>()
                    / samples.len() as f64;
                let mean_free =
                    samples.iter().map(|(_, _, free)| free).sum::<f64>() / samples.len() as f64;
                accepted.push(CellReport {
                    hour,
                    class: class.to_owned(),
                    samples: count,
                    factor,
                    observed_kmh: mean_observed,
                    free_flow_kmh: mean_free,
                });
                by_bucket
                    .entry((bucket.start_hour, bucket.end_hour, class.to_owned()))
                    .or_default()
                    .push((factor, 0.0, 0.0));
            }
        }
    }

    let out_buckets: Vec<TimeBucket> = buckets
        .iter()
        .map(|bucket| {
            let mut factors = ClassFactors::default();
            for class in classes {
                let key = (bucket.start_hour, bucket.end_hour, class.to_owned());
                if let Some(values) = by_bucket.get(&key) {
                    let mut factors_only: Vec<f64> =
                        values.iter().map(|(factor, _, _)| *factor).collect();
                    if !factors_only.is_empty() {
                        set_factor(&mut factors, class, median(&mut factors_only));
                    }
                }
            }
            TimeBucket {
                label: bucket.label.clone(),
                start_hour: bucket.start_hour,
                end_hour: bucket.end_hour,
                factors,
            }
        })
        .collect();

    BuiltPrior {
        prior: TrafficPrior {
            source: PriorSource::Measured,
            generated_at: Some(now_unix().to_string()),
            buckets: out_buckets,
        },
        report: PriorBuildReport {
            points: traces.len(),
            usable_segments,
            accepted_cells: accepted,
            skipped_cells: skipped,
            source: PriorSource::Measured,
            hops,
        },
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearing_and_angle_wrap_correctly() {
        let a = LatLng::new(31.23, 121.47);
        let b = LatLng::new(31.24, 121.47);
        assert!((bearing_deg(a, b) - 0.0).abs() < 1.0, "north is ~0°");
        assert!((angle_between(350.0, 10.0) - 20.0).abs() < 1e-9);
        assert!((angle_between(10.0, 350.0) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn class_mapping_follows_the_router() {
        assert_eq!(class_name("trunk_link"), "trunk");
        assert_eq!(class_name("living_street"), "residential");
        assert_eq!(class_name("cycleway"), "other");
    }

    #[test]
    fn median_of_odd_and_even_lengths() {
        assert!((median(&mut [3.0, 1.0, 2.0]) - 2.0).abs() < 1e-9);
        assert!((median(&mut [4.0, 1.0, 3.0, 2.0]) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn a_bucket_without_samples_stays_at_one() {
        // No traces at all: the prior must not invent factors.
        let graph = crate::graph::RoadGraph {
            nodes: Vec::new(),
            signals: Vec::new(),
            ways: Vec::new(),
            offsets: vec![0],
            edges: Vec::new(),
        };
        let buckets = vec![TimeBucket {
            label: "夜".to_owned(),
            start_hour: 0,
            end_hour: 6,
            factors: ClassFactors::default(),
        }];
        let built = build_prior(&graph, &[], &buckets, 1);
        assert_eq!(built.prior.source, PriorSource::Measured);
        assert_eq!(built.prior.buckets[0].factors.trunk, 1.0);
        assert_eq!(built.report.accepted_cells.len(), 0);
        assert_eq!(built.report.skipped_cells.len(), 6 * 6);
    }
}
