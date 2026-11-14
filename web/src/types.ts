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
