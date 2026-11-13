use crate::error::AiError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub inputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    pub embeddings: Vec<Vec<f32>>,
    pub model: String,
    pub dimensions: usize,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiHealthReport {
    pub healthy: bool,
    pub provider: String,
    pub chat_model: String,
    pub embedding_model: String,
    pub latency_ms: f64,
    pub message: String,
}

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn health_check(&self) -> Result<AiHealthReport, AiError>;
    async fn chat_completion(&self, req: &ChatRequest) -> Result<ChatResponse, AiError>;
    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse, AiError>;
}
