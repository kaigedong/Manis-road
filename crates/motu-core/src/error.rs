//! Error types shared by the core modules.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MotuError {
    #[error("invalid coordinate: {0}")]
    InvalidCoordinate(String),
    #[error("cryptographic operation failed: {0}")]
    Crypto(String),
    #[error("invalid API request: {0}")]
    Api(String),
    /// The transport and protocol succeeded, but the official service
    /// returned a business error (`code != 0`).
    #[error("{0}")]
    Server(String),
    #[error("transport failed: {0}")]
    Transport(String),
    #[error("camera scan input is invalid: {0}")]
    Scan(String),
    #[error("route geometry is invalid: {0}")]
    Route(String),
    #[error("vehicle profile is invalid: {0}")]
    Vehicle(String),
    #[error("local store failed: {0}")]
    Store(String),
    #[error("serialisation failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, MotuError>;
