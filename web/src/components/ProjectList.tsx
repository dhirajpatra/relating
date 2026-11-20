import React from 'react';
import { neo4jTheme } from '../theme/neo4j';
import { ProjectSummary } from '../types';
import { ArchitectureHero } from './ArchitectureHero';
import { Neo4jLogo } from './Neo4jLogo';

interface ProjectListProps {
  projects: ProjectSummary[];
  loading: boolean;
  onSelectProject: (id: string) => void;
  onDeleteProject: (id: string) => void;
  onNewProject: () => void;
}

export const ProjectList: React.FC<ProjectListProps> = ({
  projects,
  loading,
  onSelectProject,
  onDeleteProject,
  onNewProject,
}) => {
  if (loading && projects.length === 0) {
    return (
      <div style={{ textAlign: 'center', padding: '4rem 0', color: 'var(--text-muted)' }}>
        Loading projects...
      </div>
    );
  }

  if (projects.length === 0) {
    return (
      <div style={{ maxWidth: '1200px', margin: '2rem auto', padding: '0 1.5rem' }}>
        <ArchitectureHero />
        <div
          style={{
            textAlign: 'center',
            padding: '3rem 2rem',
            backgroundColor: 'var(--bg-secondary)',
            borderRadius: '12px',
            border: '1px dashed var(--border-color)',
            maxWidth: '600px',
            margin: '0 auto',
          }}
        >
          <div style={{ display: 'flex', justifyContent: 'center', marginBottom: '1rem' }}>
            <Neo4jLogo size={44} />
          </div>
          <h3 style={{ fontSize: '1.25rem', fontWeight: 600, marginBottom: '0.5rem' }}>
            No graph projects yet
          </h3>
          <p
            style={{
              color: 'var(--text-muted)',
              fontSize: '0.9rem',
              maxWidth: '400px',
              margin: '0 auto 1.5rem',
            }}
          >
            Create an isolated graph project backed by an embedded Relate database and AI services
            to get started.
          </p>
          <button
            onClick={onNewProject}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.6rem 1.25rem',
              fontWeight: 600,
              fontSize: '0.875rem',
            }}
          >
            + Create First Project
          </button>
        </div>
      </div>
    );
  }

  return (
    <div style={{ maxWidth: '1200px', margin: '2rem auto', padding: '0 1.5rem' }}>
      <ArchitectureHero />
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          marginBottom: '1.5rem',
        }}
      >
        <div>
          <h2 style={{ fontSize: '1.25rem', fontWeight: 700 }}>Your Graph Projects</h2>
          <p style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>
            Each project has an isolated Relate store and dedicated AI configuration.
          </p>
        </div>
        <span
          style={{
            fontSize: '0.85rem',
            color: 'var(--text-secondary)',
            backgroundColor: 'var(--bg-secondary)',
            padding: '0.3rem 0.8rem',
            borderRadius: '6px',
            border: '1px solid var(--border-color)',
          }}
        >
          {projects.length} {projects.length === 1 ? 'project' : 'projects'}
        </span>
      </div>

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(350px, 1fr))',
          gap: '1.25rem',
        }}
      >
        {projects.map((p) => (
          <div
            key={p.id}
            style={{
              backgroundColor: 'var(--bg-secondary)',
              borderRadius: '10px',
              border: '1px solid var(--border-color)',
              padding: '1.25rem',
              display: 'flex',
              flexDirection: 'column',
              justifyContent: 'space-between',
              transition: 'border-color 0.15s ease, transform 0.15s ease',
            }}
          >
            <div>
              <div
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'flex-start',
                  marginBottom: '0.5rem',
                }}
              >
                <h3
                  onClick={() => onSelectProject(p.id)}
                  style={{
                    fontSize: '1.1rem',
                    fontWeight: 600,
                    cursor: 'pointer',
                    color: 'var(--text-primary)',
                  }}
                  onMouseOver={(e) => (e.currentTarget.style.color = 'var(--accent-light)')}
                  onMouseOut={(e) => (e.currentTarget.style.color = 'var(--text-primary)')}
                >
                  {p.name}
                </h3>
                <span
                  style={{
                    fontSize: '0.75rem',
                    padding: '0.15rem 0.5rem',
                    borderRadius: '4px',
                    backgroundColor: neo4jTheme.accentSoft(0.15),
                    color: 'var(--accent-light)',
                    textTransform: 'uppercase',
                    fontWeight: 600,
                  }}
                >
                  {p.provider_type}
                </span>
              </div>

              <p
                style={{
                  fontSize: '0.85rem',
                  color: 'var(--text-secondary)',
                  marginBottom: '1rem',
                  minHeight: '2.5rem',
                }}
              >
                {p.description || 'No description provided.'}
              </p>

              {/* Stats Bar */}
              <div
                style={{
                  display: 'flex',
                  gap: '1rem',
                  padding: '0.6rem 0.8rem',
                  backgroundColor: 'var(--bg-primary)',
                  borderRadius: '6px',
                  marginBottom: '1rem',
                  fontSize: '0.8rem',
                }}
              >
                <div>
                  <span style={{ color: 'var(--text-muted)', display: 'block', fontSize: '0.7rem' }}>
                    NODES
                  </span>
                  <span style={{ fontWeight: 600, color: 'var(--text-primary)' }}>
                    {p.node_count}
                  </span>
                </div>
                <div style={{ width: '1px', backgroundColor: 'var(--border-color)' }} />
                <div>
                  <span style={{ color: 'var(--text-muted)', display: 'block', fontSize: '0.7rem' }}>
                    RELATIONSHIPS
                  </span>
                  <span style={{ fontWeight: 600, color: 'var(--text-primary)' }}>
                    {p.relationship_count}
                  </span>
                </div>
              </div>
            </div>

            <div
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
                paddingTop: '0.75rem',
                borderTop: '1px solid var(--border-color)',
              }}
            >
              <button
                onClick={() => onSelectProject(p.id)}
                style={{
                  backgroundColor: 'var(--accent)',
                  color: '#fff',
                  border: 'none',
                  borderRadius: '4px',
                  padding: '0.4rem 0.8rem',
                  fontSize: '0.8rem',
                  fontWeight: 600,
                }}
              >
                Open Workbench →
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  if (confirm(`Are you sure you want to delete "${p.name}"?`)) {
                    onDeleteProject(p.id);
                  }
                }}
                style={{
                  backgroundColor: 'transparent',
                  color: 'var(--danger)',
                  border: 'none',
                  fontSize: '0.8rem',
                  cursor: 'pointer',
                  padding: '0.4rem',
                }}
              >
                Delete
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};
