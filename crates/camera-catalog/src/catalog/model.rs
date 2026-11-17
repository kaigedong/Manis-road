use serde::{Deserialize, Serialize};

/// 采集过程的审计事件（不是交通违法抓拍事件）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogEventKind {
    SourceObserved,
    SourceChanged,
    CandidateDiscovered,
    NeedsManualReview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEvent {
    pub id: String,
    pub source_id: String,
    pub kind: CatalogEventKind,
    pub record_id: Option<String>,
    pub snapshot_sha256: String,
    pub recorded_at_unix_ms: u64,
    pub detail: String,
}

/// 公告来源。公告发布日期不是设备启用日期，也不是现场照片拍摄日期。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub title: String,
    pub url: String,
    /// ISO 8601 YYYY-MM-DD
    pub published_on: String,
    pub review_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLevel {
    /// 从检索或转载发现，尚未人工核对原文与具体点位。
    Candidate,
    /// 已核对原始来源及坐标，且保留审核记录。
    Reviewed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coordinate {
    pub longitude: f64,
    pub latitude: f64,
    /// 例如人工核对照片、地面测量；不可用模糊地址猜坐标。
    pub reference: String,
    /// MVP 仅接受 gcj02，避免在高德底图上混用坐标系。
    pub system: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraRecord {
    pub id: String,
    pub source_id: String,
    pub location_text: String,
    /// 公告说明的设备类别；不推断为摩托车专用设备。
    pub equipment: String,
    /// None 表示公告未确认针对摩托车抓拍。
    pub applies_to_motorcycle: Option<bool>,
    pub coordinate: Option<Coordinate>,
    pub photo_url: Option<String>,
    /// 照片实际拍摄日期；不能用公告日期/上传日期代替。
    pub photo_taken_on: Option<String>,
    pub evidence_level: EvidenceLevel,
    pub review_note: String,
}
