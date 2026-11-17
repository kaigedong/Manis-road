//! Compare offline routes against a locally captured official baseline.
//!
//! ```text
//! motu-parity compare --graph var/offline/graph-od.bin \
//!   --baseline var/offline/baseline-calpath.json [--hour 8] [--prior p.json] \
//!   [--avoid a.json] [--report var/offline/parity-report.md]
//! ```
//!
//! Exit code is non-zero when any route exceeds the detour budget, so CI and the
//! data-preparation step can gate on it.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use motu_core::LatLng;
use motu_routing::{
    Baseline, ParityReport, RoadGraph, RouteContext, RouteProfile, TrafficPrior, parity::compare,
};

fn main() -> ExitCode {
    match run() {
        Ok(ok) => {
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("motu-parity: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or_else(usage)?;
    if command != "compare" {
        return Err(format!("unknown command {command}\n{}", usage()));
    }

    let mut graph_path: Option<PathBuf> = None;
    let mut baseline_path: Option<PathBuf> = None;
    let mut prior_path: Option<PathBuf> = None;
    let mut avoid_path: Option<PathBuf> = None;
    let mut report_path: Option<PathBuf> = None;
    let mut hour: u8 = 12;

    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--graph" => graph_path = Some(PathBuf::from(value()?)),
            "--baseline" => baseline_path = Some(PathBuf::from(value()?)),
            "--prior" => prior_path = Some(PathBuf::from(value()?)),
            "--avoid" => avoid_path = Some(PathBuf::from(value()?)),
            "--report" => report_path = Some(PathBuf::from(value()?)),
            "--hour" => {
                let raw = value()?;
                hour = raw
                    .parse()
                    .map_err(|error| format!("bad hour {raw}: {error}"))?;
                if hour > 23 {
                    return Err(format!("hour must be 0..=23, got {hour}"));
                }
            }
            other => return Err(format!("unknown flag {other}\n{}", usage())),
        }
    }

    let graph_path = graph_path.ok_or("--graph is required")?;
    let baseline_path = baseline_path.ok_or("--baseline is required")?;

    let graph = RoadGraph::load(&graph_path).map_err(|error| error.to_string())?;
    let baseline = Baseline::load(&baseline_path).map_err(|error| error.to_string())?;
    let prior = match prior_path {
        Some(path) => Some(TrafficPrior::load(&path).map_err(|error| error.to_string())?),
        None => None,
    };
    let avoid = match avoid_path {
        Some(path) => load_avoid(&path)?,
        None => Vec::new(),
    };
    let context = RouteContext {
        prior: prior.as_ref(),
        hour,
        avoid: &avoid,
    };
    let profile = RouteProfile::motorcycle_default();

    let report: ParityReport = compare(&graph, &baseline, &profile, &context)
        .map_err(|error| error.to_string())?;
    let markdown = report.to_markdown();
    if let Some(path) = report_path.as_ref() {
        std::fs::write(path, &markdown)
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    print!("{markdown}");
    Ok(report.ok())
}

/// `[[lat, lon], ...]` per polygon.
fn load_avoid(path: &Path) -> Result<Vec<Vec<LatLng>>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let raw: Vec<Vec<[f64; 2]>> =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(raw
        .into_iter()
        .map(|ring| {
            ring.into_iter()
                .map(|[lat, lon]| LatLng::new(lat, lon))
                .collect()
        })
        .collect())
}

fn usage() -> String {
    "usage: motu-parity compare --graph <graph.bin> --baseline <baseline.json>
                           [--prior <traffic-prior.json>] [--hour 0..23]
                           [--avoid <avoid.json>] [--report <out.md>]"
        .to_owned()
}
