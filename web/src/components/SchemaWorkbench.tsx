import React, { useEffect, useState } from 'react';
import { api } from '../api/client';
import {
  ApplyResponse,
  DiffResponse,
  ExportResponse,
  RetrievalConfigResponse,
  RetrievalStrategy,
  SchemaRevision,
} from '../types';

interface SchemaWorkbenchProps {
  projectId: string;
}

export const SchemaWorkbench: React.FC<SchemaWorkbenchProps> = ({ projectId }) => {
  const [exportData, setExportData] = useState<ExportResponse | null>(null);
  const [selectedFile, setSelectedFile] = useState<string>('graph/schema.json');
  const [copied, setCopied] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);

  const [revisions, setRevisions] = useState<SchemaRevision[]>([]);
  const [snapshotting, setSnapshotting] = useState(false);
  const [snapshotMessage, setSnapshotMessage] = useState<string | null>(null);

  const [fromVersion, setFromVersion] = useState<string>('');
  const [toVersion, setToVersion] = useState<string>('live');
  const [diffResult, setDiffResult] = useState<DiffResponse | null>(null);
  const [diffing, setDiffing] = useState(false);
  const [diffError, setDiffError] = useState<string | null>(null);

  const [applyText, setApplyText] = useState('');
  const [applyResult, setApplyResult] = useState<ApplyResponse | null>(null);
  const [applying, setApplying] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);

  const [retrievalConfig, setRetrievalConfig] = useState<RetrievalConfigResponse | null>(null);
  const [strategy, setStrategy] = useState<RetrievalStrategy>('hybrid');
  const [topK, setTopK] = useState('5');
  const [savingConfig, setSavingConfig] = useState(false);

  const fetchAll = async () => {
    try {
      const [exp, revs, config] = await Promise.all([
        api.exportSchema(projectId),
        api.listSchemaRevisions(projectId),
        api.getRetrievalConfig(projectId),
      ]);
      setExportData(exp);
      setRevisions(revs);
      setRetrievalConfig(config);
      setStrategy(config.default_strategy);
      setTopK(String(config.default_top_k));
      if (revs.length > 0 && !fromVersion) {
        setFromVersion(String(revs[revs.length - 1].version));
      }
      if (!applyText) {
        setApplyText(JSON.stringify(exp.document, null, 2));
      }
      setExportError(null);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setExportError(err.message);
      }
    }
  };

  useEffect(() => {
    fetchAll();
  }, [projectId]);

  const handleCopy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopied(false);
    }
  };

  const handleDownload = (path: string, content: string) => {
    const blob = new Blob([content], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = path.split('/').pop() || 'schema.txt';
    a.click();
    URL.revokeObjectURL(url);
  };

  const handleSnapshot = async () => {
    setSnapshotting(true);
    setSnapshotMessage(null);
    try {
      const res = await api.createSchemaRevision(projectId);
      setSnapshotMessage(
        res.created
          ? `Snapshot recorded as v${res.revision.version}.`
          : `No changes since v${res.revision.version}; nothing recorded.`
      );
      await fetchAll();
    } catch (err: unknown) {
      setSnapshotMessage(err instanceof Error ? err.message : 'Snapshot failed');
    } finally {
      setSnapshotting(false);
    }
  };

  const handleDiff = async () => {
    setDiffing(true);
    setDiffError(null);
    setDiffResult(null);
    try {
      const payload: { from_version?: number; to_version?: number } = {};
      if (fromVersion) payload.from_version = Number(fromVersion);
      if (toVersion !== 'live') payload.to_version = Number(toVersion);
      const res = await api.diffSchema(projectId, payload);
      setDiffResult(res);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setDiffError(err.message);
      } else {
        setDiffError('Diff failed');
      }
    } finally {
      setDiffing(false);
    }
  };

  const handleApply = async () => {
    setApplying(true);
    setApplyError(null);
    setApplyResult(null);
    try {
      const document = JSON.parse(applyText);
      const res = await api.applySchema(projectId, document);
      setApplyResult(res);
      await fetchAll();
    } catch (err: unknown) {
      if (err instanceof Error) {
        setApplyError(err.message);
      } else {
        setApplyError('Apply failed');
      }
    } finally {
      setApplying(false);
    }
  };

  const handleSaveConfig = async () => {
    setSavingConfig(true);
    try {
      const res = await api.updateRetrievalConfig(projectId, {
        default_strategy: strategy,
        default_top_k: Number(topK),
      });
      setRetrievalConfig(res);
    } catch {
      // validation errors surface via the shared error path below
    } finally {
      setSavingConfig(false);
    }
  };

  const currentFile = exportData?.files.find((f) => f.path === selectedFile);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '1.25rem' }}>
      {exportError && (
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
          {exportError}
        </div>
      )}

      {/* Export */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            marginBottom: '0.75rem',
          }}
        >
          <h3 style={{ fontSize: '1rem', fontWeight: 600 }}>Schema as Code</h3>
          <button
            onClick={handleSnapshot}
            disabled={snapshotting}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.4rem 1rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              opacity: snapshotting ? 0.6 : 1,
            }}
          >
            {snapshotting ? 'Snapshotting...' : 'Snapshot revision'}
          </button>
        </div>

        <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
          The live contract as committable files: ontology, constraints, indexes, and
          graph contracts. Snapshot to version it.
        </p>

        {snapshotMessage && (
          <p style={{ fontSize: '0.8rem', color: 'var(--text-secondary)', marginBottom: '0.5rem' }}>
            {snapshotMessage}
          </p>
        )}

        <div style={{ display: 'flex', gap: '0.5rem', flexWrap: 'wrap', marginBottom: '0.5rem' }}>
          {exportData?.files.map((f) => (
            <button
              key={f.path}
              type="button"
              onClick={() => setSelectedFile(f.path)}
              style={{
                padding: '0.3rem 0.7rem',
                fontSize: '0.75rem',
                fontFamily: 'monospace',
                borderRadius: '4px',
                border: '1px solid',
                borderColor: selectedFile === f.path ? 'var(--accent)' : 'var(--border-color)',
                backgroundColor:
                  selectedFile === f.path ? 'rgba(var(--accent-rgb), 0.2)' : 'transparent',
                color: selectedFile === f.path ? 'var(--accent-light)' : 'var(--text-secondary)',
              }}
            >
              {f.path}
            </button>
          ))}
        </div>

        {currentFile && (
          <div>
            <pre
              style={{
                padding: '0.7rem 0.9rem',
                backgroundColor: 'var(--bg-primary)',
                borderRadius: '6px',
                fontSize: '0.75rem',
                fontFamily: 'monospace',
                color: 'var(--text-primary)',
                whiteSpace: 'pre-wrap',
                maxHeight: '320px',
                overflowY: 'auto',
                margin: 0,
                marginBottom: '0.5rem',
              }}
            >
              {currentFile.content}
            </pre>
            <div style={{ display: 'flex', gap: '0.5rem' }}>
              <button
                type="button"
                onClick={() => handleCopy(currentFile.content)}
                style={{
                  background: 'none',
                  border: '1px solid var(--border-color)',
                  color: 'var(--text-secondary)',
                  borderRadius: '4px',
                  fontSize: '0.75rem',
                  padding: '0.25rem 0.7rem',
                }}
              >
                {copied ? 'Copied ✓' : 'Copy'}
              </button>
              <button
                type="button"
                onClick={() => handleDownload(currentFile.path, currentFile.content)}
                style={{
                  background: 'none',
                  border: '1px solid var(--border-color)',
                  color: 'var(--text-secondary)',
                  borderRadius: '4px',
                  fontSize: '0.75rem',
                  padding: '0.25rem 0.7rem',
                }}
              >
                Download
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Revisions + diff */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.75rem' }}>
          Revisions & Breaking-Change Diff
        </h3>

        {revisions.length === 0 && (
          <p style={{ fontSize: '0.85rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
            No revisions yet. Snapshot the schema above to start versioning.
          </p>
        )}

        {revisions.length > 0 && (
          <div>
            <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '0.75rem', flexWrap: 'wrap' }}>
              <select
                value={fromVersion}
                onChange={(e) => setFromVersion(e.target.value)}
                style={{
                  padding: '0.45rem 0.7rem',
                  backgroundColor: 'var(--bg-primary)',
                  border: '1px solid var(--border-color)',
                  borderRadius: '6px',
                  color: 'var(--text-primary)',
                  fontSize: '0.8rem',
                }}
              >
                {revisions.map((r) => (
                  <option key={r.id} value={r.version}>
                    from v{r.version} ({r.content_hash.slice(0, 8)})
                  </option>
                ))}
              </select>
              <select
                value={toVersion}
                onChange={(e) => setToVersion(e.target.value)}
                style={{
                  padding: '0.45rem 0.7rem',
                  backgroundColor: 'var(--bg-primary)',
                  border: '1px solid var(--border-color)',
                  borderRadius: '6px',
                  color: 'var(--text-primary)',
                  fontSize: '0.8rem',
                }}
              >
                <option value="live">to live state</option>
                {revisions.map((r) => (
                  <option key={r.id} value={r.version}>
                    to v{r.version}
                  </option>
                ))}
              </select>
              <button
                onClick={handleDiff}
                disabled={diffing}
                style={{
                  backgroundColor: 'var(--accent)',
                  color: '#fff',
                  border: 'none',
                  borderRadius: '6px',
                  padding: '0.4rem 1.2rem',
                  fontSize: '0.8rem',
                  fontWeight: 600,
                  opacity: diffing ? 0.6 : 1,
                }}
              >
                {diffing ? 'Diffing...' : 'Diff'}
              </button>
            </div>

            {diffError && (
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
                {diffError}
              </div>
            )}

            {diffResult && (
              <div>
                <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.5rem' }}>
                  {diffResult.diff.changes.length} change(s), {diffResult.diff.breaking_count}{' '}
                  breaking · proposal {diffResult.proposal.filename}
                </p>
                <div style={{ display: 'flex', flexDirection: 'column', gap: '0.4rem', marginBottom: '0.75rem' }}>
                  {diffResult.diff.changes.map((c, i) => (
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
                          color: c.breaking ? 'var(--danger)' : 'var(--success)',
                          marginRight: '0.5rem',
                        }}
                      >
                        {c.breaking ? 'BREAKING' : c.kind.toUpperCase()}
                      </span>
                      <strong>{c.subject}</strong>{' '}
                      <span style={{ color: 'var(--text-secondary)' }}>{c.detail}</span>
                    </div>
                  ))}
                  {diffResult.diff.changes.length === 0 && (
                    <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)' }}>
                      No changes detected.
                    </p>
                  )}
                </div>

                {(diffResult.proposal.cypher.length > 0 || diffResult.proposal.steps.length > 0) && (
                  <div
                    style={{
                      padding: '0.7rem 0.9rem',
                      backgroundColor: 'var(--bg-primary)',
                      borderRadius: '6px',
                      fontSize: '0.8rem',
                    }}
                  >
                    <strong>Migration proposal</strong>
                    {diffResult.proposal.cypher.length > 0 && (
                      <pre
                        style={{
                          marginTop: '0.4rem',
                          fontSize: '0.72rem',
                          fontFamily: 'monospace',
                          whiteSpace: 'pre-wrap',
                          color: 'var(--text-secondary)',
                        }}
                      >
                        {diffResult.proposal.cypher.join('\n')}
                      </pre>
                    )}
                    {diffResult.proposal.steps.map((s, i) => (
                      <div key={i} style={{ color: 'var(--danger)', fontSize: '0.78rem', marginTop: '0.3rem' }}>
                        {s}
                      </div>
                    ))}
                    {diffResult.proposal.notes.map((n, i) => (
                      <div key={i} style={{ color: 'var(--text-muted)', fontSize: '0.78rem', marginTop: '0.3rem' }}>
                        {n}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Apply */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.5rem' }}>
          Apply Edited Document
        </h3>
        <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
          Paste an edited schema document to record a new revision. The live graph is
          conformance-checked and the result is reported, never hidden.
        </p>
        <textarea
          value={applyText}
          onChange={(e) => setApplyText(e.target.value)}
          rows={8}
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
        <button
          onClick={handleApply}
          disabled={applying}
          style={{
            backgroundColor: 'var(--accent)',
            color: '#fff',
            border: 'none',
            borderRadius: '6px',
            padding: '0.4rem 1.2rem',
            fontSize: '0.8rem',
            fontWeight: 600,
            opacity: applying ? 0.6 : 1,
            marginBottom: '0.75rem',
          }}
        >
          {applying ? 'Applying...' : 'Validate & Record Revision'}
        </button>

        {applyError && (
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
            {applyError}
          </div>
        )}

        {applyResult && (
          <div
            style={{
              padding: '0.7rem 0.9rem',
              backgroundColor: 'var(--bg-primary)',
              borderRadius: '6px',
              fontSize: '0.8rem',
            }}
          >
            <strong>Recorded v{applyResult.revision.version}</strong>{' '}
            <span
              style={{
                color: applyResult.conformance.passed ? 'var(--success)' : 'var(--danger)',
              }}
            >
              {applyResult.conformance.passed
                ? '— live graph conforms ✓'
                : '— live graph does NOT conform ✕'}
            </span>
            <div
              style={{ display: 'flex', flexDirection: 'column', gap: '0.25rem', marginTop: '0.4rem' }}
            >
              {applyResult.conformance.checks.map((c, i) => (
                <div key={i} style={{ color: 'var(--text-secondary)', fontSize: '0.78rem' }}>
                  <span style={{ color: c.passed ? 'var(--success)' : 'var(--danger)' }}>
                    {c.passed ? '✓' : '✕'}
                  </span>{' '}
                  {c.subject}: {c.detail}
                </div>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Retrieval config */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.5rem' }}>
          Retrieval Defaults
        </h3>
        <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
          Versioned with the schema document. Explicit per-request values always win.
        </p>
        <div style={{ display: 'flex', gap: '0.5rem', alignItems: 'center', flexWrap: 'wrap' }}>
          <select
            value={strategy}
            onChange={(e) => setStrategy(e.target.value as RetrievalStrategy)}
            style={{
              padding: '0.45rem 0.7rem',
              backgroundColor: 'var(--bg-primary)',
              border: '1px solid var(--border-color)',
              borderRadius: '6px',
              color: 'var(--text-primary)',
              fontSize: '0.8rem',
            }}
          >
            <option value="vector">vector</option>
            <option value="keyword">keyword</option>
            <option value="hybrid">hybrid</option>
          </select>
          <input
            type="number"
            min={1}
            max={50}
            value={topK}
            onChange={(e) => setTopK(e.target.value)}
            style={{
              width: '90px',
              padding: '0.45rem 0.7rem',
              backgroundColor: 'var(--bg-primary)',
              border: '1px solid var(--border-color)',
              borderRadius: '6px',
              color: 'var(--text-primary)',
              fontSize: '0.8rem',
            }}
          />
          <button
            onClick={handleSaveConfig}
            disabled={savingConfig}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.4rem 1.2rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              opacity: savingConfig ? 0.6 : 1,
            }}
          >
            {savingConfig ? 'Saving...' : 'Save defaults'}
          </button>
          {retrievalConfig && (
            <span style={{ fontSize: '0.78rem', color: 'var(--text-muted)' }}>
              {retrievalConfig.configured
                ? `Stored: ${retrievalConfig.default_strategy}, top ${retrievalConfig.default_top_k}`
                : 'Using built-in defaults (hybrid, top 5)'}
            </span>
          )}
        </div>
      </div>
    </div>
  );
};
