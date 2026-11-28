use chrono::{DateTime, Utc};
use relating_core::{OidcConfig, ProjectId};
use relating_graph::RelateBackend;
use relating_metadata::DbPool;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Failed-authentication tracker for one client key (IP or fallback label).
#[derive(Debug, Clone, Default)]
pub struct AttemptState {
    pub fails: u32,
    pub locked_until: Option<DateTime<Utc>>,
}

/// Authentication state snapshotted at boot: the OIDC configuration plus the
/// in-memory 401 lockout table. `Default` is hermetic (SSO off, empty table)
/// for tests; `AuthState::boot` reads the process environment for servers.
#[derive(Clone)]
pub struct AuthState {
    pub oidc: Option<OidcConfig>,
    pub attempts: Arc<Mutex<HashMap<String, AttemptState>>>,
}

impl Default for AuthState {
    fn default() -> Self {
        Self {
            oidc: None,
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl AuthState {
    pub fn boot() -> Self {
        Self {
            oidc: OidcConfig::from_env(),
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_oidc(oidc: OidcConfig) -> Self {
        Self {
            oidc: Some(oidc),
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Consecutive 401/403 failures before a client key is locked out.
pub const MAX_AUTH_FAILURES: u32 = 5;
/// Lockout duration after the failure budget is spent.
pub const AUTH_LOCKOUT_SECS: i64 = 60;

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
    pub auth: AuthState,
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
            auth: AuthState::boot(),
        }
    }

    pub fn project_dir(&self, id: &ProjectId) -> PathBuf {
        self.data_dir.join("projects").join(id.to_string())
    }

    pub fn project_graph_path(&self, id: &ProjectId) -> PathBuf {
        self.project_dir(id).join("graph.rl")
    }
}
