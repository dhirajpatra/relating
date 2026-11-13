use crate::error::AiError;
use crate::provider::{
    AiHealthReport, AiProvider, ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse,
};
use async_trait::async_trait;

pub struct MockAiProvider {
    pub chat_model: String,
    pub embedding_model: String,
    pub dimensions: usize,
}

impl Default for MockAiProvider {
    fn default() -> Self {
        Self {
            chat_model: "mock-chat-v1".to_string(),
            embedding_model: "mock-embed-v1".to_string(),
            dimensions: 1536,
        }
    }
}

impl MockAiProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AiProvider for MockAiProvider {
    async fn health_check(&self) -> Result<AiHealthReport, AiError> {
        Ok(AiHealthReport {
            healthy: true,
            provider: "mock".to_string(),
            chat_model: self.chat_model.clone(),
            embedding_model: self.embedding_model.clone(),
            latency_ms: 0.5,
            message: "Mock AI provider is operational (offline mode)".to_string(),
        })
    }

    async fn chat_completion(&self, req: &ChatRequest) -> Result<ChatResponse, AiError> {
        let last_msg = req
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("empty prompt");

        Ok(ChatResponse {
            content: format!("[Mock AI Response to: '{}']", last_msg),
            model: self.chat_model.clone(),
            prompt_tokens: 10,
            completion_tokens: 15,
        })
    }

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse, AiError> {
        let mut embeddings = Vec::new();
        for input in &req.inputs {
            // Generate deterministic pseudo-embedding based on input hash/length
            let seed = input.len() as f32;
            let mut vec = Vec::with_capacity(self.dimensions);
            for i in 0..self.dimensions {
                vec.push(((i as f32 + seed) % 100.0) / 100.0);
            }
            embeddings.push(vec);
        }

        Ok(EmbeddingResponse {
            embeddings,
            model: self.embedding_model.clone(),
            dimensions: self.dimensions,
            total_tokens: req.inputs.len() as u32 * 5,
        })
    }
}
