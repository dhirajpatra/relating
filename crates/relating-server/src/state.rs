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
}

impl AppState {
    pub fn new(pool: DbPool, data_dir: PathBuf) -> Self {
        Self {
            pool,
            graph: Arc::new(RelateBackend::new()),
            data_dir,
        }
    }

    pub fn project_dir(&self, id: &ProjectId) -> PathBuf {
        self.data_dir.join("projects").join(id.to_string())
    }

    pub fn project_graph_path(&self, id: &ProjectId) -> PathBuf {
        self.project_dir(id).join("graph.rl")
    }
}
