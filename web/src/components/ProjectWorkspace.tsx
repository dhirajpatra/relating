import React, { useEffect, useState } from 'react';
import { api } from '../api/client';
import {
  AiHealthReport,
  GraphQueryResult,
  OntologyStatusResponse,
  ProjectDetail,
  QueryTargetsResponse,
  Source,
} from '../types';
import { AuraMigration } from './AuraMigration';
import { GraphExplorer } from './GraphExplorer';
import { McpConnect } from './McpConnect';
import { OntologyReview } from './OntologyReview';
import { PortabilityWorkbench } from './PortabilityWorkbench';
import { RetrievalWorkbench } from './RetrievalWorkbench';
import { SchemaWorkbench } from './SchemaWorkbench';
import { ShadowDebugWorkbench } from './ShadowDebugWorkbench';
import { SynthAdvisorWorkbench } from './SynthAdvisorWorkbench';
import { SourceManager } from './SourceManager';

interface ProjectWorkspaceProps {
  projectId: string;
  onBack: () => void;
}

export const ProjectWorkspace: React.FC<ProjectWorkspaceProps> = ({ projectId, onBack }) => {
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [sources, setSources] = useState<Source[]>([]);
  const [ontologyStatus, setOntologyStatus] = useState<OntologyStatusResponse>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'sources' | 'ontology' | 'explorer' | 'cypher' | 'ai' | 'retrieval' | 'agents' | 'aura' | 'schema' | 'portability' | 'debug' | 'synth'>('sources');

  // Cypher workbench state
  const [query, setQuery] = useState('MATCH (n) RETURN n LIMIT 25');
  const [queryRunning, setQueryRunning] = useState(false);
  const [queryResult, setQueryResult] = useState<GraphQueryResult | null>(null);
  const [queryError, setQueryError] = useState<string | null>(null);
  // Backend flip: '' runs on the local Relate store; otherwise an Aura connection id.
  const [queryTargets, setQueryTargets] = useState<QueryTargetsResponse | null>(null);
  const [queryTarget, setQueryTarget] = useState('');
  const [confirmRemoteWrite, setConfirmRemoteWrite] = useState(false);

  // AI test state
  const [aiTesting, setAiTesting] = useState(false);
  const [aiReport, setAiReport] = useState<AiHealthReport | null>(null);

  const fetchWorkspaceData = async () => {
    try {
      setLoading(true);
      const [projData, sourcesData, ontData] = await Promise.all([
        api.getProject(projectId),
        api.listSources(projectId),
        api.getOntology(projectId),
      ]);
      setDetail(projData);
      setSources(sourcesData);
      setOntologyStatus(ontData);
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
    fetchWorkspaceData();
  }, [projectId]);

  useEffect(() => {
    if (activeTab !== 'cypher' || queryTargets) return;
    api.listQueryTargets(projectId).then(setQueryTargets).catch(() => {
      // Target listing is additive; the console still works against local.
    });
  }, [activeTab, projectId]);

  const handleRunQuery = async (isWrite: boolean) => {
    if (!query.trim()) return;
    setQueryRunning(true);
    setQueryError(null);
    setQueryResult(null);

    try {
      if (isWrite) {
        const writeRes = await api.runCypherWrite(
          projectId,
          query,
          undefined,
          queryTarget || undefined,
          queryTarget ? confirmRemoteWrite : undefined
        );
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
        // Refresh project data for local writes; remote writes leave the local store untouched.
        if (!queryTarget) {
          const updated = await api.getProject(projectId);
          setDetail(updated);
        }
      } else {
        const res = await api.runCypherRead(projectId, query, undefined, undefined, queryTarget || undefined);
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
  const hasApprovedOntology = ontologyStatus.approved !== undefined && ontologyStatus.approved !== null;

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
              <strong>{sources.length}</strong> sources
            </span>
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
      <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '1.25rem', borderBottom: '1px solid var(--border-color)' }}>
        {[
          { id: 'sources', label: `1. Sources (${sources.length})` },
          {
            id: 'ontology',
            label: `2. Ontology ${hasApprovedOntology ? '✓' : ''}`,
          },
          { id: 'explorer', label: '3. Graph Explorer' },
          { id: 'cypher', label: '4. Cypher Console' },
          { id: 'ai', label: '5. AI Service' },
          { id: 'retrieval', label: '6. Retrieval & Q&A' },
          { id: 'agents', label: '7. Agents' },
          { id: 'aura', label: '8. Aura Migration' },
          { id: 'schema', label: '9. Schema & Contracts' },
          { id: 'portability', label: '10. Portability' },
          { id: 'debug', label: '11. Shadow & Debug' },
          { id: 'synth', label: '12. Test Data & Advisor' },
        ].map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id as any)}
            style={{
              padding: '0.6rem 1.1rem',
              backgroundColor: 'transparent',
              border: 'none',
              borderBottom: activeTab === tab.id ? '2px solid var(--accent)' : '2px solid transparent',
              color: activeTab === tab.id ? 'var(--accent-light)' : 'var(--text-muted)',
              fontWeight: 600,
              fontSize: '0.85rem',
            }}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Tab Content */}
      {activeTab === 'sources' && (
        <SourceManager
          projectId={projectId}
          sources={sources}
          onSourcesChanged={fetchWorkspaceData}
        />
      )}

      {activeTab === 'ontology' && (
        <OntologyReview
          projectId={projectId}
          ontologyStatus={ontologyStatus}
          hasSources={sources.length > 0}
          onOntologyChanged={fetchWorkspaceData}
        />
      )}

      {activeTab === 'explorer' && (
        <GraphExplorer
          projectId={projectId}
          hasApprovedOntology={hasApprovedOntology}
          onGraphMutated={fetchWorkspaceData}
        />
      )}

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

            <div style={{ display: 'flex', gap: '0.75rem', alignItems: 'center', marginBottom: '0.5rem' }}>
              <label style={{ fontSize: '0.78rem', color: 'var(--text-muted)' }}>
                Target:
              </label>
              <select
                value={queryTarget}
                onChange={(e) => {
                  setQueryTarget(e.target.value);
                  setConfirmRemoteWrite(false);
                }}
                style={{
                  padding: '0.35rem 0.6rem',
                  backgroundColor: 'var(--bg-primary)',
                  border: '1px solid var(--border-color)',
                  borderRadius: '6px',
                  color: 'var(--text-primary)',
                  fontSize: '0.8rem',
                }}
              >
                <option value="">Local Relate store (relate://local)</option>
                {(queryTargets?.aura || []).map((t) => (
                  <option key={t.connection_id} value={t.connection_id}>
                    Aura: {t.name} ({t.connection_string})
                  </option>
                ))}
              </select>
              {queryTarget && (
                <label style={{ fontSize: '0.78rem', color: 'var(--text-secondary)', display: 'flex', gap: '0.35rem', alignItems: 'center' }}>
                  <input
                    type="checkbox"
                    checked={confirmRemoteWrite}
                    onChange={(e) => setConfirmRemoteWrite(e.target.checked)}
                  />
                  Confirm remote writes
                </label>
              )}
            </div>
            <p style={{ fontSize: '0.75rem', color: 'var(--text-muted)', marginBottom: '0.5rem' }}>
              Same query, either backend: flip the target to run identical Cypher on Aura after migration.
            </p>

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
                color: 'var(--accent-light)',
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

      {activeTab === 'retrieval' && (
        <RetrievalWorkbench projectId={projectId} hasSources={sources.length > 0} />
      )}

      {activeTab === 'agents' && (
        <McpConnect projectId={projectId} projectName={project.name} />
      )}

      {activeTab === 'aura' && <AuraMigration projectId={projectId} />}

      {activeTab === 'schema' && <SchemaWorkbench projectId={projectId} />}

      {activeTab === 'portability' && <PortabilityWorkbench projectId={projectId} />}

      {activeTab === 'debug' && <ShadowDebugWorkbench projectId={projectId} />}

      {activeTab === 'synth' && <SynthAdvisorWorkbench projectId={projectId} />}

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
