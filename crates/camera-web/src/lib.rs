//! Rust/WASM 前端：仅预览公告来源和候选点位，不将未审核资料冒充地图定位。
use manis_camera_catalog::{Catalog, CatalogError, EvidenceLevel};
use std::fmt::Write;

pub fn render_catalog_json(json: &str) -> Result<String, CatalogError> {
    let catalog = Catalog::parse_str(json)?;
    Ok(render_catalog(&catalog))
}

fn render_catalog(catalog: &Catalog) -> String {
    let mut html = String::new();
    let _ = write!(
        html,
        "<section class=\"summary\"><h2>资料概览</h2><p>公告来源：{} · 候选记录：{} · 已审核地图点位：{}</p>\
         <p class=\"notice\">当前不是电子眼全量地图。未审核记录不能作为道路无监控的依据。</p></section>",
        catalog.sources.len(),
        catalog.records.len(),
        catalog.map_points().len()
    );
    html.push_str("<section aria-label=\"候选点位\"><h2>待核验记录</h2><ul class=\"records\">");
    for record in &catalog.records {
        let source = catalog
            .sources
            .iter()
            .find(|source| source.id == record.source_id);
        let _ = write!(
            html,
            "<li><h3>{}</h3><p>状态：{} · 摩托车适用性：{}</p><p>照片拍摄日期：{}</p>",
            escape(&record.location_text),
            match record.evidence_level {
                EvidenceLevel::Candidate => "待核验",
                EvidenceLevel::Reviewed => "已审核",
            },
            match record.applies_to_motorcycle {
                Some(true) => "已注明适用",
                Some(false) => "已注明不适用",
                None => "未知",
            },
            record
                .photo_taken_on
                .as_deref()
                .unwrap_or("未知（公告日期不等于拍摄日期）")
        );
        if let Some(source) = source {
            let _ = write!(
                html,
                "<p>来源：<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>（公告日期：{}）</p>",
                escape(&source.url),
                escape(&source.title),
                escape(&source.published_on)
            );
        }
        let _ = write!(
            html,
            "<p>核验备注：{}</p></li>",
            escape(&record.review_note)
        );
    }
    html.push_str("</ul></section><section><h2>待处理公告</h2><ul>");
    for source in &catalog.sources {
        let _ = write!(
            html,
            "<li><a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>：{}</li>",
            escape(&source.url),
            escape(&source.title),
            escape(&source.review_note)
        );
    }
    html.push_str("</ul></section>");
    html
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn render_catalog(json: &str) -> Result<String, JsValue> {
        super::render_catalog_json(json).map_err(|error| JsValue::from_str(&error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escapes_untrusted_announcements() {
        assert_eq!(escape("<&\"'>"), "&lt;&amp;&quot;&#39;&gt;");
    }

    #[test]
    fn renders_actual_candidate_file_without_fake_verified_points() {
        let fixture = include_str!("../../camera-catalog/data/shanghai-cameras.json");
        let html = render_catalog_json(fixture).expect("valid test fixture");
        assert!(html.contains("候选记录：3 · 已审核地图点位：0"));
        assert!(html.contains("照片拍摄日期：未知"));
        assert!(html.contains("罗秀路出龙州路东约160米"));
        assert!(html.contains("宝山新增固定式交通技术监控设备94套"));
    }
}
