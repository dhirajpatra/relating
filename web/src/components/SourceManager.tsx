import React, { useRef, useState } from 'react';
import { api } from '../api/client';
import { Source } from '../types';

interface SourceManagerProps {
  projectId: string;
  sources: Source[];
  onSourcesChanged: () => void;
}

export const SourceManager: React.FC<SourceManagerProps> = ({
  projectId,
  sources,
  onSourcesChanged,
}) => {
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleFileUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    setUploading(true);
    setError(null);

    try {
      const text = await file.text();
      await api.registerSource(projectId, file.name, text, file.type || undefined);
      if (fileInputRef.current) fileInputRef.current.value = '';
      onSourcesChanged();
    } catch (err: unknown) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('Failed to upload source file');
      }
    } finally {
      setUploading(false);
    }
  };

  const handleAddSampleData = async () => {
    setUploading(true);
    setError(null);
    try {
      const sampleCsv = `id,name,role,department,company_id\n1,Alice Chen,Tech Lead,Engineering,100\n2,Marcus Vance,Product Manager,Product,100\n3,Elena Rostova,Security Architect,InfoSec,101`;
      await api.registerSource(projectId, 'team_members.csv', sampleCsv, 'text/csv');

      const sampleCompanies = `[\n  {"id": "100", "name": "Relating Systems", "domain": "Knowledge Graphs"},\n  {"id": "101", "name": "Aura Cloud", "domain": "Managed Databases"}\n]`;
      await api.registerSource(projectId, 'companies.json', sampleCompanies, 'application/json');

      onSourcesChanged();
    } catch (err: unknown) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('Failed to add sample data');
      }
    } finally {
      setUploading(false);
    }
  };

  const handleDelete = async (sourceId: string, filename: string) => {
    if (!confirm(`Delete source file "${filename}"?`)) return;
    try {
      await api.deleteSource(projectId, sourceId);
      onSourcesChanged();
    } catch (err: unknown) {
      alert(err instanceof Error ? err.message : 'Delete failed');
    }
  };

  return (
    <div style={{ backgroundColor: 'var(--bg-secondary)', borderRadius: '8px', border: '1px solid var(--border-color)', padding: '1.5rem' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1.5rem' }}>
        <div>
          <h3 style={{ fontSize: '1.1rem', fontWeight: 600 }}>Registered Data Sources</h3>
          <p style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>
            Upload CSV, JSON, or text documents. Content hashes and table lineage are tracked automatically.
          </p>
        </div>

        <div style={{ display: 'flex', gap: '0.75rem' }}>
          <button
            type="button"
            onClick={handleAddSampleData}
            disabled={uploading}
            style={{
              padding: '0.5rem 0.9rem',
              backgroundColor: 'rgba(99, 102, 241, 0.1)',
              border: '1px solid var(--accent)',
              color: 'var(--accent-light)',
              borderRadius: '6px',
              fontSize: '0.85rem',
              fontWeight: 600,
            }}
          >
            + Add Sample CSV & JSON
          </button>

          <input
            ref={fileInputRef}
            type="file"
            onChange={handleFileUpload}
            style={{ display: 'none' }}
            accept=".csv,.json,.txt,.md,.doc,.docx"
          />
          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            disabled={uploading}
            style={{
              padding: '0.5rem 1rem',
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              fontSize: '0.85rem',
              fontWeight: 600,
            }}
          >
            {uploading ? 'Processing File...' : '↑ Upload File'}
          </button>
        </div>
      </div>

      {error && (
        <div style={{ padding: '0.75rem', backgroundColor: 'rgba(239, 68, 68, 0.1)', border: '1px solid var(--danger)', color: 'var(--danger)', borderRadius: '6px', marginBottom: '1rem', fontSize: '0.85rem' }}>
          {error}
        </div>
      )}

      {sources.length === 0 ? (
        <div style={{ textAlign: 'center', padding: '3rem 1rem', backgroundColor: 'var(--bg-primary)', borderRadius: '6px', border: '1px dashed var(--border-color)' }}>
          <p style={{ color: 'var(--text-muted)', fontSize: '0.9rem', marginBottom: '1rem' }}>
            No sources uploaded yet. Add CSV or JSON files to start building your graph.
          </p>
          <button
            onClick={handleAddSampleData}
            style={{ padding: '0.4rem 0.8rem', backgroundColor: 'transparent', border: '1px solid var(--accent)', color: 'var(--accent-light)', borderRadius: '4px', fontSize: '0.8rem' }}
          >
            Load Sample Datasets
          </button>
        </div>
      ) : (
        <div style={{ overflowX: 'auto' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
            <thead>
              <tr style={{ backgroundColor: 'var(--bg-primary)', textAlign: 'left', borderBottom: '1px solid var(--border-color)' }}>
                <th style={{ padding: '0.6rem 1rem', color: 'var(--text-secondary)' }}>File</th>
                <th style={{ padding: '0.6rem 1rem', color: 'var(--text-secondary)' }}>Type</th>
                <th style={{ padding: '0.6rem 1rem', color: 'var(--text-secondary)' }}>Size / Records</th>
                <th style={{ padding: '0.6rem 1rem', color: 'var(--text-secondary)' }}>SHA-256 Hash</th>
                <th style={{ padding: '0.6rem 1rem', color: 'var(--text-secondary)', textAlign: 'right' }}>Action</th>
              </tr>
            </thead>
            <tbody>
              {sources.map((s) => (
                <tr key={s.id} style={{ borderBottom: '1px solid rgba(255, 255, 255, 0.05)' }}>
                  <td style={{ padding: '0.6rem 1rem', fontWeight: 600 }}>{s.filename}</td>
                  <td style={{ padding: '0.6rem 1rem', color: 'var(--text-secondary)', fontFamily: 'monospace' }}>
                    {s.content_type}
                  </td>
                  <td style={{ padding: '0.6rem 1rem' }}>
                    {s.byte_size} bytes
                    {s.metadata.total_records !== undefined && (
                      <span style={{ color: 'var(--text-muted)', marginLeft: '0.5rem' }}>
                        ({s.metadata.total_records} records)
                      </span>
                    )}
                  </td>
                  <td style={{ padding: '0.6rem 1rem', fontFamily: 'monospace', fontSize: '0.75rem', color: 'var(--text-muted)' }}>
                    {s.content_hash.slice(0, 12)}...
                  </td>
                  <td style={{ padding: '0.6rem 1rem', textAlign: 'right' }}>
                    <button
                      onClick={() => handleDelete(s.id, s.filename)}
                      style={{ background: 'none', border: 'none', color: 'var(--danger)', cursor: 'pointer', fontSize: '0.8rem' }}
                    >
                      Delete
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
};
