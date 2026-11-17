//! Errors returned by the offline routing crate.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid graph file: {0}")]
    InvalidGraph(String),
    #[error("OSM data is invalid: {0}")]
    Osm(String),
    #[error("origin or destination is not on the road network")]
    SnapFailed,
    #[error("no route found")]
    NoPath,
    #[error("origin or destination coordinate is invalid")]
    InvalidCoordinate,
    #[error("the road graph has not been built")]
    GraphMissing,
}

pub type Result<T> = std::result::Result<T, RoutingError>;
