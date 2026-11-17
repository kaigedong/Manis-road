//! Motorcycle routing profile: how much each road class and each turn costs.
//!
//! The original router minimised `distance / speed`, which biases every route
//! toward wide, fast arterial roads — the opposite of what a motorcycle rider
//! wants. This module turns those implicit preferences into explicit weights so
//! route quality becomes a tunable, testable property instead of an accident of
//! the speed model.
//!
//! Cost model (see [`RouteProfile::motorcycle_default`]):
//!
//! ```text
//! edge cost = duration_s × class_weight(highway)
//!           + turn_penalty(angle)              // per junction
//! ```
//!
//! All class weights are ≥ 1.0 on purpose: that keeps the A* heuristic
//! (`distance / max_speed`) admissible, so the search stays optimal. Preferring
//! side streets is therefore expressed as a penalty on arterials, never as a
//! discount on small roads.

use serde::{Deserialize, Serialize};

/// Per-road-class multipliers applied to travel time.
///
/// Higher means "avoid unless it saves a lot of time". Trunk roads are the most
/// penalised because they are the ones riders most often want to skip.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassWeights {
    pub trunk: f64,
    pub primary: f64,
    pub secondary: f64,
    pub tertiary: f64,
    pub residential: f64,
    pub other: f64,
}

impl Default for ClassWeights {
    fn default() -> Self {
        Self {
            // Tuned on the Shanghai graph: these values flip the 徐家汇→五角场
            // route from 82% arterial to 6% arterial for +3% distance.
            trunk: 3.6,
            primary: 2.4,
            secondary: 1.5,
            tertiary: 1.1,
            residential: 1.0,
            other: 1.1,
        }
    }
}

impl ClassWeights {
    /// Weight for one OSM `highway` value.
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

/// Extra seconds charged for changing direction at a junction, by how sharp the
/// turn is. Values are seconds, so they compete directly with travel time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnPenalties {
    /// Heading change below this is treated as carrying straight on.
    pub straight_max_deg: f64,
    /// Heading change below this is a gentle lane change.
    pub slight_max_deg: f64,
    /// Heading change below this is a normal turn.
    pub turn_max_deg: f64,
    pub straight_s: f64,
    pub slight_s: f64,
    pub turn_s: f64,
    pub sharp_s: f64,
    pub u_turn_s: f64,
}

impl Default for TurnPenalties {
    fn default() -> Self {
        Self {
            straight_max_deg: 25.0,
            slight_max_deg: 60.0,
            turn_max_deg: 120.0,
            straight_s: 0.0,
            slight_s: 1.5,
            turn_s: 5.0,
            sharp_s: 9.0,
            u_turn_s: 25.0,
        }
    }
}

impl TurnPenalties {
    /// Penalty in seconds for a heading change of `angle_deg` (0..=180).
    #[must_use]
    pub fn for_angle(&self, angle_deg: f64) -> f64 {
        let angle = angle_deg.abs();
        if angle < self.straight_max_deg {
            self.straight_s
        } else if angle < self.slight_max_deg {
            self.slight_s
        } else if angle < self.turn_max_deg {
            self.turn_s
        } else if angle < 170.0 {
            self.sharp_s
        } else {
            self.u_turn_s
        }
    }
}

/// Full cost profile for one routing call.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteProfile {
    pub classes: ClassWeights,
    pub turns: TurnPenalties,
    /// Seconds charged for arriving at a signalised junction.
    ///
    /// Anecdotal-but-consistent: a light costs 10–20 s of waiting plus an
    /// acceleration penalty. Ignoring it is why the old model happily routed
    /// riders down arterials with a light every 200 m.
    pub signal_penalty_s: f64,
}

impl Default for RouteProfile {
    fn default() -> Self {
        Self::motorcycle_default()
    }
}

impl RouteProfile {
    /// The profile the app uses: avoid arterials and expressways, tolerate a
    /// little extra distance to stay on quiet roads, and punish U-turns hard.
    #[must_use]
    pub fn motorcycle_default() -> Self {
        Self {
            classes: ClassWeights::default(),
            turns: TurnPenalties::default(),
            signal_penalty_s: 12.0,
        }
    }

    /// Cost multiplier for an edge on `highway`.
    #[must_use]
    pub fn class_weight(&self, highway: &str) -> f64 {
        self.classes.for_highway(highway)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_streets_are_cheaper_than_arterials() {
        let profile = RouteProfile::motorcycle_default();
        assert!(profile.class_weight("residential") < profile.class_weight("tertiary"));
        assert!(profile.class_weight("tertiary") < profile.class_weight("secondary"));
        assert!(profile.class_weight("secondary") < profile.class_weight("primary"));
        assert!(profile.class_weight("primary") < profile.class_weight("trunk"));
    }

    #[test]
    fn weights_never_discount_below_travel_time() {
        // A weight < 1.0 would make the A* heuristic inadmissible.
        let profile = RouteProfile::motorcycle_default();
        for highway in [
            "trunk",
            "trunk_link",
            "primary",
            "primary_link",
            "secondary",
            "tertiary",
            "residential",
            "unclassified",
            "living_street",
            "service",
            "track",
        ] {
            assert!(
                profile.class_weight(highway) >= 1.0,
                "{highway} must not be discounted"
            );
        }
    }

    #[test]
    fn signals_cost_real_time() {
        let profile = RouteProfile::motorcycle_default();
        assert!(
            profile.signal_penalty_s >= 8.0,
            "a signal must cost more than a gentle turn"
        );
    }

    #[test]
    fn turn_penalty_grows_with_sharpness() {
        let turns = TurnPenalties::default();
        let straight = turns.for_angle(5.0);
        let slight = turns.for_angle(40.0);
        let turn = turns.for_angle(90.0);
        let sharp = turns.for_angle(150.0);
        let u_turn = turns.for_angle(180.0);
        assert!(straight < slight);
        assert!(slight < turn);
        assert!(turn < sharp);
        assert!(sharp < u_turn);
        assert!(u_turn >= 20.0, "U-turns must be strongly discouraged");
    }

    #[test]
    fn angle_is_symmetric() {
        let turns = TurnPenalties::default();
        assert_eq!(turns.for_angle(90.0), turns.for_angle(-90.0));
    }
}
