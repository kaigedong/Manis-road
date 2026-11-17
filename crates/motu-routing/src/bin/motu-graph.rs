//! Build, inspect and query the offline road graph.
//!
//! ```text
//! motu-graph build var/offline/osm-central.json var/offline/graph.bin
//! motu-graph info  var/offline/graph.bin
//! motu-graph route var/offline/graph.bin 31.2304,121.4737 31.2397,121.4998
//! motu-graph metrics var/offline/graph.bin 31.2304,121.4737 31.2397,121.4998 [--plain] [--primary 2.4]
//! ```

use std::{path::Path, process::ExitCode};

use motu_core::LatLng;
use motu_routing::{RoadGraph, RouteProfile, build_from_osm_json};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("motu-graph: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or_else(usage)?;
    match command.as_str() {
        "build" => {
            let input = args.next().ok_or_else(usage)?;
            let output = args.next().ok_or_else(usage)?;
            println!("reading {input} ...");
            let bytes = std::fs::read(&input).map_err(|error| format!("{input}: {error}"))?;
            let (graph, report) = build_from_osm_json(&bytes).map_err(|error| error.to_string())?;
            graph
                .save(Path::new(&output))
                .map_err(|error| error.to_string())?;
            println!(
                "nodes {}/{}  ways {}/{}  edges {}  -> {output}",
                report.nodes_used,
                report.nodes_total,
                report.ways_used,
                report.ways_total,
                report.edges
            );
        }
        "info" => {
            let input = args.next().ok_or_else(usage)?;
            let graph = RoadGraph::load(Path::new(&input)).map_err(|error| error.to_string())?;
            let bounds = graph.bounds();
            println!(
                "nodes {}  edges {}  ways {}",
                graph.node_count(),
                graph.edge_count(),
                graph.ways.len()
            );
            if let Some((south, west, north, east)) = bounds {
                println!("bounds {south:.5},{west:.5} .. {north:.5},{east:.5}");
            }
        }
        "route" => {
            let input = args.next().ok_or_else(usage)?;
            let origin = parse_point(&args.next().ok_or_else(usage)?)?;
            let destination = parse_point(&args.next().ok_or_else(usage)?)?;
            let graph = RoadGraph::load(Path::new(&input)).map_err(|error| error.to_string())?;
            let plan = graph
                .route(origin, destination)
                .map_err(|error| error.to_string())?;
            let json = serde_json::to_string_pretty(&plan).map_err(|error| error.to_string())?;
            println!("{json}");
        }
        "metrics" => {
            let input = args.next().ok_or_else(usage)?;
            let origin = parse_point(&args.next().ok_or_else(usage)?)?;
            let destination = parse_point(&args.next().ok_or_else(usage)?)?;
            let mut force_neutral = false;
            let mut profile = RouteProfile::motorcycle_default();
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--plain" => force_neutral = true,
                    "--trunk" => profile.classes.trunk = parse_weight(args.next())?,
                    "--primary" => profile.classes.primary = parse_weight(args.next())?,
                    "--secondary" => profile.classes.secondary = parse_weight(args.next())?,
                    "--tertiary" => profile.classes.tertiary = parse_weight(args.next())?,
                    "--residential" => profile.classes.residential = parse_weight(args.next())?,
                    other => return Err(format!("unknown flag {other}")),
                }
            }
            if force_neutral {
                profile = neutral_profile();
            }
            let neutral = profile.classes.primary <= 1.0 && profile.classes.secondary <= 1.0;
            let graph = RoadGraph::load(Path::new(&input)).map_err(|error| error.to_string())?;
            let (plan, metrics) = graph
                .route_report(origin, destination, &profile, None)
                .map_err(|error| error.to_string())?;
            let report = serde_json::json!({
                "origin": { "lat": origin.latitude, "lon": origin.longitude },
                "destination": { "lat": destination.latitude, "lon": destination.longitude },
                "distanceKm": plan.distance_m / 1000.0,
                "durationMin": plan.duration_s / 60.0,
                "points": plan.points.len(),
                "steps": plan.steps.len(),
                "profile": if neutral { "plain" } else { "motorcycle" },
                "metrics": metrics,
            });
            let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
            println!("{json}");
        }
        other => return Err(format!("unknown command {other}\n{}", usage())),
    }
    Ok(())
}

/// The pre-profile cost model, for before/after comparison only.
fn neutral_profile() -> RouteProfile {
    RouteProfile {
        classes: motu_routing::ClassWeights {
            trunk: 1.0,
            primary: 1.0,
            secondary: 1.0,
            tertiary: 1.0,
            residential: 1.0,
            other: 1.0,
        },
        turns: motu_routing::TurnPenalties {
            straight_max_deg: 0.0,
            slight_max_deg: 0.0,
            turn_max_deg: 0.0,
            straight_s: 0.0,
            slight_s: 0.0,
            turn_s: 0.0,
            sharp_s: 0.0,
            u_turn_s: 0.0,
        },
    }
}

fn parse_weight(value: Option<String>) -> Result<f64, String> {
    let raw = value.ok_or_else(|| "missing weight value".to_owned())?;
    raw.parse::<f64>()
        .map_err(|error| format!("bad weight {raw}: {error}"))
}

fn parse_point(value: &str) -> Result<LatLng, String> {
    let (lat, lon) = value
        .split_once(',')
        .ok_or_else(|| format!("expected lat,lon, got {value}"))?;
    let lat = lat
        .trim()
        .parse::<f64>()
        .map_err(|error| format!("bad latitude: {error}"))?;
    let lon = lon
        .trim()
        .parse::<f64>()
        .map_err(|error| format!("bad longitude: {error}"))?;
    let point = LatLng::new(lat, lon);
    if !point.is_valid() {
        return Err(format!("coordinate out of range: {value}"));
    }
    Ok(point)
}

fn usage() -> String {
    "usage:
  motu-graph build <osm.json> <graph.bin>
  motu-graph info  <graph.bin>
  motu-graph route <graph.bin> <lat,lon> <lat,lon>"
        .to_owned()
}
