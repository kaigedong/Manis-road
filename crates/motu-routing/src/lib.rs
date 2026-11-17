//! Offline road graph, router and navigation steps for the Motu client.
//!
//! Everything here runs without a network: the graph is built once from an OSM
//! extract during the preparation phase, stored as a compact binary, and loaded
//! into memory at start-up. Routing uses A* over the directed graph with a
//! motorcycle profile (no expressways, respect oneway/access tags).
//!
//! The graph lives in WGS-84, matching GPS and Leaflet; Motu's official camera
//! points stay in `GCJ-02` and are converted at the display/caching boundary.

// Fallible APIs document their errors in the error enum.
#![allow(clippy::missing_errors_doc)]

pub mod avoid;
pub mod error;
pub mod graph;
pub mod metrics;
pub mod osm;
pub mod profile;
pub mod replan;
pub mod route;
pub mod traffic;

pub use error::{Result, RoutingError};
pub use avoid::{CameraDeterrent, cameras_within};
pub use graph::{GraphNode, RoadGraph, WayInfo};
pub use osm::{OsmBuildReport, build_from_osm_json};
pub use metrics::{ClassDistance, RouteMetrics, measure_edges};
pub use profile::{ClassWeights, RouteProfile, TurnPenalties};
pub use replan::{Candidate, ReplanPolicy, ReplanReason, ReplanState, route_overlap};
pub use route::{RouteContext, RoutePlan, RouteStep, Snap};
pub use traffic::{ClassFactors, PriorSource, TimeBucket, TrafficPrior};
