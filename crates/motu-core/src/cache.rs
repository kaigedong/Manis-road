//! Local SQLite cache for synced camera points, camera details and images.
//!
//! The cache exists to keep the official service load low: a synced point or a
//! previously opened detail is served from disk without another request. It is
//! a *personal debugging cache* living inside the git-ignored data directory
//! (`var/motu-web` by default). Nothing here is bundled with the repository or
//! redistributed; [`CameraCache::clear`] drops every row.
//!
//! Positions are stored in the server's `GCJ-02` frame so the cache stays
//! authoritative for the core; callers convert to WGS-84 at their own boundary.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::{MotuError, Result};
use crate::geo::LatLng;
use crate::model::TrafficCamera;
use crate::areas::MapAreaSnapshot;
use crate::official::CameraDetail;

/// One cached camera point plus when it was first/last seen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedCamera {
    pub camera: TrafficCamera,
    #[serde(default)]
    pub city_code: String,
    pub first_seen_ms: i64,
    pub last_synced_ms: i64,
}

/// A cached detail payload; the position inside is still `GCJ-02`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedDetail {
    pub detail: CameraDetail,
    pub fetched_at_ms: i64,
}

/// One recorded official-sync area. The bounds are the extent of the points the
/// server actually returned (in `GCJ-02`), so later viewports inside it can be
/// answered locally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRegion {
    pub city_code: String,
    pub bounds: CameraBounds,
    pub synced_at_ms: i64,
}

/// A cached image body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedImage {
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub fetched_at_ms: i64,
}

/// Row counts and total image size, used by `/api/cache`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub cameras: i64,
    pub details: i64,
    pub images: i64,
    pub image_bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_camera_sync_ms: Option<i64>,
}

/// Rectangular WGS-free bounds in the cache's own (`GCJ-02`) coordinate frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraBounds {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

impl CameraBounds {
    #[must_use]
    pub fn new(south: f64, west: f64, north: f64, east: f64) -> Self {
        Self {
            south: south.min(north),
            west: west.min(east),
            north: south.max(north),
            east: west.max(east),
        }
    }

    #[must_use]
    pub fn is_valid(self) -> bool {
        self.south >= -90.0
            && self.north <= 90.0
            && self.west >= -180.0
            && self.east <= 180.0
            && self.south < self.north
            && self.west < self.east
    }
}

/// The cache holds third-party ticket photos and contact details, so keep it
/// private to the local user account.
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn db_error(error: impl std::fmt::Display) -> MotuError {
    MotuError::Store(format!("sqlite: {error}"))
}

/// SQLite-backed cache. One connection is cheap; the web layer opens a short
/// lived handle per request so no connection crosses an await point.
pub struct CameraCache {
    conn: Connection,
}

impl CameraCache {
    /// Open (or create) the cache at `path`, creating parent directories.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).map_err(db_error)?;
        restrict_permissions(path);
        Self::initialise(conn)
    }

    /// In-memory cache for tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(db_error)?;
        Self::initialise(conn)
    }

    fn initialise(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS cameras (
                 cache_key     TEXT PRIMARY KEY,
                 wire_id       TEXT,
                 latitude      REAL NOT NULL,
                 longitude     REAL NOT NULL,
                 city_code     TEXT NOT NULL DEFAULT '',
                 payload       TEXT NOT NULL,
                 first_seen_ms INTEGER NOT NULL,
                 last_synced_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS cameras_position
                 ON cameras(latitude, longitude);
             CREATE TABLE IF NOT EXISTS camera_details (
                 wire_id       TEXT PRIMARY KEY,
                 payload       TEXT NOT NULL,
                 fetched_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sync_regions (
                 id           INTEGER PRIMARY KEY AUTOINCREMENT,
                 city_code    TEXT NOT NULL,
                 south        REAL NOT NULL,
                 west         REAL NOT NULL,
                 north        REAL NOT NULL,
                 east         REAL NOT NULL,
                 synced_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS map_areas (
                 id            INTEGER PRIMARY KEY CHECK (id = 1),
                 payload       TEXT NOT NULL,
                 fetched_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS images (
                 url           TEXT PRIMARY KEY,
                 content_type  TEXT NOT NULL,
                 bytes         BLOB NOT NULL,
                 fetched_at_ms INTEGER NOT NULL
             );",
        )
        .map_err(db_error)?;
        Ok(Self { conn })
    }

    /// Insert or refresh camera points and return how many rows were written.
    ///
    /// `first_seen_ms` is preserved on conflict, so a point keeps its original
    /// discovery time across repeated syncs.
    pub fn upsert_cameras(
        &mut self,
        cameras: &[TrafficCamera],
        city_code: &str,
        now_ms: i64,
    ) -> Result<usize> {
        let tx = self.conn.transaction().map_err(db_error)?;
        let mut written = 0;
        for camera in cameras {
            let key = camera.cache_key();
            let payload = serde_json::to_string(camera)?;
            tx.execute(
                "INSERT INTO cameras
                     (cache_key, wire_id, latitude, longitude, city_code, payload,
                      first_seen_ms, last_synced_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT(cache_key) DO UPDATE SET
                     wire_id = excluded.wire_id,
                     latitude = excluded.latitude,
                     longitude = excluded.longitude,
                     city_code = excluded.city_code,
                     payload = excluded.payload,
                     last_synced_ms = excluded.last_synced_ms",
                params![
                    key,
                    camera.member_by_string,
                    camera.position.latitude,
                    camera.position.longitude,
                    city_code,
                    payload,
                    now_ms,
                ],
            )
            .map_err(db_error)?;
            written += 1;
        }
        tx.commit().map_err(db_error)?;
        Ok(written)
    }

    /// Every cached point, newest sync first.
    pub fn cameras(&self) -> Result<Vec<CachedCamera>> {
        self.query_cameras(None)
    }

    /// Cached points inside `bounds`.
    pub fn cameras_in(&self, bounds: CameraBounds) -> Result<Vec<CachedCamera>> {
        self.query_cameras(Some(bounds))
    }

    fn query_cameras(&self, bounds: Option<CameraBounds>) -> Result<Vec<CachedCamera>> {
        let map = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(String, String, i64, i64)> {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        };
        let (sql, parameters): (&str, Vec<f64>) = match bounds {
            Some(bounds) => (
                "SELECT payload, city_code, first_seen_ms, last_synced_ms
                   FROM cameras
                  WHERE latitude BETWEEN ?1 AND ?2
                    AND longitude BETWEEN ?3 AND ?4
                  ORDER BY last_synced_ms DESC, cache_key",
                vec![bounds.south, bounds.north, bounds.west, bounds.east],
            ),
            None => (
                "SELECT payload, city_code, first_seen_ms, last_synced_ms
                   FROM cameras
                  ORDER BY last_synced_ms DESC, cache_key",
                Vec::new(),
            ),
        };
        let mut statement = self.conn.prepare(sql).map_err(db_error)?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(parameters), map)
            .map_err(db_error)?;
        let mut cameras = Vec::new();
        for row in rows {
            let (payload, city_code, first_seen_ms, last_synced_ms) = row.map_err(db_error)?;
            let camera: TrafficCamera = serde_json::from_str(&payload)?;
            cameras.push(CachedCamera {
                camera,
                city_code,
                first_seen_ms,
                last_synced_ms,
            });
        }
        Ok(cameras)
    }

    /// Every cached detail, ordered by wire id (used by the Android export).
    pub fn all_details(&self) -> Result<Vec<(String, CachedDetail)>> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT wire_id, payload, fetched_at_ms
                   FROM camera_details
                  ORDER BY wire_id",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(db_error)?;
        let mut details = Vec::new();
        for row in rows {
            let (wire_id, payload, fetched_at_ms) = row.map_err(db_error)?;
            details.push((
                wire_id,
                CachedDetail {
                    detail: serde_json::from_str(&payload)?,
                    fetched_at_ms,
                },
            ));
        }
        Ok(details)
    }

    /// Cached detail for one wire id, if any.
    pub fn detail(&self, wire_id: &str) -> Result<Option<CachedDetail>> {
        let row = self
            .conn
            .query_row(
                "SELECT payload, fetched_at_ms FROM camera_details WHERE wire_id = ?1",
                params![wire_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        row.map(|(payload, fetched_at_ms)| {
            Ok(CachedDetail {
                detail: serde_json::from_str(&payload)?,
                fetched_at_ms,
            })
        })
        .transpose()
    }

    /// Insert or replace a detail payload.
    pub fn put_detail(&mut self, wire_id: &str, detail: &CameraDetail, now_ms: i64) -> Result<()> {
        let payload = serde_json::to_string(detail)?;
        self.conn
            .execute(
                "INSERT INTO camera_details (wire_id, payload, fetched_at_ms)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(wire_id) DO UPDATE SET
                     payload = excluded.payload,
                     fetched_at_ms = excluded.fetched_at_ms",
                params![wire_id, payload, now_ms],
            )
            .map_err(db_error)?;
        Ok(())
    }

    /// Replace the recorded sync area for `city_code`.
    pub fn record_region(
        &mut self,
        city_code: &str,
        bounds: CameraBounds,
        now_ms: i64,
    ) -> Result<()> {
        let tx = self.conn.transaction().map_err(db_error)?;
        tx.execute(
            "DELETE FROM sync_regions WHERE city_code = ?1",
            params![city_code],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT INTO sync_regions (city_code, south, west, north, east, synced_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                city_code,
                bounds.south,
                bounds.west,
                bounds.north,
                bounds.east,
                now_ms
            ],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(())
    }

    /// Freshest recorded region that fully contains `bounds`.
    pub fn region_covering(&self, bounds: CameraBounds) -> Result<Option<SyncRegion>> {
        let row = self
            .conn
            .query_row(
                "SELECT city_code, south, west, north, east, synced_at_ms
                   FROM sync_regions
                  WHERE south <= ?1 AND west <= ?2 AND north >= ?3 AND east >= ?4
                  ORDER BY synced_at_ms DESC
                  LIMIT 1",
                params![bounds.south, bounds.west, bounds.north, bounds.east],
                |row| {
                    Ok(SyncRegion {
                        city_code: row.get(0)?,
                        bounds: CameraBounds::new(row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?),
                        synced_at_ms: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)?;
        Ok(row)
    }

    /// Store the whole restricted-area layer (single row).
    pub fn put_areas(&mut self, snapshot: &MapAreaSnapshot) -> Result<()> {
        let payload = serde_json::to_string(snapshot)?;
        self.conn
            .execute(
                "INSERT INTO map_areas (id, payload, fetched_at_ms) VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET
                     payload = excluded.payload,
                     fetched_at_ms = excluded.fetched_at_ms",
                params![payload, snapshot.fetched_at_ms],
            )
            .map_err(db_error)?;
        Ok(())
    }

    /// Cached restricted-area layer, if it was fetched before.
    pub fn areas(&self) -> Result<Option<MapAreaSnapshot>> {
        let payload = self
            .conn
            .query_row("SELECT payload FROM map_areas WHERE id = 1", [], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(db_error)?;
        payload
            .map(|text| serde_json::from_str(&text).map_err(MotuError::from))
            .transpose()
    }

    /// Cached image body for an absolute URL.
    pub fn image(&self, url: &str) -> Result<Option<CachedImage>> {
        let row = self
            .conn
            .query_row(
                "SELECT content_type, bytes, fetched_at_ms FROM images WHERE url = ?1",
                params![url],
                |row| {
                    Ok(CachedImage {
                        content_type: row.get(0)?,
                        bytes: row.get(1)?,
                        fetched_at_ms: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)?;
        Ok(row)
    }

    /// Insert or replace an image body.
    pub fn put_image(
        &mut self,
        url: &str,
        content_type: &str,
        bytes: &[u8],
        now_ms: i64,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO images (url, content_type, bytes, fetched_at_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(url) DO UPDATE SET
                     content_type = excluded.content_type,
                     bytes = excluded.bytes,
                     fetched_at_ms = excluded.fetched_at_ms",
                params![url, content_type, bytes, now_ms],
            )
            .map_err(db_error)?;
        Ok(())
    }

    /// Counts used by the debug UI.
    pub fn stats(&self) -> Result<CacheStats> {
        let (cameras, details, images, image_bytes) = self
            .conn
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM cameras),
                     (SELECT COUNT(*) FROM camera_details),
                     (SELECT COUNT(*) FROM images),
                     (SELECT COALESCE(SUM(LENGTH(bytes)), 0) FROM images)",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .map_err(db_error)?;
        let last_camera_sync_ms = self
            .conn
            .query_row("SELECT MAX(last_synced_ms) FROM cameras", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .map_err(db_error)?;
        Ok(CacheStats {
            cameras,
            details,
            images,
            image_bytes,
            last_camera_sync_ms,
        })
    }

    /// Delete every cached row. The database file itself stays.
    pub fn clear(&mut self) -> Result<()> {
        self.conn
            .execute_batch(
                "DELETE FROM cameras;
                 DELETE FROM camera_details;
                 DELETE FROM images;
                 DELETE FROM map_areas;
                 DELETE FROM sync_regions;",
            )
            .map_err(db_error)?;
        Ok(())
    }

    /// Drop cached details only; useful when only detail content looks stale.
    pub fn clear_details(&mut self) -> Result<()> {
        self.conn
            .execute("DELETE FROM camera_details", [])
            .map_err(db_error)?;
        Ok(())
    }
}

/// A cached point converted to WGS-84 for leaflet.
#[must_use]
pub fn wgs84_position(camera: &TrafficCamera) -> LatLng {
    crate::geo::gcj02_to_wgs84(camera.position)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CameraTabType;
    use crate::official::{CameraDetailType, CameraRule, CameraTicket};

    fn camera(id: &str, wire: Option<&str>, latitude: f64, longitude: f64) -> TrafficCamera {
        let mut camera = TrafficCamera::new(
            id,
            LatLng::new(latitude, longitude),
            [CameraTabType::RedLight, CameraTabType::Speeding],
        );
        camera.member_by_string = wire.map(str::to_owned);
        camera.distance_m = Some(12.5);
        camera
    }

    fn detail(id: &str) -> CameraDetail {
        CameraDetail {
            camera_id: id.to_owned(),
            title: "电子摄像头".to_owned(),
            detail: "由东向西抓拍".to_owned(),
            address: "上海市某路口".to_owned(),
            position: Some(LatLng::new(31.230_4, 121.473_7)),
            types: vec![CameraDetailType {
                tab_type: CameraTabType::RedLight,
                name: "闯红灯".to_owned(),
                remark: "红灯亮起后越过停止线".to_owned(),
            }],
            rules: vec![CameraRule {
                plate_name: "外地车牌".to_owned(),
                day_type_names: vec!["工作日".to_owned()],
                tail: vec![1, 2],
                time: "07:00-20:00".to_owned(),
            }],
            tickets_count: 3,
            praise_count: 5,
            comment_count: 2,
            white: false,
            avoid: false,
            avoid_type: 0,
            contact_person: String::new(),
            contact_way: String::new(),
            user_photo: String::new(),
            photo: String::new(),
            create_time: None,
            avoid_id: None,
            avoid_white_id: None,
            tickets: vec![CameraTicket {
                id: 7,
                title: "违章罚单".to_owned(),
                remark: "未按车道行驶".to_owned(),
                photo: "https://example/photo.jpg".to_owned(),
                create_date: Some(1_720_000_000_000),
                contact_person: "车友".to_owned(),
                user_photo: "https://example/avatar.jpg".to_owned(),
                city_code: String::new(),
                contact_way: String::new(),
                eyes_id: 0,
                user_id: 0,
            }],
        }
    }

    #[test]
    fn cameras_round_trip_and_keep_first_seen() {
        let mut cache = CameraCache::open_in_memory().expect("cache");
        let points = vec![
            camera("31.23,121.47:4", Some("wire-1"), 31.23, 121.47),
            camera("31.24,121.48:5", None, 31.24, 121.48),
        ];
        assert_eq!(
            cache.upsert_cameras(&points, "021", 1_000).expect("insert"),
            2
        );
        assert_eq!(
            cache.upsert_cameras(&points, "021", 2_000).expect("update"),
            2
        );
        let cached = cache.cameras().expect("list");
        assert_eq!(cached.len(), 2);
        let first = cached
            .iter()
            .find(|row| row.camera.cache_key() == "wire-1")
            .expect("wire row");
        assert_eq!(first.first_seen_ms, 1_000);
        assert_eq!(first.last_synced_ms, 2_000);
        assert_eq!(first.camera.cache_key(), "wire-1");
        assert_eq!(first.city_code, "021");
    }

    #[test]
    fn bounds_filter_narrows_results() {
        let mut cache = CameraCache::open_in_memory().expect("cache");
        let points = vec![
            camera("31.23,121.47:4", Some("wire-1"), 31.23, 121.47),
            camera("31.40,121.60:5", Some("wire-2"), 31.40, 121.60),
        ];
        cache.upsert_cameras(&points, "021", 1_000).expect("insert");
        let bounds = CameraBounds::new(31.20, 121.44, 31.26, 121.50);
        assert!(bounds.is_valid());
        let cached = cache.cameras_in(bounds).expect("list");
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].camera.cache_key(), "wire-1");
    }

    #[test]
    fn all_details_returns_every_row_in_id_order() {
        let mut cache = CameraCache::open_in_memory().expect("cache");
        cache.put_detail("b", &detail("b"), 2).expect("put");
        cache.put_detail("a", &detail("a"), 1).expect("put");
        let rows = cache.all_details().expect("list");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "a");
        assert_eq!(rows[1].0, "b");
    }

    #[test]
    fn detail_round_trips_with_tickets() {
        let mut cache = CameraCache::open_in_memory().expect("cache");
        assert!(cache.detail("wire-1").expect("empty").is_none());
        let detail = detail("wire-1");
        cache.put_detail("wire-1", &detail, 4_242).expect("put");
        let cached = cache.detail("wire-1").expect("get").expect("row");
        assert_eq!(cached.fetched_at_ms, 4_242);
        assert_eq!(cached.detail, detail);
        assert_eq!(cached.detail.tickets.len(), 1);
    }

    #[test]
    fn images_round_trip_and_count_bytes() {
        let mut cache = CameraCache::open_in_memory().expect("cache");
        cache
            .put_image("https://example/photo.jpg", "image/jpeg", b"jpeg", 5)
            .expect("put");
        let image = cache
            .image("https://example/photo.jpg")
            .expect("get")
            .expect("row");
        assert_eq!(image.content_type, "image/jpeg");
        assert_eq!(image.bytes, b"jpeg");
        let stats = cache.stats().expect("stats");
        assert_eq!(stats.images, 1);
        assert_eq!(stats.image_bytes, 4);
    }

    #[test]
    fn stats_and_clear_reset_every_table() {
        let mut cache = CameraCache::open_in_memory().expect("cache");
        cache
            .upsert_cameras(&[camera("31.23,121.47:4", None, 31.23, 121.47)], "021", 10)
            .expect("insert");
        cache.put_detail("wire-1", &detail("wire-1"), 10).expect("put");
        let stats = cache.stats().expect("stats");
        assert_eq!(stats.cameras, 1);
        assert_eq!(stats.details, 1);
        assert_eq!(stats.last_camera_sync_ms, Some(10));

        cache.clear_details().expect("clear details");
        let stats = cache.stats().expect("stats");
        assert_eq!(stats.details, 0);
        assert_eq!(stats.cameras, 1);

        cache.clear().expect("clear");
        assert_eq!(cache.stats().expect("stats").cameras, 0);
    }

    #[test]
    fn map_areas_round_trip_and_clear() {
        use crate::areas::{MapAreaShape, MapAreaSnapshot};

        let mut cache = CameraCache::open_in_memory().expect("cache");
        assert!(cache.areas().expect("empty").is_none());

        let snapshot = MapAreaSnapshot {
            areas: vec![MapAreaShape {
                id: 1,
                name: "外牌限行".to_owned(),
                shape_type: crate::areas::SHAPE_POLYGON.to_owned(),
                points: vec![LatLng::new(31.20, 121.40), LatLng::new(31.30, 121.50)],
            }],
            version: 3,
            fetched_at_ms: 9_000,
        };
        cache.put_areas(&snapshot).expect("put");
        let cached = cache.areas().expect("get").expect("row");
        assert_eq!(cached, snapshot);

        cache.clear().expect("clear");
        assert!(cache.areas().expect("empty").is_none());
    }

    #[test]
    fn sync_regions_cover_only_contained_viewports() {
        let mut cache = CameraCache::open_in_memory().expect("cache");
        let synced = CameraBounds::new(30.8, 121.0, 31.6, 121.9);

        cache
            .record_region("021", synced, 1_000)
            .expect("record");
        let inner = CameraBounds::new(31.1, 121.3, 31.3, 121.6);
        let region = cache.region_covering(inner).expect("query").expect("hit");
        assert_eq!(region.city_code, "021");
        assert_eq!(region.synced_at_ms, 1_000);

        let outside = CameraBounds::new(31.1, 120.0, 31.3, 120.4);
        assert!(cache.region_covering(outside).expect("query").is_none());

        // A new sync for the same city replaces the old area.
        let smaller = CameraBounds::new(31.0, 121.2, 31.2, 121.5);
        cache
            .record_region("021", smaller, 2_000)
            .expect("replace");
        assert!(
            cache
                .region_covering(CameraBounds::new(31.1, 121.3, 31.15, 121.4))
                .expect("query")
                .is_some()
        );
        assert!(cache.region_covering(inner).expect("query").is_none());

        cache.clear().expect("clear");
        assert!(cache.region_covering(inner).expect("query").is_none());
    }

    #[test]
    fn open_creates_the_parent_directory() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("nested").join("cache.sqlite");
        let cache = CameraCache::open(&path).expect("open");
        assert_eq!(cache.stats().expect("stats").cameras, 0);
        assert!(path.exists());
    }
}
