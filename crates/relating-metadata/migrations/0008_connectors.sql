CREATE TABLE IF NOT EXISTS data_connectors (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    config_json TEXT NOT NULL,
    mapping_json TEXT NOT NULL DEFAULT '{}',
    watermark TEXT,
    last_sync TEXT,
    sync_hash TEXT,
    poll_interval_secs INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
    UNIQUE(project_id, name)
);

CREATE INDEX IF NOT EXISTS idx_data_connectors_project_id ON data_connectors(project_id);
