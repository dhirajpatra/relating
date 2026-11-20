pub mod env_config;
pub mod error;
pub mod models;
pub mod secrets;

pub use env_config::{
    aura_connection_from_env, aura_env_connection_id, aura_env_status, aura_uri_looks_valid,
    AuraEnvStatus, AURA_DATABASE_VAR, AURA_NAME_VAR, AURA_PASSWORD_VAR, AURA_URI_VAR,
    AURA_USERNAME_VAR,
};

/// Load `.env` from the working directory, leniently.
///
/// Real environment variables always win; the file only fills in gaps.
/// Unlike `dotenvy::dotenv`, a single malformed line does not abort the
/// whole file: it is skipped (mirroring `start.sh`, which warns about bad
/// lines). This matters because compose-oriented lines such as
/// `OLLAMA_PRELOAD_MODELS=llama3.1 nomic-embed-text` are valid for Docker
/// but not strict-dotenv. Returns the number of variables applied.
pub fn load_dotenv() -> usize {
    let mut applied = 0;
    let Ok(iter) = dotenvy::from_filename_iter(".env") else {
        return 0;
    };
    for item in iter {
        let Ok((key, value)) = item else {
            continue;
        };
        if std::env::var_os(&key).is_none() {
            std::env::set_var(&key, &value);
            applied += 1;
        }
    }
    applied
}
pub use error::RelatingError;
pub use models::*;
pub use secrets::{
    backend_order, delete_secret, keyring_available, resolve_secret, store_secret, SecretBackend,
};
