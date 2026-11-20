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
