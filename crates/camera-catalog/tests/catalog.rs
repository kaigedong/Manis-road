use manis_camera_catalog::{
    CameraRecord, Catalog, CatalogError, Coordinate, EvidenceLevel, Source,
};

fn sample() -> Catalog {
    Catalog {
        sources: vec![Source {
            id: "xuhui-20250828".into(),
            title: "徐汇区设备公告".into(),
            url: "https://www.shanghai.gov.cn/example".into(),
            published_on: "2025-08-28".into(),
            review_note: "等待核对官方图片".into(),
        }],
        records: vec![CameraRecord {
            id: "xu-001".into(),
            source_id: "xuhui-20250828".into(),
            location_text: "罗秀路出龙州路东约160米".into(),
            equipment: "固定式交通技术监控设备".into(),
            applies_to_motorcycle: None,
            coordinate: None,
            photo_url: None,
            photo_taken_on: None,
            evidence_level: EvidenceLevel::Candidate,
            review_note: "仅来自搜索摘要，未核对原始公告".into(),
        }],
    }
}

#[test]
fn candidates_are_retained_but_not_displayed_as_verified_points() -> Result<(), CatalogError> {
    let catalog = sample();
    catalog.validate()?;
    assert_eq!(catalog.records.len(), 1);
    assert!(catalog.map_points().is_empty());
    Ok(())
}

#[test]
fn rejects_orphan_and_duplicate_records() {
    let mut catalog = sample();
    catalog.records[0].source_id = "missing".into();
    assert!(catalog.validate().is_err());

    let mut catalog = sample();
    catalog.records.push(catalog.records[0].clone());
    assert!(catalog.validate().is_err());
}

#[test]
fn photo_date_requires_a_photo_and_a_known_date() {
    let mut catalog = sample();
    catalog.records[0].photo_taken_on = Some("2025-08-28".into());
    assert!(catalog.validate().is_err());
    catalog.records[0].photo_url = Some("https://example.org/photo.jpg".into());
    assert!(catalog.validate().is_ok());
    catalog.records[0].photo_taken_on = Some("2025-02-31".into());
    assert!(catalog.validate().is_err());
}

#[test]
fn only_reviewed_precise_locations_appear_on_map() -> Result<(), CatalogError> {
    let mut catalog = sample();
    catalog.records[0].evidence_level = EvidenceLevel::Reviewed;
    assert!(catalog.validate().is_err());
    catalog.records[0].coordinate = Some(Coordinate {
        longitude: 121.45,
        latitude: 31.13,
        reference: "人工审核测定".into(),
        system: "gcj02".into(),
    });
    catalog.validate()?;
    assert_eq!(catalog.map_points().len(), 1);
    catalog.records[0]
        .coordinate
        .as_mut()
        .expect("test fixture")
        .latitude = 120.0;
    assert!(catalog.validate().is_err());
    Ok(())
}

#[tokio::test]
async fn collected_announcement_data_stays_in_review_queue() -> Result<(), CatalogError> {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/shanghai-cameras.json");
    let catalog = Catalog::load(fixture).await?;
    assert_eq!(catalog.sources.len(), 2);
    assert_eq!(catalog.records.len(), 3);
    assert!(catalog.map_points().is_empty());
    assert!(
        catalog
            .records
            .iter()
            .all(|item| item.photo_taken_on.is_none())
    );
    Ok(())
}

#[test]
fn parse_str_and_published_date_are_not_photo_date() -> Result<(), CatalogError> {
    let catalog = sample();
    let json = serde_json::to_string(&catalog)?;
    let restored = Catalog::parse_str(&json)?;
    assert_eq!(restored.sources[0].published_on, "2025-08-28");
    assert_eq!(restored.records[0].photo_taken_on, None);
    Ok(())
}

#[test]
fn published_date_is_not_the_photo_date() -> Result<(), CatalogError> {
    let catalog = sample();
    let json = serde_json::to_string(&catalog)?;
    let restored: Catalog = serde_json::from_str(&json)?;
    assert_eq!(restored.sources[0].published_on, "2025-08-28");
    assert_eq!(restored.records[0].photo_taken_on, None);
    Ok(())
}
