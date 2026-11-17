use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(pub Uuid);

impl ProjectId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for ProjectId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderType {
    Openai,
    Ollama,
    Anthropic,
    Mock,
}

impl Default for AiProviderType {
    fn default() -> Self {
        Self::Mock
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderConfig {
    pub provider_type: AiProviderType,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_chat_model")]
    pub chat_model: String,
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    #[serde(default)]
    pub embedding_dimensions: Option<usize>,
}

fn default_chat_model() -> String {
    "gpt-4o-mini".to_string()
}

fn default_embedding_model() -> String {
    "text-embedding-3-small".to_string()
}

impl Default for AiProviderConfig {
    fn default() -> Self {
        Self {
            provider_type: AiProviderType::Mock,
            base_url: None,
            api_key: None,
            chat_model: "mock-chat-v1".to_string(),
            embedding_model: "mock-embed-v1".to_string(),
            embedding_dimensions: Some(1536),
        }
    }
}

impl AiProviderConfig {
    pub fn mock() -> Self {
        Self {
            provider_type: AiProviderType::Mock,
            base_url: None,
            api_key: None,
            chat_model: "mock-chat-v1".to_string(),
            embedding_model: "mock-embed-v1".to_string(),
            embedding_dimensions: Some(1536),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub ai_config: AiProviderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub ai_config: Option<AiProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: ProjectId,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub node_count: u64,
    pub relationship_count: u64,
    pub provider_type: AiProviderType,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub Uuid);

impl JobId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub project_id: ProjectId,
    pub job_type: String,
    pub status: JobStatus,
    pub progress: f32,
    pub message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(pub Uuid);

impl SourceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for SourceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Pending,
    Processed,
    Failed,
}

impl Default for SourceStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: SourceId,
    pub project_id: ProjectId,
    pub filename: String,
    pub content_hash: String,
    pub content_type: String,
    pub byte_size: u64,
    pub status: SourceStatus,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterSourceRequest {
    pub filename: String,
    pub content: String,
    #[serde(default)]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OntologyId(pub Uuid);

impl OntologyId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for OntologyId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for OntologyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OntologyStatus {
    Proposed,
    Approved,
    Rejected,
}

impl Default for OntologyStatus {
    fn default() -> Self {
        Self::Proposed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyDefinition {
    pub name: String,
    pub data_type: String,
    #[serde(default)]
    pub is_primary_key: bool,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityDefinition {
    pub label: String,
    pub display_name: String,
    #[serde(default)]
    pub properties: Vec<PropertyDefinition>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipDefinition {
    pub rel_type: String,
    pub source_label: String,
    pub target_label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OntologySchema {
    #[serde(default)]
    pub entities: Vec<EntityDefinition>,
    #[serde(default)]
    pub relationships: Vec<RelationshipDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ontology {
    pub id: OntologyId,
    pub project_id: ProjectId,
    pub version: u32,
    pub status: OntologyStatus,
    pub schema: OntologySchema,
    pub created_at: DateTime<Utc>,
    pub approved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedNode {
    pub id: String,
    pub labels: Vec<String>,
    pub properties: std::collections::HashMap<String, serde_json::Value>,
    pub source_id: SourceId,
    pub record_ref: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedRelationship {
    pub rel_type: String,
    pub source_node_id: String,
    pub target_node_id: String,
    pub properties: std::collections::HashMap<String, serde_json::Value>,
    pub source_id: SourceId,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EmbeddingProfileId(pub Uuid);

impl EmbeddingProfileId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for EmbeddingProfileId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EmbeddingProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingProfile {
    pub id: EmbeddingProfileId,
    pub project_id: ProjectId,
    pub name: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub model_version: Option<String>,
    pub dimensions: usize,
    #[serde(default = "default_modality")]
    pub modality: String,
    pub created_at: DateTime<Utc>,
}

fn default_modality() -> String {
    "prose".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChunkId(pub Uuid);

impl ChunkId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for ChunkId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ChunkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: ChunkId,
    pub project_id: ProjectId,
    pub profile_id: EmbeddingProfileId,
    pub source_id: SourceId,
    pub ordinal: u32,
    pub content: String,
    pub content_hash: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embedding: Vec<f32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalStrategy {
    Vector,
    Keyword,
    Hybrid,
}

impl Default for RetrievalStrategy {
    fn default() -> Self {
        Self::Hybrid
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedPassage {
    pub chunk_id: ChunkId,
    pub source_id: SourceId,
    pub filename: String,
    pub score: f32,
    pub rank: u32,
    pub content: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalTrace {
    pub query: String,
    pub strategy: RetrievalStrategy,
    pub profile_id: EmbeddingProfileId,
    pub profile_name: String,
    pub embedding_model: String,
    pub dimensions: usize,
    pub top_k: usize,
    pub passages: Vec<RetrievedPassage>,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerCitation {
    pub index: u32,
    pub chunk_id: ChunkId,
    pub source_id: SourceId,
    pub filename: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionAnswer {
    pub question: String,
    pub answer: String,
    pub grounded: bool,
    pub citations: Vec<AnswerCitation>,
    pub trace: RetrievalTrace,
    pub chat_model: String,
    pub prompt_version: String,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvaluationId(pub Uuid);

impl EvaluationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for EvaluationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EvaluationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionEvaluation {
    pub question: String,
    pub answer: String,
    pub grounded: bool,
    pub citation_count: usize,
    pub citation_precision: f32,
    pub retrieval_latency_ms: f64,
    pub answer_latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationMetrics {
    pub question_count: usize,
    pub grounded_count: usize,
    pub grounded_rate: f32,
    pub mean_citation_precision: f32,
    pub mean_latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationRun {
    pub id: EvaluationId,
    pub project_id: ProjectId,
    pub strategy: RetrievalStrategy,
    pub chat_model: String,
    pub embedding_model: String,
    pub prompt_version: String,
    pub metrics_version: String,
    pub metrics: EvaluationMetrics,
    pub questions: Vec<QuestionEvaluation>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuraConnectionId(pub Uuid);

impl AuraConnectionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for AuraConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AuraConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuraConnection {
    pub id: AuraConnectionId,
    pub project_id: ProjectId,
    pub name: String,
    pub uri: String,
    pub username: String,
    #[serde(default)]
    pub database: Option<String>,
    /// Name of the environment variable holding the password at runtime.
    /// Secrets are never stored in the database.
    pub auth_env_var: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MigrationId(pub Uuid);

impl MigrationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for MigrationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MigrationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStatus {
    Draft,
    Analyzed,
    Planned,
    Approved,
    Running,
    Paused,
    Completed,
    Failed,
    Verified,
}

impl Default for MigrationStatus {
    fn default() -> Self {
        Self::Draft
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MigrationProgress {
    #[serde(default)]
    pub batches_done: usize,
    #[serde(default)]
    pub batches_total: usize,
    #[serde(default)]
    pub nodes_migrated: u64,
    #[serde(default)]
    pub relationships_migrated: u64,
    #[serde(default)]
    pub completed_batches: Vec<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Migration {
    pub id: MigrationId,
    pub project_id: ProjectId,
    pub connection_id: AuraConnectionId,
    pub status: MigrationStatus,
    #[serde(default)]
    pub plan: serde_json::Value,
    #[serde(default)]
    pub progress: MigrationProgress,
    #[serde(default)]
    pub report: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
