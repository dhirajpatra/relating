pub mod error;
pub mod factory;
pub mod meta_api;
pub mod mock;
pub mod openai_compatible;
pub mod provider;

pub use error::AiError;
pub use factory::create_ai_provider;
pub use meta_api::{meta_ai_config_from_env, meta_backend_selected, MetaApiProvider};
pub use mock::MockAiProvider;
pub use openai_compatible::OpenAiCompatibleProvider;
pub use provider::{
    AiHealthReport, AiProvider, ChatMessage, ChatRequest, ChatResponse, EmbeddingRequest,
    EmbeddingResponse,
};
