//! SQL database sources: read-only snapshot queries over SQLite, Postgres,
//! and MySQL through one driver pool.
//!
//! Queries are validated read-only (`SELECT`/`WITH`, single statement) before
//! they ever reach a socket. Passwords resolve through the secrets chain at
//! sync time and never touch stored config.

use crate::error::ConnectorError;
use crate::mapping::RowMap;
use serde::{Deserialize, Serialize};
use sqlx::any::{install_default_drivers, AnyPoolOptions, AnyTypeInfoKind};
use sqlx::{Column, Row};
use std::collections::HashMap;
use std::sync::Once;
use std::time::Duration;

static INSTALL_DRIVERS: Once = Once::new();

fn ensure_drivers() {
    INSTALL_DRIVERS.call_once(install_default_drivers);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlConfig {
    /// `sqlite`, `postgres` (`postgresql` accepted), or `mysql`.
    pub driver: String,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    /// Env var holding the password, resolved via the secrets chain.
    #[serde(default)]
    pub password_env: Option<String>,
    /// Filesystem path for `sqlite` (absolute recommended).
    #[serde(default)]
    pub path: Option<String>,
    /// Read-only snapshot query. Must start with SELECT or WITH and hold a
    /// single statement.
    pub query: String,
    /// Optional high-water column for incremental syncs (ints/timestamps).
    #[serde(default)]
    pub watermark_column: Option<String>,
    #[serde(default)]
    pub limit_per_sync: Option<u64>,
    /// Postgres only: `disable` or `prefer` (default). `require` is
    /// rejected until the TLS follow-up lands (no TLS stack compiled in).
    #[serde(default)]
    pub ssl_mode: Option<String>,
}

pub const DEFAULT_LIMIT_PER_SYNC: u64 = 5000;
pub const MAX_LIMIT_PER_SYNC: u64 = 50_000;

impl SqlConfig {
    pub fn normalized_driver(&self) -> Result<&'static str, ConnectorError> {
        match self.driver.trim().to_ascii_lowercase().as_str() {
            "sqlite" => Ok("sqlite"),
            "postgres" | "postgresql" => Ok("postgres"),
            "mysql" => Ok("mysql"),
            other => Err(ConnectorError::Config(format!(
                "Unknown SQL driver '{}': use sqlite, postgres, or mysql",
                other
            ))),
        }
    }

    pub fn effective_limit(&self) -> u64 {
        self.limit_per_sync
            .unwrap_or(DEFAULT_LIMIT_PER_SYNC)
            .clamp(1, MAX_LIMIT_PER_SYNC)
    }

    pub fn validate(&self) -> Result<(), ConnectorError> {
        let driver = self.normalized_driver()?;
        validate_read_only_query(&self.query)?;
        if let Some(column) = self.watermark_column.as_deref() {
            validate_identifier(column)?;
        }
        match driver {
            "sqlite" => {
                if self.path.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(ConnectorError::Config(
                        "sqlite sources need a filesystem path".to_string(),
                    ));
                }
            }
            _ => {
                if self.host.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(ConnectorError::Config(format!(
                        "{} sources need a host",
                        driver
                    )));
                }
                if self.database.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(ConnectorError::Config(format!(
                        "{} sources need a database name",
                        driver
                    )));
                }
            }
        }
        if let Some(mode) = self.ssl_mode.as_deref() {
            match mode.trim().to_ascii_lowercase().as_str() {
                "disable" | "prefer" => {}
                "require" => {
                    return Err(ConnectorError::Config(
                        "sslmode=require needs the TLS follow-up (no TLS stack is compiled in); use disable or prefer on private networks".to_string(),
                    ));
                }
                other => {
                    return Err(ConnectorError::Config(format!(
                        "Unknown ssl_mode '{}': use disable or prefer",
                        other
                    )));
                }
            }
        }
        Ok(())
    }

    /// Build the driver URL, resolving the password through the secrets
    /// chain when `password_env` is set. Errors name the variable, never
    /// the value.
    pub fn build_url(&self) -> Result<String, ConnectorError> {
        let driver = self.normalized_driver()?;
        if driver == "sqlite" {
            let path = self.path.as_deref().unwrap_or("").trim();
            return Ok(format!("sqlite://{}", path));
        }
        let password = match self.password_env.as_deref() {
            Some(var) if !var.trim().is_empty() => {
                Some(relating_core::resolve_secret(var.trim()).map_err(ConnectorError::Sql)?)
            }
            _ => None,
        };
        let userinfo = match (&self.username, password) {
            (Some(user), Some(password)) => {
                format!("{}:{}@", url_encode(user), url_encode(&password))
            }
            (Some(user), None) => format!("{}@", url_encode(user)),
            (None, _) => String::new(),
        };
        let host = self.host.as_deref().unwrap_or("localhost").trim();
        let port = self.port.map(|p| format!(":{}", p)).unwrap_or_default();
        let database = self.database.as_deref().unwrap_or("").trim();
        let mut url = format!("{}://{}{}{}/{}", driver, userinfo, host, port, database);
        if driver == "postgres" {
            if let Some(mode) = self.ssl_mode.as_deref() {
                url.push_str(&format!("?sslmode={}", mode.trim().to_ascii_lowercase()));
            }
        }
        Ok(url)
    }

    /// Display URL with any password redacted (safe for logs and API output).
    pub fn redacted_url(&self) -> String {
        match self.build_url() {
            Ok(url) => redact_password(&url),
            Err(_) => format!("{}://<invalid>", self.driver),
        }
    }

    /// Wrap the snapshot query with a high-water filter when a watermark
    /// column and value are both present. Values outside a strict charset
    /// fall back to a full snapshot rather than risking injection.
    pub fn query_for_sync(&self, watermark: Option<&str>) -> String {
        let base = self.query.trim().trim_end_matches(';').trim();
        let (Some(column), Some(mark)) = (self.watermark_column.as_deref(), watermark) else {
            return base.to_string();
        };
        if !watermark_value_is_safe(mark) {
            tracing::warn!("Unsafe watermark value; falling back to a full snapshot");
            return base.to_string();
        }
        format!(
            "SELECT * FROM ({}) AS _sync WHERE _sync.\"{}\" > '{}' ORDER BY _sync.\"{}\" LIMIT {}",
            base,
            column.replace('"', "\"\""),
            mark.replace('\'', "''"),
            column.replace('"', "\"\""),
            self.effective_limit()
        )
    }
}

fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn redact_password(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some(pair) => pair,
        None => return "<redacted-url>".to_string(),
    };
    let Some(at) = rest.find('@') else {
        return url.to_string();
    };
    let (userinfo, host) = rest.split_at(at);
    if userinfo.contains(':') {
        format!("{}://***{}", scheme, host)
    } else {
        url.to_string()
    }
}

/// Read-only gate: SELECT/WITH prefix, single statement, no stacked queries.
pub fn validate_read_only_query(query: &str) -> Result<(), ConnectorError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(ConnectorError::Config(
            "SQL query cannot be empty".to_string(),
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    let allowed = lower.starts_with("select") || lower.starts_with("with");
    if !allowed {
        return Err(ConnectorError::Config(
            "SQL sources accept read-only queries only (SELECT or WITH)".to_string(),
        ));
    }
    // One statement: a semicolon may only trail the end (comments excluded
    // by keeping the rule strict — stacked statements never execute).
    let body = trimmed.trim_end_matches(';').trim();
    if body.contains(';') {
        return Err(ConnectorError::Config(
            "SQL sources accept a single statement; stacked queries are rejected".to_string(),
        ));
    }
    Ok(())
}

fn validate_identifier(name: &str) -> Result<(), ConnectorError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.');
    if ok {
        Ok(())
    } else {
        Err(ConnectorError::Config(format!(
            "Invalid column identifier '{}'",
            name
        )))
    }
}

/// Watermark values render into SQL text, so only timestamp/numeric-shaped
/// values pass; anything else falls back to a full snapshot.
fn watermark_value_is_safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" .:-+TZ".contains(&b))
}

/// Decode one column by its Any kind. The Any driver only covers nine
/// primitive kinds: temporal/uuid/numeric columns must be cast to text in
/// the query itself (e.g. `updated_at::text`), or the driver fails the
/// whole fetch naming the unsupported type.
fn decode_column(row: &sqlx::any::AnyRow, idx: usize) -> Result<serde_json::Value, ConnectorError> {
    use serde_json::Value;
    let kind = row.columns()[idx].type_info().kind();
    let decode = || -> Result<Value, sqlx::Error> {
        Ok(match kind {
            AnyTypeInfoKind::Null => Value::Null,
            AnyTypeInfoKind::Bool => match row.try_get::<Option<bool>, _>(idx)? {
                Some(b) => Value::Bool(b),
                None => Value::Null,
            },
            AnyTypeInfoKind::SmallInt => match row.try_get::<Option<i16>, _>(idx)? {
                Some(n) => (n as i64).into(),
                None => Value::Null,
            },
            AnyTypeInfoKind::Integer => match row.try_get::<Option<i32>, _>(idx)? {
                Some(n) => (n as i64).into(),
                None => Value::Null,
            },
            AnyTypeInfoKind::BigInt => match row.try_get::<Option<i64>, _>(idx)? {
                Some(n) => n.into(),
                None => Value::Null,
            },
            AnyTypeInfoKind::Real => match row.try_get::<Option<f32>, _>(idx)? {
                Some(f) => serde_json::json!(f),
                None => Value::Null,
            },
            AnyTypeInfoKind::Double => match row.try_get::<Option<f64>, _>(idx)? {
                Some(f) => serde_json::json!(f),
                None => Value::Null,
            },
            AnyTypeInfoKind::Text => match row.try_get::<Option<String>, _>(idx)? {
                Some(s) => Value::String(s),
                None => Value::Null,
            },
            AnyTypeInfoKind::Blob => match row.try_get::<Option<Vec<u8>>, _>(idx)? {
                Some(bytes) => Value::String(String::from_utf8_lossy(&bytes).into_owned()),
                None => Value::Null,
            },
        })
    };
    decode().map_err(|e| {
        let name = &row.columns()[idx].name();
        ConnectorError::Sql(format!(
            "Failed to decode column '{}' ({:?}); cast exotic types to text in the query: {}",
            name,
            kind,
            sanitize_sql_error(&e.to_string())
        ))
    })
}

pub struct SqlFetch {
    pub rows: Vec<RowMap>,
    pub new_watermark: Option<String>,
}

/// Run the snapshot query and return rows plus the observed high-water mark
/// (max of the watermark column when configured).
pub async fn fetch_sql_rows(
    config: &SqlConfig,
    watermark: Option<&str>,
) -> Result<SqlFetch, ConnectorError> {
    config.validate()?;
    ensure_drivers();
    let url = config.build_url()?;
    let pool = AnyPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
        .map_err(|e| ConnectorError::Sql(sanitize_sql_error(&e.to_string())))?;
    let query_text = config.query_for_sync(watermark);
    let query_text = if config.watermark_column.is_none()
        && !query_text.to_ascii_lowercase().contains("limit")
    {
        format!(
            "SELECT * FROM ({}) AS _sync LIMIT {}",
            query_text,
            config.effective_limit()
        )
    } else {
        query_text
    };
    let rows = tokio::time::timeout(
        Duration::from_secs(60),
        sqlx::query(&query_text).fetch_all(&pool),
    )
    .await
    .map_err(|_| ConnectorError::Sql("SQL source query timed out after 60s".to_string()))?
    .map_err(|e| ConnectorError::Sql(sanitize_sql_error(&e.to_string())))?;

    let mut out = Vec::with_capacity(rows.len());
    let mut high_water: Option<String> = None;
    for row in &rows {
        let mut map = HashMap::with_capacity(row.columns().len());
        for (idx, column) in row.columns().iter().enumerate() {
            let json = decode_column(row, idx)?;
            if config.watermark_column.as_deref() == Some(column.name()) {
                let candidate = match &json {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    _ => String::new(),
                };
                if !candidate.is_empty()
                    && high_water
                        .as_deref()
                        .map(|h| candidate.as_str() > h)
                        .unwrap_or(true)
                {
                    high_water = Some(candidate);
                }
            }
            map.insert(column.name().to_string(), json);
        }
        out.push(map);
    }
    Ok(SqlFetch {
        rows: out,
        new_watermark: high_water,
    })
}

/// Strip embedded credentials from driver errors before they reach callers.
fn sanitize_sql_error(message: &str) -> String {
    // Driver errors can echo the connection URL; redact any userinfo block.
    let mut out = message.to_string();
    for scheme in ["postgres://", "postgresql://", "mysql://", "sqlite://"] {
        while let Some(start) = out.find(scheme) {
            let rest = &out[start + scheme.len()..];
            if let Some(at) = rest.find('@') {
                let before_host = rest[..at].find(['/', ' ', '"', '\'']);
                // Only redact when userinfo precedes the host (no path chars).
                if before_host.is_none() {
                    let end = start + scheme.len() + at + 1;
                    out.replace_range(start + scheme.len()..end - 1, "***");
                    continue;
                }
            }
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite_config(query: &str) -> SqlConfig {
        SqlConfig {
            driver: "sqlite".to_string(),
            host: None,
            port: None,
            database: None,
            username: None,
            password_env: None,
            path: Some("/tmp/source.db".to_string()),
            query: query.to_string(),
            watermark_column: None,
            limit_per_sync: None,
            ssl_mode: None,
        }
    }

    #[test]
    fn test_read_only_gate() {
        assert!(validate_read_only_query("SELECT * FROM t").is_ok());
        assert!(validate_read_only_query("  with x as (select 1) select * from x;").is_ok());
        for bad in [
            "",
            "INSERT INTO t VALUES (1)",
            "UPDATE t SET a = 1",
            "DELETE FROM t",
            "SELECT 1; DROP TABLE t;",
            "SELECT 1; SELECT 2",
        ] {
            assert!(validate_read_only_query(bad).is_err(), "{}", bad);
        }
    }

    #[test]
    fn test_driver_and_field_validation() {
        assert!(sqlite_config("SELECT 1").validate().is_ok());
        let mut bad_driver = sqlite_config("SELECT 1");
        bad_driver.driver = "oracle".to_string();
        assert!(bad_driver.validate().is_err());
        let mut pg = sqlite_config("SELECT 1");
        pg.driver = "postgres".to_string();
        assert!(pg.validate().is_err());
        pg.host = Some("db.internal".to_string());
        pg.database = Some("app".to_string());
        assert!(pg.validate().is_ok());
        let mut bad_col = sqlite_config("SELECT 1");
        bad_col.watermark_column = Some("id\" OR 1=1 --".to_string());
        assert!(bad_col.validate().is_err());
    }

    #[test]
    fn test_url_building_and_redaction() {
        let cfg = SqlConfig {
            driver: "postgres".to_string(),
            host: Some("db.internal".to_string()),
            port: Some(5433),
            database: Some("app".to_string()),
            username: Some("reader".to_string()),
            password_env: None,
            path: None,
            query: "SELECT 1".to_string(),
            watermark_column: None,
            limit_per_sync: None,
            ssl_mode: Some("prefer".to_string()),
        };
        let url = cfg.build_url().unwrap();
        assert!(
            url.starts_with("postgres://reader@db.internal:5433/app?sslmode=prefer"),
            "{}",
            url
        );
        let mut tls = cfg.clone();
        tls.ssl_mode = Some("require".to_string());
        assert!(tls.validate().is_err());
        assert_eq!(
            redact_password("postgres://reader:s3cret@db.internal:5433/app"),
            "postgres://***@db.internal:5433/app"
        );
        assert_eq!(
            redact_password("postgres://reader@db.internal/app"),
            "postgres://reader@db.internal/app"
        );
        let sqlite = sqlite_config("SELECT 1");
        assert_eq!(sqlite.build_url().unwrap(), "sqlite:///tmp/source.db");
    }

    #[test]
    fn test_watermark_wrap_and_unsafe_fallback() {
        let mut cfg = sqlite_config("SELECT id, updated FROM t");
        cfg.watermark_column = Some("updated".to_string());
        let wrapped = cfg.query_for_sync(Some("2026-01-01T00:00:00Z"));
        assert!(
            wrapped.contains("WHERE _sync.\"updated\" > '2026-01-01T00:00:00Z'"),
            "{}",
            wrapped
        );
        assert!(wrapped.contains("ORDER BY"), "{}", wrapped);
        let fallback = cfg.query_for_sync(Some("x'; DROP TABLE t; --"));
        assert!(!fallback.contains("WHERE"), "{}", fallback);
        assert_eq!(cfg.query_for_sync(None), "SELECT id, updated FROM t");
    }

    #[tokio::test]
    async fn test_sqlite_fetch_round_trip_with_watermark() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.db");
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let setup = sqlx::sqlite::SqlitePoolOptions::new()
            .connect(&url)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE people (uid TEXT, name TEXT, updated TEXT)")
            .execute(&setup)
            .await
            .unwrap();
        for (uid, name, updated) in [
            ("u-1", "Ada", "2026-01-01T00:00:00Z"),
            ("u-2", "Bob", "2026-02-01T00:00:00Z"),
        ] {
            sqlx::query("INSERT INTO people (uid, name, updated) VALUES (?, ?, ?)")
                .bind(uid)
                .bind(name)
                .bind(updated)
                .execute(&setup)
                .await
                .unwrap();
        }
        drop(setup);

        let mut cfg = sqlite_config("SELECT uid, name, updated FROM people");
        cfg.path = Some(path.to_string_lossy().to_string());
        cfg.watermark_column = Some("updated".to_string());
        let full = fetch_sql_rows(&cfg, None).await.unwrap();
        assert_eq!(full.rows.len(), 2);
        assert_eq!(full.new_watermark.as_deref(), Some("2026-02-01T00:00:00Z"));

        let delta = fetch_sql_rows(&cfg, full.new_watermark.as_deref())
            .await
            .unwrap();
        assert!(delta.rows.is_empty());
    }
}
