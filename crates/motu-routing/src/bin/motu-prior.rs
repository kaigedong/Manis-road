//! Build a measured time-of-day prior from rides recorded on this device.
//!
//! ```text
//! motu-prior build --graph var/offline/graph-od.bin \
//!   --trace var/traces/ride-1.jsonl --out var/offline/traffic-prior-measured.json
//! ```
//!
//! Exit code is non-zero when the traces produced no trustworthy cell at all, so a
//! data-prep pipeline cannot mistake "no data" for "a prior".

use std::path::PathBuf;
use std::process::ExitCode;

use motu_routing::graph::RoadGraph;
use motu_routing::prior_build::{MIN_SAMPLES_PER_CELL, build_prior, load_trace};
use motu_routing::traffic::{ClassFactors, TimeBucket, TrafficPrior};

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!("motu-prior: traces produced no cell with enough samples");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("motu-prior: {error}");
            ExitCode::FAILURE
        }
    }
}

fn default_buckets() -> Vec<TimeBucket> {
    vec![
        TimeBucket {
            label: "早高峰".to_owned(),
            start_hour: 7,
            end_hour: 10,
            factors: ClassFactors::default(),
        },
        TimeBucket {
            label: "平峰".to_owned(),
            start_hour: 10,
            end_hour: 17,
            factors: ClassFactors::default(),
        },
        TimeBucket {
            label: "晚高峰".to_owned(),
            start_hour: 17,
            end_hour: 20,
            factors: ClassFactors::default(),
        },
        TimeBucket {
            label: "夜间".to_owned(),
            start_hour: 20,
            end_hour: 7,
            factors: ClassFactors::default(),
        },
    ]
}

fn run() -> Result<bool, String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or_else(usage)?;
    if command != "build" {
        return Err(format!("unknown command {command}\n{}", usage()));
    }

    let mut graph: Option<PathBuf> = None;
    let mut trace: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut report: Option<PathBuf> = None;
    let mut explain = false;
    let mut min_samples = MIN_SAMPLES_PER_CELL;

    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--graph" => graph = Some(PathBuf::from(value()?)),
            "--trace" => trace = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--report" => report = Some(PathBuf::from(value()?)),
            "--explain" => explain = true,
            "--min-samples" => {
                min_samples = value()?
                    .parse()
                    .map_err(|error| format!("bad --min-samples: {error}"))?;
            }
            other => return Err(format!("unknown flag {other}\n{}", usage())),
        }
    }

    let graph_path = graph.ok_or("--graph is required")?;
    let trace_path = trace.ok_or("--trace is required")?;
    let out_path = out.ok_or("--out is required")?;

    let graph = RoadGraph::load(&graph_path).map_err(|error| error.to_string())?;
    let points = load_trace(&trace_path).map_err(|error| error.to_string())?;
    let built = build_prior(&graph, &points, &default_buckets(), min_samples);

    if explain {
        // 逐跳说明为什么被丢：轨迹没被采纳时，用户需要知道是数据问题还是匹配问题。
        for hop in built.report.hops.iter().take(40) {
            let verdict = match (&hop.rejection, &hop.class) {
                (None, Some(class)) => format!("OK  {class}"),
                (Some(reason), _) => format!("丢弃 {reason:?}"),
                (None, None) => "丢弃 ?".to_owned(),
            };
            println!("#{:<3} {:>6.1} km/h  {}", hop.index, hop.observed_kmh, verdict);
        }
        if built.report.hops.len() > 40 {
            println!("… 其余 {} 跳省略", built.report.hops.len() - 40);
        }
    }

    let markdown = built.report.to_markdown();
    if let Some(path) = report.as_ref() {
        std::fs::write(path, &markdown).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    print!("{markdown}");

    let json = serde_json::to_vec_pretty(&built.prior).map_err(|error| error.to_string())?;
    std::fs::write(&out_path, &json).map_err(|error| format!("{}: {error}", out_path.display()))?;
    println!("-> {} ({} bytes)", out_path.display(), json.len());

    Ok(built.prior != TrafficPrior::neutral() && !built.report.accepted_cells.is_empty())
}

fn usage() -> String {
    "usage: motu-prior build --graph <graph.bin> --trace <ride.jsonl> --out <prior.json>
                        [--report <out.md>] [--min-samples 20] [--explain]"
        .to_owned()
}
