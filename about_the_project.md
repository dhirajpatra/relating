## Inspiration

Modern AI applications need more than vector search and flat document retrieval. They need connected knowledge, contextual understanding, relationships, and the ability to reason across complex data.

We were inspired by the potential of [Relate](https://relatedb.ai/) as a lightweight, developer-friendly graph database and the enterprise capabilities of Neo4j Aura. We wanted to bring these worlds together with AI, GraphRAG, and ontology engineering into one unified platform.

Our vision was to make graph-powered AI development accessible to every developer — from local experimentation to enterprise deployment — without requiring developers to rewrite queries or build the entire infrastructure themselves.

This is how **Relating** was born: Relate + Intelligence.

## What it does

Relating is an AI-native, developer-first platform for graph development, GraphRAG engineering, ontology management, Graph DevOps, and intelligent agent integration.

Its core capabilities include:

- **Graph Development Workbench:** Create, manage, query, and visualize isolated graph projects using Relate.
- **AI-Powered Knowledge Engineering:** Ingest CSV, JSON, plain text, PDF, and DOCX files with full source lineage, transforming them into connected knowledge graphs through AI-assisted entity extraction, relationship discovery, and approval-gated ontology generation.
- **Hybrid GraphRAG & Evaluation:** Combine vector search, full-text search, and Cypher graph traversal via Reciprocal Rank Fusion (RRF) to deliver grounded answers with traceable citations and evaluation metrics:
  $$\text{RRF}(d) = \sum_{r \in R} \frac{1}{k + \text{rank}_r(d)}$$
- **Mandatory AI Service:** Integrate configurable local (Ollama) and hosted (OpenAI-compatible) AI providers for reasoning, embeddings, extraction, reranking, and evaluation.
- **Neo4j Aura Migration & Zero-Rewrite Backend Flip:** Run pre-flight compatibility, vector-index, and tier-capacity fit checks, preview dry-run plans, execute idempotent merge-upsert transfers over Bolt, verify counts, content hashes, degrees, and constraints, roll back exactly what a migration created, and flip applications between Relate and Neo4j Aura without rewriting a single query.
- **Graph DevOps & Portability Engine:** Manage schema-as-code revisions and diffs, scan repositories with the Portability Compiler, run CI/CD validation gates via the `relating` CLI and GitHub Actions, replay shadow traffic across backends, generate synthetic fixtures, and get sizing/performance recommendations.
- **MCP Integration:** Expose project-scoped read-only tools (and policy-gated write tools) over stdio and Streamable HTTP so developer tools and AI agents such as Claude Code, Codex, and Cursor can connect safely.

Relating is designed to remain a complete graph development and GraphRAG platform even when users never migrate to Neo4j Aura.

## How we built it

We built a modular monolith combining Rust, React, and TypeScript across 16 specialized crates and a 12-tab workbench:

- **Rust + Axum + Tokio:** Backend control plane, REST API, policy enforcement, shadow replay, portability compiler, and Bolt migration engine (`relating-core`, `relating-server`, `relating-migrate`, `relating-portability`, `relating-shadow`, `relating-advisor`).
- **Relate & `GraphBackend` Abstraction:** Embedded project-scoped Relate stores (`.rl` / directory stores) behind a unified `GraphBackend` interface (`RelateBackend` and `Neo4jBackend`) that enables seamless Relate $\leftrightarrow$ Aura cutover.
- **React + TypeScript + Cytoscape.js:** A 12-tab developer workbench covering Sources, Ontology, Graph Explorer, Cypher Console, AI Service, Retrieval & Q&A, Agents, Aura Migration, Schema & Contracts, Portability, Shadow & Debug, and Test Data & Advisor.
- **AI Provider Abstraction (`relating-ai`):** Required provider adapters for chat, embeddings, structured extraction, reranking, and LLM-as-a-judge evaluation, isolating embedding profiles by model ID, modality, and vector dimension $d$.
- **Rust ingestion pipelines (`relating-ingest`):** Native parsing and chunking for PDF, DOCX, CSV, JSON, and unstructured text with row/chunk provenance. (SKILL.md reserves an optional isolated-Python-worker interface for future AI/data-library needs; nothing in the current pipeline shells out to Python.)
- **SQLite & PostgreSQL Metadata:** Local SQLite persistence alongside team PostgreSQL schema provisioning and validation, paired with runtime secret resolution via environment variables, mounted secret files, or the OS keychain.
- **MCP & CLI (`relating-mcp`, `relating-cli`):** Standardized stdio/HTTP MCP server and headless CLI for CI/CD gates, contract tests, and debugging.
- **Docker:** One-command local startup (`./start.sh`) and reproducible `docker compose` deployment.

## Challenges we ran into

Building an end-to-end path from local embedded graph prototyping to cloud Aura production surfaced several deep engineering challenges:

- **Bridging Relate and Neo4j Aura Without Query Rewrites:** Embedded Relate and Neo4j Aura differ in index DDL, constraint syntax, vector index rules, and tier memory limits. We had to build a versioned capability registry (`SUPPORTED`, `RECREATE`, `TRANSFORM`, `UNSUPPORTED`, `REQUIRES_MANUAL_REVIEW`), a Portability Compiler, and pre-flight Aura vector-index fit checks so developers catch incompatibilities before deployment.
- **Preventing Vector Space Corruption:** Supporting multiple embedding models across prose, tables, and code risked mixing incompatible embeddings. We enforced strict profile isolation so cosine similarity $\text{sim}(\mathbf{u}, \mathbf{v}) = \frac{\mathbf{u} \cdot \mathbf{v}}{\|\mathbf{u}\| \|\mathbf{v}\|}$ is only ever computed within matching `(provider, model_id, dimensions)` spaces.
- **Safe, Repeatable Cloud Migrations:** Startups rarely perform a single one-time cutover; they iterate locally and sync updates to staging/production. Designing resumable, idempotent merge-upsert batch transfers over Bolt—complete with dry-run gates, explicit disclosure of target mutations, count/hash/degree/sample verification, tagged rollback, and audit manifests—required strict state boundaries.
- **Multi-Format Ingestion with Provenance:** Parsing CSV, JSON, PDF, and DOCX files while maintaining exact source-to-node lineage and transparent, editable ontology proposals required decoupling document parsing from schema-constrained AI extraction.
- **Secure Agent Access & Team Secret Hygiene:** Exposing graph tools to external coding agents over MCP without leaking Aura credentials or allowing unrestricted graph mutations meant enforcing project-scoped read-only defaults, bounded query limits, and runtime secret resolution via OS keychains and mounted files rather than committing secrets.

## Accomplishments that we're proud of

We delivered all core milestones (A–E) and releases (R1–R6), turning Relating into a working end-to-end GraphRAG and Graph DevOps platform:

- **Zero-Rewrite Relate $\leftrightarrow$ Aura Backend Flip:** Developers prototype locally against Relate and switch production traffic to Neo4j Aura through a unified backend abstraction without rewriting application queries.
- **Full Graph DevOps Lifecycle:** Beyond basic CRUD and visualization, Relating ships with schema-as-code diffing, a Cypher Portability Compiler, GitHub Actions CI/CD gates, dual-backend shadow testing, a GraphRAG trace debugger, synthetic data generation, and a performance/sizing advisor.
- **Complete Multi-Format Ingestion & Hybrid GraphRAG:** End-to-end ingestion across CSV, JSON, text, PDF, and DOCX with human-in-the-loop ontology approval, multi-profile embeddings, hybrid RRF retrieval, and grounded Q&A with citations.
- **Verified Aura Migration with Pre-Flight Fit Checks:** Automated Aura tier and vector-index compatibility validation, dry-run simulation, idempotent merge-upsert transfer over Bolt, and post-migration verification reports.
- **MCP-First & Team-Ready Architecture:** Out-of-the-box MCP connectivity for Claude Code, Codex, and Cursor, backed by PostgreSQL team metadata provisioning and OS keychain / secret-file credential management.

## What we learned

- **Graph Portability Must Be Built In From Day One:** Treating cloud migration as an afterthought leads to painful rewrites. Designing `GraphBackend` abstractions, schema-as-code, and portability checks into the core architecture makes local-to-cloud graduation predictable.
- **GraphRAG Needs Engineering Rigor Beyond Vector Search:** Reliable answers depend on approval-gated ontologies, entity resolution, strict embedding-space isolation, hybrid retrieval, and trace-level evaluation.
- **Developer Adoption Hinges on Day-One Simplicity:** Letting a developer run `./start.sh` with zero cloud accounts, experiment on local PDFs and CSVs, connect their favorite AI coding agent via MCP, and only spin up Neo4j Aura when ready removes the biggest barrier to graph adoption.
- **Transparency Builds Trust in AI & Migrations:** Whether an AI is proposing an ontology or a migration engine is writing batches to Neo4j Aura, explicit dry runs, merge-upsert disclosures, and approval gates are essential for developer confidence.

## What's next for Relating

With the core platform, Graph DevOps tooling, and Relate-to-Aura migration pipeline complete, our next steps focus on enterprise scale and ecosystem expansion:

1. **Automated Change-Data-Capture (CDC) & Continuous Sync:** Evolving our idempotent merge-upsert transfers into automated incremental synchronization with conflict reconciliation between local/staging graphs and Aura.
2. **Enterprise Identity & Vault Integration:** Adding OIDC single sign-on, granular role-based access control (RBAC), exportable compliance audit logs, and HashiCorp Vault / cloud KMS secret backends.
3. **Expanded Data Connectors & Language SDKs:** Adding native SQL database and REST API connectors alongside ergonomic Python and TypeScript client SDKs.
4. **Desktop & Kubernetes Packaging:** Shipping standalone desktop builds via Tauri and Helm charts for multi-tenant Kubernetes deployments.

**Relating — Connect your data. Understand your knowledge. Empower your agents.**