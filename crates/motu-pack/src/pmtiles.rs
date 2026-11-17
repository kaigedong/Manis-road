//! Minimal `PMTiles` v3 reader.
//!
//! The Android app renders from `tiles.pmtiles` inside the `.mrdp` package, so the
//! container has to be readable offline without a browser or a tile server. This
//! module reads the header, walks the directory tree and hands back the **raw
//! tile bytes** — they are still gzip-compressed, which is exactly what a tile
//! client wants (`Content-Encoding: gzip`, no recompression on our side).
//!
//! Only the parts we need are implemented: no writer, no metadata decoding, no
//! multi-range requests.

use std::path::Path;

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};

use crate::PackError;

const MAGIC: &[u8; 7] = b"PMTiles";
const HEADER_LEN: usize = 127;
/// `1 = none, 2 = gzip, 3 = brotli, 4 = zstd`. Easy to get wrong: gzip is **2**,
/// not 1 — verified against a real `pmtiles convert` output whose header byte is
/// 2 and whose tile payload starts with `1f 8b`.
const COMPRESSION_GZIP: u8 = 2;
/// `1` in the spec; only the test encoder writes it, hence the allow below.
#[cfg_attr(not(test), allow(dead_code))]
const COMPRESSION_NONE: u8 = 1;

/// Header fields we actually use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TileSetHeader {
    pub spec_version: u8,
    pub tile_type: u8,
    pub tile_compression: u8,
    pub internal_compression: u8,
    pub min_zoom: u8,
    pub max_zoom: u8,
    pub addressed_tiles: u64,
}

impl TileSetHeader {
    /// Tile type per spec: 1 = MVT, 2 = PNG, 3 = JPEG, 4 = WebP, 5 = AVIF.
    #[must_use]
    pub fn is_vector(&self) -> bool {
        self.tile_type == 1
    }

    /// Compression per spec: 0 = unknown, 1 = none, 2 = gzip, 3 = brotli, 4 = zstd.
    #[must_use]
    pub fn tile_is_gzip(&self) -> bool {
        self.tile_compression == COMPRESSION_GZIP
    }
}

/// One directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    tile_id: u64,
    run_length: u64,
    length: usize,
    offset: u64,
}

/// A parsed `PMTiles` file held in memory.
///
/// Tile data for a city extract is a few megabytes, and the app already loads the
/// whole graph; keeping the archive in memory avoids seeking on every tile.
#[derive(Debug)]
pub struct TileSet {
    header: TileSetHeader,
    bytes: Vec<u8>,
    root_offset: usize,
    root_length: usize,
    leaf_offset: usize,
    leaf_length: usize,
    tile_data_offset: usize,
    tile_data_length: usize,
}

impl TileSet {
    /// Open a `.pmtiles` file.
    ///
    /// # Errors
    /// Returns [`PackError::Io`] when the file cannot be read and
    /// [`PackError::Format`] when the header or magic is wrong.
    pub fn open(path: &Path) -> Result<Self, PackError> {
        let bytes = std::fs::read(path)
            .map_err(|error| PackError::Format(format!("{}: {error}", path.display())))?;
        Self::from_bytes(bytes)
    }

    /// Parse an in-memory archive.
    ///
    /// # Errors
    /// Returns [`PackError::Format`] for a bad magic, an unsupported version or a
    /// truncated header.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, PackError> {
        if bytes.len() < HEADER_LEN || &bytes[..7] != MAGIC {
            return Err(PackError::Format("not a .pmtiles archive".to_owned()));
        }
        let spec_version = bytes[7];
        if spec_version != 3 {
            return Err(PackError::Format(format!(
                "unsupported pmtiles spec version {spec_version}"
            )));
        }
        let u64_at = |offset: usize| -> u64 {
            let mut buf = [0_u8; 8];
            buf.copy_from_slice(&bytes[offset..offset + 8]);
            u64::from_le_bytes(buf)
        };
        let usize_at = |offset: usize| -> usize { usize::try_from(u64_at(offset)).unwrap_or(usize::MAX) };

        let header = TileSetHeader {
            spec_version,
            tile_type: bytes[99],
            tile_compression: bytes[98],
            internal_compression: bytes[97],
            min_zoom: bytes[100],
            max_zoom: bytes[101],
            addressed_tiles: u64_at(72),
        };
        if !header.is_vector() {
            return Err(PackError::Format(format!(
                "tile type {} is not vector (MVT)",
                header.tile_type
            )));
        }
        Ok(Self {
            header,
            root_offset: usize_at(8),
            root_length: usize_at(16),
            leaf_offset: usize_at(40),
            leaf_length: usize_at(48),
            tile_data_offset: usize_at(56),
            tile_data_length: usize_at(64),
            bytes,
        })
    }

    #[must_use]
    pub fn header(&self) -> &TileSetHeader {
        &self.header
    }

    /// Raw tile bytes (still gzip-compressed when [`TileSetHeader::tile_is_gzip`]).
    ///
    /// # Errors
    /// Returns [`PackError::Format`] if the directory tree is inconsistent.
    pub fn tile(&self, zoom: u8, x: u32, y: u32) -> Result<Option<&[u8]>, PackError> {
        if zoom < self.header.min_zoom || zoom > self.header.max_zoom {
            return Ok(None);
        }
        let Some(target) = tile_id(zoom, x, y) else {
            return Ok(None);
        };

        let mut directory = self.load_directory(self.root_offset, self.root_length)?;
        for _ in 0..4 {
            let Some(entry) = directory
                .iter()
                .find(|entry| target >= entry.tile_id && target < entry.tile_id + entry.run_length)
                .copied()
            else {
                return Ok(None);
            };
            if entry.run_length > 0 {
                let start = self
                    .tile_data_offset
                    .checked_add(usize::try_from(entry.offset).unwrap_or(usize::MAX))
                    .ok_or_else(|| PackError::Format("tile offset overflow".to_owned()))?;
                let end = start
                    .checked_add(entry.length)
                    .ok_or_else(|| PackError::Format("tile length overflow".to_owned()))?;
                if end > self.bytes.len() {
                    return Err(PackError::Format("tile exceeds archive".to_owned()));
                }
                return Ok(Some(&self.bytes[start..end]));
            }
            // run_length == 0 marks a pointer into the leaf directory section.
            let leaf_start = self
                .leaf_offset
                .checked_add(usize::try_from(entry.offset).unwrap_or(usize::MAX))
                .ok_or_else(|| PackError::Format("leaf offset overflow".to_owned()))?;
            if leaf_start > self.leaf_offset.saturating_add(self.leaf_length).min(self.bytes.len()) {
                return Err(PackError::Format("leaf pointer outside leaf section".to_owned()));
            }
            directory = self.load_directory(leaf_start, self.leaf_length)?;
        }
        Ok(None)
    }

    /// Number of bytes in memory, for logging.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Bytes in the tile-data section.
    #[must_use]
    pub fn tile_data_length(&self) -> usize {
        self.tile_data_length
    }

    fn load_directory(&self, offset: usize, length: usize) -> Result<Vec<Entry>, PackError> {
        let end = offset
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| PackError::Format("directory outside archive".to_owned()))?;
        let raw = &self.bytes[offset..end];
        let plain = if self.header.internal_compression == COMPRESSION_GZIP {
            let mut decoder = GzDecoder::new(raw);
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut decoder, &mut out)
                .map_err(|error| PackError::Format(format!("gzip directory: {error}")))?;
            out
        } else {
            raw.to_vec()
        };
        parse_directory(&plain)
    }
}

/// Decode a `PMTiles` directory.
///
/// The encoding is **columnar**, not per-entry: entry count first, then all
/// `TileID` deltas, then all `RunLength`s, then all `Length`s, then all `Offset`s
/// (spec §4.2). Reading it as four interleaved varints per entry silently
/// produces plausible-looking garbage.
fn parse_directory(bytes: &[u8]) -> Result<Vec<Entry>, PackError> {
    let mut cursor = 0_usize;
    let next = |cursor: &mut usize| -> Result<u64, PackError> {
        let mut value = 0_u64;
        let mut shift = 0_u32;
        loop {
            let byte = *bytes
                .get(*cursor)
                .ok_or_else(|| PackError::Format("directory truncated".to_owned()))?;
            *cursor += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
            if shift > 63 {
                return Err(PackError::Format("varint too long".to_owned()));
            }
        }
    };

    let count = usize::try_from(next(&mut cursor)?)
        .map_err(|_| PackError::Format("entry count too large".to_owned()))?;
    if count == 0 {
        return Ok(Vec::new());
    }

    let mut tile_ids = Vec::with_capacity(count);
    let mut running = 0_u64;
    for _ in 0..count {
        running = running
            .checked_add(next(&mut cursor)?)
            .ok_or_else(|| PackError::Format("tile id overflow".to_owned()))?;
        tile_ids.push(running);
    }

    let mut run_lengths = Vec::with_capacity(count);
    for _ in 0..count {
        run_lengths.push(next(&mut cursor)?);
    }

    let mut lengths = Vec::with_capacity(count);
    for _ in 0..count {
        lengths.push(usize::try_from(next(&mut cursor)?).unwrap_or(usize::MAX));
    }

    // Offsets use a compact form: `0` means "directly after the previous entry"
    // (contiguous blobs), any other value `n` means `offset = n - 1`.
    let mut offsets = Vec::with_capacity(count);
    let mut previous_end = 0_u64;
    for length in &lengths {
        let raw = next(&mut cursor)?;
        let offset = if raw == 0 {
            previous_end
        } else {
            raw - 1
        };
        previous_end = offset.saturating_add(u64::try_from(*length).unwrap_or(0));
        offsets.push(offset);
    }

    Ok((0..count)
        .map(|index| Entry {
            tile_id: tile_ids[index],
            run_length: run_lengths[index],
            length: lengths[index],
            offset: offsets[index],
        })
        .collect())
}

/// Hilbert-curve tile id used by `PMTiles`.
///
/// Returns `None` for out-of-range coordinates so callers cannot index nonsense.
#[must_use]
pub fn tile_id(zoom: u8, x: u32, y: u32) -> Option<u64> {
    if zoom > 30 {
        return None;
    }
    let side = 1_u64 << zoom;
    if u64::from(x) >= side || u64::from(y) >= side {
        return None;
    }
    // Number of tiles in all shallower zooms.
    let base = ((1_u64 << (2 * u32::from(zoom))) - 1) / 3;
    let mut rx;
    let mut ry;
    let mut index = 0_u64;
    let mut x = u64::from(x);
    let mut y = u64::from(y);
    let mut step = side / 2;
    while step > 0 {
        rx = u64::from((x & step) > 0);
        ry = u64::from((y & step) > 0);
        index += step * step * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                // 旋转用的是整幅边长（2^zoom），不是当前的 step——写成 step 会下溢。
                x = side - 1 - x;
                y = side - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        step /= 2;
    }
    base.checked_add(index)
}

#[cfg(test)]
mod tests {
    use super::*;



    fn varint(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                return;
            }
        }
    }

    /// Build a minimal single-tile archive with an uncompressed root directory.
    fn single_tile_archive(tile: &[u8]) -> Vec<u8> {
        // Columnar layout: count, then tile ids, run lengths, lengths, offsets.
        let mut directory = Vec::new();
        varint(1, &mut directory); // entry count
        varint(0, &mut directory); // tile id delta (tile 0)
        varint(1, &mut directory); // run length
        varint(u64::try_from(tile.len()).expect("len"), &mut directory);
        varint(1, &mut directory); // offset 0 → encoded as offset + 1

        let header_len = 127_u64;
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(3); // spec version
        out.extend_from_slice(&header_len.to_le_bytes()); // root offset
        out.extend_from_slice(
            &u64::try_from(directory.len())
                .expect("len")
                .to_le_bytes(),
        ); // root length
        out.extend_from_slice(&0_u64.to_le_bytes()); // metadata offset
        out.extend_from_slice(&0_u64.to_le_bytes()); // metadata length
        out.extend_from_slice(&0_u64.to_le_bytes()); // leaf offset
        out.extend_from_slice(&0_u64.to_le_bytes()); // leaf length
        let tile_offset = header_len + u64::try_from(directory.len()).expect("len");
        out.extend_from_slice(&tile_offset.to_le_bytes()); // tile data offset
        out.extend_from_slice(
            &u64::try_from(tile.len())
                .expect("len")
                .to_le_bytes(),
        ); // tile data length
        out.extend_from_slice(&1_u64.to_le_bytes()); // addressed
        out.extend_from_slice(&1_u64.to_le_bytes()); // entries
        out.extend_from_slice(&1_u64.to_le_bytes()); // contents
        out.push(1); // clustered
        out.push(COMPRESSION_NONE); // internal compression: none
        out.push(COMPRESSION_GZIP); // tile compression: gzip
        out.push(1); // tile type: mvt
        out.push(0); // min zoom
        out.push(0); // max zoom
        out.extend(std::iter::repeat_n(0, 4 + 4 + 4 + 4)); // min/max lon/lat
        out.push(0); // center zoom
        out.extend(std::iter::repeat_n(0, 8)); // center lon/lat
        assert_eq!(out.len(), HEADER_LEN, "header must be exactly 127 bytes");
        out.extend_from_slice(&directory);
        out.extend_from_slice(tile);
        out
    }

    #[test]
    fn reads_a_tile_from_a_minimal_archive() {
        let tile = vec![0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0];
        let archive = TileSet::from_bytes(single_tile_archive(&tile)).expect("parse");
        assert_eq!(archive.header().tile_type, 1);
        assert!(archive.header().tile_is_gzip());
        let found = archive.tile(0, 0, 0).expect("tile");
        assert_eq!(found, Some(tile.as_slice()));
    }

    #[test]
    fn out_of_range_tiles_are_none_not_errors() {
        let archive = TileSet::from_bytes(single_tile_archive(&[0x1f, 0x8b])).expect("parse");
        assert!(archive.tile(0, 0, 0).expect("tile").is_some());
        assert!(archive.tile(1, 0, 0).expect("tile").is_none(), "above max zoom");
        assert!(archive.tile(0, 3, 0).expect("tile").is_none(), "x out of range");
    }

    #[test]
    fn rejects_foreign_files() {
        assert!(TileSet::from_bytes(b"not a pmtiles file".to_vec()).is_err());
    }

    #[test]
    fn hilbert_ids_are_a_bijection_per_zoom() {
        assert_eq!(tile_id(0, 0, 0), Some(0));
        for zoom in 1..=4_u8 {
            let side = 1_u32 << zoom;
            let base = ((1_u64 << (2 * u32::from(zoom))) - 1) / 3;
            let mut seen = std::collections::HashSet::new();
            for x in 0..side {
                for y in 0..side {
                    let id = tile_id(zoom, x, y).expect("in range");
                    assert!(id >= base, "id below zoom base at z{zoom}");
                    assert!(seen.insert(id), "duplicate id {id} at z{zoom}");
                }
            }
            assert_eq!(
                seen.len(),
                usize::try_from(u64::from(side) * u64::from(side)).expect("count")
            );
        }
    }

    /// Real file check; skipped when the dataset is not present locally.
    #[test]
    fn reads_a_real_city_archive_when_available() {
        let path = Path::new("../../var/offline/roads-od.pmtiles");
        if !path.is_file() {
            return;
        }
        let archive = TileSet::open(path).expect("open");
        assert!(archive.header().is_vector());
        assert!(archive.header().tile_is_gzip());
        let tile = archive
            .tile(14, 13719, 6692)
            .expect("read")
            .expect("tile present");
        assert!(tile.len() > 1000, "tile too small: {}", tile.len());
        assert_eq!(&tile[..2], &[0x1f, 0x8b], "tile should still be gzip");
    }
}
