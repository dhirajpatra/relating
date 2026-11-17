import {
  AiHealthReport,
  AnalyzeResponse,
  AuraConnection,
  ConnectionTestReport,
  CreateProjectRequest,
  CytoscapeGraph,
  DryRunReport,
  EmbeddingStatusResponse,
  EvaluationRun,
  GenerateEmbeddingsResponse,
  GraphQueryResult,
  IngestionResultResponse,
  McpHealth,
  GeneratedTest,
  Migration,
  AdvisorResponse,
  ApplyResponse,
  CreateRevisionResponse,
  DiffResponse,
  EdgeCase,
  ExportResponse,
  GenerateResponse,
  GraphProfile,
  Ontology,
  OntologyStatusResponse,
  PlanResponse,
  Project,
  DebugTraceResponse,
  PortableReport,
  ProjectDetail,
  ProjectSummary,
  QuestionAnswer,
  RunTestsResponse,
  RetrievalConfigResponse,
  RetrievalStrategy,
  RetrievalTrace,
  SchemaRevision,
  ShadowQuerySpec,
  ShadowRunResponse,
  Source,
  SystemHealth,
  VerificationReport,
} from '../types';

const BASE_URL = '/api/v1';

async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE_URL}${path}`, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...options?.headers,
    },
  });

  if (!res.ok) {
    let errorMsg = `HTTP ${res.status}`;
    try {
      const errJson = await res.json();
      if (errJson?.error?.message) {
        errorMsg = errJson.error.message;
      }
    } catch {
      // ignore
    }
    throw new Error(errorMsg);
  }

  return res.json() as Promise<T>;
}

export const api = {
  getHealth: () => request<SystemHealth>('/health'),

  listProjects: () => request<ProjectSummary[]>('/projects'),

  getProject: (id: string) => request<ProjectDetail>(`/projects/${id}`),

  createProject: (payload: CreateProjectRequest) =>
    request<Project>('/projects', {
      method: 'POST',
      body: JSON.stringify(payload),
    }),

  deleteProject: async (id: string) => {
    const res = await fetch(`${BASE_URL}/projects/${id}`, { method: 'DELETE' });
    if (!res.ok) {
      throw new Error(`Failed to delete project: HTTP ${res.status}`);
    }
  },

  testProjectAi: (id: string) =>
    request<AiHealthReport>(`/projects/${id}/ai/test`, {
      method: 'POST',
    }),

  runCypherRead: (id: string, query: string, params?: Record<string, unknown>, limit?: number) =>
    request<GraphQueryResult>(`/projects/${id}/cypher/read`, {
      method: 'POST',
      body: JSON.stringify({ query, params: params || {}, limit }),
    }),

  runCypherWrite: (id: string, query: string, params?: Record<string, unknown>) =>
    request<{ nodes_created: number; relationships_created: number; elapsed_ms: number }>(
      `/projects/${id}/cypher/write`,
      {
        method: 'POST',
        body: JSON.stringify({ query, params: params || {} }),
      }
    ),

  // Slice 2 API endpoints
  listSources: (projectId: string) =>
    request<Source[]>(`/projects/${projectId}/sources`),

  registerSource: (projectId: string, filename: string, content: string, contentType?: string) =>
    request<Source>(`/projects/${projectId}/sources`, {
      method: 'POST',
      body: JSON.stringify({
        filename,
        content,
        content_type: contentType,
      }),
    }),

  deleteSource: async (projectId: string, sourceId: string) => {
    const res = await fetch(`${BASE_URL}/projects/${projectId}/sources/${sourceId}`, {
      method: 'DELETE',
    });
    if (!res.ok) {
      throw new Error(`Failed to delete source: HTTP ${res.status}`);
    }
  },

  getOntology: (projectId: string) =>
    request<OntologyStatusResponse>(`/projects/${projectId}/ontology`),

  proposeOntology: (projectId: string, domainHint?: string) =>
    request<Ontology>(`/projects/${projectId}/ontology/propose`, {
      method: 'POST',
      body: JSON.stringify({ domain_hint: domainHint }),
    }),

  approveOntology: (projectId: string, ontologyId: string) =>
    request<{ success: boolean; message: string }>(`/projects/${projectId}/ontology/approve`, {
      method: 'POST',
      body: JSON.stringify({ ontology_id: ontologyId }),
    }),

  runIngestion: (projectId: string) =>
    request<IngestionResultResponse>(`/projects/${projectId}/ingest`, {
      method: 'POST',
    }),

  getGraphElements: (projectId: string, limit?: number) =>
    request<CytoscapeGraph>(
      `/projects/${projectId}/graph/elements${limit ? `?limit=${limit}` : ''}`
    ),

  // Slice 3 API endpoints
  getEmbeddingsStatus: (projectId: string) =>
    request<EmbeddingStatusResponse>(`/projects/${projectId}/embeddings`),

  generateEmbeddings: (projectId: string) =>
    request<GenerateEmbeddingsResponse>(`/projects/${projectId}/embeddings`, {
      method: 'POST',
      body: JSON.stringify({}),
    }),

  runRetrieval: (
    projectId: string,
    query: string,
    strategy?: RetrievalStrategy,
    topK?: number
  ) =>
    request<RetrievalTrace>(`/projects/${projectId}/retrieval`, {
      method: 'POST',
      body: JSON.stringify({ query, strategy, top_k: topK }),
    }),

  askQuestion: (projectId: string, question: string, strategy?: RetrievalStrategy) =>
    request<QuestionAnswer>(`/projects/${projectId}/questions`, {
      method: 'POST',
      body: JSON.stringify({ question, strategy }),
    }),

  createEvaluation: (projectId: string, questions: string[], strategy?: RetrievalStrategy) =>
    request<EvaluationRun>(`/projects/${projectId}/evaluations`, {
      method: 'POST',
      body: JSON.stringify({ questions, strategy }),
    }),

  listEvaluations: (projectId: string) =>
    request<EvaluationRun[]>(`/projects/${projectId}/evaluations`),

  // R1 API endpoints
  listAuraConnections: (projectId: string) =>
    request<AuraConnection[]>(`/projects/${projectId}/connections/aura`),

  createAuraConnection: (
    projectId: string,
    payload: {
      name: string;
      uri: string;
      username?: string;
      database?: string;
      auth_env_var?: string;
    }
  ) =>
    request<AuraConnection>(`/projects/${projectId}/connections/aura`, {
      method: 'POST',
      body: JSON.stringify(payload),
    }),

  deleteAuraConnection: async (projectId: string, connectionId: string) => {
    const res = await fetch(`${BASE_URL}/projects/${projectId}/connections/aura/${connectionId}`, {
      method: 'DELETE',
    });
    if (!res.ok) {
      throw new Error(`Failed to delete connection: HTTP ${res.status}`);
    }
  },

  testAuraConnection: (projectId: string, connectionId: string) =>
    request<ConnectionTestReport>(
      `/projects/${projectId}/connections/aura/${connectionId}/test`,
      { method: 'POST' }
    ),

  analyzeMigration: (projectId: string, connectionId?: string) =>
    request<AnalyzeResponse>(`/projects/${projectId}/migrations/analyze`, {
      method: 'POST',
      body: JSON.stringify({ connection_id: connectionId }),
    }),

  planMigration: (projectId: string, connectionId: string, batchSize?: number) =>
    request<PlanResponse>(`/projects/${projectId}/migrations/plan`, {
      method: 'POST',
      body: JSON.stringify({ connection_id: connectionId, batch_size: batchSize }),
    }),

  listMigrations: (projectId: string) =>
    request<Migration[]>(`/projects/${projectId}/migrations`),

  getMigration: (projectId: string, migrationId: string) =>
    request<Migration>(`/projects/${projectId}/migrations/${migrationId}`),

  dryRunMigration: (projectId: string, migrationId: string) =>
    request<DryRunReport>(`/projects/${projectId}/migrations/${migrationId}/dry-run`, {
      method: 'POST',
    }),

  approveMigration: (projectId: string, migrationId: string, approved: boolean) =>
    request<Migration>(`/projects/${projectId}/migrations/${migrationId}/approve`, {
      method: 'POST',
      body: JSON.stringify({ approved }),
    }),

  executeMigration: (projectId: string, migrationId: string) =>
    request<Migration>(`/projects/${projectId}/migrations/${migrationId}/execute`, {
      method: 'POST',
    }),

  verifyMigration: (projectId: string, migrationId: string) =>
    request<VerificationReport>(`/projects/${projectId}/migrations/${migrationId}/verify`, {
      method: 'POST',
    }),

  mcpHealth: async (): Promise<McpHealth> => {
    const res = await fetch('/mcp/health');
    if (!res.ok) {
      throw new Error(`MCP health check failed: HTTP ${res.status}`);
    }
    return res.json() as Promise<McpHealth>;
  },

  // R2 API endpoints
  exportSchema: (projectId: string) =>
    request<ExportResponse>(`/projects/${projectId}/schema/export`),

  createSchemaRevision: (projectId: string, document?: unknown) =>
    request<CreateRevisionResponse>(`/projects/${projectId}/schema/revisions`, {
      method: 'POST',
      body: JSON.stringify({ document }),
    }),

  listSchemaRevisions: (projectId: string) =>
    request<SchemaRevision[]>(`/projects/${projectId}/schema/revisions`),

  diffSchema: (
    projectId: string,
    payload: { from_version?: number; to_version?: number; to_document?: unknown }
  ) =>
    request<DiffResponse>(`/projects/${projectId}/schema/diff`, {
      method: 'POST',
      body: JSON.stringify(payload),
    }),

  applySchema: (projectId: string, document: unknown) =>
    request<ApplyResponse>(`/projects/${projectId}/schema/apply`, {
      method: 'POST',
      body: JSON.stringify({ document }),
    }),

  getRetrievalConfig: (projectId: string) =>
    request<RetrievalConfigResponse>(`/projects/${projectId}/retrieval-config`),

  updateRetrievalConfig: (
    projectId: string,
    payload: { default_strategy?: RetrievalStrategy; default_top_k?: number }
  ) =>
    request<RetrievalConfigResponse>(`/projects/${projectId}/retrieval-config`, {
      method: 'PUT',
      body: JSON.stringify(payload),
    }),

  // R3 API endpoints
  scanRepo: (projectId: string, repoPath: string) =>
    request<PortableReport>(`/projects/${projectId}/portability/scan`, {
      method: 'POST',
      body: JSON.stringify({ repo_path: repoPath }),
    }),

  runPortabilityTests: (
    projectId: string,
    connectionId: string,
    tests: GeneratedTest[]
  ) =>
    request<RunTestsResponse>(`/projects/${projectId}/portability/run-tests`, {
      method: 'POST',
      body: JSON.stringify({ connection_id: connectionId, tests }),
    }),

  // R5 API endpoints
  runShadow: (
    projectId: string,
    connectionId: string,
    queries: ShadowQuerySpec[],
    redactParams?: boolean
  ) =>
    request<ShadowRunResponse>(`/projects/${projectId}/shadow/runs`, {
      method: 'POST',
      body: JSON.stringify({
        connection_id: connectionId,
        queries,
        redact_params: redactParams,
      }),
    }),

  debugTrace: (projectId: string, question: string, strategy?: RetrievalStrategy) =>
    request<DebugTraceResponse>(`/projects/${projectId}/debug/trace`, {
      method: 'POST',
      body: JSON.stringify({ question, strategy }),
    }),

  // R6 API endpoints
  synthProfile: (projectId: string) =>
    request<GraphProfile>(`/projects/${projectId}/synth/profile`, {
      method: 'POST',
    }),

  synthGenerate: (
    projectId: string,
    payload: {
      scale?: number;
      seed?: number;
      edge_cases?: EdgeCase[];
      materialize?: boolean;
      target_name?: string;
    }
  ) =>
    request<GenerateResponse>(`/projects/${projectId}/synth/generate`, {
      method: 'POST',
      body: JSON.stringify(payload),
    }),

  advisorAnalyze: (
    projectId: string,
    queries: { name: string; cypher: string }[],
    connectionId?: string,
    measure?: boolean
  ) =>
    request<AdvisorResponse>(`/projects/${projectId}/advisor/analyze`, {
      method: 'POST',
      body: JSON.stringify({ queries, connection_id: connectionId, measure }),
    }),
};
