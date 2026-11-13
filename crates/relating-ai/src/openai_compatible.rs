use crate::error::AiError;
use crate::provider::{
    AiHealthReport, AiProvider, ChatMessage, ChatRequest, ChatResponse, EmbeddingRequest,
    EmbeddingResponse,
};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub struct OpenAiCompatibleProvider {
    pub base_url: String,
    pub api_key: Option<String>,
    pub chat_model: String,
    pub embedding_model: String,
    client: reqwest::Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        base_url: Option<String>,
        api_key: Option<String>,
        chat_model: String,
        embedding_model: String,
    ) -> Self {
        let base_url = base_url
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string())
            .trim_end_matches('/')
            .to_string();

        Self {
            base_url,
            api_key,
            chat_model,
            embedding_model,
            client: reqwest::Client::new(),
        }
    }

    fn build_headers(&self) -> Result<HeaderMap, AiError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(ref key) = self.api_key {
            let auth_val = format!("Bearer {}", key);
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&auth_val)
                    .map_err(|e| AiError::Internal(format!("Invalid auth header: {}", e)))?,
            );
        }

        Ok(headers)
    }
}

#[derive(Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct OpenAiChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChatChoice>,
    model: Option<String>,
    usage: Option<OpenAiUsage>,
}

#[derive(Serialize)]
struct OpenAiEmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct OpenAiEmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
    model: Option<String>,
    usage: Option<OpenAiUsage>,
}

#[async_trait]
impl AiProvider for OpenAiCompatibleProvider {
    async fn health_check(&self) -> Result<AiHealthReport, AiError> {
        let start = Instant::now();
        let headers = self.build_headers()?;
        let test_url = format!("{}/models", self.base_url);

        let resp = self
            .client
            .get(&test_url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| AiError::ConnectionFailed(format!("Failed to reach {}: {}", test_url, e)))?;

        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        let status = resp.status();

        if status.is_success() {
            Ok(AiHealthReport {
                healthy: true,
                provider: "openai_compatible".to_string(),
                chat_model: self.chat_model.clone(),
                embedding_model: self.embedding_model.clone(),
                latency_ms,
                message: format!("Successfully verified endpoint at {}", self.base_url),
            })
        } else {
            let error_text = resp.text().await.unwrap_or_default();
            Err(AiError::ProviderError {
                status: status.as_u16(),
                message: error_text,
            })
        }
    }

    async fn chat_completion(&self, req: &ChatRequest) -> Result<ChatResponse, AiError> {
        let headers = self.build_headers()?;
        let url = format!("{}/chat/completions", self.base_url);

        let body = OpenAiChatRequest {
            model: &self.chat_model,
            messages: &req.messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
        };

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::ConnectionFailed(format!("POST {} failed: {}", url, e)))?;

        let status = resp.status();
        if !status.is_success() {
            let message = resp.text().await.unwrap_or_default();
            return Err(AiError::ProviderError {
                status: status.as_u16(),
                message,
            });
        }

        let parsed: OpenAiChatResponse = resp
            .json()
            .await
            .map_err(|e| AiError::Serialization(format!("Failed to parse response: {}", e)))?;

        let first_choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| AiError::Internal("No completion choices returned".to_string()))?;

        let (prompt_tokens, completion_tokens) = parsed
            .usage
            .map(|u| (u.prompt_tokens.unwrap_or(0), u.completion_tokens.unwrap_or(0)))
            .unwrap_or((0, 0));

        Ok(ChatResponse {
            content: first_choice.message.content,
            model: parsed.model.unwrap_or_else(|| self.chat_model.clone()),
            prompt_tokens,
            completion_tokens,
        })
    }

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse, AiError> {
        let headers = self.build_headers()?;
        let url = format!("{}/embeddings", self.base_url);

        let body = OpenAiEmbeddingRequest {
            model: &self.embedding_model,
            input: &req.inputs,
        };

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::ConnectionFailed(format!("POST {} failed: {}", url, e)))?;

        let status = resp.status();
        if !status.is_success() {
            let message = resp.text().await.unwrap_or_default();
            return Err(AiError::ProviderError {
                status: status.as_u16(),
                message,
            });
        }

        let parsed: OpenAiEmbeddingResponse = resp
            .json()
            .await
            .map_err(|e| AiError::Serialization(format!("Failed to parse embeddings: {}", e)))?;

        let dimensions = parsed
            .data
            .first()
            .map(|d| d.embedding.len())
            .unwrap_or(0);

        let embeddings = parsed.data.into_iter().map(|d| d.embedding).collect();
        let total_tokens = parsed.usage.and_then(|u| u.total_tokens).unwrap_or(0);

        Ok(EmbeddingResponse {
            embeddings,
            model: parsed.model.unwrap_or_else(|| self.embedding_model.clone()),
            dimensions,
            total_tokens,
        })
    }
}
