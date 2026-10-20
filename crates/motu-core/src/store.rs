//! Local persistence for vehicles, settings and user-defined avoid areas.
//!
//! Motu stores this in SQLite/SharedPreferences. The core uses one JSON
//! document with a schema version so the Android layer can back it with
//! `filesDir` and the tests can use a temporary directory.

use std::path::{Path, PathBuf};

use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::LoginUser;
use crate::route::AvoidArea;
use crate::vehicle::Vehicle;

/// Current on-disk schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// User-facing toggles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "default_true")]
    pub camera_alerts_enabled: bool,
    #[serde(default = "default_true")]
    pub avoid_areas_enabled: bool,
    /// Optional endpoint override; empty means the recovered base URL.
    #[serde(default)]
    pub api_base_url: String,
    /// `AMap` city code used by official camera sync; empty falls back to 010.
    #[serde(default = "default_city_code")]
    pub city_code: String,
    /// Camera radius override. `None` uses the recovered 300 m / 500 m rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_radius_override_m: Option<f64>,
}

fn default_city_code() -> String {
    "021".to_owned()
}

const fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            camera_alerts_enabled: true,
            avoid_areas_enabled: true,
            api_base_url: String::new(),
            city_code: default_city_code(),
            camera_radius_override_m: None,
        }
    }
}

/// Signed-in account state. The token is sent in the `token` header and is
/// never derived from the password after login.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSession {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub nick_name: String,
    #[serde(default)]
    pub real_name: String,
    #[serde(default)]
    pub photo: String,
    #[serde(default)]
    pub is_pro: bool,
    #[serde(default)]
    pub mem_end_time_ms: Option<i64>,
    #[serde(default)]
    pub saved_at_ms: i64,
}

impl AuthSession {
    #[must_use]
    pub fn is_logged_in(&self) -> bool {
        !self.token.is_empty()
    }

    #[must_use]
    pub fn from_login(user: &LoginUser, saved_at_ms: i64) -> Self {
        Self {
            token: user.token.clone(),
            user_id: user.user_id,
            phone: user.phone.clone(),
            nick_name: user.nick_name.clone(),
            real_name: user.real_name.clone(),
            photo: user.photo.clone(),
            is_pro: user.is_pro != 0,
            mem_end_time_ms: user.mem_end_time,
            saved_at_ms,
        }
    }
}

/// Everything the core persists locally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreState {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Stable per-install identifier sent as the `deviceId` header.
    #[serde(default)]
    pub device_id: String,
    /// Signed-in account, if any.
    #[serde(default)]
    pub session: AuthSession,
    #[serde(default)]
    pub vehicles: Vec<Vehicle>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub avoid_areas: Vec<AvoidArea>,
    /// Free-form favourite POI ids, matching the recovered favourite feature.
    #[serde(default)]
    pub favorite_poi_ids: Vec<String>,
}

const fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Default for CoreState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            device_id: String::new(),
            session: AuthSession::default(),
            vehicles: Vec::new(),
            settings: Settings::default(),
            avoid_areas: Vec::new(),
            favorite_poi_ids: Vec::new(),
        }
    }
}

impl CoreState {
    /// The vehicle that should be used for route requests.
    #[must_use]
    pub fn default_vehicle(&self) -> Option<&Vehicle> {
        Vehicle::find_default(&self.vehicles)
    }
}

/// JSON-file backed store rooted at a directory the Android app owns.
#[derive(Debug, Clone)]
pub struct FileStore {
    root: PathBuf,
}

impl FileStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn state_path(&self) -> PathBuf {
        self.root.join("motu-core-state.json")
    }

    pub fn ensure_root(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        Ok(())
    }

    /// Load state, creating an empty document when the file does not exist.
    pub fn load(&self) -> Result<CoreState> {
        let path = self.state_path();
        if !path.exists() {
            return Ok(CoreState::default());
        }
        let bytes = std::fs::read(&path)?;
        if bytes.is_empty() {
            return Ok(CoreState::default());
        }
        let mut state: CoreState = serde_json::from_slice(&bytes)?;
        if state.schema_version > SCHEMA_VERSION {
            return Err(crate::error::MotuError::Store(format!(
                "state schema {} is newer than supported {}",
                state.schema_version, SCHEMA_VERSION
            )));
        }
        state.schema_version = SCHEMA_VERSION;
        Ok(state)
    }

    /// Persist state atomically (write temp file, then rename).
    pub fn save(&self, state: &CoreState) -> Result<()> {
        self.ensure_root()?;
        let path = self.state_path();
        let temp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(state)?;
        std::fs::write(&temp, &json)?;
        std::fs::rename(&temp, &path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Load, apply `mutate`, then save.
    pub fn update<F>(&self, mutate: F) -> Result<CoreState>
    where
        F: FnOnce(&mut CoreState),
    {
        let mut state = self.load()?;
        mutate(&mut state);
        Vehicle::normalize_default(&mut state.vehicles);
        self.save(&state)?;
        Ok(state)
    }

    /// Load state and create the per-install `deviceId` if it is missing.
    pub fn ensure_identity(&self) -> Result<CoreState> {
        let mut state = self.load()?;
        if state.device_id.is_empty() {
            state.device_id = generate_device_id();
            self.save(&state)?;
        }
        Ok(state)
    }

    /// Persist a login session.
    pub fn save_session(&self, session: AuthSession) -> Result<CoreState> {
        self.update(|state| state.session = session)
    }

    /// Clear the login session while keeping vehicles and settings.
    pub fn clear_session(&self) -> Result<CoreState> {
        self.update(|state| state.session = AuthSession::default())
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn generate_device_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(32);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::LatLng;
    use crate::vehicle::PlateColor;

    fn temp_store() -> (tempfile::TempDir, FileStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path().join("core"));
        (dir, store)
    }

    #[test]
    fn missing_file_yields_default_state() {
        let (_dir, store) = temp_store();
        let state = store.load().unwrap();
        assert_eq!(state.schema_version, SCHEMA_VERSION);
        assert_eq!(state.vehicles.len(), 0);
        assert!(state.settings.camera_alerts_enabled);
    }

    #[test]
    fn round_trips_state() {
        let (_dir, store) = temp_store();
        let mut state = CoreState::default();
        state
            .vehicles
            .push(Vehicle::new("沪A12345", PlateColor::Yellow));
        state.avoid_areas.push(AvoidArea::polygon(
            1,
            vec![
                LatLng::new(31.0, 121.0),
                LatLng::new(31.0, 121.1),
                LatLng::new(31.1, 121.1),
            ],
        ));
        store.save(&state).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.vehicles.len(), 1);
        assert_eq!(loaded.avoid_areas[0].id, 1);
    }

    #[test]
    fn update_normalizes_default_vehicle() {
        let (_dir, store) = temp_store();
        let state = store
            .update(|state| {
                state
                    .vehicles
                    .push(Vehicle::new("沪A12345", PlateColor::Yellow));
                state
                    .vehicles
                    .push(Vehicle::new("沪B54321", PlateColor::Blue));
                state.vehicles[1].is_default = true;
            })
            .unwrap();
        assert!(!state.vehicles[0].is_default);
        assert!(state.vehicles[1].is_default);
        assert_eq!(state.default_vehicle().unwrap().number_plate, "沪B54321");
    }

    #[test]
    fn newer_schema_is_rejected() {
        let (_dir, store) = temp_store();
        store.ensure_root().unwrap();
        let state = CoreState {
            schema_version: SCHEMA_VERSION + 1,
            ..CoreState::default()
        };
        std::fs::write(store.state_path(), serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn empty_file_is_treated_as_default() {
        let (_dir, store) = temp_store();
        store.ensure_root().unwrap();
        std::fs::write(store.state_path(), b"").unwrap();
        assert_eq!(store.load().unwrap().vehicles.len(), 0);
    }

    #[test]
    fn ensure_identity_creates_a_stable_device_id() {
        let (_dir, store) = temp_store();
        let first = store.ensure_identity().unwrap();
        assert_eq!(first.device_id.len(), 32);
        assert!(first.device_id.bytes().all(|b| b.is_ascii_hexdigit()));
        let second = store.ensure_identity().unwrap();
        assert_eq!(first.device_id, second.device_id);
    }

    #[test]
    fn session_round_trips_and_clears() {
        let (_dir, store) = temp_store();
        let session = AuthSession {
            token: "token-1".to_owned(),
            user_id: Some(7),
            phone: "13800000000".to_owned(),
            is_pro: true,
            ..AuthSession::default()
        };
        store.save_session(session.clone()).unwrap();
        assert_eq!(store.load().unwrap().session, session);
        assert!(store.load().unwrap().session.is_logged_in());
        store.clear_session().unwrap();
        assert!(!store.load().unwrap().session.is_logged_in());
    }
}
