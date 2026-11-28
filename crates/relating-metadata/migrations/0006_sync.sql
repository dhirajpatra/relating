CREATE TABLE IF NOT EXISTS sync_state (
    project_id TEXT PRIMARY KEY NOT NULL,
    migration_id TEXT,
    high_water_mark TEXT,
    last_sync TEXT,
    sync_mode TEXT,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);
