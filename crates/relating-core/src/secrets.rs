//! Team-friendly secret resolution for passwords and provider keys.
//!
//! Single developers keep using plain environment variables. Teams sharing a
//! staging instance can mount secrets as files (Docker/Kubernetes style) or,
//! with the `os-keychain` cargo feature, read them from the OS keychain.
//!
//! Resolution order is controlled by `RELATING_SECRET_BACKEND`:
//! - unset or `auto` (default): environment variable, then `<NAME>_FILE`.
//!   This preserves the historical behavior exactly when no `_FILE` sidecar
//!   is configured.
//! - `env`: environment variable only (strict historical behavior).
//! - `file`: `<NAME>_FILE` first, then the environment variable.
//! - `keyring`: OS keychain first (service `relating`, account `<NAME>`),
//!   then environment, then file. Requires the `os-keychain` feature.
//!
//! Secrets are never logged: errors name the variable, never the value.

/// Backends consulted by [`resolve_secret`], in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretBackend {
    Env,
    File,
    Keyring,
}

/// Parse `RELATING_SECRET_BACKEND` into an ordered backend list.
/// Unknown values fall back to `auto` rather than failing closed at config
/// time; callers surface resolution errors that name the variable.
pub fn backend_order() -> Vec<SecretBackend> {
    let mode = std::env::var("RELATING_SECRET_BACKEND")
        .unwrap_or_else(|_| "auto".to_string())
        .trim()
        .to_ascii_lowercase();
    match mode.as_str() {
        "env" => vec![SecretBackend::Env],
        "file" => vec![SecretBackend::File, SecretBackend::Env],
        "keyring" => vec![
            SecretBackend::Keyring,
            SecretBackend::Env,
            SecretBackend::File,
        ],
        _ => vec![SecretBackend::Env, SecretBackend::File],
    }
}

fn read_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn read_file(name: &str) -> Option<String> {
    let path_var = format!("{}_FILE", name);
    let path = std::env::var(&path_var).ok()?;
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(feature = "os-keychain")]
fn read_keyring(name: &str) -> Option<String> {
    keyring::Entry::new("relating", name)
        .ok()
        .and_then(|entry| entry.get_password().ok())
}

#[cfg(not(feature = "os-keychain"))]
fn read_keyring(_name: &str) -> Option<String> {
    None
}

/// Returns true when the OS keychain backend can actually serve reads in
/// this build (the `os-keychain` cargo feature is enabled).
pub fn keyring_available() -> bool {
    cfg!(feature = "os-keychain")
}

// Compile-time guard: the os-keychain feature must enable a REAL backend.
// keyring 3.x has no default features, so without the backend features in
// the workspace keyring spec, `Entry` silently uses the in-memory mock store
// (writes report success and vanish on exit). These imports fail the build
// if the backends ever go missing.
#[cfg(all(feature = "os-keychain", target_os = "linux"))]
#[allow(unused_imports)]
use keyring::secret_service as _real_backend_guard;
#[cfg(all(feature = "os-keychain", target_os = "macos"))]
#[allow(unused_imports)]
use keyring::macos as _real_backend_guard;
#[cfg(all(feature = "os-keychain", target_os = "windows"))]
#[allow(unused_imports)]
use keyring::windows as _real_backend_guard;

/// Resolve a secret by its environment-variable name.
///
/// `name` is the logical name (for example `AURA_PASSWORD`). The file
/// backend looks at `<NAME>_FILE`, and the keyring backend uses the OS
/// keychain entry `relating/<NAME>`. Errors name the variable and the
/// backends tried, never the secret value.
pub fn resolve_secret(name: &str) -> Result<String, String> {
    for backend in backend_order() {
        let hit = match backend {
            SecretBackend::Env => read_env(name),
            SecretBackend::File => read_file(name),
            SecretBackend::Keyring => read_keyring(name),
        };
        if let Some(value) = hit {
            return Ok(value);
        }
    }
    Err(missing_secret_message(name))
}

/// Store a secret in the OS keychain under service `relating`.
///
/// Requires the `os-keychain` cargo feature; without it, returns a message
/// explaining how to rebuild. Used by `relating secrets set` so teams can
/// provision shared staging credentials without env files.
pub fn store_secret(name: &str, value: &str) -> Result<(), String> {
    #[cfg(feature = "os-keychain")]
    {
        keyring::Entry::new("relating", name)
            .and_then(|entry| entry.set_password(value))
            .map_err(|e| {
                format!(
                    "Failed to store secret '{}' in the OS keychain: {}",
                    name, e
                )
            })
    }
    #[cfg(not(feature = "os-keychain"))]
    {
        let _ = (name, value);
        Err(
            "OS keychain support is not enabled in this build; rebuild with --features os-keychain"
                .to_string(),
        )
    }
}

/// Delete a secret from the OS keychain. See [`store_secret`].
pub fn delete_secret(name: &str) -> Result<(), String> {
    #[cfg(feature = "os-keychain")]
    {
        keyring::Entry::new("relating", name)
            .and_then(|entry| entry.delete_credential())
            .map_err(|e| {
                format!(
                    "Failed to delete secret '{}' from the OS keychain: {}",
                    name, e
                )
            })
    }
    #[cfg(not(feature = "os-keychain"))]
    {
        let _ = name;
        Err(
            "OS keychain support is not enabled in this build; rebuild with --features os-keychain"
                .to_string(),
        )
    }
}

fn missing_secret_message(name: &str) -> String {
    let mut hints = vec![format!(
        "export {}='<secret>' (the value is never stored)",
        name
    )];
    hints.push(format!(
        "or set {}_FILE to a file containing the secret (Docker/Kubernetes style)",
        name
    ));
    if keyring_available() {
        hints.push(
            "or store it in the OS keychain under service 'relating' and set RELATING_SECRET_BACKEND=keyring".to_string(),
        );
    } else {
        hints.push(
            "or rebuild with --features os-keychain to read it from the OS keychain".to_string(),
        );
    }
    format!("Secret '{}' is not set. {}", name, hints.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Environment is process-global; serialize the tests that mutate it.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        saved_backend: Option<String>,
        saved_value: Option<String>,
        saved_file: Option<String>,
        name: String,
    }

    impl EnvGuard {
        fn take(name: &str) -> EnvGuard {
            let file_var = format!("{}_FILE", name);
            let guard = EnvGuard {
                saved_backend: std::env::var("RELATING_SECRET_BACKEND").ok(),
                saved_value: std::env::var(name).ok(),
                saved_file: std::env::var(&file_var).ok(),
                name: name.to_string(),
            };
            std::env::remove_var("RELATING_SECRET_BACKEND");
            std::env::remove_var(name);
            std::env::remove_var(&file_var);
            guard
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.saved_backend {
                Some(v) => std::env::set_var("RELATING_SECRET_BACKEND", v),
                None => std::env::remove_var("RELATING_SECRET_BACKEND"),
            }
            match &self.saved_value {
                Some(v) => std::env::set_var(&self.name, v),
                None => std::env::remove_var(&self.name),
            }
            let file_var = format!("{}_FILE", self.name);
            match &self.saved_file {
                Some(v) => std::env::set_var(&file_var, v),
                None => std::env::remove_var(&file_var),
            }
        }
    }

    #[test]
    fn test_default_backend_order_preserves_env_first() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::take("RELATING_TEST_SECRET_X1");
        assert_eq!(
            backend_order(),
            vec![SecretBackend::Env, SecretBackend::File]
        );
        std::env::set_var("RELATING_SECRET_BACKEND", "env");
        assert_eq!(backend_order(), vec![SecretBackend::Env]);
        std::env::set_var("RELATING_SECRET_BACKEND", "nonsense");
        assert_eq!(
            backend_order(),
            vec![SecretBackend::Env, SecretBackend::File]
        );
    }

    #[test]
    fn test_resolve_prefers_env_by_default() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::take("RELATING_TEST_SECRET_X2");
        std::env::set_var("RELATING_TEST_SECRET_X2", "from-env");
        assert_eq!(
            resolve_secret("RELATING_TEST_SECRET_X2").unwrap(),
            "from-env"
        );
    }

    #[test]
    fn test_resolve_falls_back_to_file_sidecar() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::take("RELATING_TEST_SECRET_X3");
        let dir = std::env::temp_dir().join("relating-secret-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x3.txt");
        std::fs::write(&path, "from-file\n").unwrap();
        std::env::set_var("RELATING_TEST_SECRET_X3_FILE", &path);
        assert_eq!(
            resolve_secret("RELATING_TEST_SECRET_X3").unwrap(),
            "from-file"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_missing_secret_names_variable_not_value() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::take("RELATING_TEST_SECRET_X4");
        std::env::set_var("RELATING_TEST_SECRET_X4", "super-secret-value");
        std::env::remove_var("RELATING_TEST_SECRET_X4");
        let err = resolve_secret("RELATING_TEST_SECRET_X4").unwrap_err();
        assert!(err.contains("RELATING_TEST_SECRET_X4"));
        assert!(err.contains("RELATING_TEST_SECRET_X4_FILE"));
        assert!(!err.contains("super-secret-value"));
    }
}
