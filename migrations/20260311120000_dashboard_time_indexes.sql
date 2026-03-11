-- Support time-window-driven dashboard and Cost Explorer reads.

CREATE INDEX IF NOT EXISTS idx_metrics_time_resource_metric
    ON metrics(seconds_from_now, resource_id, metric_name);

CREATE INDEX IF NOT EXISTS idx_cost_time_resource
    ON cost_records(seconds_from_now, resource_id);
