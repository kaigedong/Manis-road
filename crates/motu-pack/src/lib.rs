//! Offline data package (`.mrdp`).
//!
//! The app is only "fully offline" if everything it needs at runtime arrives as
//! one artifact a user can copy onto the phone: the routing graph, the
//! time-of-day prior, the forbidden areas, the camera points and (later) the map
//! tiles. This crate defines that artifact.
//!
//! Layout:
//!
//! ```text
//! magic           8 bytes  "MRDPKG01"
//! format version  u32 LE
//! manifest length u32 LE
//! manifest        JSON
//! blobs           concatenated, offsets relative to blob start
//! ```
//!
//! Each entry carries its own length and CRC32 so a truncated or corrupted copy
//! is detected on open instead of surfacing later as a routing bug. CRC32 is a
//! corruption check, not a signature: it proves the bytes match what was packed,
//! not who packed them.

// Fallible APIs document their errors on the type, not on every method.
#![allow(clippy::missing_errors_doc)]

use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 8] = b"MRDPKG01";
const FORMAT_VERSION: u32 = 1;
const HEADER_LEN: usize = 8 + 4 + 4;

/// One file inside the package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Stable name the app looks up, e.g. `graph.bin`.
    pub name: String,
    /// Offset of the blob relative to the start of the blob section.
    pub offset: u64,
    pub length: u64,
    /// CRC32 of the blob.
    pub crc32: u32,
}

/// Where the packaged data came from. Kept inside the package so attribution
/// travels with the data instead of living in a README nobody copies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub osm_attribution: String,
    /// `[south, west, north, east]`, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
    #[serde(default)]
    pub note: String,
}

impl Default for SourceInfo {
    fn default() -> Self {
        Self {
            osm_attribution: "© OpenStreetMap contributors (ODbL)".to_owned(),
            bbox: None,
            note: String::new(),
        }
    }
}

/// Parsed package manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format_version: u32,
    /// Unix seconds.
    pub created_at_unix: u64,
    pub source: SourceInfo,
    pub entries: Vec<Entry>,
}

/// Package errors.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("not an .mrdp package (bad magic)")]
    BadMagic,
    #[error("unsupported package format version {0}")]
    BadVersion(u32),
    #[error("manifest is invalid: {0}")]
    Manifest(String),
    #[error("entry {0} is missing")]
    MissingEntry(String),
    #[error("entry {name} is out of bounds")]
    OutOfBounds { name: String },
    #[error("entry {name} is corrupt (crc mismatch)")]
    Corrupt { name: String },
    #[error("duplicate entry name {0}")]
    Duplicate(String),
}

/// Build a package.
#[derive(Debug, Default)]
pub struct PackWriter {
    entries: Vec<(String, Vec<u8>)>,
}

impl PackWriter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one blob. Names must be unique and non-empty.
    pub fn add(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), PackError> {
        if name.trim().is_empty() {
            return Err(PackError::Manifest("entry name is empty".to_owned()));
        }
        if self.entries.iter().any(|(existing, _)| existing == name) {
            return Err(PackError::Duplicate(name.to_owned()));
        }
        self.entries.push((name.to_owned(), bytes));
        Ok(())
    }

    /// Serialise to `path`, stamping `source` into the manifest.
    pub fn write(&self, path: &Path, source: &SourceInfo) -> Result<Manifest, PackError> {
        let mut entries = Vec::with_capacity(self.entries.len());
        let mut blobs = Vec::new();
        for (name, bytes) in &self.entries {
            let length = u64::try_from(bytes.len())
                .map_err(|_| PackError::Manifest(format!("entry {name} is too large")))?;
            entries.push(Entry {
                name: name.clone(),
                offset: u64::try_from(blobs.len())
                    .map_err(|_| PackError::Manifest("package is too large".to_owned()))?,
                length,
                crc32: crc32(bytes),
            });
            blobs.extend_from_slice(bytes);
        }

        let manifest = Manifest {
            format_version: FORMAT_VERSION,
            created_at_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs()),
            source: source.clone(),
            entries,
        };
        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|error| PackError::Manifest(error.to_string()))?;
        let manifest_len = u32::try_from(manifest_bytes.len())
            .map_err(|_| PackError::Manifest("manifest is too large".to_owned()))?;

        let mut out = Vec::with_capacity(HEADER_LEN + manifest_bytes.len() + blobs.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&manifest_len.to_le_bytes());
        out.extend_from_slice(&manifest_bytes);
        out.extend_from_slice(&blobs);
        std::fs::write(path, out)?;
        Ok(manifest)
    }
}

/// Read a package from memory.
#[derive(Debug, Clone)]
pub struct PackReader {
    bytes: Vec<u8>,
    manifest: Manifest,
    blob_start: u64,
}

impl PackReader {
    /// Open and parse the header and manifest (does not verify blob CRCs).
    pub fn open(path: &Path) -> Result<Self, PackError> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(bytes)
    }

    /// Parse an in-memory package.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, PackError> {
        if bytes.len() < HEADER_LEN || &bytes[..8] != MAGIC {
            return Err(PackError::BadMagic);
        }
        let version = u32::from_le_bytes(
            bytes[8..12]
                .try_into()
                .map_err(|_| PackError::Manifest("truncated header".to_owned()))?,
        );
        if version != FORMAT_VERSION {
            return Err(PackError::BadVersion(version));
        }
        let manifest_len = u32::from_le_bytes(
            bytes[12..16]
                .try_into()
                .map_err(|_| PackError::Manifest("truncated header".to_owned()))?,
        ) as usize;
        let manifest_end = 16_usize
            .checked_add(manifest_len)
            .ok_or_else(|| PackError::Manifest("manifest length overflow".to_owned()))?;
        if bytes.len() < manifest_end {
            return Err(PackError::Manifest("manifest runs past end of file".to_owned()));
        }
        let manifest: Manifest = serde_json::from_slice(&bytes[16..manifest_end])
            .map_err(|error| PackError::Manifest(error.to_string()))?;
        if manifest.format_version != FORMAT_VERSION {
            return Err(PackError::BadVersion(manifest.format_version));
        }
        Ok(Self {
            bytes,
            manifest,
            blob_start: u64::try_from(manifest_end).unwrap_or(u64::MAX),
        })
    }

    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Entry names in pack order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.manifest
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    fn blob_range(&self, entry: &Entry) -> Result<(usize, usize), PackError> {
        let start = self
            .blob_start
            .checked_add(entry.offset)
            .ok_or(PackError::OutOfBounds {
                name: entry.name.clone(),
            })?;
        let end = start.checked_add(entry.length).ok_or(PackError::OutOfBounds {
            name: entry.name.clone(),
        })?;
        if end > u64::try_from(self.bytes.len()).unwrap_or(u64::MAX) {
            return Err(PackError::OutOfBounds {
                name: entry.name.clone(),
            });
        }
        Ok((
            usize::try_from(start).unwrap_or(usize::MAX),
            usize::try_from(end).unwrap_or(usize::MAX),
        ))
    }

    /// Read one blob without verifying its checksum.
    pub fn entry(&self, name: &str) -> Result<&[u8], PackError> {
        let entry = self
            .manifest
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| PackError::MissingEntry(name.to_owned()))?;
        let (start, end) = self.blob_range(entry)?;
        Ok(&self.bytes[start..end])
    }

    /// Recompute every CRC and report the first problem.
    ///
    /// The error keeps the offending name so the caller can tell the user which
    /// part of the download is broken.
    pub fn verify(&self) -> Result<(), PackError> {
        for entry in &self.manifest.entries {
            let (start, end) = self.blob_range(entry)?;
            let actual = crc32(&self.bytes[start..end]);
            if actual != entry.crc32 {
                return Err(PackError::Corrupt {
                    name: entry.name.clone(),
                });
            }
        }
        Ok(())
    }

    /// One-line per entry, for the `info` command.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "package format v{}  created {}  entries {}",
            self.manifest.format_version,
            self.manifest.created_at_unix,
            self.manifest.entries.len()
        );
        let _ = writeln!(out, "attribution: {}", self.manifest.source.osm_attribution);
        if let Some([south, west, north, east]) = self.manifest.source.bbox {
            let _ = writeln!(out, "bbox: {south:.5},{west:.5} .. {north:.5},{east:.5}");
        }
        if !self.manifest.source.note.is_empty() {
            let _ = writeln!(out, "note: {}", self.manifest.source.note);
        }
        for entry in &self.manifest.entries {
            let _ = writeln!(
                out,
                "  {:<20} {:>10} bytes  crc32 {:08x}",
                entry.name, entry.length, entry.crc32
            );
        }
        out
    }
}

/// CRC32 (IEEE 802.3), the same polynomial `zip` and PNG use.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0_u32; 256];
    for (index, slot) in table.iter_mut().enumerate() {
        let mut value = u32::try_from(index).unwrap_or(0);
        for _ in 0..8 {
            value = if value & 1 != 0 {
                (value >> 1) ^ 0xEDB8_8320
            } else {
                value >> 1
            };
        }
        *slot = value;
    }
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        let index = ((crc ^ u32::from(*byte)) & 0xFF) as usize;
        crc = (crc >> 8) ^ table[index];
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    fn sample() -> PackWriter {
        let mut writer = PackWriter::new();
        writer.add("graph.bin", b"graph-bytes".to_vec()).expect("add");
        writer
            .add("traffic-prior.json", b"{\"buckets\":[]}".to_vec())
            .expect("add");
        writer
    }

    #[test]
    fn roundtrip_preserves_bytes_and_manifest() {
        let dir = std::env::temp_dir().join("motu-pack-roundtrip");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("test.mrdp");
        let source = SourceInfo {
            bbox: Some([31.1, 121.3, 31.4, 121.7]),
            ..SourceInfo::default()
        };
        sample().write(&path, &source).expect("write");

        let reader = PackReader::open(&path).expect("open");
        assert_eq!(reader.names(), vec!["graph.bin", "traffic-prior.json"]);
        assert_eq!(reader.entry("graph.bin").expect("entry"), b"graph-bytes");
        assert_eq!(reader.manifest().source.bbox, Some([31.1, 121.3, 31.4, 121.7]));
        reader.verify().expect("verify");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn detects_a_flipped_byte() {
        let dir = std::env::temp_dir().join("motu-pack-corrupt");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("test.mrdp");
        sample().write(&path, &SourceInfo::default()).expect("write");

        let mut bytes = std::fs::read(&path).expect("read");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let reader = PackReader::from_bytes(bytes).expect("parse");
        match reader.verify() {
            Err(PackError::Corrupt { name }) => assert_eq!(name, "traffic-prior.json"),
            other => panic!("expected a corrupt entry, got {other:?}"),
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_a_truncated_file() {
        let dir = std::env::temp_dir().join("motu-pack-truncated");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("test.mrdp");
        sample().write(&path, &SourceInfo::default()).expect("write");

        let bytes = std::fs::read(&path).expect("read");
        let reader = PackReader::from_bytes(bytes[..bytes.len() - 4].to_vec()).expect("parse");
        assert!(reader.verify().is_err(), "truncation must fail verification");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_bad_magic_and_duplicates() {
        assert!(matches!(
            PackReader::from_bytes(b"not-a-package-at-all".to_vec()),
            Err(PackError::BadMagic)
        ));
        let mut writer = PackWriter::new();
        writer.add("a", vec![1]).expect("add");
        assert!(matches!(writer.add("a", vec![2]), Err(PackError::Duplicate(_))));
        assert!(writer.add("  ", vec![2]).is_err());
    }

    #[test]
    fn missing_entry_is_reported_by_name() {
        let dir = std::env::temp_dir().join("motu-pack-missing");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("test.mrdp");
        sample().write(&path, &SourceInfo::default()).expect("write");
        let reader = PackReader::open(&path).expect("open");
        assert!(matches!(
            reader.entry("tiles.pmtiles"),
            Err(PackError::MissingEntry(name)) if name == "tiles.pmtiles"
        ));
        std::fs::remove_file(&path).ok();
    }
}
