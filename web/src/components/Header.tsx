import React from 'react';
import { SystemHealth } from '../types';

interface HeaderProps {
  health: SystemHealth | null;
  onNewProject: () => void;
  onBackToDashboard?: () => void;
  currentProjectName?: string;
}

export const Header: React.FC<HeaderProps> = ({
  health,
  onNewProject,
  onBackToDashboard,
  currentProjectName,
}) => {
  return (
    <header
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        padding: '1rem 2rem',
        backgroundColor: 'var(--bg-secondary)',
        borderBottom: '1px solid var(--border-color)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: '1rem' }}>
        <div
          onClick={onBackToDashboard}
          style={{
            cursor: onBackToDashboard ? 'pointer' : 'default',
            display: 'flex',
            alignItems: 'center',
            gap: '0.75rem',
          }}
        >
          <div
            style={{
              width: '32px',
              height: '32px',
              borderRadius: '8px',
              background: 'linear-gradient(135deg, #6366f1 0%, #a855f7 100%)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              fontWeight: 'bold',
              fontSize: '1.2rem',
              color: '#fff',
            }}
          >
            R
          </div>
          <div>
            <h1 style={{ fontSize: '1.25rem', fontWeight: 700, letterSpacing: '-0.025em' }}>
              Relating
            </h1>
            <p style={{ fontSize: '0.75rem', color: 'var(--text-muted)' }}>
              Graph Development & GraphRAG Workbench
            </p>
          </div>
        </div>

        {currentProjectName && (
          <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem', marginLeft: '1rem' }}>
            <span style={{ color: 'var(--text-muted)' }}>/</span>
            <span
              style={{
                fontSize: '0.9rem',
                fontWeight: 600,
                color: 'var(--accent-light)',
                backgroundColor: 'rgba(99, 102, 241, 0.15)',
                padding: '0.2rem 0.6rem',
                borderRadius: '4px',
              }}
            >
              {currentProjectName}
            </span>
          </div>
        )}
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: '1.25rem' }}>
        {/* System Health Pill */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: '0.5rem',
            fontSize: '0.8rem',
            padding: '0.3rem 0.75rem',
            borderRadius: '9999px',
            backgroundColor: health ? 'rgba(16, 185, 129, 0.1)' : 'rgba(239, 68, 68, 0.1)',
            border: `1px solid ${health ? 'rgba(16, 185, 129, 0.3)' : 'rgba(239, 68, 68, 0.3)'}`,
            color: health ? 'var(--success)' : 'var(--danger)',
          }}
        >
          <span
            style={{
              width: '8px',
              height: '8px',
              borderRadius: '50%',
              backgroundColor: health ? 'var(--success)' : 'var(--danger)',
            }}
          />
          {health ? `Core v${health.version}` : 'Connecting...'}
        </div>

        <button
          onClick={onNewProject}
          style={{
            backgroundColor: 'var(--accent)',
            color: '#fff',
            border: 'none',
            borderRadius: '6px',
            padding: '0.5rem 1rem',
            fontWeight: 600,
            fontSize: '0.875rem',
            display: 'flex',
            alignItems: 'center',
            gap: '0.5rem',
            transition: 'background-color 0.15s ease',
          }}
          onMouseOver={(e) => (e.currentTarget.style.backgroundColor = 'var(--accent-hover)')}
          onMouseOut={(e) => (e.currentTarget.style.backgroundColor = 'var(--accent)')}
        >
          + New Project
        </button>
      </div>
    </header>
  );
};
