//! Meta API LLM backend (OpenAI-style Responses API).
//!
//! Configuration comes from the environment (`.env`), matching the keys the
//! project already documents:
//!
//! - `LLM_BACKEND=meta_api` — enables this backend. Any other value (or an
//!   unset variable) leaves project configuration untouched.
//! - `META_API_KEY` (or `MODEL_API_KEY`) — bearer token, resolved through
//!   the usual secret backends so `_FILE`/keychain sidecars keep working.
//! - `META_API_URL` — full Responses endpoint
//!   (default `https://api.meta.ai/v1/responses`).
//! - `META_MODEL` (default `muse-spark-1.3-contributor`).
//! - `MAX_OUTPUT_TOKENS` (default `1500`).
//! - `FALLBACK_TIMEOUT` — seconds to wait for Meta before trying Ollama
//!   (default `25`).
//!
//! Behavior notes (all verified live against the API):
//!
//! - Chat posts `{model, instructions?, input[], max_output_tokens,
//!   reasoning: {effort: "minimal"}}`. System messages become
//!   `instructions`; user turns use `input_text` items and assistant turns
//!   use `output_text` items (the API rejects `input_text` on assistant
//!   messages). Reasoning defaults to `minimal`: the default effort burns
//!   the whole token budget on reasoning before emitting any text.
//! - Requests carry a floor of 512 output tokens: below that, reasoning
//!   alone exhausts the budget and the API returns `incomplete` with no
//!   message. Larger caller budgets are always honored.
//! - Meta exposes no embeddings endpoint (`/v1/embeddings` is 404), so
//!   `embed` delegates to local Ollama (`OLLAMA_BASE_URL`, default
//!   `http://localhost:${OLLAMA_PORT:-11434}/v1`, model `OLLAMA_EMBED_MODEL`
//!   default `nomic-embed-text`). Chat falls back to Ollama
//!   (`OLLAMA_CHAT_MODEL`, default `llama3.1`) on any Meta error/timeout and
//!   returns the original Meta error when Ollama fails too.

use crate::error::AiError;
use crate::openai_compatible::OpenAiCompatibleProvider;
use crate::provider::{
    AiHealthReport, AiProvider, ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse,
};
use async_trait::async_trait;
use relating_core::{AiProviderConfig, AiProviderType};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const DEFAULT_META_API_URL: &str = "https://api.meta.ai/v1/responses";
pub const DEFAULT_META_MODEL: &str = "muse-spark-1.3-contributor";
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1500;
pub const DEFAULT_FALLBACK_TIMEOUT_SECS: u64 = 25;
pub const DEFAULT_OLLAMA_CHAT_MODEL: &str = "llama3.1";
pub const DEFAULT_OLLAMA_EMBED_MODEL: &str = "nomic-embed-text";
/// Responses from this API always spend 100+ tokens on reasoning, even at
/// `minimal` effort, so smaller budgets can never yield text.
pub const MIN_OUTPUT_TOKENS: u32 = 512;
/// Upper bound for the Ollama fallback attempt so a down fallback fails fast.
const FALLBACK_ATTEMPT_TIMEOUT_SECS: u64 = 10;

fn read_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn read_env_u64(name: &str, default: u64) -> u64 {
    read_env(name)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn read_env_u32(name: &str, default: u32) -> u32 {
    read_env(name)
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(default)
}

/// True when the operator selected the Meta backend in the environment.
pub fn meta_backend_selected() -> bool {
    read_env("LLM_BACKEND")
        .map(|v| v.eq_ignore_ascii_case("meta_api"))
        .unwrap_or(false)
}

fn meta_api_key() -> Option<String> {
    relating_core::resolve_secret("META_API_KEY")
        .ok()
        .or_else(|| relating_core::resolve_secret("MODEL_API_KEY").ok())
}

fn meta_api_url() -> String {
    read_env("META_API_URL").unwrap_or_else(|| DEFAULT_META_API_URL.to_string())
}

fn meta_model() -> String {
    read_env("META_MODEL").unwrap_or_else(|| DEFAULT_META_MODEL.to_string())
}

fn ollama_base_url() -> String {
    if let Some(base) = read_env("OLLAMA_BASE_URL") {
        return base;
    }
    let port = read_env("OLLAMA_PORT").unwrap_or_else(|| "11434".to_string());
    format!("http://localhost:{}/v1", port)
}

/// Derive the `/v1/models` health endpoint from a Responses endpoint URL.
fn models_url_for(responses_url: &str) -> String {
    let trimmed = responses_url.trim_end_matches('/');
    if let Some(base) = trimmed.strip_suffix("/responses") {
        format!("{}/models", base)
    } else {
        format!("{}/models", trimmed)
    }
}

/// Effective project configuration for the Meta backend from `.env`.
///
/// `None` when `LLM_BACKEND` is not `meta_api` or no API key resolves.
/// The embedding model names the Ollama model used for embeddings (Meta has
/// no embeddings endpoint), so embedding profiles record an honest model.
pub fn meta_ai_config_from_env() -> Option<AiProviderConfig> {
    if !meta_backend_selected() {
        return None;
    }
    let api_key = meta_api_key()?;
    Some(AiProviderConfig {
        provider_type: AiProviderType::MetaApi,
        base_url: Some(meta_api_url()),
        api_key: Some(api_key),
        chat_model: meta_model(),
        embedding_model: read_env("OLLAMA_EMBED_MODEL")
            .unwrap_or_else(|| DEFAULT_OLLAMA_EMBED_MODEL.to_string()),
        embedding_dimensions: None,
    })
}

// ---------------------------------------------------------------------------
// Responses API wire format
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ResponsesReasoning {
    effort: &'static str,
}

#[derive(Debug, Serialize)]
struct ResponsesContentItem {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct ResponsesInputItem {
    role: String,
    content: Vec<ResponsesContentItem>,
}

#[derive(Debug, Serialize)]
struct ResponsesRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    input: Vec<ResponsesInputItem>,
    max_output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    reasoning: ResponsesReasoning,
}

/// Map chat messages onto a Responses request.
///
/// System/developer messages join `instructions`; user and assistant turns
/// become `input` items. Assistant content must use `output_text` (the API
/// rejects `input_text` there with `invalid_request_error`).
fn build_responses_body(
    messages: &[crate::provider::ChatMessage],
    model: &str,
    max_output_tokens: u32,
    temperature: Option<f32>,
) -> ResponsesRequest {
    let mut instructions: Vec<String> = Vec::new();
    let mut input: Vec<ResponsesInputItem> = Vec::new();
    for msg in messages {
        let role = msg.role.trim().to_ascii_lowercase();
        match role.as_str() {
            "system" | "developer" => instructions.push(msg.content.clone()),
            "assistant" => input.push(ResponsesInputItem {
                role: "assistant".to_string(),
                content: vec![ResponsesContentItem {
                    kind: "output_text".to_string(),
                    text: msg.content.clone(),
                }],
            }),
            _ => input.push(ResponsesInputItem {
                role: "user".to_string(),
                content: vec![ResponsesContentItem {
                    kind: "input_text".to_string(),
                    text: msg.content.clone(),
                }],
            }),
        }
    }
    if input.is_empty() {
        input.push(ResponsesInputItem {
            role: "user".to_string(),
            content: vec![ResponsesContentItem {
                kind: "input_text".to_string(),
                text: "(empty)".to_string(),
            }],
        });
    }

    ResponsesRequest {
        model: model.to_string(),
        instructions: if instructions.is_empty() {
            None
        } else {
            Some(instructions.join("\n\n"))
        },
        input,
        max_output_tokens,
        temperature,
        reasoning: ResponsesReasoning { effort: "minimal" },
    }
}

#[derive(Debug, Deserialize)]
struct ResponsesErrorBody {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponsesOutputContent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponsesOutputItem {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    content: Option<Vec<ResponsesOutputContent>>,
}

#[derive(Debug, Deserialize)]
struct ResponsesUsage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ResponsesResponse {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    output: Vec<ResponsesOutputItem>,
    #[serde(default)]
    usage: Option<ResponsesUsage>,
    #[serde(default)]
    error: Option<ResponsesErrorBody>,
}

/// Extract assistant text plus token counts from a Responses payload.
fn extract_text_and_usage(body: &ResponsesResponse) -> Result<(String, u32, u32), String> {
    if let Some(err) = &body.error {
        return Err(err
            .message
            .clone()
            .unwrap_or_else(|| "Meta API returned an error".to_string()));
    }
    let mut texts = Vec::new();
    for item in &body.output {
        if item.kind != "message" {
            continue;
        }
        for content in item.content.as_deref().unwrap_or(&[]) {
            if content.kind == "output_text" {
                if let Some(text) = &content.text {
                    texts.push(text.clone());
                }
            }
        }
    }
    let text = texts.join("").trim().to_string();
    if text.is_empty() {
        let status = body.status.as_deref().unwrap_or("unknown");
        return Err(format!(
            "Meta API returned status '{}' with no message text (the token budget may have been exhausted by reasoning)",
            status
        ));
    }
    let (prompt_tokens, completion_tokens) = body
        .usage
        .as_ref()
        .map(|u| (u.input_tokens.unwrap_or(0), u.output_tokens.unwrap_or(0)))
        .unwrap_or((0, 0));
    Ok((text, prompt_tokens, completion_tokens))
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

pub struct MetaApiProvider {
    responses_url: String,
    models_url: String,
    api_key: String,
    chat_model: String,
    embedding_model: String,
    default_max_output_tokens: u32,
    timeout: Duration,
    ollama: OpenAiCompatibleProvider,
    client: reqwest::Client,
}

impl MetaApiProvider {
    fn build(
        responses_url: String,
        api_key: String,
        chat_model: String,
        embedding_model: String,
    ) -> Self {
        let timeout_secs =
            read_env_u64("FALLBACK_TIMEOUT", DEFAULT_FALLBACK_TIMEOUT_SECS).clamp(1, 300);
        let default_max =
            read_env_u32("MAX_OUTPUT_TOKENS", DEFAULT_MAX_OUTPUT_TOKENS).clamp(1, 32000);
        let ollama = OpenAiCompatibleProvider::new(
            Some(ollama_base_url()),
            None,
            read_env("OLLAMA_CHAT_MODEL").unwrap_or_else(|| DEFAULT_OLLAMA_CHAT_MODEL.to_string()),
            embedding_model.clone(),
        );
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let models_url = models_url_for(&responses_url);
        Self {
            responses_url,
            models_url,
            api_key,
            chat_model,
            embedding_model,
            default_max_output_tokens: default_max,
            timeout: Duration::from_secs(timeout_secs),
            ollama,
            client,
        }
    }

    /// Build from the environment. `None` when the Meta backend is not
    /// selected or no API key resolves.
    pub fn try_from_env() -> Option<Self> {
        let config = meta_ai_config_from_env()?;
        Some(Self::build(
            config.base_url.unwrap_or_else(meta_api_url),
            config.api_key.unwrap_or_default(),
            config.chat_model,
            config.embedding_model,
        ))
    }

    /// Build from an explicit `meta_api` project config. Stored base URL,
    /// key and chat model win when set; empty fields fall back to the
    /// environment so projects can share the server `.env` credentials.
    pub fn from_config(config: &AiProviderConfig) -> Result<Self, AiError> {
        let responses_url = config
            .base_url
            .clone()
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(meta_api_url);
        let api_key = config
            .api_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(meta_api_key)
            .ok_or_else(|| {
                AiError::MissingCredentials(
                    "meta_api provider needs an API key: set META_API_KEY in .env or store one on the project".to_string(),
                )
            })?;
        // The generic cross-provider default (`gpt-4o-mini`) is not a Meta
        // model: when a project carries it verbatim, the env model wins.
        let chat_model =
            if config.chat_model.trim().is_empty() || config.chat_model == "gpt-4o-mini" {
                meta_model()
            } else {
                config.chat_model.clone()
            };
        let embedding_model = if config.embedding_model.trim().is_empty() {
            read_env("OLLAMA_EMBED_MODEL").unwrap_or_else(|| DEFAULT_OLLAMA_EMBED_MODEL.to_string())
        } else {
            config.embedding_model.clone()
        };
        Ok(Self::build(
            responses_url,
            api_key,
            chat_model,
            embedding_model,
        ))
    }

    fn effective_max_tokens(&self, requested: Option<u32>) -> u32 {
        requested
            .unwrap_or(self.default_max_output_tokens)
            .max(MIN_OUTPUT_TOKENS)
    }

    async fn chat_via_meta(&self, req: &ChatRequest) -> Result<ChatResponse, AiError> {
        let body = build_responses_body(
            &req.messages,
            &self.chat_model,
            self.effective_max_tokens(req.max_tokens),
            req.temperature,
        );
        let resp = self
            .client
            .post(&self.responses_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AiError::ConnectionFailed(format!(
                        "Meta API timed out after {}s (FALLBACK_TIMEOUT): {}",
                        self.timeout.as_secs(),
                        e
                    ))
                } else {
                    AiError::ConnectionFailed(format!("POST {} failed: {}", self.responses_url, e))
                }
            })?;

        let status = resp.status();
        if !status.is_success() {
            let message = resp.text().await.unwrap_or_default();
            let excerpt: String = message.chars().take(500).collect();
            return Err(AiError::ProviderError {
                status: status.as_u16(),
                message: format!("Meta API error: {}", excerpt),
            });
        }
        let parsed: ResponsesResponse = resp.json().await.map_err(|e| {
            AiError::Serialization(format!("Failed to parse Meta API response: {}", e))
        })?;
        let (content, prompt_tokens, completion_tokens) =
            extract_text_and_usage(&parsed).map_err(AiError::Internal)?;
        Ok(ChatResponse {
            content,
            model: self.chat_model.clone(),
            prompt_tokens,
            completion_tokens,
        })
    }
}

#[async_trait]
impl AiProvider for MetaApiProvider {
    async fn health_check(&self) -> Result<AiHealthReport, AiError> {
        let start = Instant::now();
        let probe = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let meta_ok = probe
            .get(&self.models_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

        // Best-effort Ollama probe so the report says whether the fallback
        // path is live. Never fails the check on its own.
        let ollama_ok = tokio::time::timeout(Duration::from_secs(2), self.ollama.health_check())
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false);

        if meta_ok {
            Ok(AiHealthReport {
                healthy: true,
                provider: "meta_api".to_string(),
                chat_model: self.chat_model.clone(),
                embedding_model: self.embedding_model.clone(),
                latency_ms,
                message: format!(
                    "Meta API reachable (model '{}'); Ollama fallback {}",
                    self.chat_model,
                    if ollama_ok {
                        "available"
                    } else {
                        "unavailable"
                    }
                ),
            })
        } else if ollama_ok {
            Ok(AiHealthReport {
                healthy: false,
                provider: "meta_api".to_string(),
                chat_model: self.chat_model.clone(),
                embedding_model: self.embedding_model.clone(),
                latency_ms,
                message: "Meta API is unreachable; Ollama fallback is available".to_string(),
            })
        } else {
            Err(AiError::ConnectionFailed(format!(
                "Meta API is unreachable at {} and no Ollama fallback is available at {}",
                self.models_url, self.ollama.base_url
            )))
        }
    }

    async fn chat_completion(&self, req: &ChatRequest) -> Result<ChatResponse, AiError> {
        match self.chat_via_meta(req).await {
            Ok(resp) => Ok(resp),
            Err(meta_err) => {
                tracing::warn!(
                    "Meta API chat failed ({}); trying Ollama fallback",
                    meta_err
                );
                match tokio::time::timeout(
                    Duration::from_secs(FALLBACK_ATTEMPT_TIMEOUT_SECS),
                    self.ollama.chat_completion(req),
                )
                .await
                {
                    Ok(Ok(resp)) => {
                        tracing::info!("Ollama fallback chat succeeded");
                        Ok(resp)
                    }
                    Ok(Err(fallback_err)) => {
                        tracing::warn!("Ollama fallback chat failed: {}", fallback_err);
                        Err(meta_err)
                    }
                    Err(_) => {
                        tracing::warn!("Ollama fallback chat timed out");
                        Err(meta_err)
                    }
                }
            }
        }
    }

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse, AiError> {
        // Meta exposes no embeddings endpoint, so this always delegates to
        // the configured Ollama model. Fail loudly when Ollama is down:
        // inventing vectors here would silently poison the embedding space.
        self.ollama.embed(req).await.map_err(|e| {
            AiError::ConnectionFailed(format!(
                "Meta API has no embeddings endpoint; Ollama fallback ({}) failed: {}",
                self.ollama.base_url, e
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ChatMessage, ChatRequest};
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
            for extra in [
                "RELATING_SECRET_BACKEND",
                "META_API_KEY_FILE",
                "MODEL_API_KEY_FILE",
            ] {
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
        "LLM_BACKEND",
        "META_API_KEY",
        "MODEL_API_KEY",
        "META_API_URL",
        "META_MODEL",
        "MAX_OUTPUT_TOKENS",
        "FALLBACK_TIMEOUT",
        "OLLAMA_BASE_URL",
        "OLLAMA_PORT",
        "OLLAMA_CHAT_MODEL",
        "OLLAMA_EMBED_MODEL",
    ];

    fn sample_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage {
                role: "system".to_string(),
                content: "Be terse.".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "My name is Ada.".to_string(),
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: "Hello Ada.".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "What is my name?".to_string(),
            },
        ]
    }

    #[test]
    fn test_request_mapping_matches_api_contract() {
        let body = build_responses_body(
            &sample_messages(),
            "muse-spark-1.3-contributor",
            900,
            Some(0.2),
        );
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(value["model"], "muse-spark-1.3-contributor");
        assert_eq!(value["instructions"], "Be terse.");
        assert_eq!(value["max_output_tokens"], 900);
        let temp = value["temperature"].as_f64().unwrap();
        assert!(
            (temp - 0.2).abs() < 1e-6,
            "temperature passthrough: {}",
            temp
        );
        assert_eq!(value["reasoning"]["effort"], "minimal");
        let input = value["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[1]["role"], "assistant");
        // The API rejects input_text on assistant messages.
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(input[2]["role"], "user");
    }

    #[test]
    fn test_empty_messages_yield_placeholder() {
        let body = build_responses_body(&[], "m", 900, None);
        let value = serde_json::to_value(&body).unwrap();
        assert!(value.get("instructions").is_none());
        assert_eq!(value["input"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_extract_text_skips_reasoning_items() {
        let raw = serde_json::json!({
            "status": "completed",
            "output": [
                {"type": "reasoning", "status": "completed"},
                {"type": "message", "role": "assistant", "status": "completed",
                 "content": [{"type": "output_text", "text": "Ada", "annotations": []}]},
            ],
            "usage": {"input_tokens": 30, "output_tokens": 62},
            "error": null,
        });
        let parsed: ResponsesResponse = serde_json::from_value(raw).unwrap();
        let (text, prompt, completion) = extract_text_and_usage(&parsed).unwrap();
        assert_eq!(text, "Ada");
        assert_eq!((prompt, completion), (30, 62));
    }

    #[test]
    fn test_extract_text_reports_api_errors() {
        let raw = serde_json::json!({
            "status": null, "output": [],
            "error": {"message": "content type `input_text` is not valid on `assistant` messages"},
        });
        let parsed: ResponsesResponse = serde_json::from_value(raw).unwrap();
        let err = extract_text_and_usage(&parsed).unwrap_err();
        assert!(err.contains("input_text"));
    }

    #[test]
    fn test_extract_text_fails_without_message() {
        let raw = serde_json::json!({
            "status": "incomplete",
            "output": [{"type": "reasoning", "status": "incomplete"}],
            "usage": {"input_tokens": 16, "output_tokens": 500},
            "error": null,
        });
        let parsed: ResponsesResponse = serde_json::from_value(raw).unwrap();
        let err = extract_text_and_usage(&parsed).unwrap_err();
        assert!(err.contains("incomplete"));
    }

    #[test]
    fn test_env_config_requires_backend_and_key() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::take(VARS);
        assert!(meta_ai_config_from_env().is_none());

        std::env::set_var("LLM_BACKEND", "meta_api");
        assert!(meta_ai_config_from_env().is_none());

        std::env::set_var("META_API_KEY", "test-key");
        let config = meta_ai_config_from_env().expect("env config");
        assert_eq!(config.provider_type, AiProviderType::MetaApi);
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://api.meta.ai/v1/responses")
        );
        assert_eq!(config.chat_model, "muse-spark-1.3-contributor");
        assert_eq!(config.embedding_model, "nomic-embed-text");

        // MODEL_API_KEY is accepted as an alias.
        std::env::remove_var("META_API_KEY");
        std::env::set_var("MODEL_API_KEY", "alias-key");
        assert!(meta_ai_config_from_env().is_some());

        // Any other backend disables the override.
        std::env::set_var("LLM_BACKEND", "ollama");
        assert!(meta_ai_config_from_env().is_none());
    }

    #[test]
    fn test_models_url_derivation() {
        assert_eq!(
            models_url_for("https://api.meta.ai/v1/responses"),
            "https://api.meta.ai/v1/models"
        );
        assert_eq!(
            models_url_for("https://api.meta.ai/v1/responses/"),
            "https://api.meta.ai/v1/models"
        );
        assert_eq!(
            models_url_for("https://example.com/custom"),
            "https://example.com/custom/models"
        );
    }

    #[test]
    fn test_token_floor_applies() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::take(VARS);
        std::env::set_var("LLM_BACKEND", "meta_api");
        std::env::set_var("META_API_KEY", "test-key");
        let provider = MetaApiProvider::try_from_env().unwrap();
        assert_eq!(provider.effective_max_tokens(None), 1500);
        assert_eq!(provider.effective_max_tokens(Some(10)), 512);
        assert_eq!(provider.effective_max_tokens(Some(2000)), 2000);
    }

    /// Live check against the real Meta API (needs network + a key).
    ///
    /// ```bash
    /// set -a; . ./.env; set +a
    /// cargo test -p relating-ai -- --ignored live_meta_chat --nocapture
    /// ```
    #[tokio::test]
    #[ignore]
    async fn live_meta_chat_answers() {
        let provider = MetaApiProvider::try_from_env()
            .expect("LLM_BACKEND=meta_api plus META_API_KEY must be set");
        let report = provider.health_check().await.expect("health check");
        assert!(report.healthy, "Meta health: {}", report.message);
        let resp = provider
            .chat_completion(&ChatRequest {
                messages: vec![ChatMessage {
                    role: "user".to_string(),
                    content: "Reply with exactly the word: pinecone".to_string(),
                }],
                temperature: Some(0.0),
                max_tokens: Some(800),
            })
            .await
            .expect("chat completion");
        assert!(
            resp.content.to_ascii_lowercase().contains("pinecone"),
            "unexpected answer: {}",
            resp.content
        );
    }
}
