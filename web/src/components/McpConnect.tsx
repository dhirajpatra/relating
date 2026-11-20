import React, { useState } from 'react';
import { api } from '../api/client';
import { McpHealth } from '../types';

interface McpConnectProps {
  projectId: string;
  projectName: string;
}

export const McpConnect: React.FC<McpConnectProps> = ({ projectId, projectName }) => {
  const [health, setHealth] = useState<McpHealth | null>(null);
  const [testing, setTesting] = useState(false);
  const [testError, setTestError] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);

  const handleTest = async () => {
    setTesting(true);
    setTestError(null);
    try {
      const data = await api.mcpHealth();
      setHealth(data);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setTestError(err.message);
      } else {
        setTestError('MCP health check failed');
      }
    } finally {
      setTesting(false);
    }
  };

  const handleCopy = async (key: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(key);
      setTimeout(() => setCopied(null), 2000);
    } catch {
      setCopied(null);
    }
  };

  const stdioCommand = `relating-mcp --stdio --project ${projectId}`;

  const jsonConfig = JSON.stringify(
    {
      mcpServers: {
        relating: {
          command: 'relating-mcp',
          args: ['--stdio', '--project', projectId],
          env: {
            RELATING_DATA_DIR: '<path-to-relating>/data',
          },
        },
      },
    },
    null,
    2
  );

  const claudeCommand = `claude mcp add relating -- relating-mcp --stdio --project ${projectId}`;
  const codexCommand = `codex mcp add relating -- relating-mcp --stdio --project ${projectId}`;

  const snippets = [
    {
      key: 'claude',
      title: 'Claude Code',
      hint: 'Run in a terminal (relating-mcp must be on PATH).',
      text: claudeCommand,
    },
    {
      key: 'codex',
      title: 'Codex',
      hint: 'Run in a terminal (relating-mcp must be on PATH).',
      text: codexCommand,
    },
    {
      key: 'cursor',
      title: 'Cursor / generic MCP client',
      hint: 'Add to the MCP settings JSON; replace the data directory.',
      text: jsonConfig,
    },
  ];

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
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            marginBottom: '0.75rem',
          }}
        >
          <h3 style={{ fontSize: '1rem', fontWeight: 600 }}>
            Connect an agent to “{projectName}”
          </h3>
          <button
            onClick={handleTest}
            disabled={testing}
            style={{
              backgroundColor: 'transparent',
              border: '1px solid var(--border-color)',
              color: 'var(--text-primary)',
              borderRadius: '6px',
              padding: '0.4rem 0.9rem',
              fontSize: '0.8rem',
              fontWeight: 600,
            }}
          >
            {testing ? 'Testing...' : 'Test MCP endpoint'}
          </button>
        </div>

        <p style={{ fontSize: '0.85rem', color: 'var(--text-secondary)', marginBottom: '0.75rem' }}>
          Agents connect to this project only — tools are read-only and scoped to the
          granted project. Build the binary once with{' '}
          <code style={{ fontFamily: 'monospace' }}>cargo install --path crates/relating-mcp</code>{' '}
          so <code style={{ fontFamily: 'monospace' }}>relating-mcp</code> is on PATH.
        </p>

        {health && (
          <div
            style={{
              padding: '0.6rem 0.8rem',
              backgroundColor: 'rgba(16, 185, 129, 0.1)',
              border: '1px solid var(--success)',
              color: 'var(--success)',
              borderRadius: '6px',
              fontSize: '0.8rem',
              marginBottom: '0.75rem',
            }}
          >
            MCP {health.mode} server operational: {health.tools} tools · protocol{' '}
            {health.protocol_version}.
          </div>
        )}

        {testError && (
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
            {testError}
          </div>
        )}

        <div
          style={{
            padding: '0.6rem 0.8rem',
            backgroundColor: 'var(--bg-primary)',
            borderRadius: '6px',
            fontSize: '0.8rem',
            fontFamily: 'monospace',
            color: 'var(--accent-light)',
            marginBottom: '1rem',
          }}
        >
          {stdioCommand}
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: '0.75rem' }}>
          {snippets.map((s) => (
            <div key={s.key}>
              <div
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'center',
                  marginBottom: '0.3rem',
                }}
              >
                <span style={{ fontSize: '0.85rem', fontWeight: 600 }}>{s.title}</span>
                <button
                  type="button"
                  onClick={() => handleCopy(s.key, s.text)}
                  style={{
                    background: 'none',
                    border: '1px solid var(--border-color)',
                    color: 'var(--text-secondary)',
                    borderRadius: '4px',
                    fontSize: '0.75rem',
                    padding: '0.2rem 0.6rem',
                  }}
                >
                  {copied === s.key ? 'Copied ✓' : 'Copy'}
                </button>
              </div>
              <pre
                style={{
                  padding: '0.6rem 0.8rem',
                  backgroundColor: 'var(--bg-primary)',
                  borderRadius: '6px',
                  fontSize: '0.75rem',
                  fontFamily: 'monospace',
                  color: 'var(--text-primary)',
                  whiteSpace: 'pre-wrap',
                  margin: 0,
                  marginBottom: '0.2rem',
                }}
              >
                {s.text}
              </pre>
              <span style={{ fontSize: '0.75rem', color: 'var(--text-muted)' }}>{s.hint}</span>
            </div>
          ))}
        </div>
      </div>

      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.5rem' }}>
          HTTP transport (self-hosted)
        </h3>
        <p style={{ fontSize: '0.85rem', color: 'var(--text-secondary)', marginBottom: '0.5rem' }}>
          Remote agents POST JSON-RPC to <code style={{ fontFamily: 'monospace' }}>/mcp</code> with
          a bearer token and the project header. The transport stays disabled until the
          server is started with <code style={{ fontFamily: 'monospace' }}>RELATING_MCP_TOKEN</code> set.
        </p>
        <pre
          style={{
            padding: '0.6rem 0.8rem',
            backgroundColor: 'var(--bg-primary)',
            borderRadius: '6px',
            fontSize: '0.75rem',
            fontFamily: 'monospace',
            color: 'var(--text-primary)',
            whiteSpace: 'pre-wrap',
            margin: 0,
          }}
        >
          {`POST /mcp
Authorization: Bearer <RELATING_MCP_TOKEN>
x-relating-project: ${projectId}
Content-Type: application/json

{"jsonrpc": "2.0", "id": 1, "method": "tools/list"}`}
        </pre>
      </div>
    </div>
  );
};
