use std::{path::Path, process::Command};

use manis_camera_catalog::{Catalog, CatalogEventKind};

#[test]
fn cli_collects_snapshot_and_reuses_persisted_events() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let seed = root.join("../camera-catalog/data/shanghai-cameras.json");
    let snapshot = root.join("tests/fixtures/notice.html");
    let output = tempfile::tempdir()?;
    let binary = env!("CARGO_BIN_EXE_camera-cli");
    let run = || {
        Command::new(binary)
            .args([
                "collect-file",
                seed.to_str().unwrap_or_default(),
                "xuhui-20250828",
                snapshot.to_str().unwrap_or_default(),
                output.path().to_str().unwrap_or_default(),
            ])
            .output()
    };
    let first = run()?;
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let catalog = Catalog::parse_str(&std::fs::read_to_string(
        output.path().join("catalog.json"),
    )?)?;
    assert_eq!(catalog.records.len(), 4);
    assert_eq!(catalog.map_points().len(), 0);
    assert_eq!(
        catalog
            .events
            .iter()
            .filter(|event| event.kind == CatalogEventKind::CandidateDiscovered)
            .count(),
        1
    );
    assert_eq!(
        std::fs::read_dir(output.path().join("snapshots"))?.count(),
        1
    );
    let second = run()?;
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let catalog = Catalog::parse_str(&std::fs::read_to_string(
        output.path().join("catalog.json"),
    )?)?;
    assert_eq!(catalog.records.len(), 4);
    assert_eq!(
        catalog
            .events
            .iter()
            .filter(|event| event.kind == CatalogEventKind::CandidateDiscovered)
            .count(),
        1
    );
    assert_eq!(
        catalog
            .events
            .iter()
            .filter(|event| event.kind == CatalogEventKind::SourceObserved)
            .count(),
        2
    );
    Ok(())
}
