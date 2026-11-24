use relating_core::ProjectId;
use relating_graph::RelateBackend;
use relating_metadata::DbPool;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub graph: Arc<RelateBackend>,
    pub data_dir: PathBuf,
    /// Optional API bearer token (`RELATING_API_TOKEN`). When set, every
    /// `/api/v1` route except `/health` requires
    /// `Authorization: Bearer <token>`. `None` leaves the API open, which is
    /// only appropriate on loopback.
    pub api_token: Option<String>,
}

/// Read the API token from the environment: trimmed, empty treated as unset.
pub fn api_token_from_env() -> Option<String> {
    std::env::var("RELATING_API_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl AppState {
    pub fn new(pool: DbPool, data_dir: PathBuf) -> Self {
        Self {
            pool,
            graph: Arc::new(RelateBackend::new()),
            data_dir,
            api_token: api_token_from_env(),
        }
    }

    pub fn project_dir(&self, id: &ProjectId) -> PathBuf {
        self.data_dir.join("projects").join(id.to_string())
    }

    pub fn project_graph_path(&self, id: &ProjectId) -> PathBuf {
        self.project_dir(id).join("graph.rl")
    }
}
