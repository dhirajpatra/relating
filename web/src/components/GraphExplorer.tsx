import cytoscape, { Core } from 'cytoscape';
import React, { useEffect, useRef, useState } from 'react';
import { api } from '../api/client';
import { CytoscapeGraph, CytoscapeNodeData, IngestionResultResponse } from '../types';

interface GraphExplorerProps {
  projectId: string;
  hasApprovedOntology: boolean;
  onGraphMutated: () => void;
}

export const GraphExplorer: React.FC<GraphExplorerProps> = ({
  projectId,
  hasApprovedOntology,
  onGraphMutated,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const cyRef = useRef<Core | null>(null);

  const [loading, setLoading] = useState(false);
  const [ingesting, setIngesting] = useState(false);
  const [ingestReport, setIngestReport] = useState<IngestionResultResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selectedNode, setSelectedNode] = useState<CytoscapeNodeData | null>(null);
  const [graphData, setGraphData] = useState<CytoscapeGraph | null>(null);

  const fetchGraphElements = async () => {
    try {
      setLoading(true);
      const data = await api.getGraphElements(projectId, 250);
      setGraphData(data);
      renderCytoscape(data);
      setError(null);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('Failed to fetch graph visualization elements');
      }
    } finally {
      setLoading(false);
    }
  };

  const renderCytoscape = (data: CytoscapeGraph) => {
    if (!containerRef.current) return;

    if (cyRef.current) {
      cyRef.current.destroy();
    }

    const elements: cytoscape.ElementDefinition[] = [];

    data.elements.nodes.forEach((n) => {
      elements.push({
        group: 'nodes',
        data: {
          id: n.data.id,
          label: n.data.label,
          name: n.data.name,
          properties: n.data.properties,
        },
      });
    });

    data.elements.edges.forEach((e) => {
      elements.push({
        group: 'edges',
        data: {
          id: e.data.id,
          source: e.data.source,
          target: e.data.target,
          label: e.data.label,
          properties: e.data.properties,
        },
      });
    });

    const cy = cytoscape({
      container: containerRef.current,
      elements,
      style: [
        {
          selector: 'node',
          style: {
            'background-color': '#6366f1',
            label: 'data(name)',
            color: '#f8fafc',
            'font-size': '11px',
            'text-valign': 'bottom',
            'text-margin-y': 6,
            width: 32,
            height: 32,
            'border-width': 2,
            'border-color': '#a5b4fc',
          },
        },
        {
          selector: 'node:selected',
          style: {
            'background-color': '#a855f7',
            'border-color': '#f8fafc',
            'border-width': 3,
          },
        },
        {
          selector: 'edge',
          style: {
            width: 2,
            'line-color': '#475569',
            'target-arrow-color': '#475569',
            'target-arrow-shape': 'triangle',
            'curve-style': 'bezier',
            label: 'data(label)',
            'font-size': '9px',
            color: '#94a3b8',
            'text-rotation': 'autorotate',
            'text-margin-y': -8,
          },
        },
      ],
      layout: {
        name: 'cose',
        animate: true,
        nodeRepulsion: () => 4500,
        idealEdgeLength: () => 80,
      },
    });

    cy.on('tap', 'node', (evt) => {
      const node = evt.target;
      setSelectedNode(node.data());
    });

    cy.on('tap', (evt) => {
      if (evt.target === cy) {
        setSelectedNode(null);
      }
    });

    cyRef.current = cy;
  };

  useEffect(() => {
    fetchGraphElements();
    return () => {
      if (cyRef.current) {
        cyRef.current.destroy();
      }
    };
  }, [projectId]);

  const handleRunIngestion = async () => {
    if (!hasApprovedOntology) {
      setError('An approved ontology is required before extracting data to graph.');
      return;
    }

    setIngesting(true);
    setError(null);
    try {
      const report = await api.runIngestion(projectId);
      setIngestReport(report);
      await fetchGraphElements();
      onGraphMutated();
    } catch (err: unknown) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('Ingestion failed');
      }
    } finally {
      setIngesting(false);
    }
  };

  const handleFit = () => {
    if (cyRef.current) {
      cyRef.current.fit(undefined, 30);
    }
  };

  const handleLayout = () => {
    if (cyRef.current) {
      cyRef.current.layout({ name: 'cose', animate: true }).run();
    }
  };

  return (
    <div style={{ backgroundColor: 'var(--bg-secondary)', borderRadius: '8px', border: '1px solid var(--border-color)', padding: '1.5rem' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1.25rem' }}>
        <div>
          <h3 style={{ fontSize: '1.1rem', fontWeight: 600 }}>Interactive Graph Explorer</h3>
          <p style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>
            Live Cytoscape.js canvas showing extracted nodes, relationships, and source provenance.
          </p>
        </div>

        <div style={{ display: 'flex', gap: '0.75rem' }}>
          <button
            onClick={handleFit}
            style={{ padding: '0.4rem 0.8rem', backgroundColor: 'var(--bg-primary)', border: '1px solid var(--border-color)', color: 'var(--text-secondary)', borderRadius: '6px', fontSize: '0.8rem' }}
          >
            Fit View
          </button>
          <button
            onClick={handleLayout}
            style={{ padding: '0.4rem 0.8rem', backgroundColor: 'var(--bg-primary)', border: '1px solid var(--border-color)', color: 'var(--text-secondary)', borderRadius: '6px', fontSize: '0.8rem' }}
          >
            Re-layout (COSE)
          </button>
          <button
            onClick={handleRunIngestion}
            disabled={ingesting || !hasApprovedOntology}
            style={{
              padding: '0.5rem 1rem',
              backgroundColor: hasApprovedOntology ? 'var(--accent)' : 'var(--bg-primary)',
              border: 'none',
              color: '#fff',
              borderRadius: '6px',
              fontSize: '0.85rem',
              fontWeight: 600,
              opacity: ingesting || !hasApprovedOntology ? 0.6 : 1,
            }}
          >
            {ingesting ? 'Extracting & Loading...' : '⚡ Ingest Sources to Graph'}
          </button>
        </div>
      </div>

      {error && (
        <div style={{ padding: '0.75rem', backgroundColor: 'rgba(239, 68, 68, 0.1)', border: '1px solid var(--danger)', color: 'var(--danger)', borderRadius: '6px', marginBottom: '1rem', fontSize: '0.85rem' }}>
          {error}
        </div>
      )}

      {ingestReport && (
        <div style={{ padding: '0.75rem 1rem', backgroundColor: 'rgba(16, 185, 129, 0.1)', border: '1px solid var(--success)', color: 'var(--success)', borderRadius: '6px', marginBottom: '1rem', fontSize: '0.85rem', display: 'flex', justifyContent: 'space-between' }}>
          <span>
            ✓ Ingested {ingestReport.sources_processed} sources: Created <strong>{ingestReport.nodes_created}</strong> nodes and <strong>{ingestReport.relationships_created}</strong> relationships in {ingestReport.elapsed_ms.toFixed(1)}ms.
          </span>
          <button onClick={() => setIngestReport(null)} style={{ background: 'none', border: 'none', color: 'inherit' }}>
            &times;
          </button>
        </div>
      )}

      {/* Main Canvas and Inspector Grid */}
      <div style={{ display: 'grid', gridTemplateColumns: selectedNode ? '1fr 320px' : '1fr', gap: '1rem', height: '550px' }}>
        <div
          ref={containerRef}
          style={{
            backgroundColor: '#0b0f19',
            borderRadius: '8px',
            border: '1px solid var(--border-color)',
            height: '100%',
            position: 'relative',
          }}
        >
          {loading && (
            <div style={{ position: 'absolute', top: '1rem', left: '1rem', color: 'var(--text-muted)', fontSize: '0.8rem', zIndex: 10 }}>
              Rendering graph...
            </div>
          )}
          {graphData && graphData.elements.nodes.length === 0 && !loading && (
            <div style={{ position: 'absolute', top: '50%', left: '50%', transform: 'translate(-50%, -50%)', textAlign: 'center', color: 'var(--text-muted)' }}>
              <p style={{ fontSize: '1rem', marginBottom: '0.5rem' }}>No graph nodes yet.</p>
              <p style={{ fontSize: '0.8rem' }}>
                Approve an ontology and click <strong>"Ingest Sources to Graph"</strong> above.
              </p>
            </div>
          )}
        </div>

        {/* Selected Node Inspector */}
        {selectedNode && (
          <div
            style={{
              backgroundColor: 'var(--bg-primary)',
              borderRadius: '8px',
              border: '1px solid var(--border-color)',
              padding: '1.25rem',
              display: 'flex',
              flexDirection: 'column',
              overflowY: 'auto',
            }}
          >
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: '1rem' }}>
              <div>
                <span style={{ fontSize: '0.75rem', padding: '0.15rem 0.5rem', backgroundColor: 'rgba(99, 102, 241, 0.2)', color: 'var(--accent-light)', borderRadius: '4px', fontWeight: 600 }}>
                  :{selectedNode.label}
                </span>
                <h4 style={{ fontSize: '1.1rem', fontWeight: 700, marginTop: '0.4rem' }}>
                  {selectedNode.name}
                </h4>
              </div>
              <button
                onClick={() => setSelectedNode(null)}
                style={{ background: 'none', border: 'none', color: 'var(--text-muted)', fontSize: '1.2rem', cursor: 'pointer' }}
              >
                &times;
              </button>
            </div>

            {/* Properties */}
            <div style={{ marginBottom: '1.25rem', flex: 1 }}>
              <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)', display: 'block', marginBottom: '0.4rem' }}>
                PROPERTIES
              </span>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '0.4rem', fontSize: '0.8rem' }}>
                {Object.entries(selectedNode.properties)
                  .filter(([k]) => !k.startsWith('_'))
                  .map(([k, v]) => (
                    <div key={k} style={{ padding: '0.4rem', backgroundColor: 'var(--bg-secondary)', borderRadius: '4px' }}>
                      <span style={{ color: 'var(--text-secondary)', display: 'block', fontSize: '0.7rem' }}>
                        {k}
                      </span>
                      <span style={{ fontFamily: 'monospace', color: 'var(--text-primary)' }}>
                        {String(v)}
                      </span>
                    </div>
                  ))}
              </div>
            </div>

            {/* Source Lineage & Provenance */}
            <div style={{ borderTop: '1px solid var(--border-color)', paddingTop: '0.75rem' }}>
              <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)', display: 'block', marginBottom: '0.4rem' }}>
                PROVENANCE LINEAGE
              </span>
              <div style={{ fontSize: '0.75rem', color: 'var(--text-secondary)', display: 'flex', flexDirection: 'column', gap: '0.3rem' }}>
                {selectedNode.properties._source_file && (
                  <div>
                    <strong style={{ color: 'var(--text-muted)' }}>Source: </strong>
                    {String(selectedNode.properties._source_file)}
                  </div>
                )}
                {selectedNode.properties._lineage_ref && (
                  <div>
                    <strong style={{ color: 'var(--text-muted)' }}>Ref: </strong>
                    <span style={{ fontFamily: 'monospace' }}>{String(selectedNode.properties._lineage_ref)}</span>
                  </div>
                )}
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
