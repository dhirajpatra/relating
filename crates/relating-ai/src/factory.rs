use crate::error::AiError;
use crate::mock::MockAiProvider;
use crate::openai_compatible::OpenAiCompatibleProvider;
use crate::provider::AiProvider;
use relating_core::{AiProviderConfig, AiProviderType};
use std::sync::Arc;

pub fn create_ai_provider(config: &AiProviderConfig) -> Result<Arc<dyn AiProvider>, AiError> {
    match config.provider_type {
        AiProviderType::Mock => Ok(Arc::new(MockAiProvider {
            chat_model: config.chat_model.clone(),
            embedding_model: config.embedding_model.clone(),
            dimensions: config.embedding_dimensions.unwrap_or(1536),
        })),
        AiProviderType::Openai | AiProviderType::Ollama => {
            Ok(Arc::new(OpenAiCompatibleProvider::new(
                config.base_url.clone(),
                config.api_key.clone(),
                config.chat_model.clone(),
                config.embedding_model.clone(),
            )))
        }
        AiProviderType::Anthropic => {
            Err(AiError::Internal("Anthropic adapter is scheduled for the next slice".to_string()))
        }
    }
}
