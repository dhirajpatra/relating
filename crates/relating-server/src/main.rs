use relating_metadata::init_db;
use relating_server::{create_router, AppState};
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 0. Load `.env` from the working directory when present. Real
    // environment variables always win; the file only fills in gaps.
    // (start.sh loads it too; this covers direct `cargo run` invocations.)
    relating_core::load_dotenv();

    // 1. Initialize structured logging
    let filter = std::env::var("RELATING_LOG_LEVEL")
        .unwrap_or_else(|_| "info,relating_server=debug,relating_graph=debug".to_string());

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(filter))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Relating Control Plane & Workbench API...");

    // Report which `.env`-driven integrations are live. Values are never
    // logged, only availability.
    if relating_ai::meta_backend_selected() {
        if relating_ai::meta_ai_config_from_env().is_some() {
            info!("LLM: Meta API selected via LLM_BACKEND=meta_api (Ollama fallback on error)");
        } else {
            info!("LLM: LLM_BACKEND=meta_api but no API key resolves (META_API_KEY/MODEL_API_KEY); project providers apply");
        }
    }
    let aura_status = relating_core::aura_env_status();
    if aura_status.available {
        info!("Aura: .env connection available ({})", aura_status.reason);
    } else {
        info!("Aura: no .env connection ({})", aura_status.reason);
    }

    // 2. Load configurations
    let port: u16 = std::env::var("RELATING_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let host = std::env::var("RELATING_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let data_dir =
        PathBuf::from(std::env::var("RELATING_DATA_DIR").unwrap_or_else(|_| "./data".to_string()));
    let db_url = std::env::var("RELATING_DATABASE_URL")
        .unwrap_or_else(|_| format!("sqlite://{}/relating_metadata.db", data_dir.display()));

    // Ensure data directory exists
    if !data_dir.exists() {
        std::fs::create_dir_all(&data_dir)?;
    }

    // 3. Initialize the metadata store (SQLite runtime default) and run migrations.
    if relating_metadata::db_url_kind(&db_url) == "postgres" {
        return Err(
            "RELATING_DATABASE_URL points at PostgreSQL, which is provisioning-only in this release: \
             SQLite remains the runtime metadata store. Provision and validate the shared schema with \
             `cargo run -p relating-cli --features postgres -- db init-postgres --url <pg-url>` \
             (see docs/postgres.md), then keep the server on SQLite for now."
                .into(),
        );
    }
    info!("Connecting to metadata database: {}", db_url);
    let pool = init_db(&db_url).await?;

    // 4. Build application state and router
    let state = AppState::new(pool, data_dir);
    let app = create_router(state);

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    info!("Relating API listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
