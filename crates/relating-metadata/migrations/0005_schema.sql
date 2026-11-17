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
