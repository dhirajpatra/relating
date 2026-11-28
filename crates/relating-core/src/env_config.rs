//! Environment-derived configuration: use `.env` values when available.
//!
//! Both helpers below are *best effort*: they return `None` when the
//! environment does not carry a complete, usable configuration, in which
//! case callers fall back to stored project configuration. Nothing here
//! panics and no secret value is ever logged.
//!
//! Aura Bolt connections from the environment need:
//!
//! - `AURA_URI` — `bolt://`, `bolt+s://`, `neo4j://` or `neo4j+s://` address
//!   (no embedded credentials).
//! - `AURA_PASSWORD` — resolved through the usual secret backends
//!   (`RELATING_SECRET_BACKEND`, `<NAME>_FILE`, OS keychain).
//! - `AURA_USERNAME` (optional, default `neo4j`),
//!   `AURA_DATABASE` (optional), `AURA_NAME` (optional).
//!
//! The derived connection carries a stable id (SHA-256 of the URI), so the
//! same `.env` yields the same connection id on every process: list, test,
//! migrate and verify round-trip through it like any stored connection.

use crate::models::{AuraConnection, AuraConnectionId, ProjectId};
use sha2::{Digest, Sha256};

/// Environment variable holding the Aura Bolt URI.
pub const AURA_URI_VAR: &str = "AURA_URI";
/// Environment variable holding the Aura username (default `neo4j`).
pub const AURA_USERNAME_VAR: &str = "AURA_USERNAME";
/// Environment variable holding the Aura database name (optional).
pub const AURA_DATABASE_VAR: &str = "AURA_DATABASE";
/// Environment variable holding the display name (default `aura (env)`).
pub const AURA_NAME_VAR: &str = "AURA_NAME";
/// Secret name holding the Aura password (via [`crate::resolve_secret`]).
pub const AURA_PASSWORD_VAR: &str = "AURA_PASSWORD";

fn read_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Stable connection id for an env-derived Aura connection.
///
/// The id depends only on the URI, never on the project or the process, so
/// every project resolves the same `.env` connection to the same id.
pub fn aura_env_connection_id(uri: &str) -> AuraConnectionId {
    let mut hasher = Sha256::new();
    hasher.update(b"relating:aura-env:v1:");
    hasher.update(uri.trim().as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    AuraConnectionId(uuid::Uuid::from_bytes(bytes))
}

/// Lightweight URI sanity check (scheme + host present, no credentials).
///
/// The Bolt client re-validates with full diagnostics at connect time; this
/// only decides whether the environment offers a *plausible* connection.
pub fn aura_uri_looks_valid(uri: &str) -> bool {
    let uri = uri.trim();
    let Some((scheme, rest)) = uri.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "bolt" | "bolt+s" | "neo4j" | "neo4j+s") {
        return false;
    }
    if rest.is_empty() {
        return false;
    }
    // Reject embedded credentials (`user:pass@host`): passwords must come
    // from the secret backends, never from the URI.
    let authority = rest.split('/').next().unwrap_or("");
    !authority.contains('@')
}

/// Build the Aura connection described by the environment, if any.
///
/// Returns `None` when `AURA_URI` is unset/invalid or the password does not
/// resolve. The returned connection is transient (never inserted into the
/// database); its `auth_env_var` stays `AURA_PASSWORD` so password
/// resolution keeps working through every backend.
pub fn aura_connection_from_env(project_id: &ProjectId) -> Option<AuraConnection> {
    let uri = read_env(AURA_URI_VAR)?;
    if !aura_uri_looks_valid(&uri) {
        return None;
    }
    // The password must resolve, but its value is never read here beyond
    // the availability check; callers resolve it again at connect time.
    crate::resolve_secret(AURA_PASSWORD_VAR).ok()?;

    Some(AuraConnection {
        id: aura_env_connection_id(&uri),
        project_id: project_id.clone(),
        name: read_env(AURA_NAME_VAR).unwrap_or_else(|| "aura (env)".to_string()),
        uri,
        username: read_env(AURA_USERNAME_VAR).unwrap_or_else(|| "neo4j".to_string()),
        database: read_env(AURA_DATABASE_VAR),
        auth_env_var: AURA_PASSWORD_VAR.to_string(),
        created_at: chrono::Utc::now(),
    })
}

/// Machine-readable availability of the env-derived Aura connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuraEnvStatus {
    pub available: bool,
    /// Short human-readable reason (names variables, never values).
    pub reason: String,
}

/// Explain why the env Aura connection is or is not usable.
pub fn aura_env_status() -> AuraEnvStatus {
    let Some(uri) = read_env(AURA_URI_VAR) else {
        return AuraEnvStatus {
            available: false,
            reason: "AURA_URI is not set".to_string(),
        };
    };
    if !aura_uri_looks_valid(&uri) {
        return AuraEnvStatus {
            available: false,
            reason: "AURA_URI is not a valid bolt/neo4j URI (expected bolt://, bolt+s://, neo4j:// or neo4j+s:// without embedded credentials)".to_string(),
        };
    }
    if let Err(message) = crate::resolve_secret(AURA_PASSWORD_VAR) {
        return AuraEnvStatus {
            available: false,
            reason: message,
        };
    }
    AuraEnvStatus {
        available: true,
        reason: format!(
            "AURA_URI plus {} resolve; username '{}'",
            AURA_PASSWORD_VAR,
            read_env(AURA_USERNAME_VAR).unwrap_or_else(|| "neo4j".to_string())
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ENV_LOCK;

    struct EnvGuard {
        saved: Vec<(String, Option<String>)>,
    }

    impl EnvGuard {
        fn take(names: &[&str]) -> EnvGuard {
            let mut saved = Vec::new();
            for name in names {
                saved.push((name.to_string(), std::env::var(name).ok()));
                std::env::remove_var(name);
            }
            // Backend + file sidecar also influence resolution.
            for extra in ["RELATING_SECRET_BACKEND", "AURA_PASSWORD_FILE"] {
                if !names.contains(&extra) {
                    saved.push((extra.to_string(), std::env::var(extra).ok()));
                    std::env::remove_var(extra);
                }
            }
            EnvGuard { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
    }

    const VARS: &[&str] = &[
        "AURA_URI",
        "AURA_USERNAME",
        "AURA_DATABASE",
        "AURA_NAME",
        "AURA_PASSWORD",
    ];

    #[test]
    fn test_absent_without_uri() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::take(VARS);
        std::env::set_var("AURA_PASSWORD", "pw");
        assert!(aura_connection_from_env(&ProjectId::new()).is_none());
        let status = aura_env_status();
        assert!(!status.available);
        assert!(status.reason.contains("AURA_URI"));
    }

    #[test]
    fn test_absent_without_password() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::take(VARS);
        std::env::set_var("AURA_URI", "neo4j+s://abc.databases.neo4j.io");
        assert!(aura_connection_from_env(&ProjectId::new()).is_none());
        let status = aura_env_status();
        assert!(!status.available);
        assert!(status.reason.contains("AURA_PASSWORD"));
    }

    #[test]
    fn test_rejects_bad_uri() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::take(VARS);
        std::env::set_var("AURA_PASSWORD", "pw");
        for bad in [
            "not-a-uri",
            "http://host:7474",
            "neo4j+ssc://host",
            "bolt://",
            "neo4j+s://user:pass@host",
        ] {
            std::env::set_var("AURA_URI", bad);
            assert!(
                aura_connection_from_env(&ProjectId::new()).is_none(),
                "URI should be rejected: {}",
                bad
            );
        }
    }

    #[test]
    fn test_builds_stable_connection() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::take(VARS);
        std::env::set_var("AURA_URI", "neo4j+s://abc.databases.neo4j.io");
        std::env::set_var("AURA_PASSWORD", "pw");
        std::env::set_var("AURA_DATABASE", "neo4j");

        let project = ProjectId::new();
        let first = aura_connection_from_env(&project).expect("env connection");
        let second = aura_connection_from_env(&project).expect("env connection");
        assert_eq!(first.id, second.id);
        assert_eq!(first.uri, "neo4j+s://abc.databases.neo4j.io");
        assert_eq!(first.username, "neo4j");
        assert_eq!(first.database.as_deref(), Some("neo4j"));
        assert_eq!(first.auth_env_var, "AURA_PASSWORD");
        assert_eq!(first.project_id, project);

        // Same URI under another project: same connection id, that project.
        let other = aura_connection_from_env(&ProjectId::new()).expect("env connection");
        assert_eq!(other.id, first.id);

        let status = aura_env_status();
        assert!(status.available);
    }
}
