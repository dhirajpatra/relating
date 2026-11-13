import React, { useEffect, useState } from 'react';
import { api } from '../api/client';
import { AiHealthReport, GraphQueryResult, ProjectDetail } from '../types';

interface ProjectWorkspaceProps {
  projectId: string;
  onBack: () => void;
}

export const ProjectWorkspace: React.FC<ProjectWorkspaceProps> = ({ projectId, onBack }) => {
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'cypher' | 'schema' | 'ai'>('cypher');

  // Cypher workbench state
  const [query, setQuery] = useState('MATCH (n) RETURN n LIMIT 25');
  const [queryRunning, setQueryRunning] = useState(false);
  const [queryResult, setQueryResult] = useState<GraphQueryResult | null>(null);
  const [queryError, setQueryError] = useState<string | null>(null);

  // AI test state
  const [aiTesting, setAiTesting] = useState(false);
  const [aiReport, setAiReport] = useState<AiHealthReport | null>(null);

  const fetchProjectDetail = async () => {
    try {
      setLoading(true);
      const data = await api.getProject(projectId);
      setDetail(data);
      setError(null);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('Failed to load project');
      }
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchProjectDetail();
  }, [projectId]);

  const handleRunQuery = async (isWrite: boolean) => {
    if (!query.trim()) return;
    setQueryRunning(true);
    setQueryError(null);
    setQueryResult(null);

    try {
      if (isWrite) {
        const writeRes = await api.runCypherWrite(projectId, query);
        setQueryResult({
          columns: ['Nodes Created', 'Relationships Created', 'Time (ms)'],
          rows: [
            {
              'Nodes Created': writeRes.nodes_created,
              'Relationships Created': writeRes.relationships_created,
              'Time (ms)': writeRes.elapsed_ms.toFixed(2),
            },
          ],
          elapsed_ms: writeRes.elapsed_ms,
          row_count: 1,
        });
        // Refresh schema counts after write
        await fetchProjectDetail();
      } else {
        const res = await api.runCypherRead(projectId, query);
        setQueryResult(res);
      }
    } catch (err: unknown) {
      if (err instanceof Error) {
        setQueryError(err.message);
      } else {
        setQueryError('Query execution failed');
      }
    } finally {
      setQueryRunning(false);
    }
  };

  const handleTestAi = async () => {
    setAiTesting(true);
    try {
      const rep = await api.testProjectAi(projectId);
      setAiReport(rep);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setAiReport({
          healthy: false,
          provider: detail?.project.ai_config.provider_type || 'unknown',
          chat_model: detail?.project.ai_config.chat_model || 'unknown',
          embedding_model: detail?.project.ai_config.embedding_model || 'unknown',
          latency_ms: 0,
          message: err.message,
        });
      }
    } finally {
      setAiTesting(false);
    }
  };

  if (loading && !detail) {
    return (
      <div style={{ textAlign: 'center', padding: '4rem 0', color: 'var(--text-muted)' }}>
        Loading project workbench...
      </div>
    );
  }

  if (error || !detail) {
    return (
      <div style={{ maxWidth: '600px', margin: '3rem auto', textAlign: 'center' }}>
        <p style={{ color: 'var(--danger)', marginBottom: '1rem' }}>{error || 'Project not found'}</p>
        <button
          onClick={onBack}
          style={{
            padding: '0.5rem 1rem',
            backgroundColor: 'var(--bg-secondary)',
            border: '1px solid var(--border-color)',
            color: 'var(--text-primary)',
            borderRadius: '6px',
          }}
        >
          ← Back to Dashboard
        </button>
      </div>
    );
  }

  const { project, schema } = detail;

  return (
    <div style={{ maxWidth: '1200px', margin: '1.5rem auto', padding: '0 1.5rem' }}>
      {/* Workbench Header */}
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          backgroundColor: 'var(--bg-secondary)',
          padding: '1.25rem 1.5rem',
          borderRadius: '10px',
          border: '1px solid var(--border-color)',
          marginBottom: '1.5rem',
        }}
      >
        <div>
          <button
            onClick={onBack}
            style={{
              background: 'none',
              border: 'none',
              color: 'var(--text-muted)',
              fontSize: '0.85rem',
              cursor: 'pointer',
              marginBottom: '0.25rem',
              padding: 0,
            }}
          >
            ← Back to Projects
          </button>
          <h2 style={{ fontSize: '1.35rem', fontWeight: 700 }}>{project.name}</h2>
          <p style={{ fontSize: '0.85rem', color: 'var(--text-secondary)' }}>
            {project.description || 'Project-scoped Relate graph store'}
          </p>
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: '1rem' }}>
          <div
            style={{
              padding: '0.4rem 0.8rem',
              backgroundColor: 'var(--bg-primary)',
              borderRadius: '6px',
              fontSize: '0.8rem',
              display: 'flex',
              gap: '1rem',
            }}
          >
            <span>
              <strong>{schema.node_count}</strong> nodes
            </span>
            <span>
              <strong>{schema.relationship_count}</strong> relationships
            </span>
          </div>

          <button
            onClick={handleTestAi}
            disabled={aiTesting}
            style={{
              backgroundColor: 'transparent',
              border: '1px solid var(--border-color)',
              color: 'var(--text-primary)',
              borderRadius: '6px',
              padding: '0.4rem 0.8rem',
              fontSize: '0.8rem',
              fontWeight: 600,
            }}
          >
            {aiTesting ? 'Testing AI...' : '⚡ Test AI Service'}
          </button>
        </div>
      </div>

      {/* AI Health Notification if tested */}
      {aiReport && (
        <div
          style={{
            padding: '0.75rem 1rem',
            borderRadius: '6px',
            marginBottom: '1.5rem',
            backgroundColor: aiReport.healthy ? 'rgba(16, 185, 129, 0.1)' : 'rgba(239, 68, 68, 0.1)',
            border: `1px solid ${aiReport.healthy ? 'var(--success)' : 'var(--danger)'}`,
            color: aiReport.healthy ? 'var(--success)' : 'var(--danger)',
            fontSize: '0.85rem',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}
        >
          <span>
            <strong>{aiReport.healthy ? '✓ AI Service Operational:' : '✕ AI Service Failure:'}</strong>{' '}
            {aiReport.message} ({aiReport.latency_ms.toFixed(1)}ms)
          </span>
          <button
            onClick={() => setAiReport(null)}
            style={{ background: 'none', border: 'none', color: 'inherit', fontSize: '1rem' }}
          >
            &times;
          </button>
        </div>
      )}

      {/* Navigation Tabs */}
      <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '1rem', borderBottom: '1px solid var(--border-color)' }}>
        {(['cypher', 'schema', 'ai'] as const).map((tab) => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            style={{
              padding: '0.6rem 1.25rem',
              backgroundColor: 'transparent',
              border: 'none',
              borderBottom: activeTab === tab ? '2px solid var(--accent)' : '2px solid transparent',
              color: activeTab === tab ? 'var(--accent-light)' : 'var(--text-muted)',
              fontWeight: 600,
              fontSize: '0.875rem',
              textTransform: 'capitalize',
            }}
          >
            {tab === 'cypher' ? 'Cypher Console' : tab === 'schema' ? 'Graph Schema' : 'AI Service'}
          </button>
        ))}
      </div>

      {/* Tab Content */}
      {activeTab === 'cypher' && (
        <div>
          <div
            style={{
              backgroundColor: 'var(--bg-secondary)',
              borderRadius: '8px',
              border: '1px solid var(--border-color)',
              padding: '1rem',
              marginBottom: '1rem',
            }}
          >
            <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '0.5rem' }}>
              <label style={{ fontSize: '0.8rem', fontWeight: 600, color: 'var(--text-secondary)' }}>
                Cypher Query Statement
              </label>
              <div style={{ display: 'flex', gap: '0.5rem' }}>
                <button
                  type="button"
                  onClick={() => setQuery('MATCH (n) RETURN n LIMIT 25')}
                  style={{
                    fontSize: '0.75rem',
                    color: 'var(--text-muted)',
                    background: 'none',
                    border: 'none',
                  }}
                >
                  Template: Match All
                </button>
                <button
                  type="button"
                  onClick={() =>
                    setQuery("CREATE (e:Entity {name: 'Knowledge Node', type: 'Concept'})")
                  }
                  style={{
                    fontSize: '0.75rem',
                    color: 'var(--text-muted)',
                    background: 'none',
                    border: 'none',
                  }}
                >
                  Template: Create Node
                </button>
              </div>
            </div>

            <textarea
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              rows={4}
              style={{
                width: '100%',
                padding: '0.75rem',
                backgroundColor: 'var(--bg-primary)',
                border: '1px solid var(--border-color)',
                borderRadius: '6px',
                color: '#38bdf8',
                fontFamily: 'monospace',
                fontSize: '0.9rem',
                lineHeight: 1.4,
              }}
            />

            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '0.75rem', marginTop: '0.75rem' }}>
              <button
                onClick={() => handleRunQuery(true)}
                disabled={queryRunning}
                style={{
                  backgroundColor: 'rgba(239, 68, 68, 0.15)',
                  border: '1px solid var(--danger)',
                  color: 'var(--danger)',
                  borderRadius: '6px',
                  padding: '0.4rem 0.9rem',
                  fontSize: '0.8rem',
                  fontWeight: 600,
                }}
              >
                Execute Write
              </button>
              <button
                onClick={() => handleRunQuery(false)}
                disabled={queryRunning}
                style={{
                  backgroundColor: 'var(--accent)',
                  color: '#fff',
                  border: 'none',
                  borderRadius: '6px',
                  padding: '0.4rem 1.2rem',
                  fontSize: '0.8rem',
                  fontWeight: 600,
                  opacity: queryRunning ? 0.7 : 1,
                }}
              >
                {queryRunning ? 'Running...' : 'Run Read Query ▷'}
              </button>
            </div>
          </div>

          {/* Query Error */}
          {queryError && (
            <div
              style={{
                padding: '0.75rem 1rem',
                backgroundColor: 'rgba(239, 68, 68, 0.1)',
                border: '1px solid var(--danger)',
                color: 'var(--danger)',
                borderRadius: '6px',
                marginBottom: '1rem',
                fontSize: '0.85rem',
                fontFamily: 'monospace',
              }}
            >
              {queryError}
            </div>
          )}

          {/* Results Table */}
          {queryResult && (
            <div
              style={{
                backgroundColor: 'var(--bg-secondary)',
                borderRadius: '8px',
                border: '1px solid var(--border-color)',
                overflow: 'hidden',
              }}
            >
              <div
                style={{
                  padding: '0.6rem 1rem',
                  backgroundColor: 'rgba(255, 255, 255, 0.02)',
                  borderBottom: '1px solid var(--border-color)',
                  fontSize: '0.75rem',
                  color: 'var(--text-muted)',
                  display: 'flex',
                  justifyContent: 'space-between',
                }}
              >
                <span>Returned {queryResult.row_count} rows</span>
                <span>Execution time: {queryResult.elapsed_ms.toFixed(2)}ms</span>
              </div>

              {queryResult.rows.length === 0 ? (
                <div style={{ padding: '2rem', textAlign: 'center', color: 'var(--text-muted)' }}>
                  (No rows returned)
                </div>
              ) : (
                <div style={{ overflowX: 'auto' }}>
                  <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
                    <thead>
                      <tr style={{ backgroundColor: 'var(--bg-primary)', textAlign: 'left' }}>
                        {queryResult.columns.map((col) => (
                          <th
                            key={col}
                            style={{
                              padding: '0.6rem 1rem',
                              borderBottom: '1px solid var(--border-color)',
                              fontWeight: 600,
                              color: 'var(--text-secondary)',
                            }}
                          >
                            {col}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {queryResult.rows.map((row, rowIdx) => (
                        <tr
                          key={rowIdx}
                          style={{
                            borderBottom: '1px solid rgba(255, 255, 255, 0.05)',
                            backgroundColor: rowIdx % 2 === 0 ? 'transparent' : 'rgba(255, 255, 255, 0.01)',
                          }}
                        >
                          {queryResult.columns.map((col) => (
                            <td
                              key={col}
                              style={{
                                padding: '0.6rem 1rem',
                                color: 'var(--text-primary)',
                                fontFamily: 'monospace',
                              }}
                            >
                              {typeof row[col] === 'object'
                                ? JSON.stringify(row[col])
                                : String(row[col] ?? 'null')}
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {activeTab === 'schema' && (
        <div
          style={{
            backgroundColor: 'var(--bg-secondary)',
            borderRadius: '8px',
            border: '1px solid var(--border-color)',
            padding: '1.5rem',
          }}
        >
          <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '1rem' }}>
            Graph Schema Overview
          </h3>

          <div style={{ marginBottom: '1.5rem' }}>
            <h4 style={{ fontSize: '0.85rem', color: 'var(--text-muted)', marginBottom: '0.5rem' }}>
              NODE LABELS ({schema.labels.length})
            </h4>
            <div style={{ display: 'flex', gap: '0.5rem', flexWrap: 'wrap' }}>
              {schema.labels.length === 0 ? (
                <span style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>
                  No node labels in graph.
                </span>
              ) : (
                schema.labels.map((l) => (
                  <span
                    key={l}
                    style={{
                      padding: '0.3rem 0.6rem',
                      backgroundColor: 'rgba(99, 102, 241, 0.15)',
                      borderRadius: '4px',
                      color: 'var(--accent-light)',
                      fontSize: '0.8rem',
                      fontFamily: 'monospace',
                    }}
                  >
                    :{l}
                  </span>
                ))
              )}
            </div>
          </div>

          <div>
            <h4 style={{ fontSize: '0.85rem', color: 'var(--text-muted)', marginBottom: '0.5rem' }}>
              RELATIONSHIP TYPES ({schema.relationship_types.length})
            </h4>
            <div style={{ display: 'flex', gap: '0.5rem', flexWrap: 'wrap' }}>
              {schema.relationship_types.length === 0 ? (
                <span style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>
                  No relationships in graph.
                </span>
              ) : (
                schema.relationship_types.map((r) => (
                  <span
                    key={r}
                    style={{
                      padding: '0.3rem 0.6rem',
                      backgroundColor: 'rgba(168, 85, 247, 0.15)',
                      borderRadius: '4px',
                      color: '#c084fc',
                      fontSize: '0.8rem',
                      fontFamily: 'monospace',
                    }}
                  >
                    [:{r}]
                  </span>
                ))
              )}
            </div>
          </div>
        </div>
      )}

      {activeTab === 'ai' && (
        <div
          style={{
            backgroundColor: 'var(--bg-secondary)',
            borderRadius: '8px',
            border: '1px solid var(--border-color)',
            padding: '1.5rem',
          }}
        >
          <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '1rem' }}>
            AI Service Configuration
          </h3>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '1rem', fontSize: '0.85rem' }}>
            <div style={{ padding: '0.8rem', backgroundColor: 'var(--bg-primary)', borderRadius: '6px' }}>
              <span style={{ color: 'var(--text-muted)', display: 'block', fontSize: '0.75rem' }}>
                PROVIDER TYPE
              </span>
              <span style={{ fontWeight: 600, textTransform: 'capitalize' }}>
                {project.ai_config.provider_type}
              </span>
            </div>
            <div style={{ padding: '0.8rem', backgroundColor: 'var(--bg-primary)', borderRadius: '6px' }}>
              <span style={{ color: 'var(--text-muted)', display: 'block', fontSize: '0.75rem' }}>
                CHAT / REASONING MODEL
              </span>
              <span style={{ fontWeight: 600 }}>{project.ai_config.chat_model}</span>
            </div>
            <div style={{ padding: '0.8rem', backgroundColor: 'var(--bg-primary)', borderRadius: '6px' }}>
              <span style={{ color: 'var(--text-muted)', display: 'block', fontSize: '0.75rem' }}>
                EMBEDDING MODEL
              </span>
              <span style={{ fontWeight: 600 }}>{project.ai_config.embedding_model}</span>
            </div>
            <div style={{ padding: '0.8rem', backgroundColor: 'var(--bg-primary)', borderRadius: '6px' }}>
              <span style={{ color: 'var(--text-muted)', display: 'block', fontSize: '0.75rem' }}>
                EMBEDDING DIMENSIONS
              </span>
              <span style={{ fontWeight: 600 }}>
                {project.ai_config.embedding_dimensions || 1536} dimensions
              </span>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
