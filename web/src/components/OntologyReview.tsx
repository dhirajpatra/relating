import React, { useState } from 'react';
import { api } from '../api/client';
import { Ontology, OntologyStatusResponse } from '../types';

interface OntologyReviewProps {
  projectId: string;
  ontologyStatus: OntologyStatusResponse;
  hasSources: boolean;
  onOntologyChanged: () => void;
}

export const OntologyReview: React.FC<OntologyReviewProps> = ({
  projectId,
  ontologyStatus,
  hasSources,
  onOntologyChanged,
}) => {
  const [proposing, setProposing] = useState(false);
  const [approving, setApproving] = useState(false);
  const [domainHint, setDomainHint] = useState('');
  const [error, setError] = useState<string | null>(null);

  const currentOntology: Ontology | undefined =
    ontologyStatus.latest || ontologyStatus.approved;

  const handlePropose = async () => {
    if (!hasSources) {
      setError('Please upload at least one data source before proposing an ontology.');
      return;
    }

    setProposing(true);
    setError(null);
    try {
      await api.proposeOntology(projectId, domainHint.trim() || undefined);
      onOntologyChanged();
    } catch (err: unknown) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('Failed to propose ontology');
      }
    } finally {
      setProposing(false);
    }
  };

  const handleApprove = async () => {
    if (!currentOntology) return;
    setApproving(true);
    setError(null);
    try {
      await api.approveOntology(projectId, currentOntology.id);
      onOntologyChanged();
    } catch (err: unknown) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('Failed to approve ontology');
      }
    } finally {
      setApproving(false);
    }
  };

  const isApproved = currentOntology?.status === 'approved';

  return (
    <div style={{ backgroundColor: 'var(--bg-secondary)', borderRadius: '8px', border: '1px solid var(--border-color)', padding: '1.5rem' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: '1.5rem' }}>
        <div>
          <div style={{ display: 'flex', alignItems: 'center', gap: '0.75rem', marginBottom: '0.25rem' }}>
            <h3 style={{ fontSize: '1.1rem', fontWeight: 600 }}>Graph Ontology Schema</h3>
            {currentOntology && (
              <span
                style={{
                  fontSize: '0.75rem',
                  fontWeight: 700,
                  padding: '0.2rem 0.6rem',
                  borderRadius: '9999px',
                  backgroundColor: isApproved ? 'rgba(16, 185, 129, 0.15)' : 'rgba(245, 158, 11, 0.15)',
                  color: isApproved ? 'var(--success)' : 'var(--warning)',
                  border: `1px solid ${isApproved ? 'var(--success)' : 'var(--warning)'}`,
                  textTransform: 'uppercase',
                }}
              >
                {currentOntology.status} (v{currentOntology.version})
              </span>
            )}
          </div>
          <p style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>
            Review AI-proposed entities, relationships, and constraints. Approval is required before graph extraction.
          </p>
        </div>

        <div style={{ display: 'flex', gap: '0.75rem' }}>
          {currentOntology && !isApproved && (
            <button
              onClick={handleApprove}
              disabled={approving}
              style={{
                padding: '0.5rem 1rem',
                backgroundColor: 'var(--success)',
                color: '#fff',
                border: 'none',
                borderRadius: '6px',
                fontSize: '0.85rem',
                fontWeight: 600,
                opacity: approving ? 0.7 : 1,
              }}
            >
              {approving ? 'Approving...' : '✓ Approve Ontology Schema'}
            </button>
          )}

          <button
            onClick={handlePropose}
            disabled={proposing || !hasSources}
            style={{
              padding: '0.5rem 1rem',
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              fontSize: '0.85rem',
              fontWeight: 600,
              opacity: proposing || !hasSources ? 0.6 : 1,
            }}
          >
            {proposing ? 'Proposing with AI...' : '✨ Propose with AI'}
          </button>
        </div>
      </div>

      {error && (
        <div style={{ padding: '0.75rem', backgroundColor: 'rgba(239, 68, 68, 0.1)', border: '1px solid var(--danger)', color: 'var(--danger)', borderRadius: '6px', marginBottom: '1rem', fontSize: '0.85rem' }}>
          {error}
        </div>
      )}

      {/* Domain Context Hint Input */}
      <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '1.5rem', alignItems: 'center' }}>
        <input
          type="text"
          value={domainHint}
          onChange={(e) => setDomainHint(e.target.value)}
          placeholder="Optional domain hint for AI (e.g. corporate org chart, clinical trial, supply chain)..."
          style={{
            flex: 1,
            padding: '0.45rem 0.8rem',
            backgroundColor: 'var(--bg-primary)',
            border: '1px solid var(--border-color)',
            borderRadius: '6px',
            color: 'var(--text-primary)',
            fontSize: '0.85rem',
          }}
        />
      </div>

      {!currentOntology ? (
        <div style={{ textAlign: 'center', padding: '3rem 1rem', backgroundColor: 'var(--bg-primary)', borderRadius: '6px', border: '1px dashed var(--border-color)' }}>
          <p style={{ color: 'var(--text-muted)', fontSize: '0.9rem', marginBottom: '1rem' }}>
            No ontology proposed yet. Click "Propose with AI" to let the AI analyze your uploaded data sources.
          </p>
        </div>
      ) : (
        <div>
          {/* Entity Types */}
          <div style={{ marginBottom: '1.5rem' }}>
            <h4 style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--text-secondary)', marginBottom: '0.75rem' }}>
              ENTITY TYPES ({currentOntology.schema.entities.length})
            </h4>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: '1rem' }}>
              {currentOntology.schema.entities.map((ent) => (
                <div
                  key={ent.label}
                  style={{
                    backgroundColor: 'var(--bg-primary)',
                    borderRadius: '8px',
                    border: '1px solid var(--border-color)',
                    padding: '1rem',
                  }}
                >
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '0.4rem' }}>
                    <span style={{ fontWeight: 700, color: 'var(--accent-light)', fontSize: '1rem' }}>
                      :{ent.label}
                    </span>
                    <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)' }}>
                      {ent.display_name}
                    </span>
                  </div>
                  <p style={{ fontSize: '0.8rem', color: 'var(--text-secondary)', marginBottom: '0.75rem' }}>
                    {ent.description}
                  </p>

                  <div style={{ fontSize: '0.75rem' }}>
                    <span style={{ color: 'var(--text-muted)', display: 'block', marginBottom: '0.25rem' }}>
                      PROPERTIES
                    </span>
                    <div style={{ display: 'flex', flexDirection: 'column', gap: '0.2rem' }}>
                      {ent.properties.map((p) => (
                        <div key={p.name} style={{ display: 'flex', justifyContent: 'space-between', color: 'var(--text-secondary)' }}>
                          <span style={{ fontFamily: 'monospace' }}>
                            {p.name} {p.is_primary_key && <strong style={{ color: 'var(--warning)' }}>(PK)</strong>}
                          </span>
                          <span style={{ color: 'var(--text-muted)' }}>{p.data_type}</span>
                        </div>
                      ))}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </div>

          {/* Relationship Types */}
          <div>
            <h4 style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--text-secondary)', marginBottom: '0.75rem' }}>
              RELATIONSHIP TYPES ({currentOntology.schema.relationships.length})
            </h4>
            {currentOntology.schema.relationships.length === 0 ? (
              <p style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>No explicit relationships defined.</p>
            ) : (
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: '1rem' }}>
                {currentOntology.schema.relationships.map((rel, idx) => (
                  <div
                    key={idx}
                    style={{
                      backgroundColor: 'var(--bg-primary)',
                      borderRadius: '8px',
                      border: '1px solid var(--border-color)',
                      padding: '0.8rem 1rem',
                      display: 'flex',
                      flexDirection: 'column',
                      gap: '0.3rem',
                    }}
                  >
                    <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem', fontSize: '0.85rem' }}>
                      <span style={{ color: 'var(--accent-light)', fontWeight: 600 }}>
                        :{rel.source_label}
                      </span>
                      <span style={{ color: 'var(--accent-light)', fontWeight: 700, fontFamily: 'monospace' }}>
                        -[:{rel.rel_type}]-&gt;
                      </span>
                      <span style={{ color: 'var(--accent-light)', fontWeight: 600 }}>
                        :{rel.target_label}
                      </span>
                    </div>
                    <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)' }}>
                      {rel.description}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};
