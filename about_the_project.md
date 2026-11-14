## Inspiration

Modern AI applications need more than vector search and flat document retrieval. They need connected knowledge, contextual understanding, relationships, and the ability to reason across complex data.

We were inspired by the potential of [Relate](https://relatedb.ai/) as a lightweight, developer-friendly graph database and the enterprise capabilities of Neo4j Aura. We wanted to bring these worlds together with AI, GraphRAG, and ontology engineering into one unified platform.

Our vision was to make graph-powered AI development accessible to every developer — from local experimentation to enterprise deployment — without requiring developers to build the entire infrastructure themselves.

This is how **Relating** was born: Relate + Intelligence.

## What it does

Relating is an AI-native, developer-first platform for graph development, GraphRAG engineering, ontology management, and intelligent agent integration.

Its core capabilities include:

- **Graph Development Workbench:** Create, manage, query, and visualize isolated graph projects using Relate.
- **AI-Powered Knowledge Engineering:** Transform documents, tables, structured datasets, and other sources into connected knowledge graphs through AI-assisted entity extraction, relationship discovery, and ontology generation.
- **GraphRAG:** Combine vector search, full-text search, graph traversal, and AI reasoning to deliver context-aware answers with source traceability.
- **Mandatory AI Service:** Integrate configurable local and hosted AI providers for reasoning, embeddings, extraction, reranking, and evaluation.
- **Neo4j Aura Migration:** Analyse compatibility, generate migration plans, execute approved transfers, and validate graph data in Neo4j Aura.
- **MCP Integration:** Allow developer tools and AI agents such as Claude Code, Codex, Cursor, and other compatible MCP clients to connect to Relating.
- **Developer-Friendly Interfaces:** Provide APIs, CLI integration, and extensible architecture for developers and enterprise teams.

Relating is designed to remain a complete graph development and GraphRAG platform even when users do not migrate to Neo4j Aura.

## How we built it

We chose a hybrid architecture combining Rust, React, TypeScript, and optional Python-based workers.

- **Rust + Axum + Tokio:** Backend services, graph orchestration, APIs, and application control plane.
- **Relate:** Embedded graph database for local-first graph storage and querying.
- **React + TypeScript:** Developer-focused web interface and graph workbench.
- **AI Provider Abstraction:** A mandatory AI service supporting configurable LLMs, embedding models, extraction, and evaluation.
- **Python:** Document processing and integration with the broader AI and machine learning ecosystem.
- **Cytoscape.js:** Interactive graph visualization.
- **SQLite / PostgreSQL:** Project metadata, configurations, users, and job management.
- **MCP:** Standardized connectivity between Relating and external AI coding agents.
- **Docker:** Reproducible local and self-hosted deployment.

The architecture separates graph storage, AI services, ingestion pipelines, application metadata, and external integrations. This makes it extensible without introducing unnecessary microservices or tightly coupling the platform to a single AI provider.

## Challenges we ran into

One of the biggest challenges was bringing together multiple technologies while keeping the developer experience simple.

Some of the key challenges include:

- Integrating an embedded graph database with a scalable application architecture.
- Supporting different data types, including documents, tables, structured records, and code, within a unified knowledge graph.
- Designing an ontology-generation and entity-resolution workflow that remains transparent and editable.
- Supporting multiple embedding models without mixing incompatible vector spaces.
- Maintaining source provenance across ingestion, graph construction, retrieval, and AI-generated responses.
- Bridging local graph development with Neo4j Aura while accounting for differences in Cypher capabilities, indexes, constraints, and data types.
- Designing secure MCP connectivity without exposing database credentials or unrestricted write access to external agents.
- Keeping the platform developer-friendly while addressing enterprise requirements such as isolation, governance, security, and auditability.

## Accomplishments that we're proud of

We established a unified product vision that connects graph database development, AI-powered knowledge engineering, GraphRAG, and agent integration.

Some of the most important accomplishments are:

- A domain-agnostic architecture that can work with different types of enterprise and developer data.
- A local-first approach that makes graph development accessible without requiring cloud infrastructure.
- A clear separation between graph storage, AI intelligence, and application services.
- A provider-independent AI architecture designed for both local and hosted models.
- An MCP-first integration strategy that brings graph knowledge closer to AI coding assistants and autonomous agents.
- A migration strategy connecting local Relate development with Neo4j Aura.
- An open-core product direction that supports individual developers as well as enterprise adoption.

Most importantly, Relating is designed not just to store connected data, but to make that data useful to developers, applications, and AI agents.

## What we learned

We learned that building effective GraphRAG applications requires much more than combining a vector database with an LLM.

Reliable knowledge engineering requires ontology management, entity resolution, provenance, hybrid retrieval, evaluation, and continuous validation.

We also recognized that developer adoption depends heavily on simplicity. Developers should be able to start locally, experiment with their own data, connect their preferred AI tools, and scale when needed.

Another important learning was that MCP can provide a common integration layer between graph knowledge systems and the rapidly evolving ecosystem of AI coding assistants and agents.

Finally, graph portability and interoperability must be designed into the architecture from the beginning rather than treated as a final migration task.

## What's next for Relating

Our immediate focus is to deliver a fully working MVP as quickly as possible, with a complete end-to-end developer workflow.

Our roadmap includes:

1. **Local Graph Workbench:** Project creation, Relate integration, graph visualization, and Cypher querying.
2. **AI Knowledge Engineering:** Document and structured-data ingestion, ontology generation, entity extraction, and relationship mapping.
3. **Complete GraphRAG Pipeline:** Multiple embedding profiles, hybrid retrieval, grounded AI responses, and evaluation.
4. **MCP Ecosystem:** Working integration with Claude Code, Codex, Cursor, and generic MCP clients.
5. **Neo4j Aura Migration:** Compatibility analysis, migration planning, controlled data transfer, and verification.
6. **Developer Ecosystem:** CLI, APIs, SDKs, Docker deployment, and extensible connectors.
7. **Enterprise Readiness:** Multi-user collaboration, RBAC, audit trails, secure deployments, and governance.

Our long-term vision is to make Relating a universal graph intelligence development environment — where developers can build knowledge graphs locally, connect AI agents seamlessly, and take their applications from experimentation to enterprise production.

**Relating — Connect your data. Understand your knowledge. Empower your agents.**