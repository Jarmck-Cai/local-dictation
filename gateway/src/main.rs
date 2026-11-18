use local_dictation_gateway::vocab::VocabularyStore;
use local_dictation_gateway::{build_router, AppState, GatewayConfig};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .init();

    let config = match GatewayConfig::from_env() {
        Ok(cfg) => cfg,
        Err(err) => {
            log::error!("Startup configuration error: {err}");
            std::process::exit(2);
        }
    };

    let bind_addr = config.bind_addr.clone();
    let device_count = config.tokens.device_count();
    let asr_ready = config.openai_api_key.is_some();
    let cleanup_ready = config.anthropic_api_key.is_some();
    let asr_model = config.openai_asr_model.clone();
    let cleanup_model = config.anthropic_cleanup_model.clone();

    let vocab = match VocabularyStore::open(&config.db_path) {
        Ok(store) => store,
        Err(err) => {
            log::error!("Failed to open vocabulary database: {err}");
            std::process::exit(1);
        }
    };

    let state = match AppState::new(config, vocab) {
        Ok(s) => s,
        Err(err) => {
            log::error!("Failed to initialize gateway state: {err}");
            std::process::exit(1);
        }
    };

    let app = build_router(state);
    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(err) => {
            log::error!("Failed to bind {bind_addr}: {err}");
            std::process::exit(1);
        }
    };

    log::info!(
        "local-dictation-gateway v{} listening on {} (devices={}, asr_ready={}, asr_model={}, cleanup_ready={}, cleanup_model={})",
        env!("CARGO_PKG_VERSION"),
        bind_addr,
        device_count,
        asr_ready,
        asr_model,
        cleanup_ready,
        cleanup_model
    );

    if let Err(err) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        log::error!("Gateway server exited with error: {err}");
        std::process::exit(1);
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sigterm) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sigterm.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    log::info!("Shutdown signal received; stopping gateway gracefully");
}
