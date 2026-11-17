//! Time-of-day traffic priors.
//!
//! A route calculated at 08:30 and at 03:00 must be able to differ, otherwise
//! "continuous re-planning" is just recomputing the same answer. This module
//! supplies that time dependence, and — equally important — keeps its
//! *provenance* explicit so an assumed prior can never be mistaken for a
//! measured one.
//!
//! The prior is a data file shipped in the offline package:
//!
//! ```json
//! {
//!   "source": "measured",
//!   "buckets": [
//!     { "label": "早高峰", "startHour": 7, "endHour": 9,
//!       "factors": { "trunk": 1.9, "primary": 1.8, "secondary": 1.4,
//!                    "tertiary": 1.15, "residential": 1.05, "other": 1.1 } }
//!   ]
//! }
//! ```
//!
//! Factors multiply the edge cost, so > 1 means "this class is slower right
//! now". Buckets may wrap midnight (`startHour` > `endHour`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Result, RoutingError};

/// Where the numbers came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PriorSource {
    /// Derived from real observations (traffic API samples or ride tracks).
    Measured,
    /// Hand-set placeholder. Usable for development, never for acceptance.
    Assumption,
}

impl PriorSource {
    /// True when the prior may back the acceptance check.
    #[must_use]
    pub fn is_measured(self) -> bool {
        matches!(self, Self::Measured)
    }
}

/// Per-road-class multipliers for one time window.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassFactors {
    pub trunk: f64,
    pub primary: f64,
    pub secondary: f64,
    pub tertiary: f64,
    pub residential: f64,
    pub other: f64,
}

impl Default for ClassFactors {
    fn default() -> Self {
        Self {
            trunk: 1.0,
            primary: 1.0,
            secondary: 1.0,
            tertiary: 1.0,
            residential: 1.0,
            other: 1.0,
        }
    }
}

impl ClassFactors {
    #[must_use]
    pub fn for_highway(&self, highway: &str) -> f64 {
        match highway {
            "trunk" | "trunk_link" => self.trunk,
            "primary" | "primary_link" => self.primary,
            "secondary" | "secondary_link" => self.secondary,
            "tertiary" | "tertiary_link" => self.tertiary,
            "residential" | "unclassified" | "living_street" | "service" => self.residential,
            _ => self.other,
        }
    }
}

/// One time window. `[startHour, endHour)`; wraps midnight when
/// `startHour > endHour`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeBucket {
    pub label: String,
    pub start_hour: u8,
    pub end_hour: u8,
    pub factors: ClassFactors,
}

impl TimeBucket {
    #[must_use]
    pub fn contains(&self, hour: u8) -> bool {
        if self.start_hour <= self.end_hour {
            hour >= self.start_hour && hour < self.end_hour
        } else {
            hour >= self.start_hour || hour < self.end_hour
        }
    }
}

/// A full day of class factors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficPrior {
    pub source: PriorSource,
    #[serde(default)]
    pub generated_at: Option<String>,
    pub buckets: Vec<TimeBucket>,
}

impl TrafficPrior {
    /// No time dependence at all: every factor is 1.0.
    #[must_use]
    pub fn neutral() -> Self {
        Self {
            source: PriorSource::Assumption,
            generated_at: None,
            buckets: vec![TimeBucket {
                label: "全天".to_owned(),
                start_hour: 0,
                end_hour: 0, // wraps: covers the whole day
                factors: ClassFactors::default(),
            }],
        }
    }

    /// Multiplier for `highway` at `hour`, or 1.0 when no bucket matches.
    #[must_use]
    pub fn factor(&self, hour: u8, highway: &str) -> f64 {
        self.buckets
            .iter()
            .find(|bucket| bucket.contains(hour))
            .map_or(1.0, |bucket| bucket.factors.for_highway(highway))
    }

    /// Load and validate a prior file.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .map_err(|error| RoutingError::Osm(format!("{}: {error}", path.display())))?;
        let prior: Self = serde_json::from_slice(&bytes)
            .map_err(|error| RoutingError::Osm(format!("{}: {error}", path.display())))?;
        prior.validate()?;
        Ok(prior)
    }

    /// Reject priors that would silently produce nonsense routes.
    pub fn validate(&self) -> Result<()> {
        if self.buckets.is_empty() {
            return Err(RoutingError::InvalidGraph(
                "traffic prior has no buckets".to_owned(),
            ));
        }
        for bucket in &self.buckets {
            let factors = [
                bucket.factors.trunk,
                bucket.factors.primary,
                bucket.factors.secondary,
                bucket.factors.tertiary,
                bucket.factors.residential,
                bucket.factors.other,
            ];
            if factors
                .iter()
                .any(|factor| !factor.is_finite() || *factor <= 0.0)
            {
                return Err(RoutingError::InvalidGraph(format!(
                    "bucket {} has a non-positive factor",
                    bucket.label
                )));
            }
            if bucket.start_hour > 23 || bucket.end_hour > 23 {
                return Err(RoutingError::InvalidGraph(format!(
                    "bucket {} has an hour outside 0..=23",
                    bucket.label
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak() -> TrafficPrior {
        TrafficPrior {
            source: PriorSource::Measured,
            generated_at: None,
            buckets: vec![
                TimeBucket {
                    label: "早高峰".to_owned(),
                    start_hour: 7,
                    end_hour: 9,
                    factors: ClassFactors {
                        primary: 1.8,
                        residential: 1.05,
                        ..ClassFactors::default()
                    },
                },
                TimeBucket {
                    label: "夜间".to_owned(),
                    start_hour: 22,
                    end_hour: 6,
                    factors: ClassFactors::default(),
                },
            ],
        }
    }

    #[test]
    fn matches_the_peak_window() {
        let prior = peak();
        assert!((prior.factor(8, "primary") - 1.8).abs() < 1e-9);
        assert!((prior.factor(8, "residential") - 1.05).abs() < 1e-9);
    }

    #[test]
    fn wraps_midnight() {
        let prior = peak();
        assert!((prior.factor(23, "primary") - 1.0).abs() < 1e-9);
        assert!((prior.factor(3, "primary") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn hours_outside_any_bucket_default_to_one() {
        let prior = peak();
        assert!((prior.factor(13, "primary") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_non_positive_factors() {
        let mut prior = peak();
        prior.buckets[0].factors.primary = 0.0;
        assert!(prior.validate().is_err());
    }

    #[test]
    fn neutral_prior_covers_every_hour() {
        let prior = TrafficPrior::neutral();
        for hour in 0..24 {
            assert!((prior.factor(hour, "primary") - 1.0).abs() < 1e-9);
        }
    }
}
