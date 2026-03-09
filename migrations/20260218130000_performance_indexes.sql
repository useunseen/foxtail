-- Performance indexes for metrics and cost records

-- Composite index for fast lookups by resource and time range
CREATE INDEX idx_metrics_resource_name_time ON metrics(resource_id, metric_name, seconds_from_now);

-- Index for cost record time lookups
CREATE INDEX idx_cost_resource_time ON cost_records(resource_id, seconds_from_now);

-- Index for pruning and general resource lookups
CREATE INDEX idx_resources_type ON resources(resource_type);
