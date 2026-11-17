export type AiProviderType = 'mock' | 'openai' | 'ollama' | 'anthropic';

export interface AiProviderConfig {
  provider_type: AiProviderType;
  base_url?: string;
  api_key?: string;
  chat_model: string;
  embedding_model: string;
  embedding_dimensions?: number;
}

export interface Project {
  id: string;
  name: string;
  description: string;
  created_at: string;
  updated_at: string;
  ai_config: AiProviderConfig;
}

export interface ProjectSummary {
  id: string;
  name: string;
  description: string;
  created_at: string;
  updated_at: string;
  node_count: number;
  relationship_count: number;
  provider_type: AiProviderType;
}

export interface GraphSchemaInfo {
  labels: string[];
  relationship_types: string[];
  node_count: number;
  relationship_count: number;
  property_keys: string[];
}

export interface ProjectDetail {
  project: Project;
  schema: GraphSchemaInfo;
}

export interface CreateProjectRequest {
  name: string;
  description?: string;
  ai_config?: AiProviderConfig;
}

export interface GraphQueryResult {
  columns: string[];
  rows: Array<Record<string, unknown>>;
  elapsed_ms: number;
  row_count: number;
}

export interface AiHealthReport {
  healthy: boolean;
  provider: string;
  chat_model: string;
  embedding_model: string;
  latency_ms: number;
  message: string;
}

export interface SystemHealth {
  status: string;
  version: string;
  product: string;
  timestamp: string;
}

// Slice 2: Sources, Ontology, Ingestion, Cytoscape
export interface Source {
  id: string;
  project_id: string;
  filename: string;
  content_hash: string;
  content_type: string;
  byte_size: number;
  status: 'pending' | 'processed' | 'failed';
  metadata: {
    total_records?: number;
    field_names?: string[];
    sample_records?: Array<Record<string, unknown>>;
    stored_path?: string;
  };
  created_at: string;
}

export interface PropertyDefinition {
  name: string;
  data_type: string;
  is_primary_key?: boolean;
  description?: string;
}

export interface EntityDefinition {
  label: string;
  display_name: string;
  properties: PropertyDefinition[];
  description: string;
}

export interface RelationshipDefinition {
  rel_type: string;
  source_label: string;
  target_label: string;
  description: string;
}

export interface OntologySchema {
  entities: EntityDefinition[];
  relationships: RelationshipDefinition[];
}

export interface Ontology {
  id: string;
  project_id: string;
  version: number;
  status: 'proposed' | 'approved' | 'rejected';
  schema: OntologySchema;
  created_at: string;
  approved_at?: string;
}

export interface OntologyStatusResponse {
  latest?: Ontology;
  approved?: Ontology;
}

export interface IngestionResultResponse {
  sources_processed: number;
  nodes_created: number;
  relationships_created: number;
  properties_set: number;
  elapsed_ms: number;
}

export interface CytoscapeNodeData {
  id: string;
  label: string;
  name: string;
  properties: Record<string, unknown>;
}

export interface CytoscapeNode {
  data: CytoscapeNodeData;
}

export interface CytoscapeEdgeData {
  id: string;
  source: string;
  target: string;
  label: string;
  properties: Record<string, unknown>;
}

export interface CytoscapeEdge {
  data: CytoscapeEdgeData;
}

export interface CytoscapeGraph {
  elements: {
    nodes: CytoscapeNode[];
    edges: CytoscapeEdge[];
  };
}

// Slice 3: Embeddings, Retrieval, Q&A, Evaluation
export type RetrievalStrategy = 'vector' | 'keyword' | 'hybrid';

export interface EmbeddingProfile {
  id: string;
  project_id: string;
  name: string;
  provider: string;
  model: string;
  model_version?: string;
  dimensions: number;
  modality: string;
  created_at: string;
}

export interface EmbeddingProfileStatus {
  profile: EmbeddingProfile;
  chunk_count: number;
}

export interface EmbeddingStatusResponse {
  profiles: EmbeddingProfileStatus[];
}

export interface GenerateEmbeddingsResponse {
  profile: EmbeddingProfile;
  sources_processed: number;
  chunks_created: number;
  chunks_skipped: number;
  dimensions: number;
  elapsed_ms: number;
}

export interface RetrievedPassage {
  chunk_id: string;
  source_id: string;
  filename: string;
  score: number;
  rank: number;
  content: string;
  metadata: Record<string, unknown>;
}

export interface RetrievalTrace {
  query: string;
  strategy: RetrievalStrategy;
  profile_id: string;
  profile_name: string;
  embedding_model: string;
  dimensions: number;
  top_k: number;
  passages: RetrievedPassage[];
  latency_ms: number;
}

export interface AnswerCitation {
  index: number;
  chunk_id: string;
  source_id: string;
  filename: string;
  excerpt: string;
}

export interface QuestionAnswer {
  question: string;
  answer: string;
  grounded: boolean;
  citations: AnswerCitation[];
  trace: RetrievalTrace;
  chat_model: string;
  prompt_version: string;
  latency_ms: number;
}

export interface QuestionEvaluation {
  question: string;
  answer: string;
  grounded: boolean;
  citation_count: number;
  citation_precision: number;
  retrieval_latency_ms: number;
  answer_latency_ms: number;
}

export interface EvaluationMetrics {
  question_count: number;
  grounded_count: number;
  grounded_rate: number;
  mean_citation_precision: number;
  mean_latency_ms: number;
}

export interface EvaluationRun {
  id: string;
  project_id: string;
  strategy: RetrievalStrategy;
  chat_model: string;
  embedding_model: string;
  prompt_version: string;
  metrics: EvaluationMetrics;
  questions: QuestionEvaluation[];
  created_at: string;
}
