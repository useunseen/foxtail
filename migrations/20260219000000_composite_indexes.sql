-- Optimized composite index for high-frequency time-series queries
CREATE INDEX IF NOT EXISTS idx_metrics_lookup ON metrics(resource_id, metric_name, seconds_from_now);
CREATE INDEX IF NOT EXISTS idx_cost_lookup ON cost_records(resource_id, seconds_from_now);
