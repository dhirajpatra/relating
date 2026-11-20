import React from 'react';

const ink = 'var(--hero-ink)';

const boxTitle: React.CSSProperties = {
  fontSize: '0.8rem',
  fontWeight: 700,
  color: ink,
  marginBottom: '0.35rem',
};

const boxLine: React.CSSProperties = {
  fontSize: '0.72rem',
  color: '#47617c',
  lineHeight: 1.55,
};

const arrow: React.CSSProperties = {
  fontSize: '0.68rem',
  color: '#47617c',
  fontStyle: 'italic',
  whiteSpace: 'nowrap',
  alignSelf: 'center',
};

/**
 * Dashboard banner mirroring the Neo4j walkthrough-slide layout: a white
 * diagram card on the navy page showing how Cypher clients reach Aura
 * through the Relating virtual graph.
 */
export const ArchitectureHero: React.FC = () => (
  <section style={{ margin: '0 auto 1.5rem', maxWidth: '1200px' }}>
    <h2
      style={{
        fontSize: '1.35rem',
        fontWeight: 800,
        letterSpacing: '-0.02em',
        color: '#fff',
        textAlign: 'center',
        marginBottom: '0.25rem',
      }}
    >
      Relating Virtual Graph on Neo4j Aura
    </h2>
    <p
      style={{
        fontSize: '0.85rem',
        color: 'var(--text-secondary)',
        textAlign: 'center',
        marginBottom: '1rem',
      }}
    >
      Develop locally, query and migrate in place on your Aura database.
    </p>

    <div
      style={{
        backgroundColor: 'var(--hero-card)',
        borderRadius: '10px',
        padding: '1.1rem 1.25rem',
        boxShadow: '0 12px 32px rgba(2, 12, 24, 0.45)',
      }}
    >
      <div
        style={{
          fontSize: '0.78rem',
          fontWeight: 700,
          color: ink,
          textAlign: 'center',
          marginBottom: '0.8rem',
        }}
      >
        How this workbench connects
      </div>
      <div
        style={{
          display: 'flex',
          alignItems: 'stretch',
          justifyContent: 'center',
          gap: '0.6rem',
          flexWrap: 'wrap',
        }}
      >
        <div
          style={{
            backgroundColor: '#eef3f8',
            border: '1px solid #d4dee9',
            borderRadius: '6px',
            padding: '0.6rem 0.9rem',
            minWidth: '150px',
            textAlign: 'center',
          }}
        >
          <div style={boxTitle}>Cypher clients</div>
          <div style={boxLine}>
            Workbench
            <br />
            Query &amp; Explore
            <br />
            Bloom &amp; drivers
          </div>
        </div>

        <div style={arrow}>Cypher ──▶</div>

        <div
          style={{
            backgroundColor: '#eaf4ff',
            border: '1.5px solid #018bff',
            borderRadius: '6px',
            padding: '0.6rem 0.9rem',
            minWidth: '170px',
            textAlign: 'center',
          }}
        >
          <div style={boxTitle}>Relating instance</div>
          <div style={boxLine}>
            Translates workbench
            <br />
            actions into Aura runs
          </div>
        </div>

        <div style={arrow}>Queries in place ──▶</div>

        <div
          style={{
            backgroundColor: '#fdf6e8',
            border: '1.5px solid #e8a33d',
            borderRadius: '6px',
            padding: '0.6rem 0.9rem',
            minWidth: '150px',
            textAlign: 'center',
          }}
        >
          <div style={boxTitle}>Aura database</div>
          <div style={boxLine}>
            Your instance
            <br />
            &amp; namespace
          </div>
        </div>
      </div>
    </div>
  </section>
);
