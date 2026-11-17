import React, { useEffect, useState } from 'react';
import { api } from '../api/client';
import {
  EmbeddingStatusResponse,
  EvaluationRun,
  QuestionAnswer,
  RetrievalStrategy,
  RetrievalTrace,
} from '../types';

interface RetrievalWorkbenchProps {
  projectId: string;
  hasSources: boolean;
}

export const RetrievalWorkbench: React.FC<RetrievalWorkbenchProps> = ({
  projectId,
  hasSources,
}) => {
  const [embedStatus, setEmbedStatus] = useState<EmbeddingStatusResponse | null>(null);
  const [embedding, setEmbedding] = useState(false);
  const [embedMessage, setEmbedMessage] = useState<string | null>(null);
  const [embedError, setEmbedError] = useState<string | null>(null);

  const [strategy, setStrategy] = useState<RetrievalStrategy>('hybrid');
  const [query, setQuery] = useState('');
  const [retrieving, setRetrieving] = useState(false);
  const [trace, setTrace] = useState<RetrievalTrace | null>(null);
  const [retrievalError, setRetrievalError] = useState<string | null>(null);

  const [question, setQuestion] = useState('');
  const [asking, setAsking] = useState(false);
  const [answer, setAnswer] = useState<QuestionAnswer | null>(null);
  const [questionError, setQuestionError] = useState<string | null>(null);
  const [showTrace, setShowTrace] = useState(false);

  const [evalInput, setEvalInput] = useState('');
  const [evaluating, setEvaluating] = useState(false);
  const [evalRun, setEvalRun] = useState<EvaluationRun | null>(null);
  const [evalHistory, setEvalHistory] = useState<EvaluationRun[]>([]);
  const [evalError, setEvalError] = useState<string | null>(null);

  const fetchEmbedStatus = async () => {
    try {
      const data = await api.getEmbeddingsStatus(projectId);
      setEmbedStatus(data);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setEmbedError(err.message);
      }
    }
  };

  const fetchEvalHistory = async () => {
    try {
      const runs = await api.listEvaluations(projectId);
      setEvalHistory(runs);
    } catch {
      // history is optional; ignore
    }
  };

  useEffect(() => {
    fetchEmbedStatus();
    fetchEvalHistory();
  }, [projectId]);

  const handleGenerateEmbeddings = async () => {
    setEmbedding(true);
    setEmbedMessage(null);
    setEmbedError(null);
    try {
      const res = await api.generateEmbeddings(projectId);
      setEmbedMessage(
        `Indexed ${res.chunks_created} new chunks (${res.chunks_skipped} already indexed) ` +
          `at ${res.dimensions} dims in ${res.elapsed_ms.toFixed(0)}ms.`
      );
      await fetchEmbedStatus();
    } catch (err: unknown) {
      if (err instanceof Error) {
        setEmbedError(err.message);
      } else {
        setEmbedError('Embedding generation failed');
      }
    } finally {
      setEmbedding(false);
    }
  };

  const handleRetrieve = async () => {
    if (!query.trim()) return;
    setRetrieving(true);
    setRetrievalError(null);
    setTrace(null);
    try {
      const res = await api.runRetrieval(projectId, query.trim(), strategy, 5);
      setTrace(res);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setRetrievalError(err.message);
      } else {
        setRetrievalError('Retrieval failed');
      }
    } finally {
      setRetrieving(false);
    }
  };

  const handleAsk = async () => {
    if (!question.trim()) return;
    setAsking(true);
    setQuestionError(null);
    setAnswer(null);
    try {
      const res = await api.askQuestion(projectId, question.trim(), strategy);
      setAnswer(res);
    } catch (err: unknown) {
      if (err instanceof Error) {
        setQuestionError(err.message);
      } else {
        setQuestionError('Question answering failed');
      }
    } finally {
      setAsking(false);
    }
  };

  const handleEvaluate = async () => {
    const questions = evalInput
      .split('\n')
      .map((q) => q.trim())
      .filter((q) => q.length > 0);
    if (questions.length === 0) return;
    setEvaluating(true);
    setEvalError(null);
    setEvalRun(null);
    try {
      const run = await api.createEvaluation(projectId, questions, strategy);
      setEvalRun(run);
      await fetchEvalHistory();
    } catch (err: unknown) {
      if (err instanceof Error) {
        setEvalError(err.message);
      } else {
        setEvalError('Evaluation failed');
      }
    } finally {
      setEvaluating(false);
    }
  };

  const hasIndex = (embedStatus?.profiles.length ?? 0) > 0;
  const totalChunks =
    embedStatus?.profiles.reduce((sum, p) => sum + p.chunk_count, 0) ?? 0;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '1.25rem' }}>
      {/* Embedding index */}
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
          <h3 style={{ fontSize: '1rem', fontWeight: 600 }}>Embedding Index</h3>
          <button
            onClick={handleGenerateEmbeddings}
            disabled={embedding || !hasSources}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.4rem 1rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              opacity: embedding || !hasSources ? 0.6 : 1,
            }}
          >
            {embedding ? 'Embedding...' : 'Generate Embeddings'}
          </button>
        </div>

        {!hasSources && (
          <p style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>
            Upload at least one source file first — embeddings are built from your sources.
          </p>
        )}

        {hasSources && !hasIndex && (
          <p style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>
            No embedding index yet. Generate embeddings to enable retrieval and grounded Q&A.
          </p>
        )}

        {hasIndex && embedStatus && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: '0.5rem' }}>
            {embedStatus.profiles.map((p) => (
              <div
                key={p.profile.id}
                style={{
                  display: 'flex',
                  gap: '1.5rem',
                  padding: '0.6rem 0.8rem',
                  backgroundColor: 'var(--bg-primary)',
                  borderRadius: '6px',
                  fontSize: '0.8rem',
                }}
              >
                <span>
                  <strong>{p.profile.name}</strong> ({p.profile.modality})
                </span>
                <span style={{ color: 'var(--text-secondary)' }}>
                  {p.profile.model} · {p.profile.dimensions} dims
                </span>
                <span style={{ color: 'var(--text-secondary)' }}>
                  <strong>{p.chunk_count}</strong> chunks
                </span>
              </div>
            ))}
            <p style={{ fontSize: '0.75rem', color: 'var(--text-muted)' }}>
              {totalChunks} chunks indexed. Retrieval only compares vectors within one
              compatible embedding space.
            </p>
          </div>
        )}

        {embedMessage && (
          <div
            style={{
              marginTop: '0.75rem',
              padding: '0.6rem 0.8rem',
              backgroundColor: 'rgba(16, 185, 129, 0.1)',
              border: '1px solid var(--success)',
              color: 'var(--success)',
              borderRadius: '6px',
              fontSize: '0.8rem',
            }}
          >
            {embedMessage}
          </div>
        )}

        {embedError && (
          <div
            style={{
              marginTop: '0.75rem',
              padding: '0.6rem 0.8rem',
              backgroundColor: 'rgba(239, 68, 68, 0.1)',
              border: '1px solid var(--danger)',
              color: 'var(--danger)',
              borderRadius: '6px',
              fontSize: '0.8rem',
            }}
          >
            {embedError}
          </div>
        )}
      </div>

      {/* Strategy selector */}
      <div style={{ display: 'flex', gap: '0.5rem', alignItems: 'center' }}>
        <span style={{ fontSize: '0.8rem', color: 'var(--text-muted)' }}>
          Retrieval strategy:
        </span>
        {(['vector', 'keyword', 'hybrid'] as RetrievalStrategy[]).map((s) => (
          <button
            key={s}
            type="button"
            onClick={() => setStrategy(s)}
            style={{
              padding: '0.35rem 0.9rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              borderRadius: '4px',
              border: '1px solid',
              borderColor: strategy === s ? 'var(--accent)' : 'var(--border-color)',
              backgroundColor:
                strategy === s ? 'rgba(99, 102, 241, 0.2)' : 'transparent',
              color: strategy === s ? 'var(--accent-light)' : 'var(--text-secondary)',
              textTransform: 'capitalize',
            }}
          >
            {s}
          </button>
        ))}
      </div>

      {/* Retrieval search */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.75rem' }}>
          Retrieval Search
        </h3>
        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '0.75rem' }}>
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleRetrieve();
            }}
            placeholder="Search indexed chunks..."
            style={{
              flex: 1,
              padding: '0.55rem 0.8rem',
              backgroundColor: 'var(--bg-primary)',
              border: '1px solid var(--border-color)',
              borderRadius: '6px',
              color: 'var(--text-primary)',
              fontSize: '0.85rem',
            }}
          />
          <button
            onClick={handleRetrieve}
            disabled={retrieving || !hasIndex}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.4rem 1.2rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              opacity: retrieving || !hasIndex ? 0.6 : 1,
            }}
          >
            {retrieving ? 'Searching...' : 'Search'}
          </button>
        </div>

        {retrievalError && (
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
            {retrievalError}
          </div>
        )}

        {trace && (
          <div>
            <p style={{ fontSize: '0.75rem', color: 'var(--text-muted)', marginBottom: '0.5rem' }}>
              {trace.passages.length} passages via {trace.strategy} · {trace.embedding_model} ·{' '}
              {trace.latency_ms.toFixed(1)}ms
            </p>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '0.5rem' }}>
              {trace.passages.map((p) => (
                <div
                  key={p.chunk_id}
                  style={{
                    padding: '0.6rem 0.8rem',
                    backgroundColor: 'var(--bg-primary)',
                    borderRadius: '6px',
                    fontSize: '0.8rem',
                  }}
                >
                  <div
                    style={{
                      display: 'flex',
                      justifyContent: 'space-between',
                      marginBottom: '0.3rem',
                      fontSize: '0.75rem',
                      color: 'var(--text-muted)',
                    }}
                  >
                    <span>
                      #{p.rank} · {p.filename}
                    </span>
                    <span>score {p.score.toFixed(3)}</span>
                  </div>
                  <pre
                    style={{
                      whiteSpace: 'pre-wrap',
                      fontFamily: 'inherit',
                      color: 'var(--text-primary)',
                      margin: 0,
                    }}
                  >
                    {p.content.length > 500 ? `${p.content.slice(0, 500)}...` : p.content}
                  </pre>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Grounded Q&A */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.75rem' }}>
          Grounded Q&A
        </h3>
        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '0.75rem' }}>
          <input
            type="text"
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleAsk();
            }}
            placeholder="Ask a question about your sources..."
            style={{
              flex: 1,
              padding: '0.55rem 0.8rem',
              backgroundColor: 'var(--bg-primary)',
              border: '1px solid var(--border-color)',
              borderRadius: '6px',
              color: 'var(--text-primary)',
              fontSize: '0.85rem',
            }}
          />
          <button
            onClick={handleAsk}
            disabled={asking}
            style={{
              backgroundColor: 'var(--accent)',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              padding: '0.4rem 1.2rem',
              fontSize: '0.8rem',
              fontWeight: 600,
              opacity: asking ? 0.6 : 1,
            }}
          >
            {asking ? 'Answering...' : 'Ask'}
          </button>
        </div>

        {questionError && (
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
            {questionError}
          </div>
        )}

        {answer && (
          <div>
            <div
              style={{
                display: 'inline-block',
                padding: '0.2rem 0.6rem',
                borderRadius: '4px',
                fontSize: '0.75rem',
                fontWeight: 600,
                marginBottom: '0.5rem',
                backgroundColor: answer.grounded
                  ? 'rgba(16, 185, 129, 0.15)'
                  : 'rgba(239, 68, 68, 0.15)',
                color: answer.grounded ? 'var(--success)' : 'var(--danger)',
              }}
            >
              {answer.grounded ? 'Grounded in evidence' : 'Not grounded — no evidence found'}
            </div>
            <p
              style={{
                fontSize: '0.9rem',
                color: 'var(--text-primary)',
                marginBottom: '0.75rem',
                whiteSpace: 'pre-wrap',
              }}
            >
              {answer.answer}
            </p>

            {answer.citations.length > 0 && (
              <div style={{ marginBottom: '0.75rem' }}>
                <span
                  style={{
                    fontSize: '0.75rem',
                    color: 'var(--text-muted)',
                    display: 'block',
                    marginBottom: '0.4rem',
                  }}
                >
                  CITATIONS
                </span>
                <div style={{ display: 'flex', flexDirection: 'column', gap: '0.4rem' }}>
                  {answer.citations.map((c) => (
                    <div
                      key={c.index}
                      style={{
                        padding: '0.5rem 0.7rem',
                        backgroundColor: 'var(--bg-primary)',
                        borderRadius: '6px',
                        fontSize: '0.8rem',
                      }}
                    >
                      <span style={{ color: 'var(--accent-light)', fontWeight: 600 }}>
                        [{c.index}]
                      </span>{' '}
                      <span style={{ color: 'var(--text-muted)' }}>{c.filename} — </span>
                      <span style={{ color: 'var(--text-secondary)' }}>{c.excerpt}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}

            <button
              type="button"
              onClick={() => setShowTrace(!showTrace)}
              style={{
                background: 'none',
                border: 'none',
                color: 'var(--text-muted)',
                fontSize: '0.75rem',
                padding: 0,
              }}
            >
              {showTrace ? 'Hide retrieval trace' : 'Show retrieval trace'} (
              {answer.trace.latency_ms.toFixed(1)}ms · {answer.trace.embedding_model} ·{' '}
              {answer.prompt_version})
            </button>

            {showTrace && (
              <div
                style={{
                  marginTop: '0.5rem',
                  display: 'flex',
                  flexDirection: 'column',
                  gap: '0.4rem',
                }}
              >
                {answer.trace.passages.map((p) => (
                  <div
                    key={p.chunk_id}
                    style={{
                      padding: '0.5rem 0.7rem',
                      backgroundColor: 'var(--bg-primary)',
                      borderRadius: '6px',
                      fontSize: '0.75rem',
                      color: 'var(--text-secondary)',
                    }}
                  >
                    #{p.rank} · {p.filename} · score {p.score.toFixed(3)}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Evaluation */}
      <div
        style={{
          backgroundColor: 'var(--bg-secondary)',
          borderRadius: '8px',
          border: '1px solid var(--border-color)',
          padding: '1.25rem',
        }}
      >
        <h3 style={{ fontSize: '1rem', fontWeight: 600, marginBottom: '0.75rem' }}>
          Retrieval Evaluation
        </h3>
        <textarea
          value={evalInput}
          onChange={(e) => setEvalInput(e.target.value)}
          placeholder={'One question per line, e.g.\nWho is Alice?\nWhich department has most staff?'}
          rows={3}
          style={{
            width: '100%',
            padding: '0.6rem 0.8rem',
            backgroundColor: 'var(--bg-primary)',
            border: '1px solid var(--border-color)',
            borderRadius: '6px',
            color: 'var(--text-primary)',
            fontSize: '0.85rem',
            marginBottom: '0.75rem',
          }}
        />
        <button
          onClick={handleEvaluate}
          disabled={evaluating || !hasIndex}
          style={{
            backgroundColor: 'var(--accent)',
            color: '#fff',
            border: 'none',
            borderRadius: '6px',
            padding: '0.4rem 1.2rem',
            fontSize: '0.8rem',
            fontWeight: 600,
            opacity: evaluating || !hasIndex ? 0.6 : 1,
            marginBottom: '0.75rem',
          }}
        >
          {evaluating ? 'Evaluating...' : 'Run Evaluation'}
        </button>

        {evalError && (
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
            {evalError}
          </div>
        )}

        {evalRun && (
          <div style={{ marginBottom: '0.75rem' }}>
            <div
              style={{
                display: 'grid',
                gridTemplateColumns: '1fr 1fr 1fr 1fr',
                gap: '0.5rem',
                marginBottom: '0.75rem',
              }}
            >
              {[
                { label: 'QUESTIONS', value: String(evalRun.metrics.question_count) },
                {
                  label: 'GROUNDED RATE',
                  value: `${(evalRun.metrics.grounded_rate * 100).toFixed(0)}%`,
                },
                {
                  label: 'CITATION PRECISION',
                  value: evalRun.metrics.mean_citation_precision.toFixed(2),
                },
                {
                  label: 'MEAN LATENCY',
                  value: `${evalRun.metrics.mean_latency_ms.toFixed(0)}ms`,
                },
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
                  <span
                    style={{
                      color: 'var(--text-muted)',
                      display: 'block',
                      fontSize: '0.7rem',
                    }}
                  >
                    {m.label}
                  </span>
                  <span style={{ fontWeight: 700, fontSize: '1rem' }}>{m.value}</span>
                </div>
              ))}
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '0.4rem' }}>
              {evalRun.questions.map((q, idx) => (
                <div
                  key={idx}
                  style={{
                    padding: '0.5rem 0.7rem',
                    backgroundColor: 'var(--bg-primary)',
                    borderRadius: '6px',
                    fontSize: '0.75rem',
                    color: 'var(--text-secondary)',
                  }}
                >
                  <span style={{ color: q.grounded ? 'var(--success)' : 'var(--danger)' }}>
                    {q.grounded ? '✓' : '✕'}
                  </span>{' '}
                  {q.question} — precision {q.citation_precision.toFixed(2)} ·{' '}
                  {(q.retrieval_latency_ms + q.answer_latency_ms).toFixed(0)}ms
                </div>
              ))}
            </div>
          </div>
        )}

        {evalHistory.length > 0 && (
          <div>
            <span
              style={{
                fontSize: '0.75rem',
                color: 'var(--text-muted)',
                display: 'block',
                marginBottom: '0.4rem',
              }}
            >
              PAST RUNS
            </span>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '0.3rem' }}>
              {evalHistory.slice(0, 5).map((run) => (
                <button
                  key={run.id}
                  type="button"
                  onClick={() => setEvalRun(run)}
                  style={{
                    textAlign: 'left',
                    padding: '0.4rem 0.7rem',
                    backgroundColor: 'var(--bg-primary)',
                    border: '1px solid var(--border-color)',
                    borderRadius: '6px',
                    fontSize: '0.75rem',
                    color: 'var(--text-secondary)',
                  }}
                >
                  {run.metrics.question_count} questions ·{' '}
                  {(run.metrics.grounded_rate * 100).toFixed(0)}% grounded · {run.strategy} ·{' '}
                  {new Date(run.created_at).toLocaleString()}
                </button>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
