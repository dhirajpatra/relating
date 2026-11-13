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
