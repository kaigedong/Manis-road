use std::{env, process::ExitCode};

use manis_camera_catalog::Catalog;

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let command = args.next();
    let path = args.next();
    let (command, path) = match (command, path, args.next()) {
        (Some(command), Some(path), None) => (command, path),
        _ => {
            eprintln!("用法: cargo run -- <validate|stats|points> <数据文件.json>");
            return ExitCode::FAILURE;
        }
    };
    let catalog = match Catalog::load(path).await {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    match command.as_str() {
        "validate" => println!("资料校验通过"),
        "stats" => println!(
            "来源: {}，候选/已审核记录合计: {}，可显示的已审核点位: {}",
            catalog.sources.len(),
            catalog.records.len(),
            catalog.map_points().len()
        ),
        "points" => match serde_json::to_string_pretty(&catalog.map_points()) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("输出失败: {error}");
                return ExitCode::FAILURE;
            }
        },
        _ => {
            eprintln!("仅支持 validate / stats / points");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
