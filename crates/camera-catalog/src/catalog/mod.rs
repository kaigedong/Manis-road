use std::collections::HashSet;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod model;
pub use model::{CameraRecord, CatalogEvent, CatalogEventKind, Coordinate, EvidenceLevel, Source};

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("解析目录失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("资料校验失败: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub sources: Vec<Source>,
    pub records: Vec<CameraRecord>,
    #[serde(default)]
    pub events: Vec<CatalogEvent>,
}

impl Catalog {
    /// 纯同步解析/验证：原生 CLI 和浏览器 WASM 复用同一规则。
    pub fn parse_str(content: &str) -> Result<Self, CatalogError> {
        let catalog: Self = serde_json::from_str(content)?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<(), CatalogError> {
        let mut source_ids = HashSet::new();
        for source in &self.sources {
            required(&source.id, "来源 ID")?;
            required(&source.title, "来源标题")?;
            required(&source.review_note, "来源审核备注")?;
            check_url(&source.url, "公告链接")?;
            check_date(&source.published_on, "公告日期")?;
            if !source_ids.insert(source.id.as_str()) {
                return Err(invalid(format!("重复的来源 ID: {}", source.id)));
            }
        }
        let mut record_ids = HashSet::new();
        for record in &self.records {
            required(&record.id, "点位 ID")?;
            required(&record.location_text, "地点描述")?;
            required(&record.equipment, "设备类别")?;
            required(&record.review_note, "点位审核备注")?;
            if !record_ids.insert(record.id.as_str()) {
                return Err(invalid(format!("重复的点位 ID: {}", record.id)));
            }
            if !source_ids.contains(record.source_id.as_str()) {
                return Err(invalid(format!("{} 引用不存在的来源", record.id)));
            }
            if let Some(photo_url) = &record.photo_url {
                check_url(photo_url, "照片链接")?;
            }
            if let Some(date) = &record.photo_taken_on {
                if record.photo_url.is_none() {
                    return Err(invalid(format!("{} 没有照片，不能填写拍摄日期", record.id)));
                }
                check_date(date, "照片拍摄日期")?;
            }
            if let Some(point) = &record.coordinate {
                // 上海及临近道路的粗略防误填范围；不能作为行政区判定依据。
                if !point.longitude.is_finite()
                    || !point.latitude.is_finite()
                    || !(120.8..=122.3).contains(&point.longitude)
                    || !(30.6..=31.9).contains(&point.latitude)
                    || point.system != "gcj02"
                    || point.reference.trim().is_empty()
                {
                    return Err(invalid(format!("{} 坐标超出试点范围或缺少依据", record.id)));
                }
            }
            if matches!(record.evidence_level, EvidenceLevel::Reviewed)
                && record.coordinate.is_none()
            {
                return Err(invalid(format!("{} 标记已审核但缺少精确坐标", record.id)));
            }
        }
        let mut event_ids = HashSet::new();
        for event in &self.events {
            if !event_ids.insert(event.id.as_str()) {
                return Err(invalid(format!("重复的事件 ID: {}", event.id)));
            }
            if !source_ids.contains(event.source_id.as_str()) {
                return Err(invalid(format!("事件 {} 引用不存在的来源", event.id)));
            }
            if let Some(record_id) = &event.record_id
                && !record_ids.contains(record_id.as_str())
            {
                return Err(invalid(format!("事件 {} 引用不存在的点位", event.id)));
            }
        }
        Ok(())
    }

    /// 地图只展示人工审核且有坐标的记录；不能从空结果推断区域没有监控。
    pub fn map_points(&self) -> Vec<&CameraRecord> {
        self.records
            .iter()
            .filter(|item| {
                matches!(item.evidence_level, EvidenceLevel::Reviewed) && item.coordinate.is_some()
            })
            .collect()
    }
}

fn invalid(message: String) -> CatalogError {
    CatalogError::Invalid(message)
}

fn required(value: &str, field: &str) -> Result<(), CatalogError> {
    if value.trim().is_empty() {
        return Err(invalid(format!("{field}不能为空")));
    }
    Ok(())
}

fn check_url(value: &str, field: &str) -> Result<(), CatalogError> {
    if !value.starts_with("https://") || value.contains(char::is_whitespace) {
        return Err(invalid(format!("{field}须为 HTTPS 链接")));
    }
    Ok(())
}

fn check_date(value: &str, field: &str) -> Result<(), CatalogError> {
    if value.len() != 10 || NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err() {
        return Err(invalid(format!("{field}须为有效 YYYY-MM-DD 日期")));
    }
    Ok(())
}
