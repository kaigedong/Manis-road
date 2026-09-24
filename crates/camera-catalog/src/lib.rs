//! 可追溯的交通监控资料目录；候选记录不会当作已验证地图点位输出。
mod catalog;
mod model;

pub use catalog::{Catalog, CatalogError};
pub use model::{CameraRecord, Coordinate, EvidenceLevel, Source};
