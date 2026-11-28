//! REST API sources: paginated JSON fetches with authentication, retry,
//! and rate limiting.
//!
//! One sync walks pages until an empty page or `max_pages`, sleeping the
//! rate-limit interval between requests and retrying transport errors,
//! 429s, and 5xx with capped exponential backoff. Bodies are truncated in
//! errors so response payloads never flood logs.

use crate::error::ConnectorError;
use crate::mapping::RowMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationConfig {
    /// `none`, `offset` (`?offset=N&limit=S`), or `page` (`?page=N&limit=S`).
    pub mode: String,
    #[serde(default)]
    pub offset_param: Option<String>,
    #[serde(default)]
    pub page_param: Option<String>,
    #[serde(default)]
    pub size_param: Option<String>,
    #[serde(default)]
    pub page_size: Option<u64>,
    #[serde(default)]
    pub max_pages: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestConfig {
    /// Absolute `http(s)` endpoint returning JSON.
    pub base_url: String,
    /// `GET` (default) or `POST`.
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Env var holding a bearer token, resolved via the secrets chain.
    #[serde(default)]
    pub bearer_env: Option<String>,
    #[serde(default)]
    pub basic_username: Option<String>,
    /// Env var holding the basic-auth password.
    #[serde(default)]
    pub basic_password_env: Option<String>,
    /// JSON body for POST requests (pagination still rides query params).
    #[serde(default)]
    pub body_json: Option<serde_json::Value>,
    /// Dot path to the record array (`data.items`); empty means the root
    /// response must itself be an array.
    #[serde(default)]
    pub record_path: Option<String>,
    #[serde(default)]
    pub pagination: Option<PaginationConfig>,
    /// Requests per second between pages (default 5, max 50).
    #[serde(default)]
    pub rate_limit_per_sec: Option<f64>,
    /// Attempts per page before the sync fails (default 3, max 6).
    #[serde(default)]
    pub max_attempts: Option<u32>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Scheduler hint: seconds between polls (`/connectors/due` honors it).
    #[serde(default)]
    pub poll_interval_secs: Option<u64>,
}

pub struct RestFetch {
    pub rows: Vec<RowMap>,
    pub pages_fetched: u64,
    pub items_skipped: u64,
}

impl RestConfig {
    pub fn effective_method(&self) -> Result<&'static str, ConnectorError> {
        match self
            .method
            .as_deref()
            .unwrap_or("GET")
            .trim()
            .to_ascii_uppercase()
            .as_str()
        {
            "GET" => Ok("GET"),
            "POST" => Ok("POST"),
            other => Err(ConnectorError::Config(format!(
                "Unsupported REST method '{}': use GET or POST",
                other
            ))),
        }
    }

    pub fn effective_rate(&self) -> f64 {
        self.rate_limit_per_sec.unwrap_or(5.0).clamp(0.1, 50.0)
    }

    pub fn effective_attempts(&self) -> u32 {
        self.max_attempts.unwrap_or(3).clamp(1, 6)
    }

    pub fn effective_timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.unwrap_or(15).clamp(1, 120))
    }

    pub fn validate(&self) -> Result<(), ConnectorError> {
        let url = reqwest::Url::parse(self.base_url.trim())
            .map_err(|e| ConnectorError::Config(format!("Invalid base_url: {}", e)))?;
        match url.scheme() {
            "http" | "https" => {}
            other => {
                return Err(ConnectorError::Config(format!(
                    "base_url scheme '{}' is not http or https",
                    other
                )));
            }
        }
        self.effective_method()?;
        if let Some(path) = self.record_path.as_deref() {
            validate_record_path(path)?;
        }
        for (name, value) in &self.headers {
            if name.trim().is_empty() || name.bytes().any(|b| b.is_ascii_whitespace() || b == b':')
            {
                return Err(ConnectorError::Config(format!(
                    "Invalid header name '{}'",
                    name
                )));
            }
            if value.bytes().any(|b| b == b'\r' || b == b'\n') {
                return Err(ConnectorError::Config(format!(
                    "Invalid header value for '{}'",
                    name
                )));
            }
        }
        if self.body_json.is_some() && self.effective_method()? != "POST" {
            return Err(ConnectorError::Config(
                "body_json needs method POST".to_string(),
            ));
        }
        if let Some(pagination) = self.pagination.as_ref() {
            match pagination.mode.trim().to_ascii_lowercase().as_str() {
                "none" | "offset" | "page" => {}
                other => {
                    return Err(ConnectorError::Config(format!(
                        "Unknown pagination mode '{}': use none, offset, or page",
                        other
                    )));
                }
            }
        }
        Ok(())
    }
}

fn validate_record_path(path: &str) -> Result<(), ConnectorError> {
    if path.is_empty() {
        return Ok(());
    }
    let ok = !path.starts_with('.')
        && !path.ends_with('.')
        && !path.contains("..")
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.');
    if ok {
        Ok(())
    } else {
        Err(ConnectorError::Config(format!(
            "Invalid record_path '{}': dot-separated keys only",
            path
        )))
    }
}

/// Extract the record array at a dot path. Non-object items are skipped and
/// counted; a missing path or non-array value is a config-shaped error.
pub fn extract_records(
    body: &serde_json::Value,
    record_path: &str,
) -> Result<(Vec<RowMap>, u64), ConnectorError> {
    let mut current = body;
    if !record_path.is_empty() {
        for key in record_path.split('.') {
            current = current.get(key).ok_or_else(|| {
                ConnectorError::Rest(format!(
                    "record_path '{}' not found in response (saw keys: {})",
                    record_path,
                    body.as_object()
                        .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
                        .unwrap_or_else(|| "(non-object root)".to_string())
                ))
            })?;
        }
    }
    let items = current.as_array().ok_or_else(|| {
        ConnectorError::Rest(format!(
            "record_path '{}' is not a JSON array",
            if record_path.is_empty() {
                "<root>"
            } else {
                record_path
            }
        ))
    })?;
    let mut rows = Vec::with_capacity(items.len());
    let mut skipped = 0u64;
    for item in items {
        match item.as_object() {
            Some(map) => rows.push(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            None => skipped += 1,
        }
    }
    Ok((rows, skipped))
}

fn page_url(
    base: &str,
    pagination: Option<&PaginationConfig>,
    page_index: u64,
) -> Result<String, ConnectorError> {
    let Some(pagination) = pagination else {
        return Ok(base.to_string());
    };
    let mode = pagination.mode.trim().to_ascii_lowercase();
    if mode == "none" {
        return Ok(base.to_string());
    }
    let size = pagination.page_size.unwrap_or(100).clamp(1, 1000);
    let size_param = pagination.size_param.as_deref().unwrap_or("limit");
    let mut url = reqwest::Url::parse(base)
        .map_err(|e| ConnectorError::Config(format!("Invalid base_url: {}", e)))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair(size_param, &size.to_string());
        if mode == "offset" {
            let offset_param = pagination.offset_param.as_deref().unwrap_or("offset");
            pairs.append_pair(offset_param, &(page_index * size).to_string());
        } else {
            let page_param = pagination.page_param.as_deref().unwrap_or("page");
            pairs.append_pair(page_param, &(page_index + 1).to_string());
        }
    }
    Ok(url.to_string())
}

/// Capped exponential backoff between attempts: 200ms * 2^attempt, honoring
/// `Retry-After` when the server names a longer wait (capped at 30s).
pub fn backoff_ms(attempt: u32, retry_after_secs: Option<u64>) -> u64 {
    let exponential = 200u64.saturating_mul(2u64.saturating_pow(attempt.min(8)));
    let base = exponential.min(5_000);
    match retry_after_secs {
        Some(secs) => base.max(secs.min(30).saturating_mul(1_000)),
        None => base,
    }
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
}

fn truncate_body(body: &str) -> String {
    const LIMIT: usize = 300;
    if body.len() <= LIMIT {
        body.to_string()
    } else {
        format!("{}…", &body[..LIMIT])
    }
}

/// Loopback sources (tests, sidecars) always go direct; everything else
/// honors the process proxy environment (`HTTP_PROXY`/`NO_PROXY`). Without
/// the carve-out, a sandbox or corporate `ALL_PROXY` would route even
/// `127.0.0.1` through the gateway.
fn url_is_loopback(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    matches!(
        parsed
            .host_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "localhost" | "127.0.0.1" | "::1" | "[::1]"
    )
}

pub async fn fetch_rest_rows(config: &RestConfig) -> Result<RestFetch, ConnectorError> {
    config.validate()?;
    let method = config.effective_method()?;
    let attempts = config.effective_attempts();
    let interval = Duration::from_secs_f64(1.0 / config.effective_rate());
    let max_pages = config
        .pagination
        .as_ref()
        .and_then(|p| p.max_pages)
        .unwrap_or(20)
        .clamp(1, 200);
    let record_path = config.record_path.as_deref().unwrap_or("");

    let bearer = match config.bearer_env.as_deref() {
        Some(var) if !var.trim().is_empty() => {
            Some(relating_core::resolve_secret(var.trim()).map_err(ConnectorError::Rest)?)
        }
        _ => None,
    };
    let basic_password = match config.basic_password_env.as_deref() {
        Some(var) if !var.trim().is_empty() => {
            Some(relating_core::resolve_secret(var.trim()).map_err(ConnectorError::Rest)?)
        }
        _ => None,
    };

    let mut builder = reqwest::Client::builder().timeout(config.effective_timeout());
    if url_is_loopback(&config.base_url) {
        builder = builder.no_proxy();
    }
    let client = builder
        .build()
        .map_err(|e| ConnectorError::Rest(format!("Failed to build HTTP client: {}", e)))?;

    let mut rows = Vec::new();
    let mut skipped = 0u64;
    let mut page_index = 0u64;
    loop {
        if page_index > 0 {
            tokio::time::sleep(interval).await;
        }
        let url = page_url(&config.base_url, config.pagination.as_ref(), page_index)?;
        let mut attempt = 0u32;
        let body: serde_json::Value = loop {
            let mut request = if method == "POST" {
                let mut request = client.post(&url);
                if let Some(payload) = config.body_json.as_ref() {
                    request = request.json(payload);
                }
                request
            } else {
                client.get(&url)
            };
            for (name, value) in &config.headers {
                request = request.header(name.as_str(), value.as_str());
            }
            if let Some(token) = bearer.as_deref() {
                request = request.bearer_auth(token);
            }
            if let Some(user) = config.basic_username.as_deref() {
                request = request.basic_auth(user, basic_password.clone());
            }
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        break response.json().await.map_err(|e| {
                            ConnectorError::Rest(format!("Response is not valid JSON: {}", e))
                        })?;
                    }
                    let retryable = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || status.is_server_error();
                    let retry_after = parse_retry_after(response.headers());
                    let detail = response
                        .text()
                        .await
                        .map(|t| truncate_body(t.trim()))
                        .unwrap_or_default();
                    if retryable && attempt + 1 < attempts {
                        let wait = backoff_ms(attempt, retry_after);
                        tracing::warn!(
                            "REST {} {} (attempt {}); retrying in {}ms",
                            status,
                            url,
                            attempt + 1,
                            wait
                        );
                        tokio::time::sleep(Duration::from_millis(wait)).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(ConnectorError::Rest(format!(
                        "REST request failed with {}: {}",
                        status, detail
                    )));
                }
                Err(e) if e.is_timeout() || e.is_connect() || e.is_request() => {
                    if attempt + 1 < attempts {
                        // reqwest timeouts carry no headers; plain backoff.
                        let wait = backoff_ms(attempt, None);
                        tracing::warn!(
                            "REST transport error (attempt {}); retrying in {}ms: {}",
                            attempt + 1,
                            wait,
                            e
                        );
                        tokio::time::sleep(Duration::from_millis(wait)).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(ConnectorError::Rest(format!("REST transport error: {}", e)));
                }
                Err(e) => return Err(ConnectorError::Rest(format!("REST request error: {}", e))),
            }
        };
        let (mut page_rows, page_skipped) = extract_records(&body, record_path)?;
        skipped += page_skipped;
        let done = page_rows.is_empty()
            || config
                .pagination
                .as_ref()
                .map(|p| p.mode.trim().eq_ignore_ascii_case("none") || page_index + 1 >= max_pages)
                .unwrap_or(true);
        rows.append(&mut page_rows);
        page_index += 1;
        if done {
            break;
        }
    }
    Ok(RestFetch {
        rows,
        pages_fetched: page_index,
        items_skipped: skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rest_config(url: &str) -> RestConfig {
        RestConfig {
            base_url: url.to_string(),
            method: None,
            headers: HashMap::new(),
            bearer_env: None,
            basic_username: None,
            basic_password_env: None,
            body_json: None,
            record_path: None,
            pagination: None,
            rate_limit_per_sec: None,
            max_attempts: None,
            timeout_secs: None,
            poll_interval_secs: None,
        }
    }

    #[test]
    fn test_config_validation() {
        assert!(rest_config("https://api.example/items").validate().is_ok());
        assert!(rest_config("ftp://x/y").validate().is_err());
        assert!(rest_config("not a url").validate().is_err());
        let mut bad_method = rest_config("https://api.example/items");
        bad_method.method = Some("DELETE".to_string());
        assert!(bad_method.validate().is_err());
        let mut bad_path = rest_config("https://api.example/items");
        bad_path.record_path = Some("data..items".to_string());
        assert!(bad_path.validate().is_err());
        let mut bad_header = rest_config("https://api.example/items");
        bad_header
            .headers
            .insert("X-Bad\nName".to_string(), "v".to_string());
        assert!(bad_header.validate().is_err());
        let mut body_get = rest_config("https://api.example/items");
        body_get.body_json = Some(serde_json::json!({}));
        assert!(body_get.validate().is_err());
    }

    #[test]
    fn test_extract_records_nested_and_skips_scalars() {
        let body = serde_json::json!({
            "data": {"items": [{"uid": "a"}, 42, {"uid": "b"}]}
        });
        let (rows, skipped) = extract_records(&body, "data.items").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(skipped, 1);
        assert!(extract_records(&body, "data.missing").is_err());
        assert!(extract_records(&body, "data").is_err());
        let root = serde_json::json!([{"uid": "x"}]);
        let (rows, _) = extract_records(&root, "").unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn test_page_url_modes() {
        assert_eq!(page_url("https://a/x", None, 3).unwrap(), "https://a/x");
        let none = PaginationConfig {
            mode: "none".to_string(),
            offset_param: None,
            page_param: None,
            size_param: None,
            page_size: None,
            max_pages: None,
        };
        assert_eq!(
            page_url("https://a/x", Some(&none), 3).unwrap(),
            "https://a/x"
        );
        let offset = PaginationConfig {
            mode: "offset".to_string(),
            offset_param: None,
            page_param: None,
            size_param: None,
            page_size: Some(50),
            max_pages: None,
        };
        assert_eq!(
            page_url("https://a/x", Some(&offset), 2).unwrap(),
            "https://a/x?limit=50&offset=100"
        );
        let page = PaginationConfig {
            mode: "page".to_string(),
            offset_param: None,
            page_param: Some("p".to_string()),
            size_param: Some("n".to_string()),
            page_size: None,
            max_pages: None,
        };
        assert_eq!(
            page_url("https://a/x?token=t", Some(&page), 0).unwrap(),
            "https://a/x?token=t&n=100&p=1"
        );
    }

    #[test]
    fn test_backoff_caps_and_retry_after() {
        assert_eq!(backoff_ms(0, None), 200);
        assert_eq!(backoff_ms(1, None), 400);
        assert_eq!(backoff_ms(99, None), 5_000);
        assert_eq!(backoff_ms(0, Some(2)), 2_000);
        assert_eq!(backoff_ms(0, Some(99)), 30_000);
    }

    #[test]
    fn test_loopback_detection() {
        assert!(url_is_loopback("http://127.0.0.1:9/x"));
        assert!(url_is_loopback("http://localhost/x"));
        assert!(!url_is_loopback("https://api.example/x"));
        assert!(!url_is_loopback("not a url"));
    }

    #[tokio::test]
    async fn test_fetch_retries_then_paginates() {
        use crate::test_stub::{start_json_stub, StubRoute};
        let (base, script, server) = start_json_stub(vec![(
            "/items".to_string(),
            vec![
                StubRoute::status(503, "Busy", "{}"),
                StubRoute::json(r#"[{"uid":"a"},{"uid":"b"}]"#),
                StubRoute::json(r#"[]"#),
            ],
        )])
        .await;
        let mut cfg = rest_config(&format!("{}/items", base));
        cfg.pagination = Some(PaginationConfig {
            mode: "offset".to_string(),
            offset_param: None,
            page_param: None,
            size_param: None,
            page_size: Some(2),
            max_pages: Some(5),
        });
        cfg.rate_limit_per_sec = Some(50.0);
        let fetch = fetch_rest_rows(&cfg).await.unwrap();
        assert_eq!(fetch.rows.len(), 2);
        assert_eq!(fetch.pages_fetched, 2);
        // 503 retried once, then page 0, then empty page 1.
        assert_eq!(script.hits().len(), 3);
        server.abort();
    }
}
