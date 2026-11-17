//! 可追溯的交通监控资料目录；候选记录不会当作已验证地图点位输出。
pub mod catalog;
#[cfg(not(target_arch = "wasm32"))]
pub mod collector;

pub use catalog::{
    CameraRecord, Catalog, CatalogError, CatalogEvent, CatalogEventKind, Coordinate, EvidenceLevel,
    Source,
};
