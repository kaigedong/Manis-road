//! Compact in-memory road graph with a versioned binary file format.

use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

use motu_core::LatLng;
use serde::{Deserialize, Serialize};

use crate::error::{Result, RoutingError};

const MAGIC: &[u8; 8] = b"MOTUGRPH";
const VERSION: u32 = 3;
/// Oldest layout still accepted: v1 had no signal flags, v2 no lanes/surface.
const VERSION_LEGACY: u32 = 1;
/// First version that carries per-way lanes and surface.
const VERSION_WAY_SURFACE: u32 = 3;

/// One graph node, keyed by the OSM node id.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: i64,
    pub lat: f64,
    pub lon: f64,
}

impl GraphNode {
    #[must_use]
    pub fn position(self) -> LatLng {
        LatLng::new(self.lat, self.lon)
    }
}

/// Coarse surface class. `Unknown` is the honest default: most OSM ways carry
/// no `surface` tag, and guessing "paved" would hide gravel tracks from the
/// cost model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    #[default]
    Unknown,
    Paved,
    Unpaved,
}

impl Surface {
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Paved => 1,
            Self::Unpaved => 2,
        }
    }

    #[must_use]
    pub fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Paved,
            2 => Self::Unpaved,
            _ => Self::Unknown,
        }
    }
}

/// Metadata for one OSM way used by the graph.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WayInfo {
    #[serde(default)]
    pub name: String,
    pub highway: String,
    /// OSM `lanes`, 0 when untagged.
    #[serde(default)]
    pub lanes: u8,
    #[serde(default)]
    pub surface: Surface,
}

/// One directed edge in compressed adjacency storage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    pub to: u32,
    pub distance_m: f32,
    pub speed_kmh: u16,
    pub way: u32,
}

impl Edge {
    #[must_use]
    pub fn duration_s(self) -> f64 {
        f64::from(self.distance_m) / (f64::from(self.speed_kmh) / 3.6).max(0.1)
    }
}

/// A directed road graph in CSR layout.
#[derive(Debug, Clone, Default)]
pub struct RoadGraph {
    pub nodes: Vec<GraphNode>,
    /// One flag per node: `true` when the node is a signalised junction.
    ///
    /// Empty means "this graph predates signal data" — callers must not read it
    /// as "no signals", or the `≤ 3 slips/km` acceptance check would pass on a
    /// graph that never measured them.
    pub signals: Vec<bool>,
    pub ways: Vec<WayInfo>,
    /// `offsets[i]..offsets[i + 1]` indexes into `edges` for node `i`.
    pub offsets: Vec<u64>,
    pub edges: Vec<Edge>,
}

impl RoadGraph {
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "offsets are bounded by the in-memory edge count"
    )]
    pub fn neighbors(&self, node: u32) -> &[Edge] {
        let index = node as usize;
        if index + 1 >= self.offsets.len() {
            return &[];
        }
        let start = self.offsets[index] as usize;
        let end = self.offsets[index + 1] as usize;
        &self.edges[start..end]
    }

    /// Nearest graph node to `point`, with its distance in metres.
    #[must_use]
    pub fn nearest_node(&self, point: LatLng) -> Option<(u32, f64)> {
        let mut best: Option<(u32, f64)> = None;
        for (index, node) in self.nodes.iter().enumerate() {
            let distance = haversine_m(point.latitude, point.longitude, node.lat, node.lon);
            if best.is_none_or(|(_, current)| distance < current) {
                best = Some((u32::try_from(index).ok()?, distance));
            }
        }
        best
    }

    /// Bounding box as `(south, west, north, east)`.
    #[must_use]
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let first = self.nodes.first()?;
        let (mut south, mut west, mut north, mut east) =
            (first.lat, first.lon, first.lat, first.lon);
        for node in &self.nodes {
            south = south.min(node.lat);
            north = north.max(node.lat);
            west = west.min(node.lon);
            east = east.max(node.lon);
        }
        Some((south, west, north, east))
    }

    /// Coarse road class used by the offline basemap: 1 major, 2 medium,
    /// 3 minor.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "offsets are bounded by the in-memory edge count"
    )]
    pub fn basemap_segments(
        &self,
        south: f64,
        west: f64,
        north: f64,
        east: f64,
        max_level: u8,
        max_segments: usize,
    ) -> (Vec<[f64; 5]>, bool) {
        let levels: Vec<u8> = self
            .ways
            .iter()
            .map(|way| road_level(&way.highway))
            .collect();
        let mut segments = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if node.lat < south || node.lat > north || node.lon < west || node.lon > east {
                continue;
            }
            let start = self.offsets[index] as usize;
            let end = self.offsets[index + 1] as usize;
            for edge in &self.edges[start..end] {
                let Some(level) = levels.get(edge.way as usize).copied() else {
                    continue;
                };
                if level > max_level {
                    continue;
                }
                let Some(target) = self.nodes.get(edge.to as usize) else {
                    continue;
                };
                if target.lat < south || target.lat > north || target.lon < west || target.lon > east
                {
                    continue;
                }
                segments.push([node.lat, node.lon, target.lat, target.lon, f64::from(level)]);
                if segments.len() >= max_segments {
                    return (segments, true);
                }
            }
        }
        (segments, false)
    }

    /// Road segments intersecting the box, for the offline basemap.
    ///
    /// Each segment is `[lat1, lon1, lat2, lon2]`; when `max_segments` is hit
    /// the scan stops so a huge viewport cannot blow up the response.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "offsets are bounded by the in-memory edge count"
    )]
    pub fn segments_in_bounds(
        &self,
        south: f64,
        west: f64,
        north: f64,
        east: f64,
        max_segments: usize,
    ) -> Vec<[f64; 4]> {
        let mut segments = Vec::new();
        'nodes: for (index, node) in self.nodes.iter().enumerate() {
            if node.lat < south || node.lat > north || node.lon < west || node.lon > east {
                continue;
            }
            let start = self.offsets[index] as usize;
            let end = self.offsets[index + 1] as usize;
            for edge in &self.edges[start..end] {
                let Some(target) = self.nodes.get(edge.to as usize) else {
                    continue;
                };
                if target.lat < south || target.lat > north || target.lon < west || target.lon > east
                {
                    continue;
                }
                segments.push([node.lat, node.lon, target.lat, target.lon]);
                if segments.len() >= max_segments {
                    break 'nodes;
                }
            }
        }
        segments
    }

    /// Write the graph to `path` in the compact binary format.
    pub fn save(&self, path: &Path) -> Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(MAGIC)?;
        writer.write_all(&VERSION.to_le_bytes())?;

        let node_count = u32::try_from(self.nodes.len())
            .map_err(|_| RoutingError::InvalidGraph("too many nodes".to_owned()))?;
        writer.write_all(&node_count.to_le_bytes())?;
        for node in &self.nodes {
            writer.write_all(&node.id.to_le_bytes())?;
            writer.write_all(&node.lat.to_le_bytes())?;
            writer.write_all(&node.lon.to_le_bytes())?;
        }

        for node in 0..node_count as usize {
            let flag = u8::from(self.signals.get(node).copied().unwrap_or(false));
            writer.write_all(&[flag])?;
        }

        let way_count = u32::try_from(self.ways.len())
            .map_err(|_| RoutingError::InvalidGraph("too many ways".to_owned()))?;
        writer.write_all(&way_count.to_le_bytes())?;
        for way in &self.ways {
            write_string(&mut writer, &way.name)?;
            write_string(&mut writer, &way.highway)?;
            writer.write_all(&[way.lanes, way.surface.code()])?;
        }

        let edge_count = u32::try_from(self.edges.len())
            .map_err(|_| RoutingError::InvalidGraph("too many edges".to_owned()))?;
        writer.write_all(&edge_count.to_le_bytes())?;
        for offset in &self.offsets {
            writer.write_all(&offset.to_le_bytes())?;
        }
        for edge in &self.edges {
            writer.write_all(&edge.to.to_le_bytes())?;
            writer.write_all(&edge.distance_m.to_le_bytes())?;
            writer.write_all(&edge.speed_kmh.to_le_bytes())?;
            writer.write_all(&edge.way.to_le_bytes())?;
        }
        writer.flush()?;
        Ok(())
    }

    /// Load a graph written by [`RoadGraph::save`].
    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut magic = [0_u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(RoutingError::InvalidGraph("bad magic".to_owned()));
        }
        let version = read_u32(&mut reader)?;
        if !(VERSION_LEGACY..=VERSION).contains(&version) {
            return Err(RoutingError::InvalidGraph(format!(
                "unsupported version {version}"
            )));
        }

        let node_count = read_u32(&mut reader)? as usize;
        let mut nodes = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            let id = read_i64(&mut reader)?;
            let lat = read_f64(&mut reader)?;
            let lon = read_f64(&mut reader)?;
            nodes.push(GraphNode { id, lat, lon });
        }
        let mut signals = Vec::new();
        if version >= VERSION {
            signals.reserve(node_count);
            let mut byte = [0_u8; 1];
            for _ in 0..node_count {
                reader.read_exact(&mut byte)?;
                signals.push(byte[0] != 0);
            }
        }

        let way_count = read_u32(&mut reader)? as usize;
        let mut ways = Vec::with_capacity(way_count);
        for _ in 0..way_count {
            let name = read_string(&mut reader)?;
            let highway = read_string(&mut reader)?;
            let (lanes, surface) = if version >= VERSION_WAY_SURFACE {
                let mut bytes = [0_u8; 2];
                reader.read_exact(&mut bytes)?;
                (bytes[0], Surface::from_code(bytes[1]))
            } else {
                (0, Surface::Unknown)
            };
            ways.push(WayInfo {
                name,
                highway,
                lanes,
                surface,
            });
        }

        let edge_count = read_u32(&mut reader)? as usize;
        let mut offsets = Vec::with_capacity(node_count + 1);
        for _ in 0..=node_count {
            offsets.push(read_u64(&mut reader)?);
        }
        let mut edges = Vec::with_capacity(edge_count);
        for _ in 0..edge_count {
            edges.push(Edge {
                to: read_u32(&mut reader)?,
                distance_m: read_f32(&mut reader)?,
                speed_kmh: read_u16(&mut reader)?,
                way: read_u32(&mut reader)?,
            });
        }
        if offsets.len() != node_count + 1
            || offsets.last().copied() != Some(edge_count as u64)
            || offsets.windows(2).any(|pair| pair[0] > pair[1])
        {
            return Err(RoutingError::InvalidGraph(
                "adjacency offsets are inconsistent".to_owned(),
            ));
        }
        Ok(Self {
            nodes,
            signals,
            ways,
            offsets,
            edges,
        })
    }
}

fn write_string(writer: &mut impl Write, value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let length = u16::try_from(bytes.len())
        .map_err(|_| RoutingError::InvalidGraph("string too long".to_owned()))?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(bytes)?;
    Ok(())
}

fn read_string(reader: &mut impl Read) -> Result<String> {
    let length = read_u16(reader)? as usize;
    let mut bytes = vec![0_u8; length];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes)
        .map_err(|_| RoutingError::InvalidGraph("string is not UTF-8".to_owned()))
}

fn read_u16(reader: &mut impl Read) -> Result<u16> {
    let mut bytes = [0_u8; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(reader: &mut impl Read) -> Result<u32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> Result<u64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_i64(reader: &mut impl Read) -> Result<i64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(i64::from_le_bytes(bytes))
}

fn read_f32(reader: &mut impl Read) -> Result<f32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}

fn read_f64(reader: &mut impl Read) -> Result<f64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(f64::from_le_bytes(bytes))
}

/// Classify an OSM `highway` value for basemap styling.
#[must_use]
pub fn road_level(highway: &str) -> u8 {
    match highway {
        "trunk" | "trunk_link" | "primary" | "primary_link" => 1,
        "secondary" | "secondary_link" | "tertiary" | "tertiary_link" => 2,
        _ => 3,
    }
}

/// Great-circle distance in metres.
#[must_use]
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_008.8;
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let delta_phi = (lat2 - lat1).to_radians();
    let delta_lambda = (lon2 - lon1).to_radians();
    let a = (delta_phi / 2.0).sin().powi(2)
        + phi1.cos() * phi2.cos() * (delta_lambda / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RoadGraph {
        RoadGraph {
            nodes: vec![
                GraphNode {
                    id: 1,
                    lat: 31.23,
                    lon: 121.47,
                },
                GraphNode {
                    id: 2,
                    lat: 31.24,
                    lon: 121.48,
                },
            ],
            signals: Vec::new(),
            ways: vec![WayInfo {
                name: "测试路".to_owned(),
                highway: "primary".to_owned(),
             ..WayInfo::default() }],
            offsets: vec![0, 1, 1],
            edges: vec![Edge {
                to: 1,
                distance_m: 1_500.0,
                speed_kmh: 45,
                way: 0,
            }],
        }
    }

    #[test]
    fn binary_round_trip() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("graph.bin");
        let graph = sample();
        graph.save(&path).expect("save");
        let loaded = RoadGraph::load(&path).expect("load");
        assert_eq!(loaded.node_count(), 2);
        assert_eq!(loaded.edge_count(), 1);
        assert_eq!(loaded.ways[0].name, "测试路");
        assert_eq!(loaded.neighbors(0).len(), 1);
        assert_eq!(loaded.neighbors(1).len(), 0);
    }

    #[test]
    fn nearest_node_and_bounds() {
        let graph = sample();
        let (index, distance) = graph
            .nearest_node(LatLng::new(31.230_1, 121.470_1))
            .expect("snap");
        assert_eq!(index, 0);
        assert!(distance < 50.0);
        let (south, west, north, east) = graph.bounds().expect("bounds");
        assert!(south < north && west < east);
    }

    #[test]
    fn basemap_levels_filter_and_flag_truncation() {
        let graph = sample();
        let (segments, truncated) =
            graph.basemap_segments(31.22, 121.46, 31.25, 121.49, 1, 10);
        assert_eq!(segments.len(), 1);
        assert!(!truncated);
        assert_eq!(segments[0][4], 1.0);
        let (none, _) = graph.basemap_segments(31.22, 121.46, 31.25, 121.49, 0, 10);
        assert_eq!(none.len(), 0);
        let (limited, truncated) =
            graph.basemap_segments(31.22, 121.46, 31.25, 121.49, 3, 1);
        assert_eq!(limited.len(), 1);
        assert!(truncated);
    }

    #[test]
    fn segments_respect_the_box_and_limit() {
        let graph = sample();
        let segments = graph.segments_in_bounds(31.22, 121.46, 31.25, 121.49, 10);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0][0], 31.23);
        let outside = graph.segments_in_bounds(40.0, 116.0, 40.1, 116.1, 10);
        assert_eq!(outside.len(), 0);
        assert_eq!(
            graph
                .segments_in_bounds(31.22, 121.46, 31.25, 121.49, 1)
                .len(),
            1
        );
    }

    #[test]
    fn saves_and_loads_signal_flags() {
        let graph = RoadGraph {
            nodes: vec![
                GraphNode { id: 1, lat: 31.230, lon: 121.470 },
                GraphNode { id: 2, lat: 31.231, lon: 121.471 },
            ],
            signals: vec![false, true],
            ways: vec![WayInfo {
                name: "路".to_owned(),
                highway: "residential".to_owned(),
                lanes: 2,
                surface: Surface::Unpaved,
            }],
            offsets: vec![0, 1, 1],
            edges: vec![Edge { to: 1, distance_m: 100.0, speed_kmh: 30, way: 0 }],
        };
        let dir = std::env::temp_dir().join("motu-graph-signals-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("graph.bin");
        graph.save(&path).expect("save");
        let loaded = RoadGraph::load(&path).expect("load");
        assert_eq!(loaded.signals, vec![false, true]);
        assert_eq!(loaded.nodes.len(), 2);
        assert_eq!(loaded.ways[0].lanes, 2);
        assert_eq!(loaded.ways[0].surface, Surface::Unpaved);
        std::fs::remove_file(&path).ok();
    }
}
