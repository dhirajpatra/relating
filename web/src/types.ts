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
  metrics_version: string;
  metrics: EvaluationMetrics;
  questions: QuestionEvaluation[];
  created_at: string;
}

// R1: Aura connections, migrations, MCP
export interface AuraConnection {
  id: string;
  project_id: string;
  name: string;
  uri: string;
  username: string;
  database?: string;
  auth_env_var: string;
  created_at: string;
}

export interface ConnectionTestReport {
  reachable: boolean;
  server_agent?: string;
  bolt_version?: string;
  database?: string;
  message: string;
}

export type CapabilityVerdict =
  | 'SUPPORTED'
  | 'RECREATE'
  | 'TRANSFORM'
  | 'UNSUPPORTED'
  | 'REQUIRES_MANUAL_REVIEW';

export interface CompatFinding {
  category: string;
  subject: string;
  verdict: CapabilityVerdict;
  detail: string;
  remediation?: string;
}

export interface AnalyzeResponse {
  source_findings: CompatFinding[];
  target_findings: CompatFinding[];
  target_reachable: boolean;
  target_agent?: string;
}

export type BatchKind = 'nodes' | 'relationships';

export interface BatchPlan {
  kind: BatchKind;
  name: string;
  count: number;
  batch_size: number;
  batch_count: number;
}

export interface MigrationPlan {
  node_batches: BatchPlan[];
  rel_batches: BatchPlan[];
  ddl_statements: string[];
  warnings: string[];
  total_nodes: number;
  total_relationships: number;
  total_batches: number;
}

export interface DryRunReport {
  plan: MigrationPlan;
  target_reachable: boolean;
  target_agent?: string;
  remote_writes: number;
  blockers: string[];
}

export type MigrationStatus =
  | 'draft'
  | 'analyzed'
  | 'planned'
  | 'approved'
  | 'running'
  | 'paused'
  | 'completed'
  | 'failed'
  | 'verified';

export interface MigrationProgress {
  batches_done: number;
  batches_total: number;
  nodes_migrated: number;
  relationships_migrated: number;
  completed_batches: string[];
  message?: string;
}

export interface Migration {
  id: string;
  project_id: string;
  connection_id: string;
  status: MigrationStatus;
  plan: MigrationPlan;
  progress: MigrationProgress;
  report: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface PlanResponse {
  migration: Migration;
  dry_run: DryRunReport;
}

export interface CountCheck {
  kind: string;
  name: string;
  source_count: number;
  target_count: number;
  matched: boolean;
}

export interface VerificationReport {
  checks: CountCheck[];
  samples_checked: number;
  samples_matched: number;
  passed: boolean;
  details: string[];
}

export interface McpHealth {
  status: string;
  server: string;
  version: string;
  protocol_version: string;
  tools: number;
  mode: string;
}

// R2: schema-as-code
export interface UniqueConstraint {
  label: string;
  properties: string[];
}

export interface IndexDef {
  label: string;
  properties: string[];
}

export interface ProfileSpec {
  name: string;
  model: string;
  dimensions: number;
  modality: string;
}

export interface RetrievalSpec {
  default_strategy: RetrievalStrategy;
  default_top_k: number;
}

export interface SchemaDocument {
  format_version: string;
  project_name?: string;
  ontology: OntologySchema;
  constraints: UniqueConstraint[];
  indexes: IndexDef[];
  embedding_profiles: ProfileSpec[];
  retrieval: RetrievalSpec;
}

export interface SchemaFile {
  path: string;
  content: string;
}

export interface ExportResponse {
  files: SchemaFile[];
  document: SchemaDocument;
}

export interface SchemaRevision {
  id: string;
  project_id: string;
  version: number;
  content_hash: string;
  document: SchemaDocument;
  created_at: string;
}

export interface CreateRevisionResponse {
  revision: SchemaRevision;
  created: boolean;
}

export type ChangeKind = 'added' | 'removed' | 'modified';

export interface SchemaChange {
  category: string;
  subject: string;
  kind: ChangeKind;
  breaking: boolean;
  detail: string;
}

export interface SchemaDiff {
  changes: SchemaChange[];
  breaking_count: number;
}

export interface MigrationProposal {
  filename: string;
  cypher: string[];
  steps: string[];
  notes: string[];
}

export interface DiffResponse {
  diff: SchemaDiff;
  proposal: MigrationProposal;
}

export interface ContractCheck {
  subject: string;
  passed: boolean;
  detail: string;
}

export interface ConformanceReport {
  checks: ContractCheck[];
  passed: boolean;
}

export interface ApplyResponse {
  revision: SchemaRevision;
  diff?: SchemaDiff;
  conformance: ConformanceReport;
}

export interface RetrievalConfigResponse {
  configured: boolean;
  default_strategy: RetrievalStrategy;
  default_top_k: number;
}

// R3: portability compiler
export type PortVerdict = 'PORTABLE' | 'TRANSFORM' | 'NEEDS_REWRITE' | 'NEEDS_REVIEW';

export type SourceLanguage =
  | 'python'
  | 'typescript'
  | 'javascript'
  | 'java'
  | 'go'
  | 'rust'
  | 'cypher'
  | 'unknown';

export interface QueryCandidate {
  file: string;
  line: number;
  language: SourceLanguage;
  query: string;
}

export type SignalKind =
  | 'driver_import'
  | 'connection_uri'
  | 'transaction'
  | 'apoc_usage'
  | 'constraint_ddl'
  | 'index_ddl';

export interface CodeSignal {
  file: string;
  line: number;
  kind: SignalKind;
  text: string;
}

export interface PortFinding {
  file: string;
  line: number;
  rule_id: string;
  verdict: PortVerdict;
  message: string;
  remediation?: string;
}

export interface PortSummary {
  files_scanned: number;
  queries_total: number;
  portable: number;
  transform: number;
  needs_rewrite: number;
  needs_review: number;
  signals_total: number;
}

export type TestAssertion =
  | { type: 'no_error' }
  | { type: 'row_count_eq'; expected: number }
  | { type: 'row_count_gte'; expected: number };

export interface GeneratedTest {
  name: string;
  file: string;
  line: number;
  cypher: string;
  read_only: boolean;
  assertion: TestAssertion;
}

export interface PortableReport {
  summary: PortSummary;
  findings: PortFinding[];
  breaking: PortFinding[];
  tests: GeneratedTest[];
  tests_truncated: boolean;
}

export interface TestOutcome {
  name: string;
  passed: boolean;
  detail: string;
  row_count: number;
  elapsed_ms: number;
}

export interface RunTestsResponse {
  results: TestOutcome[];
  passed: number;
  failed: number;
}

// R5: shadow testing + debugger
export interface ShadowQuerySpec {
  name: string;
  cypher: string;
  params?: Record<string, unknown>;
}

export type CompareVerdict = 'MATCH' | 'STRUCTURE_DIFF' | 'COUNT_DIFF' | 'ERROR_DIFF';
export type Confidence = 'HIGH' | 'MEDIUM' | 'LOW';

export interface QueryComparison {
  name: string;
  verdict: CompareVerdict;
  detail: string;
  source_rows: number;
  target_rows: number;
  source_latency_ms: number;
  target_latency_ms: number;
  latency_ratio: number;
}

export interface ShadowSummary {
  total: number;
  matched: number;
  structure_diffs: number;
  count_diffs: number;
  error_diffs: number;
  source_success_rate: number;
  target_success_rate: number;
  confidence: Confidence;
  confidence_reasons: string[];
}

export interface ShadowQueryEcho {
  name: string;
  cypher: string;
  params: Record<string, unknown>;
}

export interface ShadowRunResponse {
  report: {
    comparisons: QueryComparison[];
    summary: ShadowSummary;
  };
  queries: ShadowQueryEcho[];
}

export interface RankingRow {
  rank: number;
  chunk_id: string;
  filename: string;
  fused_score: number;
  vector_rank?: number;
  keyword_rank?: number;
  content_snippet: string;
}

export interface LinkedNode {
  uid: string;
  labels: string[];
  name?: string;
}

export interface GraphLink {
  chunk_id: string;
  filename: string;
  lineage_ref?: string;
  nodes: LinkedNode[];
  relationship_types: string[];
  note?: string;
}

export interface OntologyMapping {
  label: string;
  mapped_entity?: string;
  ontology_relationships: string[];
}

export interface DebugTrace {
  question: string;
  strategy: RetrievalStrategy;
  profile_name: string;
  embedding_model: string;
  dimensions: number;
  passages: RetrievedPassage[];
  ranking: RankingRow[];
  graph_links: GraphLink[];
  ontology_mapping: OntologyMapping[];
  ontology_approved: boolean;
  evidence_chars: number;
  evidence_preview: string;
  prompt_version: string;
  answer: string;
  grounded: boolean;
  citations: AnswerCitation[];
  invalid_markers: number;
  chat_model: string;
  latency_ms: number;
}

export type Severity = 'info' | 'warning';

export interface DiagnosisRule {
  code: string;
  severity: Severity;
  message: string;
}

export interface AiHypothesis {
  text: string;
  model: string;
}

export interface Diagnosis {
  rules: DiagnosisRule[];
  likely_causes: string[];
  ai_hypothesis?: AiHypothesis;
}

export interface DebugTraceResponse {
  trace: DebugTrace;
  diagnosis: Diagnosis;
}

// R6: synthetic data + advisor
export type EdgeCase = 'duplicates' | 'orphans' | 'cycle' | 'invalid_values';

export interface LabelProfile {
  label: string;
  count: number;
  properties: { name: string; data_type: string; coverage: number }[];
}

export interface RelProfile {
  rel_type: string;
  count: number;
  endpoints: { source_label: string; target_label: string }[];
}

export interface GraphProfile {
  labels: LabelProfile[];
  rel_types: RelProfile[];
  total_nodes: number;
  total_relationships: number;
}

export interface SynthStats {
  node_count: number;
  rel_count: number;
  per_label: Record<string, number>;
  per_type: Record<string, number>;
  edge_cases_applied: string[];
}

export interface MaterializeOutcome {
  nodes_created: number;
  relationships_created: number;
  properties_set: number;
}

export interface GenerateResponse {
  profile_summary: {
    labels: string[];
    rel_types: string[];
    total_nodes: number;
    total_relationships: number;
  };
  stats: SynthStats;
  notes: string[];
  sample_nodes: unknown[];
  sample_rels: unknown[];
  target_project_id?: string;
  materialized?: MaterializeOutcome;
}

export type AdvisorSeverity = 'info' | 'warning';

export interface AdvisorQueryFinding {
  rule_id: string;
  severity: AdvisorSeverity;
  message: string;
  suggestion: string;
}

export interface AdvisorQueryReport {
  name: string;
  findings: AdvisorQueryFinding[];
}

export interface StructureNote {
  rule_id: string;
  severity: AdvisorSeverity;
  message: string;
  suggestion: string;
  basis: string;
}

export interface SizingEstimate {
  tier_hint: string;
  estimated_elements: number;
  estimated_storage_mb: number;
  assumptions: string[];
  caveats: string[];
}

export interface PlanMeasurement {
  query_name: string;
  planned: boolean;
  planning_ms: number;
  error?: string;
}

export interface TargetMeasurements {
  target_nodes: number;
  target_relationships: number;
  plans: PlanMeasurement[];
}

export interface AdvisorResponse {
  queries: AdvisorQueryReport[];
  structure: StructureNote[];
  sizing: SizingEstimate;
  measurements?: TargetMeasurements;
}
