import {
  AiHealthReport,
  CreateProjectRequest,
  GraphQueryResult,
  Project,
  ProjectDetail,
  ProjectSummary,
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
};
