import React, { useEffect, useState } from 'react';
import { api } from '../api/client';
import {
  AnalyzeResponse,
  AuraConnection,
  CapabilityVerdict,
  CompatFinding,
  ConnectionTestReport,
  DryRunReport,
  Migration,
  VerificationReport,
} from '../types';

interface AuraMigrationProps {
  projectId: string;
}

const verdictColor = (verdict: CapabilityVerdict): string => {
  switch (verdict) {
    case 'SUPPORTED':
      return 'var(--success)';
    case 'RECREATE':
    case 'TRANSFORM':
      return 'var(--accent-light)';
    case 'REQUIRES_MANUAL_REVIEW':
      return 'var(--warning)';
    case 'UNSUPPORTED':
      return 'var(--danger)';
    default:
      return 'var(--text-muted)';
  }
};

const FindingTable: React.FC<{ title: string; findings: CompatFinding[] }> = ({
  title,
  findings,
}) => {
  if (findings.length === 0) return null;
  return (
    <div style={{ marginBottom: '0.75rem' }}>
      <span
        style={{
          fontSize: '0.75rem',
          color: 'var(--text-muted)',
          display: 'block',
          marginBottom: '0.4rem',
        }}
      >
        {title}
      </span>
      <div style={{ display: 'flex', flexDirection: 'column', gap: '0.4rem' }}>
        {findings.map((f, idx) => (
          <div
            key={idx}
            style={{
              padding: '0.5rem 0.7rem',
              backgroundColor: 'var(--bg-primary)',
              borderRadius: '6px',
              fontSize: '0.8rem',
            }}
          >
            <div style={{ display: 'flex', gap: '0.5rem', alignItems: 'center', marginBottom: '0.2rem' }}>
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
              <span style={{ color: 'var(--text-muted)' }}>{f.category}</span>
              <strong>{f.subject}</strong>
            </div>
            <div style={{ color: 'var(--text-secondary)' }}>{f.detail}</div>
            {f.remediation && (
              <div style={{ color: 'var(--text-muted)', fontSize: '0.75rem' }}>
                → {f.remediation}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
};

export const AuraMigration: React.FC<AuraMigrationProps> = ({ projectId }) => {
  const [connections, setConnections] = useState<AuraConnection[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [formName, setFormName] = useState('');
  const [formUri, setFormUri] = useState('neo4j+s://');
  const [formUser, setFormUser] = useState('neo4j');
  const [formDb, setFormDb] = useState('neo4j');
  const [formEnv, setFormEnv] = useState('AURA_PASSWORD');
  const [connError, setConnError] = useState<string | null>(null);
  const [testReports, setTestReports] = useState<Record<string, ConnectionTestReport>>({});
  const [testingId, setTestingId] = useState<string | null>(null);

  const [selectedConnId, setSelectedConnId] = useState<string>('');
  const [auraTier, setAuraTier] = useState<string>('free');
  const [analysis, setAnalysis] = useState<AnalyzeResponse | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [analysisError, setAnalysisError] = useState<string | null>(null);

  const [migrations, setMigrations] = useState<Migration[]>([]);
  const [selectedMigration, setSelectedMigration] = useState<Migration | null>(null);
  const [dryRun, setDryRun] = useState<DryRunReport | null>(null);
  const [verification, setVerification] = useState<VerificationReport | null>(null);
  const [working, setWorking] = useState(false);
  const [migrationError, setMigrationError] = useState<string | null>(null);

  // The server exposes the `.env` Aura connection (if configured) as a
  // list entry named like "aura (env)". Its URI/username/database are
  // known-good, so the new-connection form starts from them instead of
  // the `neo4j`/`neo4j` guesses (Aura instances often use the instance
  // id for both, and the guesses fail with an auth error).
  const envConn = connections.find((c) => c.name.endsWith('(env)'));

  const prefillFromEnv = (conn: AuraConnection) => {
    setFormUri(conn.uri);
    setFormUser(conn.username);
    setFormDb(conn.database ?? 'neo4j');
    setFormEnv(conn.auth_env_var);
  };

  const toggleForm = () => {
    if (!showForm && envConn) {
      prefillFromEnv(envConn);
    }
    setShowForm(!showForm);
  };

  const fetchConnections = async () => {
    try {
      const list = await api.listAuraConnections(projectId);
      setConnections(list);
      if (list.length > 0 && !selectedConnId) {
        setSelectedConnId(list[0].id);
      }
    } catch (err: unknown) {
      if (err instanceof Error) {
        setConnError(err.message);
      }
    }
  };

  const fetchMigrations = async () => {
    try {
      const list = await api.listMigrations(projectId);
      setMigrations(list);
    } catch {
      // optional panel; ignore
    }
  };

  useEffect(() => {
    fetchConnections();
    fetchMigrations();
  }, [projectId]);

  const handleCreateConnection = async (e: React.FormEvent) => {
    e.preventDefault();
    setConnError(null);
    try {
      const conn = await api.createAuraConnection(projectId, {
        name: formName.trim(),
        uri: formUri.trim(),
        username: formUser.trim() || undefined,
        database: formDb.trim() || undefined,
        auth_env_var: formEnv.trim() || undefined,
      });
      setConnections([...connections, conn]);
      setSelectedConnId(conn.id);
      setShowForm(false);
      setFormName('');
      setFormUri('neo4j+s://');
    } catch (err: unknown) {
      if (err instanceof Error) {
        setConnError(err.message);
      } else {
        setConnError('Failed to create connection');
      }
    }
  };

  const handleTestConnection = async (id: string) => {
    setTestingId(id);
    try {
      const report = await api.testAuraConnection(projectId, id);
      setTestReports({ ...testReports, [id]: report });
    } catch (err: unknown) {
      setTestReports({
        ...testReports,
        [id]: {
          reachable: false,
          message: err instanceof Error ? err.message : 'Connection test failed',
        },
      });
    } finally {
      setTestingId(null);
    }
  };

  const handleDeleteConnection = async (id: string) => {
    try {
      await api.deleteAuraConnection(projectId, id);
      setConnections(connections.filter((c) => c.id !== id));
      if (selectedConnId === id) {
        setSelectedConnId('');
      }
    } catch (err: unknown) {
      if (err instanceof Error) {
        setConnError(err.message);
      }
    }
  };

  const handleAnalyze = async () => {
    setAnalyzing(true);
    setAnalysisError(null);
    setAnalysis(null);
    try {
      const res = await api.analyzeMigration(projectId, selectedConnId || undefined, auraTier || undefined);
      setAnalysis(res);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setAnalysisError(err.message);
      } else {
        setAnalysisError('Analysis failed');
      }
    } finally {
      setAnalyzing(false);
    }
  };

  const handlePlan = async () => {
    if (!selectedConnId) return;
    setWorking(true);
    setMigrationError(null);
    setDryRun(null);
    setVerification(null);
    try {
      const res = await api.planMigration(projectId, selectedConnId);
      setSelectedMigration(res.migration);
      setDryRun(res.dry_run);
      await fetchMigrations();
    } catch (err: unknown) {
      if (err instanceof Error) {
        setMigrationError(err.message);
      } else {
        setMigrationError('Planning failed');
      }
    } finally {
      setWorking(false);
    }
  };

  const refreshMigration = async (id: string) => {
    const m = await api.getMigration(projectId, id);
    setSelectedMigration(m);
    await fetchMigrations();
    return m;
  };

  const handleDryRun = async () => {
    if (!selectedMigration) return;
    setWorking(true);
    setMigrationError(null);
    try {
      const report = await api.dryRunMigration(projectId, selectedMigration.id);
      setDryRun(report);
      await refreshMigration(selectedMigration.id);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setMigrationError(err.message);
      }
    } finally {
      setWorking(false);
    }
  };

  const handleApprove = async (approved: boolean) => {
    if (!selectedMigration) return;
    setWorking(true);
    setMigrationError(null);
    try {
      const m = await api.approveMigration(projectId, selectedMigration.id, approved);
      setSelectedMigration(m);
      await fetchMigrations();
    } catch (err: unknown) {
      if (err instanceof Error) {
        setMigrationError(err.message);
      }
    } finally {
      setWorking(false);
    }
  };

  const handleExecute = async () => {
    if (!selectedMigration) return;
    setWorking(true);
    setMigrationError(null);
    try {
      const m = await api.executeMigration(projectId, selectedMigration.id);
      setSelectedMigration(m);
      await fetchMigrations();
    } catch (err: unknown) {
      if (err instanceof Error) {
        setMigrationError(err.message);
      }
    } finally {
      setWorking(false);
    }
  };

  const handleVerify = async () => {
    if (!selectedMigration) return;
    setWorking(true);
    setMigrationError(null);
    try {
      const report = await api.verifyMigration(projectId, selectedMigration.id);
      setVerification(report);
      await refreshMigration(selectedMigration.id);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setMigrationError(err.message);
      }
    } finally {
      setWorking(false);
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '1.25rem' }}>
      {/* Connections */}
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
          <h3 style={{ fontSize: '1rem', fontWeight: 600 }}>Aura Connections</h3>
          <button
            onClick={toggleForm}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.4rem 1rem',
              fontSize: '0.8rem',
              fontWeight: 600,
            }}
          >
            {showForm ? 'Cancel' : '+ New Connection'}
          </button>
        </div>

        <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
          Passwords are never stored: each connection names an environment variable that
          must hold the password when the server runs (default{' '}
          <code style={{ fontFamily: 'monospace' }}>AURA_PASSWORD</code>). Tip: on Aura the
          username and database are often the instance id (e.g.{' '}
          <code style={{ fontFamily: 'monospace' }}>6325161a</code>), not{' '}
          <code style={{ fontFamily: 'monospace' }}>neo4j</code> — copy them from the Aura
          console, or use the <code style={{ fontFamily: 'monospace' }}>aura (env)</code>{' '}
          connection below, which is prefilled from your{' '}
          <code style={{ fontFamily: 'monospace' }}>.env</code>.
        </p>

        {connError && (
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
            {connError}
          </div>
        )}

        {showForm && (
          <form onSubmit={handleCreateConnection} style={{ marginBottom: '1rem' }}>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '0.5rem', marginBottom: '0.5rem' }}>
              <input
                type="text"
                value={formName}
                onChange={(e) => setFormName(e.target.value)}
                placeholder="Name (e.g. prod)"
                required
                style={{
                  padding: '0.5rem 0.7rem',
                  backgroundColor: 'var(--bg-primary)',
                  border: '1px solid var(--border-color)',
                  borderRadius: '6px',
                  color: 'var(--text-primary)',
                  fontSize: '0.85rem',
                }}
              />
              <input
                type="text"
                value={formUri}
                onChange={(e) => setFormUri(e.target.value)}
                placeholder="neo4j+s://xxxx.databases.neo4j.io"
                required
                style={{
                  padding: '0.5rem 0.7rem',
                  backgroundColor: 'var(--bg-primary)',
                  border: '1px solid var(--border-color)',
                  borderRadius: '6px',
                  color: 'var(--text-primary)',
                  fontSize: '0.85rem',
                }}
              />
              <input
                type="text"
                value={formUser}
                onChange={(e) => setFormUser(e.target.value)}
                placeholder="Username"
                style={{
                  padding: '0.5rem 0.7rem',
                  backgroundColor: 'var(--bg-primary)',
                  border: '1px solid var(--border-color)',
                  borderRadius: '6px',
                  color: 'var(--text-primary)',
                  fontSize: '0.85rem',
                }}
              />
              <input
                type="text"
                value={formDb}
                onChange={(e) => setFormDb(e.target.value)}
                placeholder="Database (default neo4j)"
                style={{
                  padding: '0.5rem 0.7rem',
                  backgroundColor: 'var(--bg-primary)',
                  border: '1px solid var(--border-color)',
                  borderRadius: '6px',
                  color: 'var(--text-primary)',
                  fontSize: '0.85rem',
                }}
              />
              <input
                type="text"
                value={formEnv}
                onChange={(e) => setFormEnv(e.target.value)}
                placeholder="Password env var"
                style={{
                  padding: '0.5rem 0.7rem',
                  backgroundColor: 'var(--bg-primary)',
                  border: '1px solid var(--border-color)',
                  borderRadius: '6px',
                  color: 'var(--text-primary)',
                  fontSize: '0.85rem',
                  gridColumn: '1 / -1',
                }}
              />
            </div>
            <button
              type="submit"
              style={{
                backgroundColor: 'var(--accent)',
                color: '#fff',
                border: 'none',
                borderRadius: '6px',
                padding: '0.4rem 1.2rem',
                fontSize: '0.8rem',
                fontWeight: 600,
              }}
            >
              Save Connection
            </button>
          </form>
        )}

        {connections.length === 0 && !showForm && (
          <p style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>
            No Aura destinations yet. Add one to analyse compatibility and migrate.
          </p>
        )}

        <div style={{ display: 'flex', flexDirection: 'column', gap: '0.5rem' }}>
          {connections.map((c) => (
            <div
              key={c.id}
              style={{
                padding: '0.6rem 0.8rem',
                backgroundColor: 'var(--bg-primary)',
                borderRadius: '6px',
                fontSize: '0.8rem',
              }}
            >
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span>
                  <strong>{c.name}</strong>{' '}
                  <span style={{ color: 'var(--text-muted)', fontFamily: 'monospace' }}>{c.uri}</span>
                </span>
                <span style={{ display: 'flex', gap: '0.5rem' }}>
                  <button
                    type="button"
                    onClick={() => handleTestConnection(c.id)}
                    disabled={testingId === c.id}
                    style={{
                      background: 'none',
                      border: '1px solid var(--border-color)',
                      color: 'var(--text-secondary)',
                      borderRadius: '4px',
                      fontSize: '0.75rem',
                      padding: '0.2rem 0.6rem',
                    }}
                  >
                    {testingId === c.id ? 'Testing...' : 'Test'}
                  </button>
                  <button
                    type="button"
                    onClick={() => handleDeleteConnection(c.id)}
                    style={{
                      background: 'none',
                      border: 'none',
                      color: 'var(--danger)',
                      fontSize: '0.75rem',
                    }}
                  >
                    Delete
                  </button>
                </span>
              </div>
              {testReports[c.id] && (
                <div
                  style={{
                    marginTop: '0.4rem',
                    color: testReports[c.id].reachable ? 'var(--success)' : 'var(--danger)',
                    fontSize: '0.75rem',
                  }}
                >
                  {testReports[c.id].message}
                </div>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* Analysis */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.75rem' }}>
          Compatibility Analysis
        </h3>
        <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.5rem' }}>
          Vector indexes are checked against the target tier (Neo4j allows 1–4096 dimensions,
          one vector per index) before any batch write. RAM figures are estimates — confirm in
          the Aura console.
        </p>
        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '0.75rem' }}>
          <select
            value={auraTier}
            onChange={(e) => setAuraTier(e.target.value)}
            title="Target Aura tier for vector RAM checks"
            style={{
              padding: '0.5rem 0.7rem',
              backgroundColor: 'var(--bg-primary)',
              border: '1px solid var(--border-color)',
              borderRadius: '6px',
              color: 'var(--text-primary)',
              fontSize: '0.85rem',
            }}
          >
            <option value="">No tier (dimensions only)</option>
            <option value="free">AuraDB Free (~1 GB)</option>
            <option value="professional-2gb">Professional 2 GB</option>
            <option value="professional-4gb">Professional 4 GB</option>
            <option value="professional-8gb">Professional 8 GB</option>
            <option value="dedicated-16gb">Dedicated 16 GB+</option>
          </select>
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
            <option value="">Source only (no target)</option>
            {connections.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name} — {c.uri}
              </option>
            ))}
          </select>
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
            {analyzing ? 'Analysing...' : 'Analyse'}
          </button>
          <button
            onClick={handlePlan}
            disabled={working || !selectedConnId}
            style={{
              backgroundColor: 'transparent',
              border: '1px solid var(--border-color)',
              color: 'var(--text-primary)',
              borderRadius: '6px',
              padding: '0.4rem 1.2rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              opacity: working || !selectedConnId ? 0.6 : 1,
            }}
          >
            Plan Migration
          </button>
        </div>

        {analysisError && (
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
            {analysisError}
          </div>
        )}

        {analysis && (
          <div>
            <p style={{ fontSize: '0.8rem', color: 'var(--text-muted)', marginBottom: '0.5rem' }}>
              Target: {analysis.target_reachable ? analysis.target_agent : 'unreachable or not selected'}
            </p>
            <FindingTable title="SOURCE FINDINGS" findings={analysis.source_findings} />
            <FindingTable title="TARGET FINDINGS" findings={analysis.target_findings} />
          </div>
        )}
      </div>

      {/* Migrations */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.75rem' }}>
          Migrations
        </h3>

        {migrationError && (
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
            {migrationError}
          </div>
        )}

        {migrations.length === 0 && (
          <p style={{ fontSize: '0.85rem', color: 'var(--text-muted)', marginBottom: '0.75rem' }}>
            No migrations yet. Select a connection above and plan one.
          </p>
        )}

        <div style={{ display: 'flex', gap: '0.5rem', flexWrap: 'wrap', marginBottom: '0.75rem' }}>
          {migrations.map((m) => (
            <button
              key={m.id}
              type="button"
              onClick={() => {
                setSelectedMigration(m);
                setDryRun(null);
                setVerification(null);
              }}
              style={{
                padding: '0.4rem 0.8rem',
                fontSize: '0.75rem',
                borderRadius: '6px',
                border: '1px solid',
                borderColor:
                  selectedMigration?.id === m.id ? 'var(--accent)' : 'var(--border-color)',
                backgroundColor:
                  selectedMigration?.id === m.id
                    ? 'rgba(var(--accent-rgb), 0.2)'
                    : 'var(--bg-primary)',
                color:
                  selectedMigration?.id === m.id
                    ? 'var(--accent-light)'
                    : 'var(--text-secondary)',
              }}
            >
              {m.id.slice(0, 8)} · {m.status}
            </button>
          ))}
        </div>

        {selectedMigration && (
          <div>
            <div
              style={{
                padding: '0.6rem 0.8rem',
                backgroundColor: 'rgba(16, 185, 129, 0.08)',
                border: '1px solid var(--success)',
                borderRadius: '6px',
                fontSize: '0.78rem',
                color: 'var(--text-secondary)',
                marginBottom: '0.6rem',
              }}
            >
              <strong style={{ color: 'var(--success)' }}>Write mode: MERGE upsert (non-destructive).</strong>{' '}
              {selectedMigration.plan.write_semantics ||
                'Appends new records and updates matched properties; never deletes or overwrites unrelated target data.'}{' '}
              Delta-friendly: re-running after adding local documents only transfers new/changed records.
              No overwrite/full-refresh mode exists in this release.
            </div>
            <div
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
                marginBottom: '0.5rem',
              }}
            >
              <span style={{ fontSize: '0.85rem' }}>
                <strong>{selectedMigration.id.slice(0, 8)}</strong>{' '}
                <span
                  style={{
                    fontSize: '0.75rem',
                    fontWeight: 700,
                    padding: '0.15rem 0.5rem',
                    borderRadius: '4px',
                    backgroundColor: 'rgba(255, 255, 255, 0.06)',
                    color:
                      selectedMigration.status === 'verified'
                        ? 'var(--success)'
                        : selectedMigration.status === 'failed'
                          ? 'var(--danger)'
                          : 'var(--accent-light)',
                  }}
                >
                  {selectedMigration.status.toUpperCase()}
                </span>
              </span>
              <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)' }}>
                {selectedMigration.progress.batches_done}/{selectedMigration.progress.batches_total} batches ·{' '}
                {selectedMigration.progress.nodes_migrated} nodes ·{' '}
                {selectedMigration.progress.relationships_migrated} rels
              </span>
            </div>

            {selectedMigration.progress.message && (
              <p style={{ fontSize: '0.8rem', color: 'var(--text-secondary)', marginBottom: '0.5rem' }}>
                {selectedMigration.progress.message}
              </p>
            )}

            <div style={{ display: 'flex', gap: '0.5rem', flexWrap: 'wrap', marginBottom: '0.75rem' }}>
              <button
                type="button"
                onClick={handleDryRun}
                disabled={working}
                style={{
                  backgroundColor: 'transparent',
                  border: '1px solid var(--border-color)',
                  color: 'var(--text-primary)',
                  borderRadius: '6px',
                  padding: '0.35rem 0.9rem',
                  fontSize: '0.78rem',
                  fontWeight: 600,
                  opacity: working ? 0.6 : 1,
                }}
              >
                Dry-run
              </button>
              {selectedMigration.status === 'approved' ? (
                <button
                  type="button"
                  onClick={() => handleApprove(false)}
                  disabled={working}
                  style={{
                    backgroundColor: 'transparent',
                    border: '1px solid var(--border-color)',
                    color: 'var(--text-secondary)',
                    borderRadius: '6px',
                    padding: '0.35rem 0.9rem',
                    fontSize: '0.78rem',
                    fontWeight: 600,
                    opacity: working ? 0.6 : 1,
                  }}
                >
                  Revoke approval
                </button>
              ) : (
                <button
                  type="button"
                  onClick={() => handleApprove(true)}
                  disabled={working}
                  style={{
                    backgroundColor: 'rgba(16, 185, 129, 0.15)',
                    border: '1px solid var(--success)',
                    color: 'var(--success)',
                    borderRadius: '6px',
                    padding: '0.35rem 0.9rem',
                    fontSize: '0.78rem',
                    fontWeight: 600,
                    opacity: working ? 0.6 : 1,
                  }}
                >
                  Approve
                </button>
              )}
              <button
                type="button"
                onClick={handleExecute}
                disabled={working || selectedMigration.status !== 'approved'}
                title={
                  selectedMigration.status !== 'approved'
                    ? 'Approve the migration first'
                    : 'Execute the approved migration (MERGE upsert: appends/updates, never deletes target data)'
                }
                style={{
                  backgroundColor: 'rgba(239, 68, 68, 0.15)',
                  border: '1px solid var(--danger)',
                  color: 'var(--danger)',
                  borderRadius: '6px',
                  padding: '0.35rem 0.9rem',
                  fontSize: '0.78rem',
                  fontWeight: 600,
                  opacity: working || selectedMigration.status !== 'approved' ? 0.5 : 1,
                }}
              >
                {working ? 'Working...' : 'Execute approved migration'}
              </button>
              <button
                type="button"
                onClick={handleVerify}
                disabled={
                  working ||
                  (selectedMigration.status !== 'completed' &&
                    selectedMigration.status !== 'verified')
                }
                style={{
                  backgroundColor: 'var(--accent)',
                  color: '#fff',
                  border: 'none',
                  borderRadius: '6px',
                  padding: '0.35rem 0.9rem',
                  fontSize: '0.78rem',
                  fontWeight: 600,
                  opacity:
                    working ||
                    (selectedMigration.status !== 'completed' &&
                      selectedMigration.status !== 'verified')
                      ? 0.5
                      : 1,
                }}
              >
                Verify
              </button>
            </div>

            {dryRun && (
              <div
                style={{
                  padding: '0.7rem 0.9rem',
                  backgroundColor: 'var(--bg-primary)',
                  borderRadius: '6px',
                  fontSize: '0.8rem',
                  marginBottom: '0.75rem',
                }}
              >
                <strong>Dry-run:</strong> {dryRun.plan.total_nodes} nodes,{' '}
                {dryRun.plan.total_relationships} relationships in {dryRun.plan.total_batches}{' '}
                batches · {dryRun.plan.ddl_statements.length} DDL statements ·{' '}
                {dryRun.remote_writes} remote writes · target{' '}
                {dryRun.target_reachable ? dryRun.target_agent : 'unreachable'}
                {dryRun.blockers.length > 0 && (
                  <div style={{ marginTop: '0.4rem', color: 'var(--danger)' }}>
                    Blockers:
                    <ul style={{ margin: '0.2rem 0 0 1.2rem', padding: 0 }}>
                      {dryRun.blockers.map((b, i) => (
                        <li key={i}>{b}</li>
                      ))}
                    </ul>
                  </div>
                )}
                {dryRun.plan.warnings.length > 0 && (
                  <div style={{ marginTop: '0.4rem', color: 'var(--text-muted)' }}>
                    Warnings:
                    <ul style={{ margin: '0.2rem 0 0 1.2rem', padding: 0 }}>
                      {dryRun.plan.warnings.map((w, i) => (
                        <li key={i}>{w}</li>
                      ))}
                    </ul>
                  </div>
                )}
                {dryRun.plan.ddl_statements.length > 0 && (
                  <pre
                    style={{
                      marginTop: '0.5rem',
                      fontSize: '0.72rem',
                      fontFamily: 'monospace',
                      whiteSpace: 'pre-wrap',
                      color: 'var(--text-secondary)',
                    }}
                  >
                    {dryRun.plan.ddl_statements.join('\n')}
                  </pre>
                )}
              </div>
            )}

            {verification && (
              <div
                style={{
                  padding: '0.7rem 0.9rem',
                  backgroundColor: 'var(--bg-primary)',
                  borderRadius: '6px',
                  fontSize: '0.8rem',
                }}
              >
                <strong style={{ color: verification.passed ? 'var(--success)' : 'var(--danger)' }}>
                  {verification.passed ? 'Verification passed ✓' : 'Verification failed ✕'}
                </strong>{' '}
                <span style={{ color: 'var(--text-muted)' }}>
                  ({verification.samples_matched}/{verification.samples_checked} samples matched)
                </span>
                <div
                  style={{ display: 'flex', flexDirection: 'column', gap: '0.25rem', marginTop: '0.4rem' }}
                >
                  {verification.checks.map((c, i) => (
                    <div key={i} style={{ color: 'var(--text-secondary)', fontSize: '0.78rem' }}>
                      <span style={{ color: c.matched ? 'var(--success)' : 'var(--danger)' }}>
                        {c.matched ? '✓' : '✕'}
                      </span>{' '}
                      {c.kind} {c.name}: source {c.source_count}, target {c.target_count}
                    </div>
                  ))}
                </div>
                {verification.details.length > 0 && (
                  <div style={{ marginTop: '0.4rem', color: 'var(--danger)', fontSize: '0.78rem' }}>
                    {verification.details.map((d, i) => (
                      <div key={i}>{d}</div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};
