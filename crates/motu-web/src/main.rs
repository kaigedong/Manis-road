mod config;
mod error;
mod model;
mod server;

use std::{
    process::ExitCode,
    sync::{Arc, Mutex},
};

use motu_core::FileStore;
use tracing_subscriber::EnvFilter;

use crate::{config::WebConfig, model::CameraSnapshot, server::AppState};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("manis_motu_web=info,tower_http=info")),
        )
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("motu-web: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let config = WebConfig::from_env()?;
    let store = FileStore::new(&config.data_dir);
    store
        .ensure_root()
        .map_err(|error| format!("cannot create {}: {error}", config.data_dir.display()))?;
    let core = store
        .ensure_identity()
        .map_err(|error| format!("cannot initialise local store: {error}"))?;
    let snapshot = CameraSnapshot::demo(core.settings.city_code.clone());
    let state = Arc::new(AppState {
        store,
        config: Arc::new(config.clone()),
        cameras: Arc::new(Mutex::new(snapshot)),
    });

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|error| format!("cannot bind {}: {error}", config.bind))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("cannot read listener address: {error}"))?;
    tracing::info!(
        "Motu web debugger on http://{address} (data: {}, signature: {}, tiles: {})",
        config.data_dir.display(),
        if config.signature_md5.is_empty() {
            "not configured"
        } else {
            "configured"
        },
        config.tile_url
    );

    axum::serve(listener, server::router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|error| format!("server failed: {error}"))
}
