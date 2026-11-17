use std::{
    env,
    path::Path,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use manis_camera_catalog::{
    Catalog,
    collector::{collect_snapshot, fetch_official, snapshot_sha256},
};

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(error) = run().await {
        eprintln!("{error:#}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [command, path] if ["validate", "stats", "points"].contains(&command.as_str()) => {
            let catalog = read_catalog(Path::new(path)).await?;
            match command.as_str() {
                "validate" => println!("资料校验通过"),
                "stats" => println!(
                    "来源: {}，候选/已审核记录合计: {}，可显示的已审核点位: {}，采集事件: {}",
                    catalog.sources.len(),
                    catalog.records.len(),
                    catalog.map_points().len(),
                    catalog.events.len()
                ),
                "points" => println!("{}", serde_json::to_string_pretty(&catalog.map_points())?),
                _ => bail!("未知命令: {command}"),
            }
        }
        [command, seed, source_id, html_path, output_dir] if command == "collect-file" => {
            let html = tokio::fs::read_to_string(html_path)
                .await
                .with_context(|| format!("读取 HTML 快照 {html_path} 失败"))?;
            collect(Path::new(seed), source_id, &html, Path::new(output_dir)).await?;
        }
        [command, seed, source_id, output_dir] if command == "collect-url" => {
            let dir = Path::new(output_dir);
            let catalog = load_current(Path::new(seed), dir).await?;
            let source = catalog
                .sources
                .iter()
                .find(|source| &source.id == source_id)
                .with_context(|| format!("没有来源: {source_id}"))?;
            let html = fetch_official(source).await?;
            collect(Path::new(seed), source_id, &html, dir).await?;
        }
        _ => bail!(
            "用法:\n  camera-cli <validate|stats|points> <catalog.json>\n  camera-cli collect-file <seed.json> <source-id> <snapshot.html> <output-dir>\n  camera-cli collect-url <seed.json> <source-id> <output-dir>"
        ),
    }
    Ok(())
}

async fn read_catalog(path: &Path) -> Result<Catalog> {
    let json = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("读取目录 {} 失败", path.display()))?;
    Catalog::parse_str(&json).context("目录内容无效")
}

async fn load_current(seed: &Path, dir: &Path) -> Result<Catalog> {
    let current = dir.join("catalog.json");
    read_catalog(if current.exists() { &current } else { seed }).await
}

async fn collect(seed: &Path, source_id: &str, html: &str, dir: &Path) -> Result<()> {
    let mut catalog = load_current(seed, dir).await?;
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("系统时钟早于 Unix 纪元")?
        .as_millis() as u64;
    let events = collect_snapshot(&mut catalog, source_id, html, time)?;
    let digest = snapshot_sha256(html);
    let snapshots = dir.join("snapshots");
    tokio::fs::create_dir_all(&snapshots).await?;
    let raw_path = snapshots.join(format!("{digest}.html"));
    if !raw_path.exists() {
        tokio::fs::write(&raw_path, html)
            .await
            .with_context(|| format!("保存原始证据 {} 失败", raw_path.display()))?;
    }
    let temp = dir.join("catalog.json.tmp");
    tokio::fs::write(&temp, serde_json::to_vec_pretty(&catalog)?).await?;
    tokio::fs::rename(&temp, dir.join("catalog.json")).await?;
    println!(
        "采集事件: {}，总候选/已审核记录: {}，已审核地图点位: {}，原始快照: {}",
        events.len(),
        catalog.records.len(),
        catalog.map_points().len(),
        raw_path.display()
    );
    for event in events {
        println!("{:?}: {}", event.kind, event.detail);
    }
    Ok(())
}
