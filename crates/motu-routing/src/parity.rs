//! Parity against the official baseline.
//!
//! The goal does not ask "does our route look nice", it asks "how far is it from
//! the official one, and is that within 20 %". This module turns that into a
//! check: load a baseline file captured from the official `calPath` API during
//! the preparation phase, plan the same origin/destination offline, and compare.
//!
//! The baseline file stays **local and out of git** (it contains data derived
//! from the vendor's routing), so a missing file is a skip, never a silent pass.

use std::path::Path;

use motu_core::LatLng;
use serde::{Deserialize, Serialize};

use crate::error::{Result, RoutingError};
use crate::graph::RoadGraph;
use crate::profile::RouteProfile;
use crate::replan::route_overlap;
use crate::route::RouteContext;

/// A route captured from the official engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineRoute {
    pub name: String,
    pub origin: [f64; 2],
    pub destination: [f64; 2],
    pub distance_m: f64,
    #[serde(default)]
    pub duration_s: f64,
    /// Official polyline, when captured. Missing points disable the overlap
    /// metric but not the distance check.
    #[serde(default)]
    pub points: Vec<[f64; 2]>,
}

/// The baseline file itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Baseline {
    /// Where the numbers came from, e.g. `calPath`.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub generated_at: Option<String>,
    pub routes: Vec<BaselineRoute>,
}

impl Baseline {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .map_err(|error| RoutingError::Osm(format!("{}: {error}", path.display())))?;
        let baseline: Self = serde_json::from_slice(&bytes)
            .map_err(|error| RoutingError::Osm(format!("{}: {error}", path.display())))?;
        if baseline.routes.is_empty() {
            return Err(RoutingError::InvalidGraph(
                "baseline contains no routes".to_owned(),
            ));
        }
        for route in &baseline.routes {
            if !(route.distance_m.is_finite() && route.distance_m > 1.0) {
                return Err(RoutingError::InvalidGraph(format!(
                    "baseline route {} has an invalid distance",
                    route.name
                )));
            }
        }
        Ok(baseline)
    }
}

/// Detour budget from the goal: our route may be at most 20 % longer.
pub const MAX_DETOUR_PCT: f64 = 20.0;

/// One route's verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteComparison {
    pub name: String,
    pub baseline_km: f64,
    pub ours_km: f64,
    /// Positive means we are longer than the baseline.
    pub detour_pct: f64,
    /// Share of our route covered by the baseline polyline, when available.
    pub overlap: Option<f64>,
    pub arterial_share: f64,
    pub side_street_share: f64,
    pub signals_per_km: Option<f64>,
    pub failures: Vec<String>,
}

impl RouteComparison {
    #[must_use]
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Whole-run result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParityReport {
    pub source: String,
    pub routes: Vec<RouteComparison>,
    pub median_detour_pct: f64,
    pub passed: usize,
    pub failed: usize,
    /// Routes the graph could not plan at all.
    pub unroutable: Vec<String>,
}

impl ParityReport {
    #[must_use]
    pub fn ok(&self) -> bool {
        self.failed == 0 && self.unroutable.is_empty()
    }

    /// Markdown table for the report file.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "# 基准对比\n");
        let source = if self.source.is_empty() { "(未标注)" } else { self.source.as_str() };
        let _ = writeln!(out, "baseline source: {source}  ");
        let _ = writeln!(
            out,
            "通过 {}/{}，中位绕行 {:+.1}%（上限 +{MAX_DETOUR_PCT:.0}%）\n",
            self.passed,
            self.routes.len(),
            self.median_detour_pct
        );
        let _ = writeln!(out, "| 路线 | 基准 km | 本机 km | 绕行 | 重合度 | 干道 | 支路 | 灯/km | 结论 |");
        let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|");
        for route in &self.routes {
            let _ = writeln!(
                out,
                "| {} | {:.2} | {:.2} | {:+.1}% | {} | {:.0}% | {:.0}% | {} | {} |",
                route.name,
                route.baseline_km,
                route.ours_km,
                route.detour_pct,
                route
                    .overlap
                    .map_or("—".to_owned(), |value| format!("{value:.2}")),
                percent(route.arterial_share),
                percent(route.side_street_share),
                route
                    .signals_per_km
                    .map_or("—".to_owned(), |value| format!("{value:.1}")),
                if route.passed() {
                    "✅".to_owned()
                } else {
                    route.failures.join("；")
                }
            );
        }
        if !self.unroutable.is_empty() {
            let _ = writeln!(out, "\n**无法规划**：{}", self.unroutable.join("、"));
        }
        out
    }
}

/// Percent for display, with `-0` collapsed to `0`.
fn percent(share: f64) -> f64 {
    let value = share * 100.0;
    if value.abs() < 0.5 { 0.0 } else { value }
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        values[middle - 1] + (values[middle] - values[middle - 1]) / 2.0
    } else {
        values[middle]
    }
}

/// Plan every baseline route offline and compare.
pub fn compare(
    graph: &RoadGraph,
    baseline: &Baseline,
    profile: &RouteProfile,
    context: &RouteContext<'_>,
) -> Result<ParityReport> {
    let mut routes = Vec::with_capacity(baseline.routes.len());
    let mut unroutable = Vec::new();
    let mut detours = Vec::new();

    for route in &baseline.routes {
        let origin = LatLng::new(route.origin[0], route.origin[1]);
        let destination = LatLng::new(route.destination[0], route.destination[1]);
        let Ok((plan, metrics)) =
            graph.route_report_with_context(origin, destination, profile, None, context)
        else {
            unroutable.push(route.name.clone());
            continue;
        };
        let detour_pct = (plan.distance_m - route.distance_m) / route.distance_m * 100.0;
        detours.push(detour_pct);

        let mut failures = Vec::new();
        if detour_pct > MAX_DETOUR_PCT {
            failures.push(format!("绕行 {detour_pct:+.1}% 超过 +{MAX_DETOUR_PCT:.0}%"));
        }
        let overlap = if route.points.len() >= 2 {
            let line: Vec<LatLng> = route
                .points
                .iter()
                .map(|point| LatLng::new(point[0], point[1]))
                .collect();
            Some(route_overlap(&plan.points, &line, 50.0))
        } else {
            None
        };

        routes.push(RouteComparison {
            name: route.name.clone(),
            baseline_km: route.distance_m / 1000.0,
            ours_km: plan.distance_m / 1000.0,
            detour_pct,
            overlap,
            arterial_share: metrics.arterial_share,
            side_street_share: metrics.side_street_share,
            signals_per_km: metrics.signals_per_km,
            failures,
        });
    }

    let passed = routes.iter().filter(|route| route.passed()).count();
    let failed = routes.len() - passed;
    Ok(ParityReport {
        source: baseline.source.clone(),
        median_detour_pct: median(&mut detours),
        passed,
        failed,
        unroutable,
        routes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline(distance_m: f64, points: Vec<[f64; 2]>) -> Baseline {
        Baseline {
            source: "calPath".to_owned(),
            generated_at: None,
            routes: vec![BaselineRoute {
                name: "测试线".to_owned(),
                origin: [31.2300, 121.4700],
                destination: [31.2300, 121.4800],
                distance_m,
                duration_s: 600.0,
                points,
            }],
        }
    }

    #[test]
    fn rejects_empty_or_invalid_baselines() {
        let empty = Baseline {
            source: String::new(),
            generated_at: None,
            routes: Vec::new(),
        };
        let path = std::env::temp_dir().join("motu-baseline-empty.json");
        std::fs::write(&path, serde_json::to_vec(&empty).expect("json")).expect("write");
        assert!(Baseline::load(&path).is_err());

        let bad = baseline(0.0, Vec::new());
        std::fs::write(&path, serde_json::to_vec(&bad).expect("json")).expect("write");
        assert!(Baseline::load(&path).is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn baseline_roundtrips_from_json() {
        let file = baseline(1500.0, vec![[31.2300, 121.4700], [31.2300, 121.4800]]);
        let path = std::env::temp_dir().join("motu-baseline-ok.json");
        std::fs::write(&path, serde_json::to_vec(&file).expect("json")).expect("write");
        let loaded = Baseline::load(&path).expect("load");
        assert_eq!(loaded.routes.len(), 1);
        assert_eq!(loaded.routes[0].points.len(), 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn median_of_even_and_odd_lengths() {
        assert!((median(&mut [3.0, 1.0, 2.0]) - 2.0).abs() < 1e-9);
        assert!((median(&mut [4.0, 1.0, 3.0, 2.0]) - 2.5).abs() < 1e-9);
        assert_eq!(median(&mut []), 0.0);
    }
}
