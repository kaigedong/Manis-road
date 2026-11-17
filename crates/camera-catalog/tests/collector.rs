use manis_camera_catalog::{Catalog, CatalogEventKind, collector::collect_snapshot};

const SOURCE: &str = "xuhui-20250828";
const HTML: &str = r#"<html><body><table><tr><th>序号</th><th>设备设置地点</th><th>设备类型</th></tr>
<tr><td>1</td><td>罗秀路出龙州路东约160米</td><td>固定式交通技术监控设备</td></tr>
<tr><td>2</td><td>龙川北路柳南路南约5米</td><td>电子警察</td></tr></table></body></html>"#;

fn fixture() -> Catalog {
    Catalog::parse_str(include_str!("../data/shanghai-cameras.json")).expect("test catalog")
}

#[test]
fn extracts_only_new_candidate_locations_and_emits_events() {
    let mut catalog = fixture();
    let events = collect_snapshot(&mut catalog, SOURCE, HTML, 123).expect("collect");
    assert_eq!(events[0].kind, CatalogEventKind::SourceObserved);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == CatalogEventKind::CandidateDiscovered)
            .count(),
        0
    );
    // 已在目录中的两条候选不能重复加入；新公告快照仍是一条真实采集事件。
    assert_eq!(catalog.records.len(), 3);
    assert!(catalog.map_points().is_empty());
    assert_eq!(catalog.events.len(), events.len());
}

#[test]
fn new_row_is_collected_once_and_a_changed_snapshot_is_reported() {
    let mut catalog = fixture();
    let first = collect_snapshot(&mut catalog, SOURCE, HTML, 123).expect("first snapshot");
    assert_eq!(first.len(), 1);
    let revised = HTML.replace(
        "</table>",
        "<tr><td>3</td><td>柳州路田林东路东约280米</td><td>固定式交通技术监控设备</td></tr></table>",
    );
    let events = collect_snapshot(&mut catalog, SOURCE, &revised, 456).expect("updated snapshot");
    assert!(
        events
            .iter()
            .any(|event| event.kind == CatalogEventKind::SourceChanged)
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == CatalogEventKind::CandidateDiscovered)
            .count(),
        1
    );
    assert_eq!(catalog.records.len(), 4);
    assert!(catalog.map_points().is_empty());
    let repeat = collect_snapshot(&mut catalog, SOURCE, &revised, 789).expect("repeat");
    assert_eq!(repeat.len(), 1);
    assert_eq!(repeat[0].kind, CatalogEventKind::SourceObserved);
    assert_eq!(catalog.records.len(), 4);
}

#[test]
fn image_only_notice_requires_human_review_and_unknown_source_is_rejected() {
    let mut catalog = fixture();
    let html = r#"<html><body><img src="table.png" alt="具体点位表"></body></html>"#;
    let events = collect_snapshot(&mut catalog, "baoshan-20260107", html, 123).expect("snapshot");
    assert!(
        events
            .iter()
            .any(|event| event.kind == CatalogEventKind::NeedsManualReview)
    );
    assert_eq!(catalog.records.len(), 3);
    let previous = catalog.events.len();
    assert!(collect_snapshot(&mut catalog, "unknown", html, 456).is_err());
    assert_eq!(catalog.events.len(), previous);
}

#[test]
fn empty_location_table_is_flagged_for_review() {
    let mut catalog = fixture();
    let html = "<table><tr><th>设备设置地点</th></tr><tr><td>未知</td></tr></table>";
    let events = collect_snapshot(&mut catalog, SOURCE, html, 555).expect("snapshot");
    assert!(
        events
            .iter()
            .any(|event| event.kind == CatalogEventKind::NeedsManualReview)
    );
    assert_eq!(catalog.records.len(), 3);
}
