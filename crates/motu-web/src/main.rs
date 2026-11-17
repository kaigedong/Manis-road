mod config;
mod error;
mod model;
mod server;

use std::{process::ExitCode, sync::Arc};

use motu_routing::RoadGraph;
use motu_core::FileStore;
use tracing_subscriber::EnvFilter;

use crate::{
    config::{SignatureSource, WebConfig},
    server::AppState,
};

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
    store
        .ensure_identity()
        .map_err(|error| format!("cannot initialise local store: {error}"))?;
    let graph = if config.graph_path.is_file() {
        match RoadGraph::load(&config.graph_path) {
            Ok(graph) => {
                tracing::info!(
                    "offline graph: {} nodes, {} edges from {}",
                    graph.node_count(),
                    graph.edge_count(),
                    config.graph_path.display()
                );
                Some(Arc::new(graph))
            }
            Err(error) => {
                tracing::warn!("cannot load {}: {error}", config.graph_path.display());
                None
            }
        }
    } else {
        tracing::warn!(
            "no offline graph at {}; /api/route is disabled",
            config.graph_path.display()
        );
        None
    };
    let state = Arc::new(AppState {
        store,
        config: Arc::new(config.clone()),
        graph,
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
        match config.signature_source {
            SignatureSource::None => "not configured".to_owned(),
            source => format!("configured via {}", source.as_str()),
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
