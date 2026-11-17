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
use motu_routing::{RoadGraph, RouteContext, RouteProfile, TrafficPrior, build_from_osm_json};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("motu-graph: {error}");
            ExitCode::FAILURE
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one flat match over CLI subcommands"
)]
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
            let tagged_lanes = graph.ways.iter().filter(|way| way.lanes > 0).count();
            let unpaved = graph
                .ways
                .iter()
                .filter(|way| way.surface == motu_routing::graph::Surface::Unpaved)
                .count();
            let paved = graph
                .ways
                .iter()
                .filter(|way| way.surface == motu_routing::graph::Surface::Paved)
                .count();
            println!(
                "ways lanes-tagged {tagged_lanes}/{}  surface paved {paved} unpaved {unpaved} unknown {}",
                graph.ways.len(),
                graph.ways.len() - paved - unpaved
            );
            if graph.signals.is_empty() {
                println!("signals unknown (graph predates signal data)");
            } else {
                let count = graph.signals.iter().filter(|flag| **flag).count();
                println!("signals {count} of {} nodes", graph.signals.len());
            }
        }
        "route" => {
            let input = args.next().ok_or_else(usage)?;
            let origin = parse_point(&args.next().ok_or_else(usage)?)?;
            let destination = parse_point(&args.next().ok_or_else(usage)?)?;
            let mut hour: Option<u8> = None;
            let mut prior_path: Option<String> = None;
            let mut avoid_path: Option<String> = None;
            let mut cameras_path: Option<String> = None;
            let mut camera_scale = 1.0;
            let mut camera_radius = motu_routing::risk::DEFAULT_CAMERA_RADIUS_M;
            let mut zone_factor = motu_routing::risk::DEFAULT_ZONE_FACTOR;
            let mut count_only = false;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--hour" => hour = Some(parse_hour(args.next())?),
                    "--prior" => prior_path = args.next(),
                    "--avoid" => avoid_path = args.next(),
                    "--cameras" => cameras_path = args.next(),
                    "--camera-radius" => camera_radius = parse_f64(args.next(), "--camera-radius")?,
                    "--zone-factor" => zone_factor = parse_f64(args.next(), "--zone-factor")?,
                    "--count-only" => count_only = true,
                    "--camera-scale" => camera_scale = parse_f64(args.next(), "--camera-scale")?,
                    other => return Err(format!("unknown flag {other}")),
                }
            }
            let avoid = match avoid_path.as_deref() {
                Some(path) => load_avoid(path)?,
                None => Vec::new(),
            };
            let mut cameras = match cameras_path.as_deref() {
                Some(path) => load_cameras(path)?,
                None => Vec::new(),
            };
            if count_only {
                for camera in &mut cameras {
                    camera.seconds = 0.0;
                }
            } else if (camera_scale - 1.0).abs() > 1e-9 {
                for camera in &mut cameras {
                    camera.seconds *= camera_scale;
                }
            }
            let risk = motu_routing::risk::RiskModel::new(
                &cameras,
                camera_radius,
                &avoid,
                zone_factor,
            );
            let prior = match prior_path.as_deref() {
                Some(path) => Some(
                    TrafficPrior::load(Path::new(path)).map_err(|error| error.to_string())?,
                ),
                None => None,
            };
            let context = RouteContext {
                prior: prior.as_ref(),
                hour: hour.unwrap_or(12),
                risk: Some(&risk),
            };
            let graph = RoadGraph::load(Path::new(&input)).map_err(|error| error.to_string())?;
            let plan = graph
                .route_report_with_context(
                    origin,
                    destination,
                    &RouteProfile::motorcycle_default(),
                    None,
                    &context,
                )
                .map(|(plan, _)| plan)
                .map_err(|error| error.to_string())?;
            let json = serde_json::to_string_pretty(&plan).map_err(|error| error.to_string())?;
            println!("{json}");
        }
        "risk-probe" => {
            let cameras_path = args.next().ok_or_else(usage)?;
            let from = parse_point(&args.next().ok_or_else(usage)?)?;
            let to = parse_point(&args.next().ok_or_else(usage)?)?;
            let cameras = load_cameras(&cameras_path)?;
            let risk = motu_routing::risk::RiskModel::new(
                &cameras,
                motu_routing::risk::DEFAULT_CAMERA_RADIUS_M,
                &[],
                motu_routing::risk::DEFAULT_ZONE_FACTOR,
            );
            let base = 30.0_f64;
            let penalty = risk.edge_penalty_s(from, to, base);
            let nearest = cameras
                .iter()
                .map(|camera| {
                    (
                        motu_routing::graph::haversine_m(
                            from.latitude, from.longitude,
                            camera.point.latitude, camera.point.longitude,
                        ),
                        camera.seconds,
                    )
                })
                .fold((f64::MAX, 0.0), |acc, item| if item.0 < acc.0 { item } else { acc });
            println!(
                "cameras={} nearest={:.0}m weight={:.0}s radius={:.0}m penalty={:.1}s",
                risk.cameras.len(),
                nearest.0,
                nearest.1,
                risk.cameras.radius_m(),
                penalty
            );
        }
        "metrics" => {
            let input = args.next().ok_or_else(usage)?;
            let origin = parse_point(&args.next().ok_or_else(usage)?)?;
            let destination = parse_point(&args.next().ok_or_else(usage)?)?;
            let mut force_neutral = false;
            let mut hour: Option<u8> = None;
            let mut prior_path: Option<String> = None;
            let mut avoid_path: Option<String> = None;
            let mut cameras_path: Option<String> = None;
            let mut camera_scale = 1.0;
            let mut camera_radius = motu_routing::risk::DEFAULT_CAMERA_RADIUS_M;
            let mut zone_factor = motu_routing::risk::DEFAULT_ZONE_FACTOR;
            let mut count_only = false;
            let mut profile = RouteProfile::motorcycle_default();
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--plain" => force_neutral = true,
                    "--trunk" => profile.classes.trunk = parse_weight(args.next())?,
                    "--primary" => profile.classes.primary = parse_weight(args.next())?,
                    "--secondary" => profile.classes.secondary = parse_weight(args.next())?,
                    "--tertiary" => profile.classes.tertiary = parse_weight(args.next())?,
                    "--residential" => profile.classes.residential = parse_weight(args.next())?,
                    "--hour" => hour = Some(parse_hour(args.next())?),
                    "--prior" => prior_path = args.next(),
                    "--avoid" => avoid_path = args.next(),
                    "--cameras" => cameras_path = args.next(),
                    "--camera-radius" => camera_radius = parse_f64(args.next(), "--camera-radius")?,
                    "--zone-factor" => zone_factor = parse_f64(args.next(), "--zone-factor")?,
                    "--count-only" => count_only = true,
                    "--camera-scale" => camera_scale = parse_f64(args.next(), "--camera-scale")?,
                    other => return Err(format!("unknown flag {other}")),
                }
            }
            if force_neutral {
                profile = neutral_profile();
            }
            let neutral = profile.classes.primary <= 1.0 && profile.classes.secondary <= 1.0;
            let prior = match prior_path.as_deref() {
                Some(path) => Some(
                    TrafficPrior::load(Path::new(path)).map_err(|error| error.to_string())?,
                ),
                None => None,
            };
            let avoid = match avoid_path.as_deref() {
                Some(path) => load_avoid(path)?,
                None => Vec::new(),
            };
            let mut cameras = match cameras_path.as_deref() {
                Some(path) => load_cameras(path)?,
                None => Vec::new(),
            };
            if count_only {
                for camera in &mut cameras {
                    camera.seconds = 0.0;
                }
            } else if (camera_scale - 1.0).abs() > 1e-9 {
                for camera in &mut cameras {
                    camera.seconds *= camera_scale;
                }
            }
            let risk = motu_routing::risk::RiskModel::new(
                &cameras,
                camera_radius,
                &avoid,
                zone_factor,
            );
            let context = RouteContext {
                prior: prior.as_ref(),
                hour: hour.unwrap_or(12),
                risk: Some(&risk),
            };
            let graph = RoadGraph::load(Path::new(&input)).map_err(|error| error.to_string())?;
            let (plan, metrics) = graph
                .route_report_with_context(origin, destination, &profile, None, &context)
                .map_err(|error| error.to_string())?;
            let report = serde_json::json!({
                "origin": { "lat": origin.latitude, "lon": origin.longitude },
                "destination": { "lat": destination.latitude, "lon": destination.longitude },
                "distanceKm": plan.distance_m / 1000.0,
                "durationMin": plan.duration_s / 60.0,
                "points": plan.points.len(),
                "steps": plan.steps.len(),
                "profile": if neutral { "plain" } else { "motorcycle" },
                "hour": context.hour,
                "priorSource": prior.as_ref().map(|prior| format!("{:?}", prior.source)),
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
        signal_penalty_s: 0.0,
        lane_step: 0.0,
        unpaved_factor: 1.0,
    }
}

/// `[[lat, lon], [lat, lon], ...]` — one polygon per element.
fn parse_f64(value: Option<String>, flag: &str) -> Result<f64, String> {
    let raw = value.ok_or_else(|| format!("{flag} needs a value"))?;
    raw.parse()
        .map_err(|error| format!("bad {flag} {raw}: {error}"))
}

/// `[{ "position": {"latitude", "longitude"}, "tabTypes": [1] }, ...]`
/// or the same wrapped in `{ "cameras": [...] }`.
fn load_cameras(path: &str) -> Result<Vec<motu_routing::risk::CameraRisk>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let raw: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))?;
    let array = raw
        .get("cameras")
        .and_then(serde_json::Value::as_array)
        .or_else(|| raw.as_array())
        .ok_or_else(|| format!("{path}: expected an array of cameras"))?;
    let mut out = Vec::with_capacity(array.len());
    for item in array {
        let position = item.get("position");
        let lat = position
            .and_then(|value| value.get("latitude"))
            .and_then(serde_json::Value::as_f64);
        let lon = position
            .and_then(|value| value.get("longitude"))
            .and_then(serde_json::Value::as_f64);
        let (Some(lat), Some(lon)) = (lat, lon) else {
            continue;
        };
        let tab_type = item
            .get("tabTypes")
            .and_then(serde_json::Value::as_array)
            .and_then(|types| types.first())
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(0);
        out.push(motu_routing::risk::CameraRisk {
            point: LatLng::new(lat, lon),
            seconds: motu_routing::risk::seconds_for_tab_type(tab_type),
            tab_type,
        });
    }
    Ok(out)
}

fn load_avoid(path: &str) -> Result<Vec<Vec<LatLng>>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    let raw: Vec<Vec<[f64; 2]>> =
        serde_json::from_slice(&bytes).map_err(|error| format!("{path}: {error}"))?;
    Ok(raw
        .into_iter()
        .map(|ring| {
            ring.into_iter()
                .map(|[lat, lon]| LatLng::new(lat, lon))
                .collect()
        })
        .collect())
}

fn parse_hour(value: Option<String>) -> Result<u8, String> {
    let raw = value.ok_or_else(|| "missing hour".to_owned())?;
    let hour: u8 = raw.parse().map_err(|error| format!("bad hour {raw}: {error}"))?;
    if hour > 23 {
        return Err(format!("hour must be 0..=23, got {hour}"));
    }
    Ok(hour)
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
