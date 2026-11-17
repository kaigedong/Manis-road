//! Build, inspect, verify and unpack `.mrdp` offline data packages.
//!
//! ```text
//! motu-pack build --out sh.mrdp --graph var/offline/graph-od.bin \
//!                 --prior crates/motu-routing/examples/traffic-prior.json \
//!                 --avoid var/offline/avoid.json --bbox 31.13,121.30,31.33,121.62
//! motu-pack info    sh.mrdp
//! motu-pack verify  sh.mrdp
//! motu-pack extract sh.mrdp --out /tmp/unpacked
//! motu-pack pmtiles-info  var/offline/roads-od.pmtiles
//! motu-pack tile var/offline/roads-od.pmtiles 14 13719 6692 --out /tmp/t.mvt
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use motu_pack::{PackError, PackReader, PackWriter, SourceInfo};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("motu-pack: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Input flags and the entry name each one maps to.
const INPUTS: &[(&str, &str)] = &[
    ("--graph", "graph.bin"),
    ("--prior", "traffic-prior.json"),
    ("--avoid", "avoid.json"),
    ("--cameras", "cameras.json"),
    ("--areas", "areas.json"),
    ("--tiles", "tiles.pmtiles"),
];

#[allow(
    clippy::too_many_lines,
    reason = "one flat match over CLI subcommands reads better than a dispatch table"
)]
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or_else(usage)?;
    match command.as_str() {
        "build" => build(args),
        "info" => {
            let path = args.next().ok_or_else(usage)?;
            let reader = open(&path)?;
            print!("{}", reader.describe());
            Ok(())
        }
        "verify" => {
            let path = args.next().ok_or_else(usage)?;
            let reader = open(&path)?;
            reader.verify().map_err(|error| error.to_string())?;
            println!(
                "ok: {} entries verified, {} bytes",
                reader.manifest().entries.len(),
                reader.manifest().entries.iter().map(|e| e.length).sum::<u64>()
            );
            Ok(())
        }
        "pmtiles-info" => {
            let path = PathBuf::from(args.next().ok_or_else(usage)?);
            let tiles = motu_pack::pmtiles::TileSet::open(&path).map_err(|error| error.to_string())?;
            let header = tiles.header();
            println!("pmtiles spec v{}", header.spec_version);
            println!(
                "zoom {}-{}  tile type {}  tile compression {}  internal compression {}",
                header.min_zoom,
                header.max_zoom,
                header.tile_type,
                header.tile_compression,
                header.internal_compression
            );
            println!(
                "addressed tiles {}  archive {} bytes  tile data {} bytes",
                header.addressed_tiles,
                tiles.len(),
                tiles.tile_data_length()
            );
            Ok(())
        }
        "tile" => {
            let path = PathBuf::from(args.next().ok_or_else(usage)?);
            let zoom: u8 = args
                .next()
                .ok_or_else(usage)?
                .parse()
                .map_err(|error| format!("bad zoom: {error}"))?;
            let x: u32 = args
                .next()
                .ok_or_else(usage)?
                .parse()
                .map_err(|error| format!("bad x: {error}"))?;
            let y: u32 = args
                .next()
                .ok_or_else(usage)?
                .parse()
                .map_err(|error| format!("bad y: {error}"))?;
            let mut out: Option<PathBuf> = None;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--out" => out = Some(PathBuf::from(args.next().ok_or_else(usage)?)),
                    other => return Err(format!("unknown flag {other}")),
                }
            }
            let tiles = motu_pack::pmtiles::TileSet::open(&path).map_err(|error| error.to_string())?;
            let bytes = tiles
                .tile(zoom, x, y)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("tile {zoom}/{x}/{y} not in archive"))?;
            if let Some(path) = out {
                {
                    std::fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
                    println!("{} bytes -> {}", bytes.len(), path.display());
                }
            } else {
                use std::io::Write as _;
                std::io::stdout()
                    .write_all(bytes)
                    .map_err(|error| format!("stdout: {error}"))?;
            }
            Ok(())
        }
        "extract" => {
            let path = args.next().ok_or_else(usage)?;
            let mut out: Option<PathBuf> = None;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--out" => out = args.next().map(PathBuf::from),
                    other => return Err(format!("unknown flag {other}")),
                }
            }
            let out = out.ok_or("extract needs --out <dir>")?;
            std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
            let reader = open(&path)?;
            for name in reader.names() {
                let bytes = reader.entry(name).map_err(|error| error.to_string())?;
                let target = out.join(name);
                std::fs::write(&target, bytes).map_err(|error| format!("{}: {error}", target.display()))?;
                println!("{} -> {}", name, target.display());
            }
            Ok(())
        }
        other => Err(format!("unknown command {other}\n{}", usage())),
    }
}

fn build(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut out: Option<PathBuf> = None;
    let mut bbox: Option<[f64; 4]> = None;
    let mut note = String::new();
    let mut inputs: Vec<(&'static str, PathBuf)> = Vec::new();

    while let Some(flag) = args.next() {
        if flag == "--out" {
            out = args.next().map(PathBuf::from);
            continue;
        }
        if flag == "--bbox" {
            let raw = args.next().ok_or("--bbox needs south,west,north,east")?;
            bbox = Some(parse_bbox(&raw)?);
            continue;
        }
        if flag == "--note" {
            note = args.next().ok_or("--note needs a value")?;
            continue;
        }
        let Some((_, entry_name)) = INPUTS.iter().find(|(name, _)| *name == flag) else {
            return Err(format!("unknown flag {flag}\n{}", usage()));
        };
        let path = args.next().ok_or_else(|| format!("{flag} needs a path"))?;
        inputs.push((entry_name, PathBuf::from(path)));
    }

    let out = out.ok_or("build needs --out <file.mrdp>")?;
    if inputs.is_empty() {
        return Err("build needs at least one input (--graph/--prior/--avoid/--cameras/--areas/--tiles)".to_owned());
    }

    let mut writer = PackWriter::new();
    for (entry_name, path) in &inputs {
        let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        writer
            .add(entry_name, bytes)
            .map_err(|error| error.to_string())?;
        println!("+ {entry_name:<20} {}", path.display());
    }

    let source = SourceInfo {
        bbox,
        note,
        ..SourceInfo::default()
    };
    let manifest = writer
        .write(&out, &source)
        .map_err(|error| error.to_string())?;
    let size = std::fs::metadata(&out).map_or(0, |meta| meta.len());
    println!(
        "-> {} ({} bytes, {} entries, format v{})",
        out.display(),
        size,
        manifest.entries.len(),
        manifest.format_version
    );
    Ok(())
}

fn parse_bbox(raw: &str) -> Result<[f64; 4], String> {
    let parts: Vec<f64> = raw
        .split(',')
        .map(|part| {
            part.trim()
                .parse::<f64>()
                .map_err(|error| format!("bad bbox value {part}: {error}"))
        })
        .collect::<Result<_, _>>()?;
    let [south, west, north, east]: [f64; 4] = parts
        .try_into()
        .map_err(|_| format!("bbox needs 4 numbers, got {raw}"))?;
    if south >= north || west >= east {
        return Err(format!("bbox is empty or inverted: {raw}"));
    }
    Ok([south, west, north, east])
}

fn open(path: &str) -> Result<PackReader, String> {
    PackReader::open(Path::new(path)).map_err(|error: PackError| error.to_string())
}

fn usage() -> String {
    "usage:
  motu-pack build --out <file.mrdp> [--graph f] [--prior f] [--avoid f]
                  [--cameras f] [--areas f] [--tiles f]
                  [--bbox south,west,north,east] [--note text]
  motu-pack info    <file.mrdp>
  motu-pack verify  <file.mrdp>
  motu-pack extract <file.mrdp> --out <dir>"
        .to_owned()
}
