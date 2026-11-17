import React, { useEffect, useState } from 'react';
import { api } from '../api/client';
import {
  AdvisorResponse,
  AuraConnection,
  EdgeCase,
  GenerateResponse,
  GraphProfile,
} from '../types';

interface SynthAdvisorWorkbenchProps {
  projectId: string;
}

const ALL_EDGE_CASES: EdgeCase[] = ['duplicates', 'orphans', 'cycle', 'invalid_values'];

const DEFAULT_ADVISOR_QUERIES = `[
  { "name": "all nodes", "cypher": "MATCH (n) RETURN n" }
]`;

export const SynthAdvisorWorkbench: React.FC<SynthAdvisorWorkbenchProps> = ({
  projectId,
}) => {
  const [profile, setProfile] = useState<GraphProfile | null>(null);
  const [scale, setScale] = useState('1');
  const [seed, setSeed] = useState('7');
  const [edgeCases, setEdgeCases] = useState<EdgeCase[]>([]);
  const [materialize, setMaterialize] = useState(true);
  const [genResult, setGenResult] = useState<GenerateResponse | null>(null);
  const [generating, setGenerating] = useState(false);
  const [synthError, setSynthError] = useState<string | null>(null);

  const [queriesText, setQueriesText] = useState(DEFAULT_ADVISOR_QUERIES);
  const [connections, setConnections] = useState<AuraConnection[]>([]);
  const [selectedConnId, setSelectedConnId] = useState('');
  const [measure, setMeasure] = useState(false);
  const [advisorResult, setAdvisorResult] = useState<AdvisorResponse | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [advisorError, setAdvisorError] = useState<string | null>(null);

  useEffect(() => {
    api
      .synthProfile(projectId)
      .then(setProfile)
      .catch(() => {
        // panel shows empty state
      });
    api
      .listAuraConnections(projectId)
      .then((list) => {
        setConnections(list);
        if (list.length > 0) setSelectedConnId(list[0].id);
      })
      .catch(() => {
        // stays empty
      });
  }, [projectId]);

  const toggleEdgeCase = (edgeCase: EdgeCase) => {
    setEdgeCases(
      edgeCases.includes(edgeCase)
        ? edgeCases.filter((e) => e !== edgeCase)
        : [...edgeCases, edgeCase]
    );
  };

  const handleGenerate = async () => {
    setGenerating(true);
    setSynthError(null);
    setGenResult(null);
    try {
      const res = await api.synthGenerate(projectId, {
        scale: Number(scale),
        seed: Number(seed),
        edge_cases: edgeCases,
        materialize,
      });
      setGenResult(res);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setSynthError(err.message);
      } else {
        setSynthError('Generation failed');
      }
    } finally {
      setGenerating(false);
    }
  };

  const handleAnalyze = async () => {
    setAnalyzing(true);
    setAdvisorError(null);
    setAdvisorResult(null);
    try {
      const queries = JSON.parse(queriesText);
      if (!Array.isArray(queries)) {
        throw new Error('Queries must be a JSON array.');
      }
      const res = await api.advisorAnalyze(
        projectId,
        queries,
        measure ? selectedConnId || undefined : undefined,
        measure || undefined
      );
      setAdvisorResult(res);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setAdvisorError(err.message);
      } else {
        setAdvisorError('Analysis failed');
      }
    } finally {
      setAnalyzing(false);
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '1.25rem' }}>
      {/* Synthetic data */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.5rem' }}>
          Synthetic Test Data
        </h3>
        <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
          Profiled from this graph, generated deterministically from a seed. No real
          customer values are copied — only shapes and distributions. Materializing
          creates an isolated sibling project; the source is never touched.
        </p>

        {profile && (
          <p style={{ fontSize: '0.8rem', color: 'var(--text-secondary)', marginBottom: '0.75rem' }}>
            Source: {profile.total_nodes} nodes ({profile.labels.map((l) => `${l.label}:${l.count}`).join(', ') || 'none'})
            · {profile.total_relationships} relationships (
            {profile.rel_types.map((r) => `${r.rel_type}:${r.count}`).join(', ') || 'none'})
          </p>
        )}

        <div style={{ display: 'flex', gap: '0.5rem', flexWrap: 'wrap', marginBottom: '0.75rem', alignItems: 'center' }}>
          <label style={{ fontSize: '0.78rem', color: 'var(--text-secondary)' }}>
            Scale{' '}
            <input
              type="number"
              min={0}
              max={1}
              step={0.1}
              value={scale}
              onChange={(e) => setScale(e.target.value)}
              style={{
                width: '70px',
                padding: '0.35rem 0.5rem',
                backgroundColor: 'var(--bg-primary)',
                border: '1px solid var(--border-color)',
                borderRadius: '4px',
                color: 'var(--text-primary)',
                fontSize: '0.8rem',
              }}
            />
          </label>
          <label style={{ fontSize: '0.78rem', color: 'var(--text-secondary)' }}>
            Seed{' '}
            <input
              type="number"
              min={0}
              value={seed}
              onChange={(e) => setSeed(e.target.value)}
              style={{
                width: '90px',
                padding: '0.35rem 0.5rem',
                backgroundColor: 'var(--bg-primary)',
                border: '1px solid var(--border-color)',
                borderRadius: '4px',
                color: 'var(--text-primary)',
                fontSize: '0.8rem',
              }}
            />
          </label>
          {ALL_EDGE_CASES.map((e) => (
            <label
              key={e}
              style={{ fontSize: '0.78rem', color: 'var(--text-secondary)', display: 'flex', gap: '0.25rem', alignItems: 'center' }}
            >
              <input
                type="checkbox"
                checked={edgeCases.includes(e)}
                onChange={() => toggleEdgeCase(e)}
              />
              {e}
            </label>
          ))}
          <label
            style={{ fontSize: '0.78rem', color: 'var(--text-secondary)', display: 'flex', gap: '0.25rem', alignItems: 'center' }}
          >
            <input
              type="checkbox"
              checked={materialize}
              onChange={(e) => setMaterialize(e.target.checked)}
            />
            Materialize to new project
          </label>
          <button
            onClick={handleGenerate}
            disabled={generating}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.4rem 1.2rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              opacity: generating ? 0.6 : 1,
            }}
          >
            {generating ? 'Generating...' : 'Generate'}
          </button>
        </div>

        {synthError && (
          <div
            style={{
              padding: '0.6rem 0.8rem',
              backgroundColor: 'rgba(239, 68, 68, 0.1)',
              border: '1px solid var(--danger)',
              color: 'var(--danger)',
              borderRadius: '6px',
              fontSize: '0.8rem',
              marginBottom: '0.75rem',
            }}
          >
            {synthError}
          </div>
        )}

        {genResult && (
          <div style={{ fontSize: '0.8rem' }}>
            <p style={{ marginBottom: '0.4rem' }}>
              <strong>
                {genResult.stats.node_count} nodes, {genResult.stats.rel_count} relationships
              </strong>{' '}
              {genResult.target_project_id && (
                <span style={{ color: 'var(--success)' }}>
                  → materialized (see the projects dashboard for the new test project)
                </span>
              )}
            </p>
            {genResult.stats.edge_cases_applied.length > 0 && (
              <p style={{ color: 'var(--text-secondary)', margin: '0 0 0.4rem 0' }}>
                Edge cases: {genResult.stats.edge_cases_applied.join(' · ')}
              </p>
            )}
            {genResult.notes.map((n, i) => (
              <p key={i} style={{ color: 'var(--text-muted)', margin: 0, fontSize: '0.78rem' }}>
                · {n}
              </p>
            ))}
            <pre
              style={{
                marginTop: '0.5rem',
                padding: '0.6rem 0.8rem',
                backgroundColor: 'var(--bg-primary)',
                borderRadius: '6px',
                fontSize: '0.72rem',
                fontFamily: 'monospace',
                whiteSpace: 'pre-wrap',
                color: 'var(--text-secondary)',
                maxHeight: '220px',
                overflowY: 'auto',
              }}
            >
              {JSON.stringify(genResult.sample_nodes.slice(0, 3), null, 1)}
            </pre>
          </div>
        )}
      </div>

      {/* Advisor */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.5rem' }}>
          Performance & Cost Advisor
        </h3>
        <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
          Static review of queries plus live structure notes. Sizing is an estimate
          with stated assumptions — measurements (optional, needs a connection) are
          observed on the target.
        </p>
        <textarea
          value={queriesText}
          onChange={(e) => setQueriesText(e.target.value)}
          rows={4}
          spellCheck={false}
          style={{
            width: '100%',
            padding: '0.6rem 0.8rem',
            backgroundColor: 'var(--bg-primary)',
            border: '1px solid var(--border-color)',
            borderRadius: '6px',
            color: 'var(--text-primary)',
            fontSize: '0.75rem',
            fontFamily: 'monospace',
            marginBottom: '0.5rem',
          }}
        />
        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '0.75rem', alignItems: 'center', flexWrap: 'wrap' }}>
          <select
            value={selectedConnId}
            onChange={(e) => setSelectedConnId(e.target.value)}
            style={{
              flex: 1,
              minWidth: '200px',
              padding: '0.5rem 0.7rem',
              backgroundColor: 'var(--bg-primary)',
              border: '1px solid var(--border-color)',
              borderRadius: '6px',
              color: 'var(--text-primary)',
              fontSize: '0.8rem',
            }}
          >
            {connections.length === 0 && <option value="">No Aura connections</option>}
            {connections.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name} — {c.uri}
              </option>
            ))}
          </select>
          <label
            style={{ fontSize: '0.78rem', color: 'var(--text-secondary)', display: 'flex', gap: '0.25rem', alignItems: 'center' }}
          >
            <input
              type="checkbox"
              checked={measure}
              onChange={(e) => setMeasure(e.target.checked)}
            />
            Measure on target
          </label>
          <button
            onClick={handleAnalyze}
            disabled={analyzing}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.4rem 1.2rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              opacity: analyzing ? 0.6 : 1,
            }}
          >
            {analyzing ? 'Analyzing...' : 'Analyze'}
          </button>
        </div>

        {advisorError && (
          <div
            style={{
              padding: '0.6rem 0.8rem',
              backgroundColor: 'rgba(239, 68, 68, 0.1)',
              border: '1px solid var(--danger)',
              color: 'var(--danger)',
              borderRadius: '6px',
              fontSize: '0.8rem',
              marginBottom: '0.75rem',
            }}
          >
            {advisorError}
          </div>
        )}

        {advisorResult && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: '0.6rem', fontSize: '0.8rem' }}>
            {advisorResult.queries.map((q, i) => (
              <div
                key={i}
                style={{
                  padding: '0.6rem 0.8rem',
                  backgroundColor: 'var(--bg-primary)',
                  borderRadius: '6px',
                }}
              >
                <strong>{q.name}</strong>
                {q.findings.map((f, j) => (
                  <div key={j} style={{ marginTop: '0.25rem', color: 'var(--text-secondary)' }}>
                    <span
                      style={{
                        fontSize: '0.7rem',
                        fontWeight: 700,
                        color: f.severity === 'warning' ? '#f59e0b' : 'var(--success)',
                        marginRight: '0.4rem',
                      }}
                    >
                      {f.rule_id}
                    </span>
                    {f.message}
                    <div style={{ color: 'var(--text-muted)', fontSize: '0.75rem' }}>
                      → {f.suggestion}
                    </div>
                  </div>
                ))}
              </div>
            ))}

            <div
              style={{
                padding: '0.6rem 0.8rem',
                backgroundColor: 'var(--bg-primary)',
                borderRadius: '6px',
              }}
            >
              <strong>Structure</strong>
              {advisorResult.structure.map((s, i) => (
                <div key={i} style={{ marginTop: '0.25rem', color: 'var(--text-secondary)', fontSize: '0.78rem' }}>
                  <span
                    style={{
                      fontSize: '0.7rem',
                      fontWeight: 700,
                      color: s.severity === 'warning' ? '#f59e0b' : 'var(--success)',
                      marginRight: '0.4rem',
                    }}
                  >
                    {s.rule_id}
                  </span>
                  {s.message} <span style={{ color: 'var(--text-muted)' }}>[{s.basis}]</span>
                </div>
              ))}
            </div>

            <div
              style={{
                padding: '0.6rem 0.8rem',
                backgroundColor: 'var(--bg-primary)',
                borderRadius: '6px',
              }}
            >
              <strong>Sizing (estimate)</strong>
              <p style={{ margin: '0.25rem 0', color: 'var(--text-secondary)' }}>
                {advisorResult.sizing.tier_hint} · {advisorResult.sizing.estimated_elements}{' '}
                elements · ~{advisorResult.sizing.estimated_storage_mb} MB
              </p>
              {advisorResult.sizing.assumptions.map((a, i) => (
                <div key={i} style={{ fontSize: '0.75rem', color: 'var(--text-muted)' }}>
                  assumes: {a}
                </div>
              ))}
              {advisorResult.sizing.caveats.map((c, i) => (
                <div key={i} style={{ fontSize: '0.75rem', color: '#f59e0b' }}>
                  caveat: {c}
                </div>
              ))}
            </div>

            {advisorResult.measurements && (
              <div
                style={{
                  padding: '0.6rem 0.8rem',
                  backgroundColor: 'var(--bg-primary)',
                  borderRadius: '6px',
                }}
              >
                <strong>Measured on target</strong>
                <p style={{ margin: '0.25rem 0', color: 'var(--text-secondary)', fontSize: '0.78rem' }}>
                  {advisorResult.measurements.target_nodes} nodes ·{' '}
                  {advisorResult.measurements.target_relationships} relationships
                </p>
                {advisorResult.measurements.plans.map((p, i) => (
                  <div key={i} style={{ fontSize: '0.78rem', color: 'var(--text-secondary)' }}>
                    <span style={{ color: p.planned ? 'var(--success)' : 'var(--danger)' }}>
                      {p.planned ? '✓' : '✕'}
                    </span>{' '}
                    {p.query_name}: {p.planned ? `planned in ${p.planning_ms.toFixed(1)}ms` : p.error}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};
