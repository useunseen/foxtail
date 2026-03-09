-- Initial schema for AWS Mock Data Service

CREATE TABLE resources (
    id TEXT PRIMARY KEY,
    resource_type TEXT NOT NULL,
    region TEXT NOT NULL,
    scenario TEXT NOT NULL,
    tags TEXT -- JSON string
);

CREATE TABLE metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    resource_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    metric_name TEXT NOT NULL,
    seconds_from_now INTEGER NOT NULL, -- Offset from current time (e.g., -3600 for 1 hour ago)
    value REAL NOT NULL,
    FOREIGN KEY (resource_id) REFERENCES resources(id) ON DELETE CASCADE
);

CREATE TABLE cost_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    resource_id TEXT NOT NULL,
    seconds_from_now INTEGER NOT NULL, -- Offset from current time (daily increments usually)
    amount REAL NOT NULL,
    currency TEXT DEFAULT 'USD',
    FOREIGN KEY (resource_id) REFERENCES resources(id) ON DELETE CASCADE
);

CREATE INDEX idx_metrics_resource_id ON metrics(resource_id);
CREATE INDEX idx_metrics_name ON metrics(namespace, metric_name);
CREATE INDEX idx_cost_resource_id ON cost_records(resource_id);
