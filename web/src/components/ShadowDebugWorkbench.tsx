import React, { useEffect, useState } from 'react';
import { api } from '../api/client';
import {
  AuraConnection,
  CompareVerdict,
  DebugTraceResponse,
  ShadowRunResponse,
} from '../types';

interface ShadowDebugWorkbenchProps {
  projectId: string;
}

const verdictColor = (verdict: CompareVerdict): string => {
  switch (verdict) {
    case 'MATCH':
      return 'var(--success)';
    case 'STRUCTURE_DIFF':
    case 'COUNT_DIFF':
      return '#f59e0b';
    case 'ERROR_DIFF':
      return 'var(--danger)';
    default:
      return 'var(--text-muted)';
  }
};

const DEFAULT_QUERIES = `[
  { "name": "node count", "cypher": "MATCH (n) RETURN count(n)" },
  { "name": "sample", "cypher": "MATCH (n) RETURN n LIMIT 5" }
]`;

export const ShadowDebugWorkbench: React.FC<ShadowDebugWorkbenchProps> = ({
  projectId,
}) => {
  const [connections, setConnections] = useState<AuraConnection[]>([]);
  const [selectedConnId, setSelectedConnId] = useState('');
  const [queriesText, setQueriesText] = useState(DEFAULT_QUERIES);
  const [redact, setRedact] = useState(true);
  const [shadowResult, setShadowResult] = useState<ShadowRunResponse | null>(null);
  const [running, setRunning] = useState(false);
  const [shadowError, setShadowError] = useState<string | null>(null);

  const [question, setQuestion] = useState('');
  const [debugResult, setDebugResult] = useState<DebugTraceResponse | null>(null);
  const [debugging, setDebugging] = useState(false);
  const [debugError, setDebugError] = useState<string | null>(null);

  useEffect(() => {
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

  const handleShadowRun = async () => {
    if (!selectedConnId) return;
    setRunning(true);
    setShadowError(null);
    setShadowResult(null);
    try {
      const queries = JSON.parse(queriesText);
      if (!Array.isArray(queries) || queries.length === 0) {
        throw new Error('Queries must be a non-empty JSON array.');
      }
      const res = await api.runShadow(projectId, selectedConnId, queries, redact);
      setShadowResult(res);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setShadowError(err.message);
      } else {
        setShadowError('Shadow run failed');
      }
    } finally {
      setRunning(false);
    }
  };

  const handleDebug = async () => {
    if (!question.trim()) return;
    setDebugging(true);
    setDebugError(null);
    setDebugResult(null);
    try {
      const res = await api.debugTrace(projectId, question.trim());
      setDebugResult(res);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setDebugError(err.message);
      } else {
        setDebugError('Debug trace failed');
      }
    } finally {
      setDebugging(false);
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '1.25rem' }}>
      {/* Shadow testing */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.5rem' }}>
          Shadow Testing
        </h3>
        <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
          Replay a safe sample of read-only queries against the local graph and the
          Aura target, then compare structure, counts, errors, and latency. Only
          queries you submit here are replayed; parameters stay redacted by default.
        </p>

        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '0.75rem' }}>
          <select
            value={selectedConnId}
            onChange={(e) => setSelectedConnId(e.target.value)}
            style={{
              flex: 1,
              padding: '0.5rem 0.7rem',
              backgroundColor: 'var(--bg-primary)',
              border: '1px solid var(--border-color)',
              borderRadius: '6px',
              color: 'var(--text-primary)',
              fontSize: '0.85rem',
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
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: '0.3rem',
              fontSize: '0.78rem',
              color: 'var(--text-secondary)',
            }}
          >
            <input type="checkbox" checked={redact} onChange={(e) => setRedact(e.target.checked)} />
            Redact params
          </label>
          <button
            onClick={handleShadowRun}
            disabled={running || !selectedConnId}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.4rem 1.2rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              opacity: running || !selectedConnId ? 0.6 : 1,
            }}
          >
            {running ? 'Replaying...' : 'Run shadow'}
          </button>
        </div>

        <textarea
          value={queriesText}
          onChange={(e) => setQueriesText(e.target.value)}
          rows={5}
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
            marginBottom: '0.75rem',
          }}
        />

        {shadowError && (
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
            {shadowError}
          </div>
        )}

        {shadowResult && (
          <div>
            <p style={{ fontSize: '0.85rem', marginBottom: '0.5rem' }}>
              Confidence:{' '}
              <strong
                style={{
                  color:
                    shadowResult.report.summary.confidence === 'HIGH'
                      ? 'var(--success)'
                      : shadowResult.report.summary.confidence === 'MEDIUM'
                        ? '#f59e0b'
                        : 'var(--danger)',
                }}
              >
                {shadowResult.report.summary.confidence}
              </strong>{' '}
              <span style={{ color: 'var(--text-muted)', fontSize: '0.8rem' }}>
                ({shadowResult.report.summary.matched}/{shadowResult.report.summary.total} matched)
              </span>
            </p>
            {shadowResult.report.summary.confidence_reasons.map((r, i) => (
              <p key={i} style={{ fontSize: '0.78rem', color: 'var(--text-muted)', margin: 0 }}>
                · {r}
              </p>
            ))}
            <div
              style={{ display: 'flex', flexDirection: 'column', gap: '0.4rem', marginTop: '0.5rem' }}
            >
              {shadowResult.report.comparisons.map((c, i) => (
                <div
                  key={i}
                  style={{
                    padding: '0.5rem 0.7rem',
                    backgroundColor: 'var(--bg-primary)',
                    borderRadius: '6px',
                    fontSize: '0.8rem',
                  }}
                >
                  <span
                    style={{
                      fontSize: '0.7rem',
                      fontWeight: 700,
                      padding: '0.1rem 0.4rem',
                      borderRadius: '4px',
                      backgroundColor: 'rgba(255, 255, 255, 0.06)',
                      color: verdictColor(c.verdict),
                      marginRight: '0.5rem',
                    }}
                  >
                    {c.verdict}
                  </span>
                  <strong>{c.name}</strong>{' '}
                  <span style={{ color: 'var(--text-secondary)' }}>{c.detail}</span>
                  <div style={{ fontSize: '0.75rem', color: 'var(--text-muted)', marginTop: '0.2rem' }}>
                    source {c.source_rows} rows in {c.source_latency_ms.toFixed(1)}ms · target{' '}
                    {c.target_rows} rows in {c.target_latency_ms.toFixed(1)}ms · ratio{' '}
                    {c.latency_ratio.toFixed(2)}x
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Debugger */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.5rem' }}>
          Graph Debugger
        </h3>
        <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
          Trace a question through retrieval, ranking, graph evidence, ontology
          mapping, and answer generation — then read what the debugger concludes.
        </p>
        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '0.75rem' }}>
          <input
            type="text"
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleDebug();
            }}
            placeholder="Why is this question returning incorrect results?"
            style={{
              flex: 1,
              padding: '0.55rem 0.8rem',
              backgroundColor: 'var(--bg-primary)',
              border: '1px solid var(--border-color)',
              borderRadius: '6px',
              color: 'var(--text-primary)',
              fontSize: '0.85rem',
            }}
          />
          <button
            onClick={handleDebug}
            disabled={debugging}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.4rem 1.2rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              opacity: debugging ? 0.6 : 1,
            }}
          >
            {debugging ? 'Tracing...' : 'Trace'}
          </button>
        </div>

        {debugError && (
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
            {debugError}
          </div>
        )}

        {debugResult && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: '0.75rem' }}>
            <div
              style={{
                padding: '0.7rem 0.9rem',
                backgroundColor: 'var(--bg-primary)',
                borderRadius: '6px',
                fontSize: '0.8rem',
              }}
            >
              <strong>Diagnosis</strong>
              <ul style={{ margin: '0.4rem 0 0 1.1rem', padding: 0 }}>
                {debugResult.diagnosis.likely_causes.map((cause, i) => (
                  <li key={i} style={{ color: 'var(--text-primary)' }}>
                    {cause}
                  </li>
                ))}
              </ul>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '0.25rem', marginTop: '0.5rem' }}>
                {debugResult.diagnosis.rules.map((r, i) => (
                  <div key={i} style={{ fontSize: '0.78rem', color: 'var(--text-secondary)' }}>
                    <span
                      style={{
                        fontSize: '0.7rem',
                        fontWeight: 700,
                        color: r.severity === 'warning' ? '#f59e0b' : 'var(--success)',
                        marginRight: '0.4rem',
                      }}
                    >
                      {r.code}
                    </span>
                    {r.message}
                  </div>
                ))}
              </div>
              {debugResult.diagnosis.ai_hypothesis && (
                <div
                  style={{
                    marginTop: '0.5rem',
                    fontSize: '0.8rem',
                    color: 'var(--text-secondary)',
                    borderTop: '1px solid var(--border-color)',
                    paddingTop: '0.5rem',
                  }}
                >
                  <strong>AI hypothesis [{debugResult.diagnosis.ai_hypothesis.model}]:</strong>{' '}
                  {debugResult.diagnosis.ai_hypothesis.text}
                </div>
              )}
            </div>

            <div
              style={{
                padding: '0.7rem 0.9rem',
                backgroundColor: 'var(--bg-primary)',
                borderRadius: '6px',
                fontSize: '0.8rem',
              }}
            >
              <strong>
                Retrieval: {debugResult.trace.passages.length} passage(s) via{' '}
                {debugResult.trace.strategy} on {debugResult.trace.profile_name} (
                {debugResult.trace.embedding_model}, {debugResult.trace.dimensions}d)
              </strong>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '0.25rem', marginTop: '0.4rem' }}>
                {debugResult.trace.ranking.map((row) => (
                  <div key={row.chunk_id} style={{ fontSize: '0.78rem', color: 'var(--text-secondary)' }}>
                    #{row.rank} {row.filename} · fused {row.fused_score.toFixed(3)} · vector rank{' '}
                    {row.vector_rank ?? '—'} · keyword rank {row.keyword_rank ?? '—'}
                    <div style={{ color: 'var(--text-muted)', fontSize: '0.75rem' }}>
                      {row.content_snippet}
                    </div>
                  </div>
                ))}
              </div>
            </div>

            <div
              style={{
                padding: '0.7rem 0.9rem',
                backgroundColor: 'var(--bg-primary)',
                borderRadius: '6px',
                fontSize: '0.8rem',
              }}
            >
              <strong>
                Graph evidence ({debugResult.trace.ontology_approved ? 'approved' : 'no approved'}{' '}
                ontology)
              </strong>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '0.25rem', marginTop: '0.4rem' }}>
                {debugResult.trace.graph_links.map((link) => (
                  <div key={link.chunk_id} style={{ fontSize: '0.78rem', color: 'var(--text-secondary)' }}>
                    {link.filename}
                    {link.lineage_ref ? ` · ${link.lineage_ref}` : ''} →{' '}
                    {link.nodes.length === 0
                      ? link.note || 'no linked entities'
                      : link.nodes
                          .map((n) => `${n.labels.join('/') || 'Node'}:${n.uid}${n.name ? ` (${n.name})` : ''}`)
                          .join(', ')}
                    {link.relationship_types.length > 0 &&
                      ` · rels: ${link.relationship_types.join(', ')}`}
                  </div>
                ))}
                {debugResult.trace.graph_links.length === 0 && (
                  <span style={{ color: 'var(--text-muted)' }}>No passages, no graph evidence.</span>
                )}
              </div>
              {debugResult.trace.ontology_mapping.length > 0 && (
                <div style={{ marginTop: '0.4rem', fontSize: '0.78rem', color: 'var(--text-muted)' }}>
                  Ontology:{' '}
                  {debugResult.trace.ontology_mapping
                    .map((m) => `${m.label}→${m.mapped_entity || 'unmapped'}`)
                    .join(', ')}
                </div>
              )}
            </div>

            <div
              style={{
                padding: '0.7rem 0.9rem',
                backgroundColor: 'var(--bg-primary)',
                borderRadius: '6px',
                fontSize: '0.8rem',
              }}
            >
              <strong>Answer {debugResult.trace.grounded ? '(grounded)' : '(not grounded)'}</strong>{' '}
              <span style={{ color: 'var(--text-muted)' }}>
                · {debugResult.trace.citations.length} citation(s)
                {debugResult.trace.invalid_markers > 0 &&
                  ` · ${debugResult.trace.invalid_markers} invalid marker(s)`}{' '}
                · {debugResult.trace.evidence_chars} evidence chars · {debugResult.trace.chat_model} ·{' '}
                {debugResult.trace.latency_ms.toFixed(0)}ms
              </span>
              <p style={{ whiteSpace: 'pre-wrap', marginTop: '0.4rem' }}>{debugResult.trace.answer}</p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
