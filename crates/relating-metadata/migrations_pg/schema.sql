-- Relating metadata schema for PostgreSQL (team/server deployments).
-- Ported 1:1 from migrations/0001..0005 (SQLite type names adjusted:
-- BLOB -> BYTEA, REAL -> DOUBLE PRECISION). Applied by `relating db
-- init-postgres` when provisioning a shared metadata database.

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    ai_config TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    job_type TEXT NOT NULL,
    status TEXT NOT NULL,
    progress DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_jobs_project_id ON jobs(project_id);
CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);

CREATE TABLE IF NOT EXISTS sources (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    filename TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    content_type TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    status TEXT NOT NULL,
    metadata TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_sources_project_id ON sources(project_id);
CREATE INDEX IF NOT EXISTS idx_sources_content_hash ON sources(content_hash);

CREATE TABLE IF NOT EXISTS ontologies (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL,
    schema_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    approved_at TEXT,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ontologies_project_id ON ontologies(project_id);

CREATE TABLE IF NOT EXISTS embedding_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    model_version TEXT,
    dimensions INTEGER NOT NULL,
    modality TEXT NOT NULL DEFAULT 'prose',
    created_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
    UNIQUE(project_id, name)
);

CREATE INDEX IF NOT EXISTS idx_embedding_profiles_project_id ON embedding_profiles(project_id);

CREATE TABLE IF NOT EXISTS chunks (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    metadata TEXT NOT NULL,
    embedding BYTEA NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY(profile_id) REFERENCES embedding_profiles(id) ON DELETE CASCADE,
    FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE,
    UNIQUE(profile_id, content_hash)
);

CREATE INDEX IF NOT EXISTS idx_chunks_project_id ON chunks(project_id);
CREATE INDEX IF NOT EXISTS idx_chunks_profile_id ON chunks(profile_id);
CREATE INDEX IF NOT EXISTS idx_chunks_source_id ON chunks(source_id);

CREATE TABLE IF NOT EXISTS evaluations (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    strategy TEXT NOT NULL,
    chat_model TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    metrics_version TEXT NOT NULL,
    metrics_json TEXT NOT NULL,
    questions_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_evaluations_project_id ON evaluations(project_id);

CREATE TABLE IF NOT EXISTS aura_connections (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    uri TEXT NOT NULL,
    username TEXT NOT NULL,
    database TEXT,
    auth_env_var TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
    UNIQUE(project_id, name)
);

CREATE INDEX IF NOT EXISTS idx_aura_connections_project_id ON aura_connections(project_id);

CREATE TABLE IF NOT EXISTS migrations (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    status TEXT NOT NULL,
    plan_json TEXT NOT NULL,
    progress_json TEXT NOT NULL,
    report_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY(connection_id) REFERENCES aura_connections(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_migrations_project_id ON migrations(project_id);
CREATE INDEX IF NOT EXISTS idx_migrations_status ON migrations(status);

CREATE TABLE IF NOT EXISTS schema_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
    UNIQUE(project_id, version)
);

CREATE INDEX IF NOT EXISTS idx_schema_revisions_project_id ON schema_revisions(project_id);

CREATE TABLE IF NOT EXISTS retrieval_configs (
    project_id TEXT PRIMARY KEY NOT NULL,
    default_strategy TEXT NOT NULL,
    default_top_k INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS sync_state (
    project_id TEXT PRIMARY KEY NOT NULL,
    migration_id TEXT,
    high_water_mark TEXT,
    last_sync TEXT,
    sync_mode TEXT,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY NOT NULL,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS identities (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    issuer TEXT NOT NULL,
    subject TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
    UNIQUE(issuer, subject)
);

CREATE INDEX IF NOT EXISTS idx_identities_user_id ON identities(user_id);

CREATE TABLE IF NOT EXISTS grants (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    project_id TEXT,
    role TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_grants_global_unique
    ON grants(user_id, role) WHERE project_id IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_grants_project_unique
    ON grants(user_id, project_id, role) WHERE project_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_grants_user_id ON grants(user_id);
CREATE INDEX IF NOT EXISTS idx_grants_project_id ON grants(project_id);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);

CREATE TABLE IF NOT EXISTS pending_logins (
    state TEXT PRIMARY KEY NOT NULL,
    nonce TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_log (
    seq BIGSERIAL PRIMARY KEY,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    resource TEXT NOT NULL,
    outcome TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT '',
    prev_hash TEXT NOT NULL,
    hash TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_log_created_at ON audit_log(created_at);
CREATE INDEX IF NOT EXISTS idx_audit_log_actor ON audit_log(actor);
