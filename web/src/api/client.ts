import {
  AiHealthReport,
  CreateProjectRequest,
  CytoscapeGraph,
  GraphQueryResult,
  IngestionResultResponse,
  Ontology,
  OntologyStatusResponse,
  Project,
  ProjectDetail,
  ProjectSummary,
  Source,
  SystemHealth,
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
};
