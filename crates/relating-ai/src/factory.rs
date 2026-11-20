use crate::error::AiError;
use crate::meta_api::MetaApiProvider;
use crate::mock::MockAiProvider;
use crate::openai_compatible::OpenAiCompatibleProvider;
use crate::provider::AiProvider;
use relating_core::{AiProviderConfig, AiProviderType};
use std::sync::Arc;

pub fn create_ai_provider(config: &AiProviderConfig) -> Result<Arc<dyn AiProvider>, AiError> {
    // `.env` wins when it carries a complete Meta setup and the project did
    // not pick an explicit provider: `mock` is the default "unconfigured"
    // state, so routing it to Meta honors `LLM_BACKEND=meta_api` without
    // touching projects that explicitly chose OpenAI/Ollama. Explicit
    // `meta_api` projects always use Meta (stored fields win, env fills
    // blanks). Nothing here logs credentials.
    if config.provider_type == AiProviderType::Mock {
        if let Some(meta) = MetaApiProvider::try_from_env() {
            tracing::info!(
                "LLM_BACKEND=meta_api with credentials in the environment: using Meta API (Ollama fallback) instead of the mock provider"
            );
            return Ok(Arc::new(meta));
        }
    }
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
        AiProviderType::MetaApi => Ok(Arc::new(MetaApiProvider::from_config(config)?)),
        AiProviderType::Anthropic => Err(AiError::Internal(
            "Anthropic adapter is scheduled for the next slice".to_string(),
        )),
    }
}
