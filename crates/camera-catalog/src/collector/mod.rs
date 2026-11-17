//! 公告采集器（仅原生环境）：保留快照哈希与采集事件，候选点位仍需人工审核。
use std::collections::HashSet;
use std::time::Duration;

use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::catalog::{
    CameraRecord, Catalog, CatalogError, CatalogEvent, CatalogEventKind, EvidenceLevel, Source,
};

const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum CollectorError {
    #[error("目录错误: {0}")]
    Catalog(#[from] CatalogError),
    #[error("未找到公告来源: {0}")]
    UnknownSource(String),
    #[error("仅允许上海政府官方公告的 HTTPS 链接")]
    DisallowedUrl,
    #[error("请求公告失败: {0}")]
    Request(#[from] reqwest::Error),
    #[error("公告 HTTP 状态码错误: {0}")]
    Http(reqwest::StatusCode),
    #[error("公告正文超过 2 MiB")]
    TooLarge,
    #[error("公告正文不是 UTF-8，需人工转换后用 collect-file 导入")]
    InvalidEncoding,
}

/// 从政府公告提取 HTML 原文（不跟随重定向、不抓图、不请求任意第三方链接）。
pub async fn fetch_official(source: &Source) -> Result<String, CollectorError> {
    let url = official_url(source)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("ManisRoad-NoticeCollector/0.1 (public announcements only)")
        .build()?;
    let mut response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(CollectorError::Http(response.status()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_HTML_BYTES as u64)
    {
        return Err(CollectorError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > MAX_HTML_BYTES - body.len() {
            return Err(CollectorError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| CollectorError::InvalidEncoding)
}

fn official_url(source: &Source) -> Result<reqwest::Url, CollectorError> {
    let url = reqwest::Url::parse(&source.url).map_err(|_| CollectorError::DisallowedUrl)?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("www.shanghai.gov.cn" | "gaj.sh.gov.cn")
        )
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(CollectorError::DisallowedUrl);
    }
    Ok(url)
}

/// 输入可信来源 ID 与获取到的 HTML（在线或离线快照），原子地更新内存中的目录。
/// 返回本轮事件；只对 HTML 表格中带“设备设置地点”等明确表头的行建待核验记录。
pub fn collect_snapshot(
    catalog: &mut Catalog,
    source_id: &str,
    html: &str,
    at_unix_ms: u64,
) -> Result<Vec<CatalogEvent>, CollectorError> {
    if html.len() > MAX_HTML_BYTES {
        return Err(CollectorError::TooLarge);
    }
    let source = catalog
        .sources
        .iter()
        .find(|source| source.id == source_id)
        .ok_or_else(|| CollectorError::UnknownSource(source_id.to_owned()))?;
    let source_title = source.title.clone();
    let digest = snapshot_sha256(html);
    let mut updated = catalog.clone();
    let last_digest = catalog.events.iter().rev().find(|event| {
        event.source_id == source_id && event.kind == CatalogEventKind::SourceObserved
    });
    let mut events = Vec::new();
    let mut event = |kind: CatalogEventKind, record_id: Option<String>, detail: String| {
        events.push(CatalogEvent {
            id: format!(
                "event-{}-{}",
                at_unix_ms,
                catalog.events.len() + events.len()
            ),
            source_id: source_id.to_owned(),
            kind,
            record_id,
            snapshot_sha256: digest.clone(),
            recorded_at_unix_ms: at_unix_ms,
            detail,
        });
    };
    event(
        CatalogEventKind::SourceObserved,
        None,
        "已取得公告 HTML 快照".into(),
    );
    if last_digest.is_some_and(|last| last.snapshot_sha256 != digest) {
        event(
            CatalogEventKind::SourceChanged,
            None,
            "公告正文发生变化，需重新核验".into(),
        );
    }
    let (locations, found_table) = extract_location_rows(html);
    let extracted_any = !locations.is_empty();
    let mut seen: HashSet<String> = updated
        .records
        .iter()
        .filter(|record| record.source_id == source_id)
        .map(|record| record.location_text.clone())
        .collect();
    for (location, equipment) in locations {
        if !seen.insert(location.clone()) {
            continue;
        }
        let row_id = snapshot_sha256(&format!("{source_id}\0{location}"));
        let id = format!("{source_id}-{}", &row_id[..16]);
        updated.records.push(CameraRecord {
            id: id.clone(),
            source_id: source_id.to_owned(),
            location_text: location,
            equipment: equipment.unwrap_or_else(|| "交通技术监控设备（待核验类型）".into()),
            applies_to_motorcycle: None,
            coordinate: None,
            photo_url: None,
            photo_taken_on: None,
            evidence_level: EvidenceLevel::Candidate,
            review_note: format!(
                "自动提取自《{source_title}》的 HTML 表格；地点/设备类型均待人工对照公告核验"
            ),
        });
        event(
            CatalogEventKind::CandidateDiscovered,
            Some(id),
            "新增待核验候选地点".into(),
        );
    }
    if !extracted_any && last_digest.is_none_or(|last| last.snapshot_sha256 != digest) {
        let detail = if found_table {
            "已找到地点表头，但未提取到可用行；需人工核对"
        } else {
            "没有可识别地点表格（可能为图片表格），需人工处理"
        };
        event(CatalogEventKind::NeedsManualReview, None, detail.into());
    }
    updated.events.extend(events.iter().cloned());
    updated.validate()?;
    *catalog = updated;
    Ok(events)
}

pub fn snapshot_sha256(html: &str) -> String {
    format!("{:x}", Sha256::digest(html.as_bytes()))
}

fn extract_location_rows(html: &str) -> (Vec<(String, Option<String>)>, bool) {
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let mut tables = Vec::new();
    descend_named(&dom.document, "table", &mut tables);
    let mut output = Vec::new();
    let mut found_table = false;
    for table in tables {
        let mut rows = Vec::new();
        descend_named(&table, "tr", &mut rows);
        let mut location_column = None;
        let mut equipment_column = None;
        for row in rows {
            let cells: Vec<String> = row
                .children
                .borrow()
                .iter()
                .filter(|child| is_named(child, "td") || is_named(child, "th"))
                .map(text_of)
                .collect();
            if let Some(column) = cells.iter().position(|value| {
                matches!(
                    value.as_str(),
                    "设备设置地点" | "设置地点" | "安装地点" | "地点"
                )
            }) {
                location_column = Some(column);
                equipment_column = cells.iter().position(|value| {
                    matches!(value.as_str(), "设备类型" | "抓拍类型" | "监控类型")
                });
                found_table = true;
                continue;
            }
            let Some(column) = location_column else {
                continue;
            };
            let Some(location) = cells.get(column) else {
                continue;
            };
            if location.is_empty()
                || location.len() > 256
                || !["路", "街", "桥", "隧道", "大道"]
                    .iter()
                    .any(|term| location.contains(term))
            {
                continue;
            }
            let equipment = equipment_column.and_then(|index| cells.get(index).cloned());
            output.push((location.clone(), equipment.filter(|text| !text.is_empty())));
        }
    }
    (output, found_table)
}

fn descend_named(node: &Handle, name: &str, output: &mut Vec<Handle>) {
    if is_named(node, name) {
        output.push(node.clone());
    }
    for child in node.children.borrow().iter() {
        descend_named(child, name, output);
    }
}

fn is_named(node: &Handle, tag: &str) -> bool {
    matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == tag)
}

fn text_of(node: &Handle) -> String {
    fn append_text(node: &Handle, output: &mut String) {
        if let NodeData::Text { contents } = &node.data {
            output.push_str(&contents.borrow());
        }
        for child in node.children.borrow().iter() {
            append_text(child, output);
        }
    }
    let mut text = String::new();
    append_text(node, &mut text);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_official_hosts_are_fetchable() {
        let mut source = Source {
            id: "s".into(),
            title: "公告".into(),
            url: "https://www.shanghai.gov.cn/nw17239/notice.html".into(),
            published_on: "2025-08-28".into(),
            review_note: "test".into(),
        };
        assert!(official_url(&source).is_ok());
        for url in [
            "http://www.shanghai.gov.cn/notice",
            "https://www.shanghai.gov.cn.evil.example/notice",
            "https://www.shanghai.gov.cn:8443/notice",
            "https://user@www.shanghai.gov.cn/notice",
            "https://127.0.0.1/notice",
        ] {
            source.url = url.into();
            assert!(
                matches!(official_url(&source), Err(CollectorError::DisallowedUrl)),
                "{url}"
            );
        }
    }
}
