use subdisc::{AppState, app, config::Config, db};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "subdisc=info,tower_http=info".into()),
        )
        .init();
    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url).await?;
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    tracing::info!(address=%config.bind,"Subdisc iniciado");
    let state = AppState::new(pool, config);
    let shutdown_state = state.clone();
    let maintenance_state = state.clone();
    let maintenance = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            tick.tick().await;
            subdisc::media::maintain(&maintenance_state).await;
        }
    });
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown().await;
        shutdown_state.emit(subdisc::websocket::Event::Shutdown);
        shutdown_state.voice.lock().await.clear();
        shutdown_state.media.shutdown().await;
    })
    .await?;
    maintenance.abort();
    Ok(())
}

async fn shutdown() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _=interrupt=>{}, _=terminate=>{} }
}
