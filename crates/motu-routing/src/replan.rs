//! Continuous re-planning.
//!
//! A navigator that recomputes on a timer without hysteresis flickers between
//! routes that are almost identical. A navigator that never recomputes strands
//! the rider after one wrong turn. This module owns the decision — *when* to
//! re-plan — and leaves the routing itself to [`crate::route`].
//!
//! Two triggers, exactly as specified:
//!
//! * **Deviation** — the rider is more than `deviation_m` off the route. Imminent
//!   and unconditional, except for a short anti-thrash cooldown.
//! * **Better route** — a candidate found within the look-ahead is at least
//!   `improvement` cheaper *and* materially different (`overlap <= max_overlap`)
//!   *and* the last switch is older than `min_interval`.
//!
//! The overlap test is what stops "re-planning" from meaning "same road,
//! different polyline": if the candidate covers most of the current route, the
//! switch would be invisible to the rider and only costs CPU.

use std::time::{Duration, Instant};

use motu_core::geo::distance_to_polyline_m;
use motu_core::LatLng;

/// Thresholds for the re-planning controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplanPolicy {
    /// Off-route distance that forces a re-plan, in metres.
    pub deviation_m: f64,
    /// How far ahead a better route is allowed to look, in metres.
    ///
    /// The controller does not compute this itself; it documents the bound the
    /// caller must apply when building a [`Candidate`].
    pub lookahead_m: f64,
    /// Minimum fractional saving (0.08 = 8 %) for a better-route switch.
    pub improvement: f64,
    /// Minimum age of the last switch before a better-route switch is allowed.
    pub min_interval: Duration,
    /// A candidate overlapping more than this is "the same route"; do not switch.
    pub max_overlap: f64,
    /// Minimum age of the last switch before a deviation switch is allowed.
    pub deviation_cooldown: Duration,
    /// Route points this close to the candidate polyline count as overlapping.
    pub overlap_tolerance_m: f64,
}

impl Default for ReplanPolicy {
    fn default() -> Self {
        Self {
            deviation_m: 50.0,
            lookahead_m: 2_000.0,
            improvement: 0.08,
            min_interval: Duration::from_secs(15),
            max_overlap: 0.7,
            deviation_cooldown: Duration::from_secs(3),
            overlap_tolerance_m: 50.0,
        }
    }
}

/// Why a re-plan was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplanReason {
    /// The rider left the route.
    Deviation,
    /// A materially better route exists ahead.
    BetterRoute,
}

/// A route computed from the rider's current position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    /// Fraction cheaper than the route being followed (0.12 = 12 % cheaper).
    pub improvement: f64,
    /// Share of the current route the candidate still covers, 0.0..=1.0.
    pub overlap: f64,
}

/// Controller state: only the last switch time and a counter.
#[derive(Debug, Clone, Default)]
pub struct ReplanState {
    last_switch: Option<Instant>,
    switches: u64,
}

impl ReplanState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many times this controller has actually switched routes.
    #[must_use]
    pub fn switches(&self) -> u64 {
        self.switches
    }

    /// Decide whether to re-plan now.
    ///
    /// Returns `None` when the current route should be kept. A returned reason
    /// also records the switch, so callers must actually apply the new route.
    pub fn evaluate(
        &mut self,
        policy: &ReplanPolicy,
        now: Instant,
        deviation_m: f64,
        candidate: Option<Candidate>,
    ) -> Option<ReplanReason> {
        let since_switch = self
            .last_switch
            .map(|last| now.saturating_duration_since(last));

        if deviation_m > policy.deviation_m {
            let cooled_down = since_switch.is_none_or(|age| age >= policy.deviation_cooldown);
            if !cooled_down {
                return None;
            }
            return Some(self.record(now, ReplanReason::Deviation));
        }

        let candidate = candidate?;
        let interval_ok = since_switch.is_none_or(|age| age >= policy.min_interval);
        let enough_gain = candidate.improvement >= policy.improvement;
        let materially_different = candidate.overlap <= policy.max_overlap;
        if interval_ok && enough_gain && materially_different {
            return Some(self.record(now, ReplanReason::BetterRoute));
        }
        None
    }

    fn record(&mut self, now: Instant, reason: ReplanReason) -> ReplanReason {
        self.last_switch = Some(now);
        self.switches += 1;
        reason
    }
}

/// Share of `current` that lies within `tolerance_m` of the `candidate` line.
///
/// Returns 0.0 when either route is empty — an unknown overlap must never be
/// read as "identical routes, keep the old one".
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "counting a few hundred samples; f64 is exact far beyond that"
)]
pub fn route_overlap(current: &[LatLng], candidate: &[LatLng], tolerance_m: f64) -> f64 {
    if current.is_empty() || candidate.is_empty() {
        return 0.0;
    }
    // Sampling keeps this cheap on long routes without changing the verdict.
    let step = (current.len() / 200).max(1);
    let mut covered = 0_usize;
    let mut total = 0_usize;
    for point in current.iter().step_by(step) {
        total += 1;
        if distance_to_polyline_m(*point, candidate).is_some_and(|distance| distance <= tolerance_m)
        {
            covered += 1;
        }
    }
    if total == 0 {
        return 0.0;
    }
    covered as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ReplanPolicy {
        ReplanPolicy::default()
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "keeps the call sites in the same shape as `evaluate`"
    )]
    fn candidate(improvement: f64, overlap: f64) -> Option<Candidate> {
        Some(Candidate {
            improvement,
            overlap,
        })
    }

    #[test]
    fn deviation_forces_a_replan() {
        let mut state = ReplanState::new();
        let now = Instant::now();
        assert_eq!(
            state.evaluate(&policy(), now, 80.0, None),
            Some(ReplanReason::Deviation)
        );
        assert_eq!(state.switches(), 1);
    }

    #[test]
    fn staying_on_route_never_replans() {
        let mut state = ReplanState::new();
        let now = Instant::now();
        assert_eq!(state.evaluate(&policy(), now, 10.0, None), None);
        assert_eq!(state.switches(), 0);
    }

    #[test]
    fn deviation_respects_the_cooldown() {
        let mut state = ReplanState::new();
        let start = Instant::now();
        assert_eq!(
            state.evaluate(&policy(), start, 90.0, None),
            Some(ReplanReason::Deviation)
        );
        // One second later: still off-route, but within the 3 s cooldown.
        assert_eq!(
            state.evaluate(&policy(), start + Duration::from_secs(1), 90.0, None),
            None
        );
        // After the cooldown it is allowed again.
        assert_eq!(
            state.evaluate(&policy(), start + Duration::from_secs(4), 90.0, None),
            Some(ReplanReason::Deviation)
        );
    }

    #[test]
    fn small_saving_is_not_worth_a_switch() {
        let mut state = ReplanState::new();
        assert_eq!(
            state.evaluate(&policy(), Instant::now(), 5.0, candidate(0.05, 0.2)),
            None
        );
    }

    #[test]
    fn near_identical_candidate_is_rejected() {
        let mut state = ReplanState::new();
        // 20 % cheaper, but it covers 90 % of the current route.
        assert_eq!(
            state.evaluate(&policy(), Instant::now(), 5.0, candidate(0.20, 0.9)),
            None
        );
    }

    #[test]
    fn better_route_needs_the_minimum_interval() {
        let mut policy = policy();
        policy.improvement = 0.08;
        let mut state = ReplanState::new();
        let start = Instant::now();
        // First decision is free (no previous switch).
        assert_eq!(
            state.evaluate(&policy, start, 5.0, candidate(0.12, 0.3)),
            Some(ReplanReason::BetterRoute)
        );
        // Five seconds later: same opportunity, but inside the 15 s interval.
        assert_eq!(
            state.evaluate(&policy, start + Duration::from_secs(5), 5.0, candidate(0.12, 0.3)),
            None
        );
        // After the interval it may switch again.
        assert_eq!(
            state.evaluate(&policy, start + Duration::from_secs(20), 5.0, candidate(0.12, 0.3)),
            Some(ReplanReason::BetterRoute)
        );
    }

    #[test]
    fn overlap_of_identical_routes_is_one() {
        let route: Vec<LatLng> = (0..10)
            .map(|step| LatLng::new(31.230, 121.470 + f64::from(step) * 0.0005))
            .collect();
        let overlap = route_overlap(&route, &route, 50.0);
        assert!(overlap > 0.99, "identical routes overlap fully: {overlap}");
    }

    #[test]
    fn overlap_of_disjoint_routes_is_zero() {
        let a: Vec<LatLng> = (0..10)
            .map(|step| LatLng::new(31.230, 121.470 + f64::from(step) * 0.0005))
            .collect();
        let b: Vec<LatLng> = (0..10)
            .map(|step| LatLng::new(31.310, 121.600 + f64::from(step) * 0.0005))
            .collect();
        assert_eq!(route_overlap(&a, &b, 50.0), 0.0);
    }

    #[test]
    fn unknown_overlap_is_zero_not_one() {
        assert_eq!(route_overlap(&[], &[], 50.0), 0.0);
    }
}
