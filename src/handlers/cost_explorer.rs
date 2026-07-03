use serde_json::{Value, json};

#[derive(Clone, Copy, Default)]
pub struct CostUsageMetricAmounts {
    pub unblended_cost: f64,
    pub usage_quantity: f64,
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
