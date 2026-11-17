import React, { useEffect, useState } from 'react';
import { api } from '../api/client';
import {
  AuraConnection,
  PortableReport,
  PortVerdict,
  RunTestsResponse,
} from '../types';

interface PortabilityWorkbenchProps {
  projectId: string;
}

const verdictColor = (verdict: PortVerdict): string => {
  switch (verdict) {
    case 'PORTABLE':
      return 'var(--success)';
    case 'TRANSFORM':
      return 'var(--accent-light)';
    case 'NEEDS_REVIEW':
      return '#f59e0b';
    case 'NEEDS_REWRITE':
      return 'var(--danger)';
    default:
      return 'var(--text-muted)';
  }
};

export const PortabilityWorkbench: React.FC<PortabilityWorkbenchProps> = ({
  projectId,
}) => {
  const [repoPath, setRepoPath] = useState('');
  const [report, setReport] = useState<PortableReport | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [verdictFilter, setVerdictFilter] = useState<'all' | PortVerdict>('all');

  const [connections, setConnections] = useState<AuraConnection[]>([]);
  const [selectedConnId, setSelectedConnId] = useState('');
  const [testResults, setTestResults] = useState<RunTestsResponse | null>(null);
  const [running, setRunning] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);

  useEffect(() => {
    api
      .listAuraConnections(projectId)
      .then((list) => {
        setConnections(list);
        if (list.length > 0) setSelectedConnId(list[0].id);
      })
      .catch(() => {
        // connections panel stays empty
      });
  }, [projectId]);

  const handleScan = async () => {
    if (!repoPath.trim()) return;
    setScanning(true);
    setScanError(null);
    setReport(null);
    setTestResults(null);
    try {
      const res = await api.scanRepo(projectId, repoPath.trim());
      setReport(res);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setScanError(err.message);
      } else {
        setScanError('Repository scan failed');
      }
    } finally {
      setScanning(false);
    }
  };

  const handleRunTests = async () => {
    if (!report || !selectedConnId) return;
    setRunning(true);
    setRunError(null);
    setTestResults(null);
    try {
      const res = await api.runPortabilityTests(projectId, selectedConnId, report.tests);
      setTestResults(res);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setRunError(err.message);
      } else {
        setRunError('Test run failed');
      }
    } finally {
      setRunning(false);
    }
  };

  const visibleFindings =
    verdictFilter === 'all'
      ? report?.findings || []
      : (report?.findings || []).filter((f) => f.verdict === verdictFilter);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '1.25rem' }}>
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.5rem' }}>
          Repository Scan
        </h3>
        <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
          Point at an application checkout readable by the server. The compiler finds
          embedded Cypher, drivers, connection strings, transactions, APOC usage, and
          inline DDL — then classifies each for Aura portability.
        </p>
        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '0.75rem' }}>
          <input
            type="text"
            value={repoPath}
            onChange={(e) => setRepoPath(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleScan();
            }}
            placeholder="/path/to/application/repo"
            style={{
              flex: 1,
              padding: '0.55rem 0.8rem',
              backgroundColor: 'var(--bg-primary)',
              border: '1px solid var(--border-color)',
              borderRadius: '6px',
              color: 'var(--text-primary)',
              fontSize: '0.85rem',
              fontFamily: 'monospace',
            }}
          />
          <button
            onClick={handleScan}
            disabled={scanning}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.4rem 1.2rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              opacity: scanning ? 0.6 : 1,
            }}
          >
            {scanning ? 'Scanning...' : 'Scan'}
          </button>
        </div>

        {scanError && (
          <div
            style={{
              padding: '0.6rem 0.8rem',
              backgroundColor: 'rgba(239, 68, 68, 0.1)',
              border: '1px solid var(--danger)',
              color: 'var(--danger)',
              borderRadius: '6px',
              fontSize: '0.8rem',
            }}
          >
            {scanError}
          </div>
        )}

        {report && (
          <div>
            <div
              style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(6, 1fr)',
                gap: '0.5rem',
                marginBottom: '0.75rem',
              }}
            >
              {[
                { label: 'FILES', value: String(report.summary.files_scanned) },
                { label: 'QUERIES', value: String(report.summary.queries_total) },
                { label: 'PORTABLE', value: String(report.summary.portable) },
                { label: 'TRANSFORM', value: String(report.summary.transform) },
                { label: 'REWRITE', value: String(report.summary.needs_rewrite) },
                { label: 'REVIEW', value: String(report.summary.needs_review) },
              ].map((m) => (
                <div
                  key={m.label}
                  style={{
                    padding: '0.6rem',
                    backgroundColor: 'var(--bg-primary)',
                    borderRadius: '6px',
                    textAlign: 'center',
                  }}
                >
                  <span style={{ color: 'var(--text-muted)', display: 'block', fontSize: '0.7rem' }}>
                    {m.label}
                  </span>
                  <span style={{ fontWeight: 700, fontSize: '1rem' }}>{m.value}</span>
                </div>
              ))}
            </div>

            <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '0.5rem', flexWrap: 'wrap' }}>
              {(['all', 'PORTABLE', 'TRANSFORM', 'NEEDS_REWRITE', 'NEEDS_REVIEW'] as const).map(
                (v) => (
                  <button
                    key={v}
                    type="button"
                    onClick={() => setVerdictFilter(v)}
                    style={{
                      padding: '0.25rem 0.7rem',
                      fontSize: '0.72rem',
                      fontWeight: 600,
                      borderRadius: '4px',
                      border: '1px solid',
                      borderColor: verdictFilter === v ? 'var(--accent)' : 'var(--border-color)',
                      backgroundColor:
                        verdictFilter === v ? 'rgba(99, 102, 241, 0.2)' : 'transparent',
                      color:
                        verdictFilter === v ? 'var(--accent-light)' : 'var(--text-secondary)',
                    }}
                  >
                    {v === 'all' ? 'ALL' : v}
                  </button>
                )
              )}
            </div>

            <div style={{ display: 'flex', flexDirection: 'column', gap: '0.4rem', maxHeight: '420px', overflowY: 'auto' }}>
              {visibleFindings.map((f, i) => (
                <div
                  key={i}
                  style={{
                    padding: '0.5rem 0.7rem',
                    backgroundColor: 'var(--bg-primary)',
                    borderRadius: '6px',
                    fontSize: '0.8rem',
                  }}
                >
                  <div style={{ display: 'flex', gap: '0.5rem', alignItems: 'center', marginBottom: '0.2rem', flexWrap: 'wrap' }}>
                    <span
                      style={{
                        fontSize: '0.7rem',
                        fontWeight: 700,
                        padding: '0.1rem 0.4rem',
                        borderRadius: '4px',
                        backgroundColor: 'rgba(255, 255, 255, 0.06)',
                        color: verdictColor(f.verdict),
                      }}
                    >
                      {f.verdict}
                    </span>
                    <span style={{ fontFamily: 'monospace', color: 'var(--text-muted)', fontSize: '0.75rem' }}>
                      {f.file}:{f.line}
                    </span>
                    <span style={{ color: 'var(--text-muted)', fontSize: '0.75rem' }}>{f.rule_id}</span>
                  </div>
                  <div style={{ color: 'var(--text-primary)' }}>{f.message}</div>
                  {f.remediation && (
                    <pre
                      style={{
                        marginTop: '0.3rem',
                        fontSize: '0.72rem',
                        fontFamily: 'monospace',
                        whiteSpace: 'pre-wrap',
                        color: 'var(--accent-light)',
                      }}
                    >
                      {f.remediation}
                    </pre>
                  )}
                </div>
              ))}
              {visibleFindings.length === 0 && (
                <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)' }}>
                  No findings under this filter.
                </p>
              )}
            </div>
          </div>
        )}
      </div>

      {report && report.tests.length > 0 && (
        <div
          style={{
            backgroundColor: 'var(--bg-secondary)',
            borderRadius: '8px',
            border: '1px solid var(--border-color)',
            padding: '1.25rem',
          }}
        >
          <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.5rem' }}>
            Generated Target Tests ({report.tests.length}
            {report.tests_truncated ? ', truncated' : ''})
          </h3>
          <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
            Read queries run as-is; write-shaped queries run behind EXPLAIN so validation
            never mutates the target.
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
            <button
              onClick={handleRunTests}
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
              {running ? 'Running...' : 'Run on target'}
            </button>
          </div>

          {runError && (
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
              {runError}
            </div>
          )}

          {testResults && (
            <div style={{ marginBottom: '0.75rem' }}>
              <p style={{ fontSize: '0.85rem', marginBottom: '0.5rem' }}>
                <strong style={{ color: 'var(--success)' }}>{testResults.passed} passed</strong>{' '}
                <strong style={{ color: testResults.failed > 0 ? 'var(--danger)' : 'var(--text-muted)' }}>
                  {testResults.failed} failed
                </strong>
              </p>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '0.3rem' }}>
                {testResults.results.map((r, i) => (
                  <div
                    key={i}
                    style={{
                      padding: '0.4rem 0.7rem',
                      backgroundColor: 'var(--bg-primary)',
                      borderRadius: '6px',
                      fontSize: '0.78rem',
                      color: 'var(--text-secondary)',
                    }}
                  >
                    <span style={{ color: r.passed ? 'var(--success)' : 'var(--danger)' }}>
                      {r.passed ? '✓' : '✕'}
                    </span>{' '}
                    {r.name} — {r.detail} ({r.row_count} rows, {r.elapsed_ms.toFixed(0)}ms)
                  </div>
                ))}
              </div>
            </div>
          )}

          <div style={{ display: 'flex', flexDirection: 'column', gap: '0.3rem' }}>
            {report.tests.slice(0, 20).map((t, i) => (
              <div
                key={i}
                style={{
                  padding: '0.4rem 0.7rem',
                  backgroundColor: 'var(--bg-primary)',
                  borderRadius: '6px',
                  fontSize: '0.75rem',
                  color: 'var(--text-muted)',
                  fontFamily: 'monospace',
                }}
              >
                {t.read_only ? '[read]' : '[explain]'} {t.name}
              </div>
            ))}
            {report.tests.length > 20 && (
              <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)' }}>
                …and {report.tests.length - 20} more
              </span>
            )}
          </div>
        </div>
      )}
    </div>
  );
};
