import {
  AdvisorResponse,
  AiHealthReport,
  AnalyzeResponse,
  ApplyResponse,
  AuditExport,
  AuraConnection,
  AuthMe,
  CallbackResponse,
  ChangesResponse,
  ConnectionTestReport,
  ConnectorSyncReport,
  ConnectorTestReport,
  CreateProjectRequest,
  CreateRevisionResponse,
  CytoscapeGraph,
  DataConnector,
  DebugTraceResponse,
  DiffResponse,
  DriftReport,
  DryRunReport,
  EdgeCase,
  EmbeddingStatusResponse,
  EvaluationRun,
  ExportResponse,
  GenerateEmbeddingsResponse,
  GenerateResponse,
  GeneratedTest,
  GrantView,
  GraphProfile,
  GraphQueryResult,
  IngestionResultResponse,
  LoginResponse,
  McpHealth,
  Migration,
  Ontology,
  OntologySchema,
  OntologyStatusResponse,
  PlanResponse,
  PortableReport,
  Project,
  ProjectDetail,
  ProjectSummary,
  QueryTargetsResponse,
  QuestionAnswer,
  RetrievalConfigResponse,
  RetrievalStrategy,
  RetrievalTrace,
  RollbackOutcome,
  RunTestsResponse,
  SchemaRevision,
  ShadowQuerySpec,
  ShadowRunResponse,
  Source,
  SystemHealth,
  VectorSizingInput,
  VerificationReport,
} from './types.js';
import { errorFromResponse } from './errors.js';

export const LOCAL_TARGET = 'relate://local';
export const TARGET_ENV_VAR = 'RELATING_TARGET';

/** Map a target string to the `connection_id` argument (backend flip). */
export function parseTarget(target: string): string | undefined {
  const text = (target || '').trim();
  if (text === LOCAL_TARGET) return undefined;
  if (text.startsWith('aura://')) {
    const id = text.slice('aura://'.length);
    if (id) return id;
  }
  throw new Error(`Invalid target '${target}': use '${LOCAL_TARGET}' or 'aura://<connection-id>'`);
}

/** Node environment without requiring `@types/node` (browser-safe). */
function nodeEnv(): Record<string, string> | undefined {
  const proc = (globalThis as Record<string, unknown>)['process'] as
    | { env?: Record<string, string> }
    | undefined;
  return proc?.env;
}

/** Effective target: `RELATING_TARGET` in Node, the local store in browsers. */
export function defaultTarget(): string {
  return nodeEnv()?.[TARGET_ENV_VAR] || LOCAL_TARGET;
}

export interface RelatingClientOptions {
  baseUrl?: string;
  token?: string;
  fetchImpl?: typeof fetch;
}

function queryString(params: Record<string, string | number | boolean | undefined>): string {
  const cleaned: Record<string, string> = {};
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) cleaned[key] = String(value);
  }
  const query = new URLSearchParams(cleaned).toString();
  return query ? `?${query}` : '';
}

/** Ergonomic client over `/api/v1`. Works in browsers and Node (fetch). */
export class RelatingClient {
  private readonly baseUrl: string;
  private readonly token?: string;
  private readonly fetchImpl: typeof fetch;

  constructor(options: RelatingClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? '/api/v1').replace(/\/$/, '');
    this.token = options.token ?? nodeEnv()?.RELATING_API_TOKEN;
    this.fetchImpl = options.fetchImpl ?? fetch;
  }

  private async request<T>(path: string, options?: RequestInit): Promise<T> {
    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (this.token) headers['Authorization'] = `Bearer ${this.token}`;
    const res = await this.fetchImpl(`${this.baseUrl}${path}`, {
      ...options,
      headers: { ...headers, ...options?.headers },
    });
    if (res.status === 204) return undefined as T;
    if (!res.ok) {
      let payload: unknown = null;
      try {
        payload = await res.json();
      } catch {
        // ignore
      }
      throw errorFromResponse(res.status, payload);
    }
    return (await res.json()) as T;
  }

  private async deleteEmpty(path: string): Promise<void> {
    await this.request<unknown>(path, { method: 'DELETE' });
  }

  // -- health --
  getHealth = () => this.request<SystemHealth>('/health');
  mcpHealth = async (): Promise<McpHealth> => {
    const base = this.baseUrl.replace(/\/api\/v1$/, '');
    const headers: Record<string, string> = {};
    if (this.token) headers['Authorization'] = `Bearer ${this.token}`;
    const res = await this.fetchImpl(`${base}/mcp/health`, { headers });
    if (!res.ok) throw errorFromResponse(res.status, await res.json().catch(() => null));
    return (await res.json()) as McpHealth;
  };

  // -- projects --
  listProjects = () => this.request<ProjectSummary[]>('/projects');
  getProject = (id: string) => this.request<ProjectDetail>(`/projects/${id}`);
  createProject = (payload: CreateProjectRequest) =>
    this.request<Project>('/projects', { method: 'POST', body: JSON.stringify(payload) });
  deleteProject = (id: string) => this.deleteEmpty(`/projects/${id}`);
  testProjectAi = (id: string) =>
    this.request<AiHealthReport>(`/projects/${id}/ai/test`, { method: 'POST' });

  // -- cypher + backend flip --
  runCypherRead = (
    id: string,
    query: string,
    params?: Record<string, unknown>,
    limit?: number,
    target?: string
  ) =>
    this.request<GraphQueryResult>(`/projects/${id}/cypher/read`, {
      method: 'POST',
      body: JSON.stringify({
        query,
        params: params || {},
        limit,
        connection_id: parseTarget(target ?? defaultTarget()),
      }),
    });

  runCypherWrite = (
    id: string,
    query: string,
    params?: Record<string, unknown>,
    target?: string,
    confirmRemoteWrite?: boolean
  ) =>
    this.request<{ nodes_created: number; relationships_created: number; elapsed_ms: number }>(
      `/projects/${id}/cypher/write`,
      {
        method: 'POST',
        body: JSON.stringify({
          query,
          params: params || {},
          connection_id: parseTarget(target ?? defaultTarget()),
          confirm_remote_write: confirmRemoteWrite || undefined,
        }),
      }
    );

  listQueryTargets = (projectId: string) =>
    this.request<QueryTargetsResponse>(`/projects/${projectId}/query-targets`);

  // -- sources --
  listSources = (projectId: string) => this.request<Source[]>(`/projects/${projectId}/sources`);
  registerSource = (projectId: string, filename: string, content: string, contentType?: string) =>
    this.request<Source>(`/projects/${projectId}/sources`, {
      method: 'POST',
      body: JSON.stringify({ filename, content, content_type: contentType }),
    });
  registerBinarySource = (
    projectId: string,
    filename: string,
    contentBase64: string,
    contentType?: string
  ) =>
    this.request<Source>(`/projects/${projectId}/sources`, {
      method: 'POST',
      body: JSON.stringify({
        filename,
        content: '',
        content_base64: contentBase64,
        content_type: contentType,
      }),
    });
  deleteSource = (projectId: string, sourceId: string) =>
    this.deleteEmpty(`/projects/${projectId}/sources/${sourceId}`);

  // -- ontology + ingest --
  getOntology = (projectId: string) =>
    this.request<OntologyStatusResponse>(`/projects/${projectId}/ontology`);
  proposeOntology = (projectId: string, domainHint?: string) =>
    this.request<Ontology>(`/projects/${projectId}/ontology/propose`, {
      method: 'POST',
      body: JSON.stringify({ domain_hint: domainHint }),
    });
  importOntology = (projectId: string, schema: OntologySchema) =>
    this.request<Ontology>(`/projects/${projectId}/ontology/import`, {
      method: 'POST',
      body: JSON.stringify({ schema }),
    });
  approveOntology = (projectId: string, ontologyId: string) =>
    this.request<{ success: boolean; message: string }>(`/projects/${projectId}/ontology/approve`, {
      method: 'POST',
      body: JSON.stringify({ ontology_id: ontologyId }),
    });
  runIngestion = (projectId: string, full?: boolean) =>
    this.request<IngestionResultResponse>(
      `/projects/${projectId}/ingest${full ? '?full=true' : ''}`,
      { method: 'POST' }
    );
  getGraphElements = (projectId: string, limit?: number) =>
    this.request<CytoscapeGraph>(
      `/projects/${projectId}/graph/elements${limit ? `?limit=${limit}` : ''}`
    );

  // -- GraphRAG --
  getEmbeddingsStatus = (projectId: string) =>
    this.request<EmbeddingStatusResponse>(`/projects/${projectId}/embeddings`);
  generateEmbeddings = (projectId: string) =>
    this.request<GenerateEmbeddingsResponse>(`/projects/${projectId}/embeddings`, {
      method: 'POST',
      body: JSON.stringify({}),
    });
  runRetrieval = (projectId: string, query: string, strategy?: RetrievalStrategy, topK?: number) =>
    this.request<RetrievalTrace>(`/projects/${projectId}/retrieval`, {
      method: 'POST',
      body: JSON.stringify({ query, strategy, top_k: topK }),
    });
  askQuestion = (projectId: string, question: string, strategy?: RetrievalStrategy) =>
    this.request<QuestionAnswer>(`/projects/${projectId}/questions`, {
      method: 'POST',
      body: JSON.stringify({ question, strategy }),
    });
  createEvaluation = (projectId: string, questions: string[], strategy?: RetrievalStrategy) =>
    this.request<EvaluationRun>(`/projects/${projectId}/evaluations`, {
      method: 'POST',
      body: JSON.stringify({ questions, strategy }),
    });
  listEvaluations = (projectId: string) =>
    this.request<EvaluationRun[]>(`/projects/${projectId}/evaluations`);
  getEvaluation = (projectId: string, evaluationId: string) =>
    this.request<EvaluationRun>(`/projects/${projectId}/evaluations/${evaluationId}`);

  // -- Aura + migration --
  listAuraConnections = (projectId: string) =>
    this.request<AuraConnection[]>(`/projects/${projectId}/connections/aura`);
  createAuraConnection = (
    projectId: string,
    payload: {
      name: string;
      uri: string;
      username?: string;
      database?: string;
      auth_env_var?: string;
    }
  ) =>
    this.request<AuraConnection>(`/projects/${projectId}/connections/aura`, {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  deleteAuraConnection = (projectId: string, connectionId: string) =>
    this.deleteEmpty(`/projects/${projectId}/connections/aura/${connectionId}`);
  testAuraConnection = (projectId: string, connectionId: string) =>
    this.request<ConnectionTestReport>(
      `/projects/${projectId}/connections/aura/${connectionId}/test`,
      { method: 'POST' }
    );
  analyzeMigration = (projectId: string, connectionId?: string, auraTier?: string) =>
    this.request<AnalyzeResponse>(`/projects/${projectId}/migrations/analyze`, {
      method: 'POST',
      body: JSON.stringify({ connection_id: connectionId, aura_tier: auraTier || undefined }),
    });
  planMigration = (
    projectId: string,
    connectionId: string,
    batchSize?: number,
    auraTier?: string
  ) =>
    this.request<PlanResponse>(`/projects/${projectId}/migrations/plan`, {
      method: 'POST',
      body: JSON.stringify({
        connection_id: connectionId,
        batch_size: batchSize,
        aura_tier: auraTier || undefined,
      }),
    });
  listMigrations = (projectId: string) =>
    this.request<Migration[]>(`/projects/${projectId}/migrations`);
  getMigration = (projectId: string, migrationId: string) =>
    this.request<Migration>(`/projects/${projectId}/migrations/${migrationId}`);
  dryRunMigration = (projectId: string, migrationId: string) =>
    this.request<DryRunReport>(`/projects/${projectId}/migrations/${migrationId}/dry-run`, {
      method: 'POST',
    });
  approveMigration = (projectId: string, migrationId: string, approved: boolean) =>
    this.request<Migration>(`/projects/${projectId}/migrations/${migrationId}/approve`, {
      method: 'POST',
      body: JSON.stringify({ approved }),
    });
  executeMigration = (projectId: string, migrationId: string) =>
    this.request<Migration>(`/projects/${projectId}/migrations/${migrationId}/execute`, {
      method: 'POST',
    });
  verifyMigration = (projectId: string, migrationId: string) =>
    this.request<VerificationReport>(`/projects/${projectId}/migrations/${migrationId}/verify`, {
      method: 'POST',
    });
  rollbackMigration = (projectId: string, migrationId: string) =>
    this.request<RollbackOutcome>(`/projects/${projectId}/migrations/${migrationId}/rollback`, {
      method: 'POST',
    });
  getMigrationReport = (projectId: string, migrationId: string) =>
    this.request<Record<string, unknown>>(
      `/projects/${projectId}/migrations/${migrationId}/report`
    );
  syncMigration = (projectId: string, migrationId: string, propagateDeletes?: boolean) =>
    this.request<Migration>(
      `/projects/${projectId}/migrations/${migrationId}/sync${
        propagateDeletes ? '?propagate_deletes=true' : ''
      }`,
      { method: 'POST' }
    );
  driftMigration = (projectId: string, migrationId: string) =>
    this.request<DriftReport>(`/projects/${projectId}/migrations/${migrationId}/drift`, {
      method: 'POST',
    });
  changes = (projectId: string, since?: string, limit?: number) =>
    this.request<ChangesResponse>(
      `/projects/${projectId}/changes${queryString({ since, limit })}`
    );

  // -- schema --
  exportSchema = (projectId: string) =>
    this.request<ExportResponse>(`/projects/${projectId}/schema/export`);
  createSchemaRevision = (projectId: string, document?: unknown) =>
    this.request<CreateRevisionResponse>(`/projects/${projectId}/schema/revisions`, {
      method: 'POST',
      body: JSON.stringify({ document }),
    });
  listSchemaRevisions = (projectId: string) =>
    this.request<SchemaRevision[]>(`/projects/${projectId}/schema/revisions`);
  getSchemaRevision = (projectId: string, version: number) =>
    this.request<SchemaRevision>(`/projects/${projectId}/schema/revisions/${version}`);
  diffSchema = (
    projectId: string,
    payload: { from_version?: number; to_version?: number; to_document?: unknown }
  ) =>
    this.request<DiffResponse>(`/projects/${projectId}/schema/diff`, {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  applySchema = (projectId: string, document: unknown) =>
    this.request<ApplyResponse>(`/projects/${projectId}/schema/apply`, {
      method: 'POST',
      body: JSON.stringify({ document }),
    });
  getRetrievalConfig = (projectId: string) =>
    this.request<RetrievalConfigResponse>(`/projects/${projectId}/retrieval-config`);
  updateRetrievalConfig = (
    projectId: string,
    payload: { default_strategy?: RetrievalStrategy; default_top_k?: number }
  ) =>
    this.request<RetrievalConfigResponse>(`/projects/${projectId}/retrieval-config`, {
      method: 'PUT',
      body: JSON.stringify(payload),
    });

  // -- portability --
  scanRepo = (projectId: string, repoPath: string) =>
    this.request<PortableReport>(`/projects/${projectId}/portability/scan`, {
      method: 'POST',
      body: JSON.stringify({ repo_path: repoPath }),
    });
  runPortabilityTests = (projectId: string, connectionId: string, tests: GeneratedTest[]) =>
    this.request<RunTestsResponse>(`/projects/${projectId}/portability/run-tests`, {
      method: 'POST',
      body: JSON.stringify({ connection_id: connectionId, tests }),
    });

  // -- shadow + debug --
  runShadow = (
    projectId: string,
    connectionId: string,
    queries: ShadowQuerySpec[],
    redactParams?: boolean
  ) =>
    this.request<ShadowRunResponse>(`/projects/${projectId}/shadow/runs`, {
      method: 'POST',
      body: JSON.stringify({ connection_id: connectionId, queries, redact_params: redactParams }),
    });
  debugTrace = (projectId: string, question: string, strategy?: RetrievalStrategy) =>
    this.request<DebugTraceResponse>(`/projects/${projectId}/debug/trace`, {
      method: 'POST',
      body: JSON.stringify({ question, strategy }),
    });

  // -- synth + advisor --
  synthProfile = (projectId: string) =>
    this.request<GraphProfile>(`/projects/${projectId}/synth/profile`, { method: 'POST' });
  synthGenerate = (
    projectId: string,
    payload: {
      scale?: number;
      seed?: number;
      edge_cases?: EdgeCase[];
      materialize?: boolean;
      target_name?: string;
    }
  ) =>
    this.request<GenerateResponse>(`/projects/${projectId}/synth/generate`, {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  advisorAnalyze = (
    projectId: string,
    queries: { name: string; cypher: string }[],
    connectionId?: string,
    measure?: boolean,
    vectors?: VectorSizingInput[],
    auraTier?: string
  ) =>
    this.request<AdvisorResponse>(`/projects/${projectId}/advisor/analyze`, {
      method: 'POST',
      body: JSON.stringify({
        queries,
        connection_id: connectionId,
        measure,
        vectors: vectors && vectors.length > 0 ? vectors : undefined,
        aura_tier: auraTier || undefined,
      }),
    });

  // -- data connectors --
  listConnectors = (projectId: string) =>
    this.request<DataConnector[]>(`/projects/${projectId}/connectors`);
  createConnector = (
    projectId: string,
    payload: {
      name: string;
      kind: string;
      config: Record<string, unknown>;
      mapping?: Record<string, unknown>;
      poll_interval_secs?: number;
    }
  ) =>
    this.request<DataConnector>(`/projects/${projectId}/connectors`, {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  getConnector = (projectId: string, connectorId: string) =>
    this.request<DataConnector>(`/projects/${projectId}/connectors/${connectorId}`);
  deleteConnector = (projectId: string, connectorId: string) =>
    this.deleteEmpty(`/projects/${projectId}/connectors/${connectorId}`);
  testConnector = (projectId: string, connectorId: string) =>
    this.request<ConnectorTestReport>(
      `/projects/${projectId}/connectors/${connectorId}/test`,
      { method: 'POST' }
    );
  syncConnector = (projectId: string, connectorId: string, full?: boolean) =>
    this.request<ConnectorSyncReport>(
      `/projects/${projectId}/connectors/${connectorId}/sync${full ? '?full=true' : ''}`,
      { method: 'POST' }
    );
  dueConnectors = (projectId: string) =>
    this.request<DataConnector[]>(`/projects/${projectId}/connectors/due`);

  // -- auth + audit --
  authLogin = (redirectUri: string) =>
    this.request<LoginResponse>('/auth/login', {
      method: 'POST',
      body: JSON.stringify({ redirect_uri: redirectUri }),
    });
  authCallback = (state: string, idToken: string) =>
    this.request<CallbackResponse>('/auth/callback', {
      method: 'POST',
      body: JSON.stringify({ state, id_token: idToken }),
    });
  authMe = () => this.request<AuthMe>('/auth/me');
  listGrants = (projectId: string) =>
    this.request<GrantView[]>(`/projects/${projectId}/grants`);
  createGrant = (projectId: string, email: string, role: string) =>
    this.request<GrantView>(`/projects/${projectId}/grants`, {
      method: 'POST',
      body: JSON.stringify({ email, role }),
    });
  deleteGrant = (projectId: string, grantId: string) =>
    this.deleteEmpty(`/projects/${projectId}/grants/${grantId}`);
  listGlobalGrants = () => this.request<GrantView[]>('/grants');
  createGlobalGrant = (email: string, role: string) =>
    this.request<GrantView>('/grants', {
      method: 'POST',
      body: JSON.stringify({ email, role }),
    });
  auditExport = (
    projectId: string,
    options?: { format?: string; after_seq?: number; actor?: string; limit?: number }
  ) =>
    this.request<AuditExport>(
      `/projects/${projectId}/audit/export${queryString(options ?? {})}`
    );
}
