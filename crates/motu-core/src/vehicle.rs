//! Motorcycle profile model, ported from `UserVehicleInfo` and the vehicle form
//! logic in `VehicleFormDialogFragment`.
//!
//! Recovered enumerations:
//! - `plateColor`: 1 = YELLOW, 2 = BLUE (0 = unknown);
//! - `powerType`: 1 = petrol, 2 = electric, 3 = hybrid;
//! - `transType`: 1 = automatic, 2 = manual, 3 = semi-automatic.

use serde::{Deserialize, Serialize};

use crate::error::{MotuError, Result};

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlateColor {
    Unknown = 0,
    Yellow = 1,
    Blue = 2,
}

impl PlateColor {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Yellow,
            2 => Self::Blue,
            _ => Self::Unknown,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }

    /// Port of `UserVehicleInfo.colorIntToStr`.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Yellow => "YELLOW",
            Self::Blue => "BLUE",
            Self::Unknown => "",
        }
    }

    /// Port of `UserVehicleInfo.colorStrToInt`.
    #[must_use]
    pub fn from_wire_str(value: &str) -> Self {
        match value {
            "YELLOW" => Self::Yellow,
            "BLUE" => Self::Blue,
            _ => Self::Unknown,
        }
    }
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerType {
    Unknown = -1,
    Petrol = 1,
    Electric = 2,
    Hybrid = 3,
}

impl PowerType {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Petrol,
            2 => Self::Electric,
            3 => Self::Hybrid,
            _ => Self::Unknown,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransType {
    Unknown = -1,
    Automatic = 1,
    Manual = 2,
    SemiAutomatic = 3,
}

impl TransType {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Automatic,
            2 => Self::Manual,
            3 => Self::SemiAutomatic,
            _ => Self::Unknown,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        self as i32
    }
}

/// Motorcycle profile. Mirrors `UserVehicleInfo` plus `cityCode` from
/// `UserVehicleInfoResult`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vehicle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// Deprecated in the original model; kept for old local records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city_license_plate_id: Option<String>,
    pub number_plate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub describe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displacement: Option<String>,
    #[serde(default = "PlateColor::unknown_id")]
    pub plate_color: i32,
    #[serde(default = "PowerType::unknown_id")]
    pub power_type: i32,
    #[serde(default = "TransType::unknown_id")]
    pub trans_type: i32,
    #[serde(default)]
    pub r#type: i32,
    #[serde(default)]
    pub is_default: bool,
}

impl PlateColor {
    const fn unknown_id() -> i32 {
        Self::Unknown.id()
    }
}

impl PowerType {
    const fn unknown_id() -> i32 {
        Self::Unknown.id()
    }
}

impl TransType {
    const fn unknown_id() -> i32 {
        Self::Unknown.id()
    }
}

impl Vehicle {
    #[must_use]
    pub fn new(number_plate: impl Into<String>, color: PlateColor) -> Self {
        Self {
            id: None,
            city_license_plate_id: None,
            number_plate: number_plate.into(),
            city_code: None,
            model: None,
            describe: None,
            displacement: None,
            plate_color: color.id(),
            power_type: PowerType::Unknown.id(),
            trans_type: TransType::Unknown.id(),
            r#type: 0,
            is_default: false,
        }
    }

    #[must_use]
    pub const fn color(&self) -> PlateColor {
        PlateColor::from_id(self.plate_color)
    }

    #[must_use]
    pub const fn power(&self) -> PowerType {
        PowerType::from_id(self.power_type)
    }

    #[must_use]
    pub const fn transmission(&self) -> TransType {
        TransType::from_id(self.trans_type)
    }

    /// Build from the `/userCarListSec` wire item.
    #[must_use]
    pub fn from_wire(
        id: Option<i64>,
        plate: impl Into<String>,
        color: &str,
        displacement: Option<String>,
        power_type: Option<i32>,
        trans_type: Option<i32>,
        is_default: Option<i64>,
    ) -> Self {
        Self {
            id,
            city_license_plate_id: None,
            number_plate: plate.into(),
            city_code: None,
            model: None,
            describe: None,
            displacement,
            plate_color: PlateColor::from_wire_str(color).id(),
            power_type: power_type
                .map_or(PowerType::Unknown, PowerType::from_id)
                .id(),
            trans_type: trans_type
                .map_or(TransType::Unknown, TransType::from_id)
                .id(),
            r#type: 0,
            is_default: is_default == Some(1),
        }
    }

    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Validate before saving or sending to the route API.
    ///
    /// Returns a report rather than a bool so the UI can show every problem.
    #[must_use]
    pub fn validate(&self) -> VehicleValidation {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        let normalized = normalize_plate(&self.number_plate);
        match parse_plate(&normalized) {
            Ok(_) => {}
            Err(reason) => errors.push(reason),
        }
        if self.color() == PlateColor::Unknown {
            warnings.push("号牌颜色未知，限行与避让规则可能不准确".to_owned());
        }
        if self.power() == PowerType::Unknown {
            warnings.push("动力类型未知".to_owned());
        }
        if self.transmission() == TransType::Unknown {
            warnings.push("变速类型未知".to_owned());
        }
        if let Some(displacement) = self.displacement.as_deref() {
            if !displacement.is_empty() && !displacement.chars().all(|c| c.is_ascii_digit()) {
                warnings.push("排量应只包含数字".to_owned());
            }
        }

        VehicleValidation {
            valid: errors.is_empty(),
            errors,
            warnings,
        }
    }

    /// Port of `UserVehicleInfo.findDefault`: first default, else first entry.
    #[must_use]
    pub fn find_default(vehicles: &[Self]) -> Option<&Self> {
        vehicles
            .iter()
            .find(|vehicle| vehicle.is_default)
            .or_else(|| vehicles.first())
    }

    /// Port of `UserVehicleInfo.normalizeDefault`.
    pub fn normalize_default(vehicles: &mut [Self]) {
        let Some(index) = vehicles
            .iter()
            .position(|vehicle| vehicle.is_default)
            .or(if vehicles.is_empty() { None } else { Some(0) })
        else {
            return;
        };
        for (position, vehicle) in vehicles.iter_mut().enumerate() {
            vehicle.is_default = position == index;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VehicleValidation {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl VehicleValidation {
    /// Return an error listing every validation failure.
    ///
    /// # Errors
    /// Returns [`MotuError::Vehicle`] when any field is invalid.
    pub fn require_valid(&self) -> Result<()> {
        if self.valid {
            Ok(())
        } else {
            Err(MotuError::Vehicle(self.errors.join("; ")))
        }
    }
}

/// Uppercase and remove separators from a plate.
#[must_use]
pub fn normalize_plate(plate: &str) -> String {
    plate
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '·')
        .flat_map(char::to_uppercase)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlateParts {
    /// Province abbreviation, for example `沪`.
    pub province: String,
    /// City letter, for example `A`.
    pub city_code: String,
    /// Remaining number, normally five or six characters.
    pub number: String,
}

/// Split a plate the way `VehicleFormDialogFragment` fills the editor.
///
/// # Errors
/// Returns a human-readable reason when the plate length, province marker or
/// number characters are invalid.
pub fn parse_plate(plate: &str) -> std::result::Result<PlateParts, String> {
    let normalized = normalize_plate(plate);
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() < 7 || chars.len() > 8 {
        return Err("车牌长度应为 7 或 8 位".to_owned());
    }
    if !chars[0].is_alphabetic() || !('一'..='鿿').contains(&chars[0]) {
        return Err("车牌第一位应为省份简称".to_owned());
    }
    if !chars[1].is_ascii_alphabetic() {
        return Err("车牌第二位应为城市字母".to_owned());
    }
    if chars[2..].iter().any(|c| !c.is_ascii_alphanumeric()) {
        return Err("车牌号部分只能包含字母和数字".to_owned());
    }
    Ok(PlateParts {
        province: chars[0].to_string(),
        city_code: chars[1].to_string(),
        number: chars[2..].iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_conversion_matches_recovered_helpers() {
        assert_eq!(PlateColor::from_wire_str("YELLOW"), PlateColor::Yellow);
        assert_eq!(PlateColor::from_wire_str("BLUE"), PlateColor::Blue);
        assert_eq!(PlateColor::from_wire_str("GREEN"), PlateColor::Unknown);
        assert_eq!(PlateColor::Yellow.as_wire_str(), "YELLOW");
        assert_eq!(PlateColor::Blue.id(), 2);
    }

    #[test]
    fn enums_map_unknown_values() {
        assert_eq!(PowerType::from_id(3), PowerType::Hybrid);
        assert_eq!(PowerType::from_id(9), PowerType::Unknown);
        assert_eq!(TransType::from_id(2), TransType::Manual);
        assert_eq!(TransType::from_id(0), TransType::Unknown);
    }

    #[test]
    fn from_wire_maps_fields() {
        let vehicle = Vehicle::from_wire(
            Some(7),
            "沪A12345",
            "YELLOW",
            Some("250".to_owned()),
            Some(1),
            Some(2),
            Some(1),
        );
        assert_eq!(vehicle.id, Some(7));
        assert_eq!(vehicle.color(), PlateColor::Yellow);
        assert_eq!(vehicle.power(), PowerType::Petrol);
        assert_eq!(vehicle.transmission(), TransType::Manual);
        assert!(vehicle.is_default);
    }

    #[test]
    fn find_default_falls_back_to_first() {
        let mut vehicles = vec![
            Vehicle::new("沪A12345", PlateColor::Yellow),
            Vehicle::new("沪B54321", PlateColor::Blue),
        ];
        assert_eq!(
            Vehicle::find_default(&vehicles).unwrap().number_plate,
            "沪A12345"
        );
        vehicles[1].is_default = true;
        assert_eq!(
            Vehicle::find_default(&vehicles).unwrap().number_plate,
            "沪B54321"
        );
        assert!(Vehicle::find_default(&[]).is_none());
    }

    #[test]
    fn normalize_default_keeps_single_default() {
        let mut vehicles = vec![
            Vehicle::new("沪A12345", PlateColor::Yellow),
            Vehicle::new("沪B54321", PlateColor::Blue),
            Vehicle::new("沪C11111", PlateColor::Yellow),
        ];
        vehicles[2].is_default = true;
        Vehicle::normalize_default(&mut vehicles);
        assert!(!vehicles[0].is_default);
        assert!(!vehicles[1].is_default);
        assert!(vehicles[2].is_default);
    }

    #[test]
    fn plate_parsing_matches_editor_split() {
        let parts = parse_plate("沪A12345").unwrap();
        assert_eq!(parts.province, "沪");
        assert_eq!(parts.city_code, "A");
        assert_eq!(parts.number, "12345");

        let new_energy = parse_plate("沪AD12345").unwrap();
        assert_eq!(new_energy.number, "D12345");

        assert!(parse_plate("沪A1234").is_err());
        assert!(parse_plate("A123456").is_err());
    }

    #[test]
    fn validation_reports_errors_and_warnings() {
        let mut vehicle = Vehicle::new("沪A12345", PlateColor::Yellow);
        let report = vehicle.validate();
        assert!(report.valid);
        assert_eq!(report.warnings.len(), 2);

        vehicle.number_plate = "1234567".to_owned();
        assert!(!vehicle.validate().valid);
        assert!(vehicle.validate().require_valid().is_err());
    }

    #[test]
    fn normalize_plate_handles_separators() {
        assert_eq!(normalize_plate("沪A·12345"), "沪A12345");
        assert_eq!(normalize_plate("沪 a 12345"), "沪A12345");
    }
}
