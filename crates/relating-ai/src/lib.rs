pub mod error;
pub mod factory;
pub mod mock;
pub mod openai_compatible;
pub mod provider;

pub use error::AiError;
pub use factory::create_ai_provider;
pub use mock::MockAiProvider;
pub use openai_compatible::OpenAiCompatibleProvider;
pub use provider::{
    AiHealthReport, AiProvider, ChatMessage, ChatRequest, ChatResponse, EmbeddingRequest,
    EmbeddingResponse,
};
