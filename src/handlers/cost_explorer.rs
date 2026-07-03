use serde_json::{Value, json};

#[derive(Clone, Copy, Default)]
pub struct CostUsageMetricAmounts {
    pub unblended_cost: f64,
    pub usage_quantity: f64,
}

pub struct CostUsageGroup {
    pub key: String,
    pub amounts: CostUsageMetricAmounts,
}

pub fn time_bucket_json(
    start: String,
    end: String,
    total: CostUsageMetricAmounts,
    groups: Vec<CostUsageGroup>,
    metrics: Option<&Vec<String>>,
) -> Value {
    json!({
        "TimePeriod": {
            "Start": start,
            "End": end
        },
        "Total": metrics_json(total, metrics),
        "Groups": groups.into_iter().map(|group| {
            json!({
                "Keys": [group.key],
                "Metrics": metrics_json(group.amounts, metrics)
            })
        }).collect::<Vec<Value>>(),
        "Estimated": true
    })
}

pub fn cost_and_usage_response(
    group_definitions: Vec<Value>,
    results_by_time: Vec<Value>,
    include_next_page_token: bool,
    granularity: Option<&String>,
    requested_metrics: Option<&Vec<String>>,
) -> Value {
    let mut response = json!({
        "GroupDefinitions": group_definitions,
        "DimensionValueAttributes": [],
        "ResultsByTime": results_by_time
    });

    if include_next_page_token {
        response["NextPageToken"] = Value::Null;
    }

    if let Some(granularity) = granularity {
        response["Granularity"] = json!(granularity);
    }
    if let Some(metrics) = requested_metrics {
        response["RequestedMetrics"] = json!(metrics);
    }

    response
}

pub fn metrics_json(amounts: CostUsageMetricAmounts, metrics: Option<&Vec<String>>) -> Value {
    let mut metric_values = serde_json::Map::new();

    if requested_metric(metrics, "UnblendedCost") {
        metric_values.insert(
            "UnblendedCost".to_string(),
            json!({
                "Amount": format!("{:.2}", amounts.unblended_cost),
                "Unit": "USD"
            }),
        );
    }

    if requested_metric(metrics, "UsageQuantity") {
        metric_values.insert(
            "UsageQuantity".to_string(),
            json!({
                "Amount": format!("{:.4}", amounts.usage_quantity),
                "Unit": "N/A"
            }),
        );
    }

    Value::Object(metric_values)
}

fn requested_metric(metrics: Option<&Vec<String>>, metric: &str) -> bool {
    match metrics {
        Some(metrics) => metrics.iter().any(|requested| requested == metric),
        None => metric == "UnblendedCost",
    }
}
