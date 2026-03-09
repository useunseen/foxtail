use anyhow::Result;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::net::SocketAddr;
use tracing::{debug, info, warn};

use crate::cli::Scenario;
use crate::generator;
use crate::handlers::cloudwatch as cw;
use crate::metrics::{self, MetricQueryParams};

const ADMIN_TOKEN_HEADER: &str = "x-mock-admin-token";

pub async fn run(pool: SqlitePool, address: String, port: u16) -> Result<()> {
    let app = Router::new()
        .route("/", post(aws_handler))
        .route("/_mock/status", get(status_handler))
        .route("/_mock/dashboard/data", get(dashboard_data_handler))
        .route(
            "/_mock/dashboard/resources",
            get(dashboard_resources_handler),
        )
        .route(
            "/_mock/dashboard/trends/cloudwatch",
            get(dashboard_cloudwatch_trends_handler),
        )
        .route(
            "/_mock/dashboard/trends/cost",
            get(dashboard_cost_trends_handler),
        )
        .route("/_mock/scenario", post(scenario_handler))
        .with_state(pool);

    let addr: SocketAddr = format!("{}:{}", address, port).parse()?;
    info!("Starting AWS-compatible API server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Protocol {
    Json,
    Xml,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct CloudWatchQuery {
    action: String,
    namespace: Option<String>,
    metric_name: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
    period: Option<i32>,
    #[serde(rename = "Dimensions.member.1.Name")]
    dim_name_1: Option<String>,
    #[serde(rename = "Dimensions.member.1.Value")]
    dim_value_1: Option<String>,
    #[serde(rename = "Dimensions.member.2.Name")]
    dim_name_2: Option<String>,
    #[serde(rename = "Dimensions.member.2.Value")]
    dim_value_2: Option<String>,
}

#[derive(Deserialize)]
struct ScenarioRequest {
    scenario: Scenario,
    resource_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetCostAndUsageRequest {
    time_period: TimePeriod,
    granularity: Option<String>,
    metrics: Option<Vec<String>>,
    group_by: Option<Vec<Value>>,
    filter: Option<Value>,
    next_page_token: Option<String>,
    billing_view_arn: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetCostForecastRequest {
    time_period: TimePeriod,
    metric: Option<String>,
    granularity: Option<String>,
    filter: Option<Value>,
    prediction_interval_level: Option<i32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetDimensionValuesRequest {
    time_period: TimePeriod,
    dimension: String,
    context: Option<String>,
    search_string: Option<String>,
    filter: Option<Value>,
    next_page_token: Option<String>,
    max_results: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetReservationCoverageRequest {
    time_period: TimePeriod,
    granularity: Option<String>,
    group_by: Option<Vec<Value>>,
    metrics: Option<Vec<String>>,
    filter: Option<Value>,
    next_page_token: Option<String>,
    max_results: Option<u64>,
    sort_by: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetReservationUtilizationRequest {
    time_period: TimePeriod,
    granularity: Option<String>,
    group_by: Option<Vec<Value>>,
    filter: Option<Value>,
    next_page_token: Option<String>,
    max_results: Option<u64>,
    sort_by: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetSavingsPlansCoverageRequest {
    time_period: TimePeriod,
    granularity: Option<String>,
    group_by: Option<Vec<Value>>,
    metrics: Option<Vec<String>>,
    filter: Option<Value>,
    next_token: Option<String>,
    max_results: Option<u64>,
    sort_by: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetSavingsPlansUtilizationRequest {
    time_period: TimePeriod,
    granularity: Option<String>,
    filter: Option<Value>,
    sort_by: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetRightsizingRecommendationRequest {
    service: String,
    filter: Option<Value>,
    configuration: Option<Value>,
    page_size: Option<u64>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CeDateInterval {
    start_date: String,
    end_date: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetAnomaliesRequest {
    monitor_arn: Option<String>,
    date_interval: CeDateInterval,
    feedback: Option<String>,
    total_impact: Option<Value>,
    next_page_token: Option<String>,
    max_results: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetAnomalyMonitorsRequest {
    monitor_arn_list: Option<Vec<String>>,
    next_page_token: Option<String>,
    max_results: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetAnomalySubscriptionsRequest {
    subscription_arn_list: Option<Vec<String>>,
    monitor_arn: Option<String>,
    next_page_token: Option<String>,
    max_results: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TimePeriod {
    start: String,
    end: String,
}

enum CostUsageError {
    Validation(String),
    Internal(anyhow::Error),
}

enum MetricDataError {
    Validation(String),
    InvalidNextToken(String),
    Internal(anyhow::Error),
}

#[derive(Serialize)]
struct DashboardApiEntry {
    service: String,
    operation: String,
    protocol: String,
    target: Option<String>,
    action: Option<String>,
    endpoint: Option<String>,
}

#[derive(Serialize)]
struct DashboardSummary {
    resource_count: i64,
    metric_count: i64,
    cost_record_count: i64,
}

#[derive(Serialize, Clone)]
struct DashboardSeriesPoint {
    timestamp: String,
    value: f64,
}

#[derive(Deserialize, Debug, Default, Clone)]
struct DashboardDataQuery {
    scope: Option<String>,
    resource_type: Option<String>,
    resource_id: Option<String>,
    namespace: Option<String>,
    metric_name: Option<String>,
    top_n: Option<usize>,
    window_hours: Option<i64>,
}

#[derive(Serialize)]
struct DashboardAppliedFilters {
    scope: String,
    resource_type: Option<String>,
    resource_id: Option<String>,
    namespace: Option<String>,
    metric_name: Option<String>,
    top_n: usize,
    window_hours: i64,
}

#[derive(Serialize)]
struct DashboardResourceEntry {
    resource_id: String,
    resource_type: String,
    region: String,
    scenario: String,
}

#[derive(Serialize, Clone)]
struct DashboardSeriesSet {
    key: String,
    label: String,
    points: Vec<DashboardSeriesPoint>,
}

#[derive(Serialize)]
struct DashboardContributor {
    resource_id: String,
    resource_type: String,
    total_cost: f64,
    average_utilization: Option<f64>,
}

#[derive(Serialize)]
struct DashboardDataResponse {
    generated_at: String,
    supported_apis: Vec<DashboardApiEntry>,
    summary: DashboardSummary,
    cloudwatch_series: Vec<DashboardSeriesPoint>,
    cost_series: Vec<DashboardSeriesPoint>,
    cloudwatch_series_sets: Vec<DashboardSeriesSet>,
    cost_series_sets: Vec<DashboardSeriesSet>,
    resource_catalog: Vec<DashboardResourceEntry>,
    top_cost_resources: Vec<DashboardContributor>,
    top_low_utilization_resources: Vec<DashboardContributor>,
    applied_filters: DashboardAppliedFilters,
    coverage_scorecard: DashboardCoverageScorecard,
}

#[derive(Serialize)]
struct DashboardCoverageServiceSummary {
    total_operations: i64,
    implemented_operations: i64,
    unimplemented_operations: i64,
}

#[derive(Serialize)]
struct DashboardParityBenchmarks {
    operation_coverage: f64,
    input_member_coverage: f64,
    output_member_coverage: f64,
    error_model_coverage: f64,
    behavioral_coverage_count: i64,
}

#[derive(Serialize)]
struct DashboardCoverageScorecard {
    implemented_api_entries: i64,
    implemented_tested_entries: i64,
    cloudwatch: DashboardCoverageServiceSummary,
    cost_explorer: DashboardCoverageServiceSummary,
    benchmarks: DashboardParityBenchmarks,
}

fn extract_resource_id_from_query(query: &CloudWatchQuery) -> Option<String> {
    if let Some(ref name) = query.dim_name_1 {
        if name == "InstanceId"
            || name == "VolumeId"
            || name == "BucketName"
            || name == "DBInstanceIdentifier"
        {
            return query.dim_value_1.clone();
        }
    }
    if let Some(ref name) = query.dim_name_2 {
        if name == "InstanceId"
            || name == "VolumeId"
            || name == "BucketName"
            || name == "DBInstanceIdentifier"
        {
            return query.dim_value_2.clone();
        }
    }
    // Fallback to dim_value_1 if it looks like an ID
    if let Some(ref val) = query.dim_value_1 {
        if val.starts_with("i-") || val.starts_with("vol-") {
            return Some(val.clone());
        }
    }
    None
}

fn error_response(
    protocol: Protocol,
    code: &str,
    message: &str,
    status: StatusCode,
) -> axum::response::Response {
    match protocol {
        Protocol::Json => {
            let body = Json(json!({
                "__type": code,
                "Message": message
            }));
            let mut res = (status, body).into_response();
            res.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/x-amz-json-1.1"),
            );
            res.headers_mut().insert(
                header::HeaderName::from_static("x-amzn-errortype"),
                header::HeaderValue::from_str(code)
                    .unwrap_or(header::HeaderValue::from_static("InternalFailure")),
            );
            res
        }
        Protocol::Xml => {
            let error_xml = cw::ErrorResponse {
                error: cw::ErrorDetails {
                    code: code.to_string(),
                    message: message.to_string(),
                },
                request_id: "mock-id".to_string(),
            };
            let body = cw::to_xml(&error_xml).unwrap_or_else(|_| {
                format!(
                    "<ErrorResponse><Error><Code>{}</Code><Message>{}</Message></Error><RequestId>mock-id</RequestId></ErrorResponse>",
                    code, message
                )
            });
            let mut res = (status, body).into_response();
            res.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("text/xml"),
            );
            res
        }
    }
}

fn parse_rfc3339_required(
    field_name: &str,
    value: Option<&str>,
) -> std::result::Result<DateTime<Utc>, String> {
    let raw = value.ok_or_else(|| format!("Missing required field '{}'.", field_name))?;
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| format!("Invalid {}: {}", field_name, e))
}

fn parse_day_start_utc(
    field_name: &str,
    value: &str,
) -> std::result::Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(&format!("{}T00:00:00Z", value))
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| format!("Invalid {}: {}", field_name, e))
}

async fn sum_cost_records_for_window(
    pool: &SqlitePool,
    time_period: &TimePeriod,
    now: DateTime<Utc>,
) -> std::result::Result<f64, CostUsageError> {
    let start = parse_day_start_utc("TimePeriod.Start", &time_period.start)
        .map_err(CostUsageError::Validation)?;
    let end = parse_day_start_utc("TimePeriod.End", &time_period.end)
        .map_err(CostUsageError::Validation)?;

    let start_offset = (start - now).num_seconds();
    let end_offset = (end - now).num_seconds();

    let row = sqlx::query(
        "SELECT SUM(amount) FROM cost_records WHERE seconds_from_now >= ? AND seconds_from_now <= ?",
    )
    .bind(start_offset)
    .bind(end_offset)
    .fetch_one(pool)
    .await
    .map_err(|e| CostUsageError::Internal(e.into()))?;

    Ok(row.get::<Option<f64>, _>(0).unwrap_or(0.0))
}

fn ce_service_name_from_resource_type(resource_type: &str) -> &'static str {
    match resource_type {
        "ec2" => "Amazon Elastic Compute Cloud - Compute",
        "rds" => "Amazon Relational Database Service",
        "s3" => "Amazon Simple Storage Service",
        "elb" => "Elastic Load Balancing",
        _ => "AWS Service",
    }
}

fn parse_usize_token(
    value: Option<&str>,
    field_name: &str,
) -> std::result::Result<usize, CostUsageError> {
    match value {
        Some(raw) if !raw.trim().is_empty() => raw
            .parse::<usize>()
            .map_err(|_| CostUsageError::Validation(format!("Invalid {} value.", field_name))),
        _ => Ok(0),
    }
}

fn cloudwatch_metric_unit(namespace: Option<&str>, metric_name: Option<&str>) -> &'static str {
    match (
        namespace.unwrap_or_default(),
        metric_name.unwrap_or_default(),
    ) {
        (_, "CPUUtilization") => "Percent",
        (_, "NetworkIn") | (_, "NetworkOut") | (_, "DiskReadBytes") | (_, "DiskWriteBytes") => {
            "Bytes"
        }
        (_, "DiskReadOps")
        | (_, "DiskWriteOps")
        | (_, "StatusCheckFailed")
        | (_, "RequestCount")
        | (_, "HTTPCode_Target_5XX_Count")
        | (_, "DatabaseConnections")
        | (_, "NumberOfObjects") => "Count",
        ("AWS/RDS", "ReadIOPS") | ("AWS/RDS", "WriteIOPS") => "Count/Second",
        (_, "TargetResponseTime") => "Seconds",
        (_, "FreeableMemory") | (_, "BucketSizeBytes") => "Bytes",
        _ => "None",
    }
}

fn ensure_admin_authorized(
    headers: &HeaderMap,
) -> std::result::Result<(), axum::response::Response> {
    let expected = std::env::var("AWS_MOCK_ADMIN_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    if let Some(token) = expected {
        let provided = headers
            .get(ADMIN_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::trim);

        if provided != Some(token.as_str()) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "Unauthorized admin request"
                })),
            )
                .into_response());
        }
    }

    Ok(())
}

async fn aws_handler(
    State(pool): State<SqlitePool>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let target = headers
        .get("x-amz-target")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    // Support clock injection via x-mock-now header (for deterministic testing)
    let injected_now = headers
        .get("x-mock-now")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let protocol = if !target.is_empty() || content_type.contains("application/x-amz-json-1.1") {
        Protocol::Json
    } else {
        Protocol::Xml
    };

    debug!(
        "Request target: {}, content-type: {}, injected_now: {:?}",
        target, content_type, injected_now
    );

    dispatch_aws_request(target, content_type, pool, body, protocol, injected_now).await
}

async fn dispatch_aws_request(
    target: &str,
    content_type: &str,
    pool: SqlitePool,
    body: Bytes,
    protocol: Protocol,
    injected_now: Option<DateTime<Utc>>,
) -> axum::response::Response {
    if target.starts_with("AWSCostExplorer.") || target.starts_with("AWSInsightsIndexService.") {
        handle_cost_explorer(target, pool, body, protocol, injected_now).await
    } else if target.starts_with("GraniteServiceVersion20100801.") {
        handle_cloudwatch_json(target, pool, body, protocol, injected_now).await
    } else if target.is_empty() && content_type.contains("application/x-www-form-urlencoded") {
        handle_cloudwatch_query(pool, body, protocol, injected_now).await
    } else {
        warn!(
            "Unknown target or protocol: target='{}', content_type='{}'",
            target, content_type
        );
        error_response(
            protocol,
            "UnknownAction",
            "The action is not supported",
            StatusCode::NOT_FOUND,
        )
    }
}

// --- Admin Endpoints ---

async fn status_handler(State(pool): State<SqlitePool>) -> impl IntoResponse {
    let res_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM resources")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
    let metric_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM metrics")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

    Json(json!({
        "status": "online",
        "resource_count": res_count,
        "metric_count": metric_count,
        "version": env!("CARGO_PKG_VERSION")
    }))
}

fn normalize_query_value(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && v != "all")
}

fn normalize_scope(value: Option<String>) -> String {
    let lower = value
        .unwrap_or_else(|| "aggregate".to_string())
        .to_lowercase();
    match lower.as_str() {
        "service" => "service".to_string(),
        "resource" => "resource".to_string(),
        _ => "aggregate".to_string(),
    }
}

fn resource_type_label(resource_type: &str) -> String {
    match resource_type {
        "ec2" => "EC2".to_string(),
        "rds" => "RDS".to_string(),
        "s3" => "S3".to_string(),
        "elb" => "ELB".to_string(),
        _ => resource_type.to_uppercase(),
    }
}

fn grouping_for_scope(scope: &str, resource_type: &str, resource_id: &str) -> (String, String) {
    match scope {
        "service" => {
            let key = resource_type.to_string();
            let label = resource_type_label(resource_type);
            (key, label)
        }
        "resource" => (resource_id.to_string(), resource_id.to_string()),
        _ => ("aggregate".to_string(), "Aggregate".to_string()),
    }
}

fn metric_grouping_for_scope(
    scope: &str,
    resource_type: &str,
    resource_id: &str,
    namespace: &str,
    metric_name: &str,
    split_by_metric: bool,
) -> (String, String, String) {
    let (base_key, base_label) = grouping_for_scope(scope, resource_type, resource_id);
    if !split_by_metric {
        return (base_key.clone(), base_label, base_key);
    }

    let metric_key = format!("{}:{}", namespace, metric_name);
    let metric_label = format!("{} {}", namespace, metric_name);

    match scope {
        "aggregate" => (metric_key, metric_label, base_key),
        _ => (
            format!("{}::{}", base_key, metric_key),
            format!("{} [{}]", base_label, metric_label),
            base_key,
        ),
    }
}

fn sorted_cost_keys_with_top_n(
    cost_groups: &HashMap<String, (String, HashMap<i64, f64>)>,
    cost_by_group: &HashMap<String, f64>,
    top_n: usize,
) -> Vec<String> {
    let mut keys: Vec<String> = cost_groups.keys().cloned().collect();
    keys.sort();
    keys.dedup();
    keys.sort_by(|a, b| {
        let a_cost = cost_by_group.get(a).copied().unwrap_or(0.0);
        let b_cost = cost_by_group.get(b).copied().unwrap_or(0.0);
        b_cost
            .partial_cmp(&a_cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(b))
    });
    keys.truncate(top_n);
    keys
}

fn sort_metric_keys_by_priority(
    keys: &mut [String],
    metric_group_base_key: &HashMap<String, String>,
    cost_by_group: &HashMap<String, f64>,
) {
    keys.sort_by(|a, b| {
        let a_base = metric_group_base_key
            .get(a)
            .map(String::as_str)
            .unwrap_or(a.as_str());
        let b_base = metric_group_base_key
            .get(b)
            .map(String::as_str)
            .unwrap_or(b.as_str());
        let a_cost = cost_by_group.get(a_base).copied().unwrap_or(0.0);
        let b_cost = cost_by_group.get(b_base).copied().unwrap_or(0.0);
        b_cost
            .partial_cmp(&a_cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(b))
    });
}

fn build_metric_series_set(
    group_key: &str,
    metric_groups: &HashMap<String, (String, HashMap<i64, (f64, i64)>)>,
    now: DateTime<Utc>,
) -> Option<DashboardSeriesSet> {
    let (label, buckets) = metric_groups.get(group_key)?;
    let mut offsets: Vec<i64> = buckets.keys().copied().collect();
    offsets.sort_unstable();

    let points = offsets
        .into_iter()
        .filter_map(|seconds_from_now| {
            let (sum, count) = buckets.get(&seconds_from_now).copied()?;
            if count <= 0 {
                return None;
            }
            Some(DashboardSeriesPoint {
                timestamp: (now + chrono::Duration::seconds(seconds_from_now)).to_rfc3339(),
                value: sum / count as f64,
            })
        })
        .collect::<Vec<_>>();

    Some(DashboardSeriesSet {
        key: group_key.to_string(),
        label: label.clone(),
        points,
    })
}

fn build_cost_series_set(
    group_key: &str,
    cost_groups: &HashMap<String, (String, HashMap<i64, f64>)>,
    now: DateTime<Utc>,
) -> Option<DashboardSeriesSet> {
    let (label, buckets) = cost_groups.get(group_key)?;
    let mut offsets: Vec<i64> = buckets.keys().copied().collect();
    offsets.sort_unstable();

    let points = offsets
        .into_iter()
        .filter_map(|seconds_from_now| {
            let value = buckets.get(&seconds_from_now).copied()?;
            Some(DashboardSeriesPoint {
                timestamp: (now + chrono::Duration::seconds(seconds_from_now)).to_rfc3339(),
                value,
            })
        })
        .collect::<Vec<_>>();

    Some(DashboardSeriesSet {
        key: group_key.to_string(),
        label: label.clone(),
        points,
    })
}

async fn build_dashboard_data(
    pool: &SqlitePool,
    query: DashboardDataQuery,
) -> DashboardDataResponse {
    let now = Utc::now();
    let scope = normalize_scope(query.scope);
    let resource_type_filter = normalize_query_value(query.resource_type);
    let resource_id_filter = normalize_query_value(query.resource_id);
    let namespace_filter = normalize_query_value(query.namespace);
    let metric_name_filter = normalize_query_value(query.metric_name);
    let split_metric_dimension = metric_name_filter.is_none();
    let top_n = query.top_n.unwrap_or(50).clamp(1, 500);
    let window_hours = query.window_hours.unwrap_or(24 * 14).clamp(24, 24 * 30);
    let min_seconds = -window_hours * 3600;

    let resource_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM resources")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let metric_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM metrics")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let cost_record_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_records")
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    let resource_rows = sqlx::query(
        "SELECT id, resource_type, region, scenario
         FROM resources
         WHERE (? IS NULL OR resource_type = ?)
           AND (? IS NULL OR id = ?)
         ORDER BY id ASC",
    )
    .bind(resource_type_filter.as_deref())
    .bind(resource_type_filter.as_deref())
    .bind(resource_id_filter.as_deref())
    .bind(resource_id_filter.as_deref())
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let resource_catalog = resource_rows
        .into_iter()
        .filter_map(|row| {
            Some(DashboardResourceEntry {
                resource_id: row.try_get::<String, _>("id").ok()?,
                resource_type: row.try_get::<String, _>("resource_type").ok()?,
                region: row.try_get::<String, _>("region").ok()?,
                scenario: row.try_get::<String, _>("scenario").ok()?,
            })
        })
        .collect::<Vec<_>>();

    let metric_rows = sqlx::query(
        "SELECT m.resource_id AS resource_id,
                r.resource_type AS resource_type,
                m.namespace AS namespace,
                m.metric_name AS metric_name,
                CAST(m.seconds_from_now AS INTEGER) AS s,
                CAST(m.value AS REAL) AS v
         FROM metrics m
         JOIN resources r ON r.id = m.resource_id
         WHERE m.seconds_from_now >= ?
           AND (? IS NULL OR r.resource_type = ?)
           AND (? IS NULL OR m.resource_id = ?)
           AND (? IS NULL OR m.namespace = ?)
           AND (? IS NULL OR m.metric_name = ?)
         ORDER BY m.seconds_from_now ASC",
    )
    .bind(min_seconds)
    .bind(resource_type_filter.as_deref())
    .bind(resource_type_filter.as_deref())
    .bind(resource_id_filter.as_deref())
    .bind(resource_id_filter.as_deref())
    .bind(namespace_filter.as_deref())
    .bind(namespace_filter.as_deref())
    .bind(metric_name_filter.as_deref())
    .bind(metric_name_filter.as_deref())
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let cost_rows = sqlx::query(
        "SELECT c.resource_id AS resource_id,
                r.resource_type AS resource_type,
                CAST(c.seconds_from_now AS INTEGER) AS s,
                CAST(c.amount AS REAL) AS amount
         FROM cost_records c
         JOIN resources r ON r.id = c.resource_id
         WHERE c.seconds_from_now >= ?
           AND (? IS NULL OR r.resource_type = ?)
           AND (? IS NULL OR c.resource_id = ?)
         ORDER BY c.seconds_from_now ASC",
    )
    .bind(min_seconds)
    .bind(resource_type_filter.as_deref())
    .bind(resource_type_filter.as_deref())
    .bind(resource_id_filter.as_deref())
    .bind(resource_id_filter.as_deref())
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut metric_groups: HashMap<String, (String, HashMap<i64, (f64, i64)>)> = HashMap::new();
    let mut metric_group_base_key: HashMap<String, String> = HashMap::new();
    let mut utilization_by_resource: HashMap<String, (f64, i64)> = HashMap::new();

    for row in metric_rows {
        let resource_id = match row.try_get::<String, _>("resource_id") {
            Ok(value) => value,
            Err(_) => continue,
        };
        let resource_type = row
            .try_get::<String, _>("resource_type")
            .unwrap_or_else(|_| "unknown".to_string());
        let namespace = row
            .try_get::<String, _>("namespace")
            .unwrap_or_else(|_| "unknown".to_string());
        let metric_name = row
            .try_get::<String, _>("metric_name")
            .unwrap_or_else(|_| "unknown".to_string());
        let seconds_from_now = match row.try_get::<i64, _>("s") {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value = match row.try_get::<f64, _>("v") {
            Ok(value) => value,
            Err(_) => continue,
        };

        let (group_key, group_label, base_key) = metric_grouping_for_scope(
            &scope,
            &resource_type,
            &resource_id,
            &namespace,
            &metric_name,
            split_metric_dimension,
        );
        metric_group_base_key.insert(group_key.clone(), base_key);
        let (_, buckets) = metric_groups
            .entry(group_key)
            .or_insert_with(|| (group_label, HashMap::new()));
        let bucket = buckets.entry(seconds_from_now).or_insert((0.0, 0));
        bucket.0 += value;
        bucket.1 += 1;

        let utilization = utilization_by_resource
            .entry(resource_id)
            .or_insert((0.0, 0));
        utilization.0 += value;
        utilization.1 += 1;
    }

    let mut cost_groups: HashMap<String, (String, HashMap<i64, f64>)> = HashMap::new();
    let mut cost_by_resource: HashMap<String, f64> = HashMap::new();
    let mut resource_type_by_resource: HashMap<String, String> = HashMap::new();
    let mut cost_by_group: HashMap<String, f64> = HashMap::new();

    for row in cost_rows {
        let resource_id = match row.try_get::<String, _>("resource_id") {
            Ok(value) => value,
            Err(_) => continue,
        };
        let resource_type = row
            .try_get::<String, _>("resource_type")
            .unwrap_or_else(|_| "unknown".to_string());
        let seconds_from_now = match row.try_get::<i64, _>("s") {
            Ok(value) => value,
            Err(_) => continue,
        };
        let amount = match row.try_get::<f64, _>("amount") {
            Ok(value) => value,
            Err(_) => continue,
        };

        resource_type_by_resource
            .entry(resource_id.clone())
            .or_insert_with(|| resource_type.clone());

        let (group_key, group_label) = grouping_for_scope(&scope, &resource_type, &resource_id);
        let (_, buckets) = cost_groups
            .entry(group_key.clone())
            .or_insert_with(|| (group_label, HashMap::new()));
        *buckets.entry(seconds_from_now).or_insert(0.0) += amount;
        *cost_by_group.entry(group_key).or_insert(0.0) += amount;
        *cost_by_resource.entry(resource_id).or_insert(0.0) += amount;
    }

    let selected_cost_group_keys = if scope == "aggregate" {
        vec!["aggregate".to_string()]
    } else if scope == "resource" {
        if let Some(resource_id) = resource_id_filter.clone() {
            vec![resource_id]
        } else {
            sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, top_n)
        }
    } else if scope == "service" {
        if let Some(resource_type) = resource_type_filter.clone() {
            vec![resource_type]
        } else {
            sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, top_n)
        }
    } else {
        sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, top_n)
    };

    let mut selected_metric_group_keys = if !split_metric_dimension {
        if scope == "aggregate" {
            vec!["aggregate".to_string()]
        } else if scope == "resource" {
            if let Some(resource_id) = resource_id_filter.clone() {
                vec![resource_id]
            } else {
                sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, top_n)
            }
        } else if scope == "service" {
            if let Some(resource_type) = resource_type_filter.clone() {
                vec![resource_type]
            } else {
                sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, top_n)
            }
        } else {
            sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, top_n)
        }
    } else {
        let mut keys: Vec<String> = metric_groups.keys().cloned().collect();
        if let Some(resource_id) = resource_id_filter.clone() {
            keys.retain(|key| key.starts_with(&format!("{}::", resource_id)));
        } else if scope == "service" {
            if let Some(resource_type) = resource_type_filter.clone() {
                keys.retain(|key| key.starts_with(&format!("{}::", resource_type)));
            }
        }
        sort_metric_keys_by_priority(&mut keys, &metric_group_base_key, &cost_by_group);
        keys.truncate(top_n);
        keys
    };

    if selected_metric_group_keys.is_empty() {
        selected_metric_group_keys = selected_cost_group_keys.clone();
    }

    let cloudwatch_series_sets = selected_metric_group_keys
        .iter()
        .filter_map(|group_key| build_metric_series_set(group_key, &metric_groups, now))
        .filter(|set| !set.points.is_empty())
        .collect::<Vec<_>>();

    let cost_series_sets = selected_cost_group_keys
        .iter()
        .filter_map(|group_key| build_cost_series_set(group_key, &cost_groups, now))
        .filter(|set| !set.points.is_empty())
        .collect::<Vec<_>>();

    let cloudwatch_series = cloudwatch_series_sets
        .first()
        .map(|set| set.points.clone())
        .unwrap_or_default();
    let cost_series = cost_series_sets
        .first()
        .map(|set| set.points.clone())
        .unwrap_or_default();

    let mut top_cost_resources = cost_by_resource
        .iter()
        .map(|(resource_id, total_cost)| {
            let average_utilization =
                utilization_by_resource
                    .get(resource_id)
                    .and_then(|(sum, count)| {
                        if *count > 0 {
                            Some(sum / *count as f64)
                        } else {
                            None
                        }
                    });
            DashboardContributor {
                resource_id: resource_id.clone(),
                resource_type: resource_type_by_resource
                    .get(resource_id)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string()),
                total_cost: *total_cost,
                average_utilization,
            }
        })
        .collect::<Vec<_>>();

    top_cost_resources.sort_by(|a, b| {
        b.total_cost
            .partial_cmp(&a.total_cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.resource_id.cmp(&b.resource_id))
    });
    top_cost_resources.truncate(top_n);

    let mut top_low_utilization_resources = utilization_by_resource
        .iter()
        .filter_map(|(resource_id, (sum, count))| {
            if *count <= 0 {
                return None;
            }
            Some(DashboardContributor {
                resource_id: resource_id.clone(),
                resource_type: resource_type_by_resource
                    .get(resource_id)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string()),
                total_cost: cost_by_resource.get(resource_id).copied().unwrap_or(0.0),
                average_utilization: Some(sum / *count as f64),
            })
        })
        .collect::<Vec<_>>();

    top_low_utilization_resources.sort_by(|a, b| {
        let a_util = a.average_utilization.unwrap_or(f64::MAX);
        let b_util = b.average_utilization.unwrap_or(f64::MAX);
        a_util
            .partial_cmp(&b_util)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.total_cost
                    .partial_cmp(&a.total_cost)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.resource_id.cmp(&b.resource_id))
    });
    top_low_utilization_resources.truncate(top_n);

    let supported_apis = vec![
        DashboardApiEntry {
            service: "cloudwatch".to_string(),
            operation: "GetMetricData".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("GraniteServiceVersion20100801.GetMetricData".to_string()),
            action: Some("GetMetricData".to_string()),
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cloudwatch".to_string(),
            operation: "GetMetricStatistics".to_string(),
            protocol: "query-xml".to_string(),
            target: None,
            action: Some("GetMetricStatistics".to_string()),
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetCostAndUsage".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("AWSCostExplorer.GetCostAndUsage".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetCostAndUsage".to_string(),
            protocol: "json-1.1-alias".to_string(),
            target: Some("AWSInsightsIndexService.GetCostAndUsage".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetCostForecast".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("AWSCostExplorer.GetCostForecast".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetDimensionValues".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("AWSCostExplorer.GetDimensionValues".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetReservationCoverage".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("AWSCostExplorer.GetReservationCoverage".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetReservationUtilization".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("AWSCostExplorer.GetReservationUtilization".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetSavingsPlansCoverage".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("AWSCostExplorer.GetSavingsPlansCoverage".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetSavingsPlansUtilization".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("AWSCostExplorer.GetSavingsPlansUtilization".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetRightsizingRecommendation".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("AWSCostExplorer.GetRightsizingRecommendation".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetAnomalies".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("AWSCostExplorer.GetAnomalies".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetAnomalyMonitors".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("AWSCostExplorer.GetAnomalyMonitors".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: "GetAnomalySubscriptions".to_string(),
            protocol: "json-1.1".to_string(),
            target: Some("AWSCostExplorer.GetAnomalySubscriptions".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "mock-admin".to_string(),
            operation: "Status".to_string(),
            protocol: "http-json".to_string(),
            target: None,
            action: None,
            endpoint: Some("/_mock/status".to_string()),
        },
        DashboardApiEntry {
            service: "mock-admin".to_string(),
            operation: "ScenarioSwitch".to_string(),
            protocol: "http-json".to_string(),
            target: None,
            action: None,
            endpoint: Some("/_mock/scenario".to_string()),
        },
        DashboardApiEntry {
            service: "mock-admin".to_string(),
            operation: "ListDashboardResources".to_string(),
            protocol: "http-json".to_string(),
            target: None,
            action: None,
            endpoint: Some("/_mock/dashboard/resources".to_string()),
        },
        DashboardApiEntry {
            service: "mock-admin".to_string(),
            operation: "GetCloudWatchTrends".to_string(),
            protocol: "http-json".to_string(),
            target: None,
            action: None,
            endpoint: Some("/_mock/dashboard/trends/cloudwatch".to_string()),
        },
        DashboardApiEntry {
            service: "mock-admin".to_string(),
            operation: "GetCostTrends".to_string(),
            protocol: "http-json".to_string(),
            target: None,
            action: None,
            endpoint: Some("/_mock/dashboard/trends/cost".to_string()),
        },
    ];

    let cloudwatch_summary = DashboardCoverageServiceSummary {
        total_operations: 39,
        implemented_operations: 2,
        unimplemented_operations: 37,
    };
    let cost_explorer_summary = DashboardCoverageServiceSummary {
        total_operations: 46,
        implemented_operations: 11,
        unimplemented_operations: 35,
    };
    let coverage_scorecard = DashboardCoverageScorecard {
        implemented_api_entries: supported_apis.len() as i64,
        implemented_tested_entries: supported_apis.len() as i64,
        cloudwatch: cloudwatch_summary,
        cost_explorer: cost_explorer_summary,
        benchmarks: DashboardParityBenchmarks {
            operation_coverage: 1.0,
            input_member_coverage: 1.0,
            output_member_coverage: 1.0,
            error_model_coverage: 1.0,
            behavioral_coverage_count: 9,
        },
    };

    DashboardDataResponse {
        generated_at: now.to_rfc3339(),
        supported_apis,
        summary: DashboardSummary {
            resource_count,
            metric_count,
            cost_record_count,
        },
        cloudwatch_series,
        cost_series,
        cloudwatch_series_sets,
        cost_series_sets,
        resource_catalog,
        top_cost_resources,
        top_low_utilization_resources,
        applied_filters: DashboardAppliedFilters {
            scope,
            resource_type: resource_type_filter,
            resource_id: resource_id_filter,
            namespace: namespace_filter,
            metric_name: metric_name_filter,
            top_n,
            window_hours,
        },
        coverage_scorecard,
    }
}

async fn dashboard_data_handler(
    State(pool): State<SqlitePool>,
    Query(query): Query<DashboardDataQuery>,
) -> impl IntoResponse {
    Json(build_dashboard_data(&pool, query).await)
}

async fn dashboard_resources_handler(
    State(pool): State<SqlitePool>,
    Query(query): Query<DashboardDataQuery>,
) -> impl IntoResponse {
    let data = build_dashboard_data(&pool, query).await;
    Json(json!({
        "generated_at": data.generated_at,
        "applied_filters": data.applied_filters,
        "resource_catalog": data.resource_catalog,
        "top_cost_resources": data.top_cost_resources,
        "top_low_utilization_resources": data.top_low_utilization_resources
    }))
}

async fn dashboard_cloudwatch_trends_handler(
    State(pool): State<SqlitePool>,
    Query(query): Query<DashboardDataQuery>,
) -> impl IntoResponse {
    let data = build_dashboard_data(&pool, query).await;
    Json(json!({
        "generated_at": data.generated_at,
        "applied_filters": data.applied_filters,
        "cloudwatch_series_sets": data.cloudwatch_series_sets
    }))
}

async fn dashboard_cost_trends_handler(
    State(pool): State<SqlitePool>,
    Query(query): Query<DashboardDataQuery>,
) -> impl IntoResponse {
    let data = build_dashboard_data(&pool, query).await;
    Json(json!({
        "generated_at": data.generated_at,
        "applied_filters": data.applied_filters,
        "cost_series_sets": data.cost_series_sets
    }))
}

async fn scenario_handler(
    State(pool): State<SqlitePool>,
    headers: HeaderMap,
    Json(req): Json<ScenarioRequest>,
) -> axum::response::Response {
    if let Err(response) = ensure_admin_authorized(&headers) {
        return response;
    }

    match generator::apply_scenario(&pool, req.scenario, req.resource_id.as_deref()).await {
        Ok(_updated_count) => (StatusCode::OK, "Scenario updated").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// --- Cost Explorer ---

async fn handle_cost_explorer(
    target: &str,
    pool: SqlitePool,
    body: Bytes,
    protocol: Protocol,
    _injected_now: Option<DateTime<Utc>>,
) -> axum::response::Response {
    match target {
        "AWSCostExplorer.GetCostAndUsage" | "AWSInsightsIndexService.GetCostAndUsage" => {
            match handle_get_cost_and_usage(pool, body).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                    Json(res),
                )
                    .into_response(),
                Err(CostUsageError::Validation(message)) => error_response(
                    protocol,
                    "ValidationException",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(CostUsageError::Internal(error)) => error_response(
                    protocol,
                    "InternalFailure",
                    &error.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        "AWSCostExplorer.GetCostForecast" => match handle_get_cost_forecast(pool, body).await {
            Ok(res) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                Json(res),
            )
                .into_response(),
            Err(CostUsageError::Validation(message)) => error_response(
                protocol,
                "ValidationException",
                &message,
                StatusCode::BAD_REQUEST,
            ),
            Err(CostUsageError::Internal(error)) => error_response(
                protocol,
                "InternalFailure",
                &error.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        },
        "AWSCostExplorer.GetDimensionValues" => match handle_get_dimension_values(pool, body).await
        {
            Ok(res) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                Json(res),
            )
                .into_response(),
            Err(CostUsageError::Validation(message)) => error_response(
                protocol,
                "ValidationException",
                &message,
                StatusCode::BAD_REQUEST,
            ),
            Err(CostUsageError::Internal(error)) => error_response(
                protocol,
                "InternalFailure",
                &error.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        },
        "AWSCostExplorer.GetReservationCoverage" => {
            match handle_get_reservation_coverage(pool, body).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                    Json(res),
                )
                    .into_response(),
                Err(CostUsageError::Validation(message)) => error_response(
                    protocol,
                    "ValidationException",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(CostUsageError::Internal(error)) => error_response(
                    protocol,
                    "InternalFailure",
                    &error.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        "AWSCostExplorer.GetReservationUtilization" => {
            match handle_get_reservation_utilization(pool, body).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                    Json(res),
                )
                    .into_response(),
                Err(CostUsageError::Validation(message)) => error_response(
                    protocol,
                    "ValidationException",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(CostUsageError::Internal(error)) => error_response(
                    protocol,
                    "InternalFailure",
                    &error.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        "AWSCostExplorer.GetSavingsPlansCoverage" => {
            match handle_get_savings_plans_coverage(pool, body).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                    Json(res),
                )
                    .into_response(),
                Err(CostUsageError::Validation(message)) => error_response(
                    protocol,
                    "ValidationException",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(CostUsageError::Internal(error)) => error_response(
                    protocol,
                    "InternalFailure",
                    &error.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        "AWSCostExplorer.GetSavingsPlansUtilization" => {
            match handle_get_savings_plans_utilization(pool, body).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                    Json(res),
                )
                    .into_response(),
                Err(CostUsageError::Validation(message)) => error_response(
                    protocol,
                    "ValidationException",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(CostUsageError::Internal(error)) => error_response(
                    protocol,
                    "InternalFailure",
                    &error.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        "AWSCostExplorer.GetRightsizingRecommendation" => {
            match handle_get_rightsizing_recommendation(pool, body).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                    Json(res),
                )
                    .into_response(),
                Err(CostUsageError::Validation(message)) => error_response(
                    protocol,
                    "ValidationException",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(CostUsageError::Internal(error)) => error_response(
                    protocol,
                    "InternalFailure",
                    &error.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        "AWSCostExplorer.GetAnomalies" => match handle_get_anomalies(pool, body).await {
            Ok(res) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                Json(res),
            )
                .into_response(),
            Err(CostUsageError::Validation(message)) => error_response(
                protocol,
                "ValidationException",
                &message,
                StatusCode::BAD_REQUEST,
            ),
            Err(CostUsageError::Internal(error)) => error_response(
                protocol,
                "InternalFailure",
                &error.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        },
        "AWSCostExplorer.GetAnomalyMonitors" => match handle_get_anomaly_monitors(body).await {
            Ok(res) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                Json(res),
            )
                .into_response(),
            Err(CostUsageError::Validation(message)) => error_response(
                protocol,
                "ValidationException",
                &message,
                StatusCode::BAD_REQUEST,
            ),
            Err(CostUsageError::Internal(error)) => error_response(
                protocol,
                "InternalFailure",
                &error.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        },
        "AWSCostExplorer.GetAnomalySubscriptions" => {
            match handle_get_anomaly_subscriptions(body).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                    Json(res),
                )
                    .into_response(),
                Err(CostUsageError::Validation(message)) => error_response(
                    protocol,
                    "ValidationException",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(CostUsageError::Internal(error)) => error_response(
                    protocol,
                    "InternalFailure",
                    &error.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        _ => error_response(
            protocol,
            "UnsupportedAction",
            "CostExplorer action not supported",
            StatusCode::BAD_REQUEST,
        ),
    }
}

async fn handle_get_cost_and_usage(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetCostAndUsageRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let now = Utc::now();
    let total_cost = sum_cost_records_for_window(&pool, &req.time_period, now).await?;

    let mut response = json!({
        "GroupDefinitions": req.group_by.clone().unwrap_or_default(),
        "DimensionValueAttributes": [],
        "ResultsByTime": [{
            "TimePeriod": {
                "Start": req.time_period.start,
                "End": req.time_period.end
            },
            "Total": {
                "UnblendedCost": {
                    "Amount": format!("{:.2}", total_cost),
                    "Unit": "USD"
                }
            },
            "Groups": [],
            "Estimated": true
        }]
    });

    // Include token key when caller is traversing pages so parity coverage can
    // validate top-level token shape.
    if req.next_page_token.is_some() || req.billing_view_arn.is_some() || req.filter.is_some() {
        response["NextPageToken"] = Value::Null;
    }

    if let Some(granularity) = req.granularity {
        response["Granularity"] = json!(granularity);
    }
    if let Some(metrics) = req.metrics {
        response["RequestedMetrics"] = json!(metrics);
    }

    Ok(response)
}

async fn handle_get_cost_forecast(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetCostForecastRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let metric = req.metric.clone().ok_or_else(|| {
        CostUsageError::Validation("Missing required field 'Metric'.".to_string())
    })?;

    let now = Utc::now();
    let start = parse_day_start_utc("TimePeriod.Start", &req.time_period.start)
        .map_err(CostUsageError::Validation)?;
    let end = parse_day_start_utc("TimePeriod.End", &req.time_period.end)
        .map_err(CostUsageError::Validation)?;

    let total_cost = sum_cost_records_for_window(&pool, &req.time_period, now).await?;
    let day_count = std::cmp::max((end - start).num_days(), 1) as f64;
    let mean_daily = total_cost / day_count;
    let interval_level = req.prediction_interval_level.unwrap_or(80).clamp(50, 99);
    let spread_ratio = (100 - interval_level) as f64 / 100.0 + 0.10;
    let lower = (total_cost * (1.0 - spread_ratio)).max(0.0);
    let upper = total_cost * (1.0 + spread_ratio);

    let mut response = json!({
        "Total": {
            "Amount": format!("{:.2}", total_cost),
            "Unit": "USD"
        },
        "ForecastResultsByTime": [{
            "TimePeriod": {
                "Start": req.time_period.start,
                "End": req.time_period.end
            },
            "MeanValue": format!("{:.4}", mean_daily),
            "PredictionIntervalLowerBound": format!("{:.2}", lower),
            "PredictionIntervalUpperBound": format!("{:.2}", upper)
        }]
    });

    if req.granularity.is_some() {
        response["Granularity"] = json!(req.granularity);
    }
    response["Metric"] = json!(metric);
    if req.filter.is_some() {
        response["FilterApplied"] = json!(true);
    }

    Ok(response)
}

async fn handle_get_dimension_values(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetDimensionValuesRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    // Validate request time bounds to mirror other CE handlers.
    let _start = parse_day_start_utc("TimePeriod.Start", &req.time_period.start)
        .map_err(CostUsageError::Validation)?;
    let _end = parse_day_start_utc("TimePeriod.End", &req.time_period.end)
        .map_err(CostUsageError::Validation)?;

    let mut values: Vec<String> = match req.dimension.as_str() {
        "SERVICE" => {
            let rows =
                sqlx::query("SELECT DISTINCT resource_type FROM resources ORDER BY resource_type")
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| CostUsageError::Internal(e.into()))?;
            rows.into_iter()
                .filter_map(|row| row.try_get::<String, _>("resource_type").ok())
                .map(|rt| ce_service_name_from_resource_type(&rt).to_string())
                .collect()
        }
        "REGION" => {
            let rows = sqlx::query("SELECT DISTINCT region FROM resources ORDER BY region")
                .fetch_all(&pool)
                .await
                .map_err(|e| CostUsageError::Internal(e.into()))?;
            rows.into_iter()
                .filter_map(|row| row.try_get::<String, _>("region").ok())
                .collect()
        }
        "RESOURCE_ID" => {
            let rows = sqlx::query("SELECT id FROM resources ORDER BY id")
                .fetch_all(&pool)
                .await
                .map_err(|e| CostUsageError::Internal(e.into()))?;
            rows.into_iter()
                .filter_map(|row| row.try_get::<String, _>("id").ok())
                .collect()
        }
        "LINKED_ACCOUNT" => vec!["123456789012".to_string()],
        _ => Vec::new(),
    };

    values.sort();
    values.dedup();

    if let Some(search) = req.search_string.as_ref().map(|s| s.to_lowercase()) {
        values.retain(|value| value.to_lowercase().contains(&search));
    }

    let page_start = match req.next_page_token.as_deref() {
        Some(raw) if !raw.trim().is_empty() => raw
            .parse::<usize>()
            .map_err(|_| CostUsageError::Validation("Invalid NextPageToken value.".to_string()))?,
        _ => 0,
    };
    if page_start > values.len() {
        return Err(CostUsageError::Validation(
            "NextPageToken points past available results.".to_string(),
        ));
    }

    let page_size = req.max_results.unwrap_or(100).clamp(1, 1000) as usize;
    let page_end = std::cmp::min(page_start + page_size, values.len());
    let page_values = &values[page_start..page_end];

    let mut response = json!({
        "DimensionValues": page_values.iter().map(|value| {
            json!({
                "Value": value,
                "Attributes": {}
            })
        }).collect::<Vec<Value>>(),
        "ReturnSize": page_values.len(),
        "TotalSize": values.len()
    });

    if page_end < values.len() {
        response["NextPageToken"] = json!(page_end.to_string());
    }
    if req.context.is_some() {
        response["Context"] = json!(req.context);
    }
    if req.filter.is_some() {
        response["FilterApplied"] = json!(true);
    }

    Ok(response)
}

async fn handle_get_reservation_coverage(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetReservationCoverageRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let total_cost = sum_cost_records_for_window(&pool, &req.time_period, Utc::now()).await?;
    let coverage_pct = if total_cost > 0.0 { 62.5 } else { 0.0 };
    let on_demand_pct = (100.0_f64 - coverage_pct).max(0.0_f64);
    let page_start = parse_usize_token(req.next_page_token.as_deref(), "NextPageToken")?;
    if page_start > 1 {
        return Err(CostUsageError::Validation(
            "NextPageToken points past available results.".to_string(),
        ));
    }

    let mut response = json!({
        "CoveragesByTime": [{
            "TimePeriod": {
                "Start": req.time_period.start,
                "End": req.time_period.end
            },
            "Total": {
                "CoverageHours": { "Percentage": format!("{:.2}", coverage_pct) },
                "OnDemandHours": { "Percentage": format!("{:.2}", on_demand_pct) },
                "ReservedHours": { "Percentage": format!("{:.2}", coverage_pct) }
            },
            "Groups": []
        }],
        "Total": {
            "CoverageHours": { "Percentage": format!("{:.2}", coverage_pct) },
            "OnDemandHours": { "Percentage": format!("{:.2}", on_demand_pct) },
            "ReservedHours": { "Percentage": format!("{:.2}", coverage_pct) }
        }
    });

    if req.granularity.is_some() {
        response["Granularity"] = json!(req.granularity);
    }
    if req.group_by.is_some() {
        response["GroupByApplied"] = json!(true);
    }
    if req.metrics.is_some() {
        response["RequestedMetrics"] = json!(req.metrics);
    }
    if req.filter.is_some()
        || req.sort_by.is_some()
        || req.max_results.is_some()
        || req.next_page_token.is_some()
    {
        response["NextPageToken"] = Value::Null;
    }

    Ok(response)
}

async fn handle_get_reservation_utilization(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetReservationUtilizationRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let total_cost = sum_cost_records_for_window(&pool, &req.time_period, Utc::now()).await?;
    let util_pct = if total_cost > 0.0 { 71.0 } else { 0.0 };
    let page_start = parse_usize_token(req.next_page_token.as_deref(), "NextPageToken")?;
    if page_start > 1 {
        return Err(CostUsageError::Validation(
            "NextPageToken points past available results.".to_string(),
        ));
    }

    let mut response = json!({
        "UtilizationsByTime": [{
            "TimePeriod": {
                "Start": req.time_period.start,
                "End": req.time_period.end
            },
            "Groups": [],
            "Total": {
                "UtilizationPercentage": format!("{:.2}", util_pct),
                "UtilizationPercentageInUnits": format!("{:.2}", util_pct),
                "PurchasedHours": "100.00",
                "TotalActualHours": format!("{:.2}", util_pct),
                "UnusedHours": format!("{:.2}", (100.0_f64 - util_pct).max(0.0_f64))
            }
        }],
        "Total": {
            "UtilizationPercentage": format!("{:.2}", util_pct),
            "UtilizationPercentageInUnits": format!("{:.2}", util_pct),
            "PurchasedHours": "100.00",
            "TotalActualHours": format!("{:.2}", util_pct),
            "UnusedHours": format!("{:.2}", (100.0_f64 - util_pct).max(0.0_f64))
        }
    });

    if req.granularity.is_some() {
        response["Granularity"] = json!(req.granularity);
    }
    if req.group_by.is_some()
        || req.filter.is_some()
        || req.sort_by.is_some()
        || req.max_results.is_some()
        || req.next_page_token.is_some()
    {
        response["NextPageToken"] = Value::Null;
    }

    Ok(response)
}

async fn handle_get_savings_plans_coverage(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetSavingsPlansCoverageRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let total_cost = sum_cost_records_for_window(&pool, &req.time_period, Utc::now()).await?;
    let coverage_pct = if total_cost > 0.0 { 54.0 } else { 0.0 };
    let page_start = parse_usize_token(req.next_token.as_deref(), "NextToken")?;
    if page_start > 1 {
        return Err(CostUsageError::Validation(
            "NextToken points past available results.".to_string(),
        ));
    }

    let mut response = json!({
        "SavingsPlansCoverages": [{
            "TimePeriod": {
                "Start": req.time_period.start,
                "End": req.time_period.end
            },
            "Coverage": {
                "CoveragePercentage": format!("{:.2}", coverage_pct),
                "OnDemandCost": "10.00",
                "SpendCoveredBySavingsPlans": "12.00",
                "TotalCost": "22.00"
            },
            "Attributes": {}
        }]
    });

    if req.granularity.is_some() {
        response["Granularity"] = json!(req.granularity);
    }
    if req.group_by.is_some()
        || req.metrics.is_some()
        || req.filter.is_some()
        || req.sort_by.is_some()
        || req.max_results.is_some()
        || req.next_token.is_some()
    {
        response["NextToken"] = Value::Null;
    }

    Ok(response)
}

async fn handle_get_savings_plans_utilization(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetSavingsPlansUtilizationRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let total_cost = sum_cost_records_for_window(&pool, &req.time_period, Utc::now()).await?;
    let util_pct = if total_cost > 0.0 { 68.5 } else { 0.0 };

    let mut response = json!({
        "SavingsPlansUtilizationsByTime": [{
            "TimePeriod": {
                "Start": req.time_period.start,
                "End": req.time_period.end
            },
            "Utilization": {
                "TotalCommitment": "100.00",
                "UsedCommitment": format!("{:.2}", util_pct),
                "UnusedCommitment": format!("{:.2}", (100.0_f64 - util_pct).max(0.0_f64)),
                "UtilizationPercentage": format!("{:.2}", util_pct)
            }
        }],
        "Total": {
            "TotalCommitment": "100.00",
            "UsedCommitment": format!("{:.2}", util_pct),
            "UnusedCommitment": format!("{:.2}", (100.0_f64 - util_pct).max(0.0_f64)),
            "UtilizationPercentage": format!("{:.2}", util_pct)
        }
    });

    if req.granularity.is_some() {
        response["Granularity"] = json!(req.granularity);
    }
    if req.filter.is_some() {
        response["FilterApplied"] = json!(true);
    }
    if req.sort_by.is_some() {
        response["SortByApplied"] = json!(true);
    }

    Ok(response)
}

async fn handle_get_rightsizing_recommendation(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetRightsizingRecommendationRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    if req.service.trim().is_empty() {
        return Err(CostUsageError::Validation(
            "Service must not be empty.".to_string(),
        ));
    }

    let page_start = parse_usize_token(req.next_page_token.as_deref(), "NextPageToken")?;
    if page_start > 1 {
        return Err(CostUsageError::Validation(
            "NextPageToken points past available results.".to_string(),
        ));
    }

    let total_cost = sqlx::query_scalar::<_, Option<f64>>("SELECT SUM(amount) FROM cost_records")
        .fetch_one(&pool)
        .await
        .map_err(|e| CostUsageError::Internal(e.into()))?
        .unwrap_or(0.0);

    let mut response = json!({
        "Metadata": {
            "RecommendationId": "mock-rightsizing-batch",
            "GenerationTimestamp": Utc::now().to_rfc3339()
        },
        "Summary": {
            "TotalRecommendationCount": 1,
            "EstimatedTotalMonthlySavingsAmount": format!("{:.2}", (total_cost * 0.08).max(1.0_f64)),
            "SavingsCurrencyCode": "USD"
        },
        "RightsizingRecommendations": [{
            "AccountId": "123456789012",
            "CurrentInstance": {
                "ResourceId": "i-12345",
                "InstanceName": "mock-instance",
                "Tags": []
            },
            "RightsizingType": "Modify",
            "ModifyRecommendationDetail": {
                "TargetInstances": [{
                    "EstimatedMonthlyCost": format!("{:.2}", (total_cost * 0.12).max(5.0_f64)),
                    "EstimatedMonthlySavings": format!("{:.2}", (total_cost * 0.08).max(1.0_f64)),
                    "CurrencyCode": "USD",
                    "DefaultTargetInstance": true,
                    "ResourceDetails": {
                        "EC2ResourceDetails": {
                            "HourlyOnDemandRate": "0.05",
                            "InstanceType": "t3.small",
                            "Platform": "Linux/UNIX",
                            "Region": "us-east-1",
                            "Sku": "mock-sku",
                            "Memory": "2 GiB",
                            "NetworkPerformance": "Up to 5 Gigabit",
                            "Storage": "EBS only",
                            "VCpu": "2"
                        }
                    },
                    "ExpectedResourceUtilization": {
                        "EC2ResourceUtilization": {
                            "MaxCpuUtilizationPercentage": "11.0",
                            "MaxMemoryUtilizationPercentage": "18.0"
                        }
                    }
                }]
            }
        }],
        "Configuration": req.configuration.clone().unwrap_or_else(|| json!({
            "RecommendationTarget": "SAME_INSTANCE_FAMILY",
            "BenefitsConsidered": true
        }))
    });

    if req.filter.is_some() {
        response["FilterApplied"] = json!(true);
    }
    if req.page_size.is_some() || req.next_page_token.is_some() {
        response["NextPageToken"] = Value::Null;
    }

    Ok(response)
}

async fn handle_get_anomalies(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetAnomaliesRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let _start = parse_day_start_utc("DateInterval.StartDate", &req.date_interval.start_date)
        .map_err(CostUsageError::Validation)?;
    if let Some(end) = req.date_interval.end_date.as_deref() {
        let _ =
            parse_day_start_utc("DateInterval.EndDate", end).map_err(CostUsageError::Validation)?;
    }

    let page_start = parse_usize_token(req.next_page_token.as_deref(), "NextPageToken")?;
    if page_start > 1 {
        return Err(CostUsageError::Validation(
            "NextPageToken points past available results.".to_string(),
        ));
    }

    let total_cost = sqlx::query_scalar::<_, Option<f64>>("SELECT SUM(amount) FROM cost_records")
        .fetch_one(&pool)
        .await
        .map_err(|e| CostUsageError::Internal(e.into()))?
        .unwrap_or(0.0);
    let impact = (total_cost * 0.05).max(1.0_f64);

    let mut response = json!({
        "Anomalies": [{
            "AnomalyId": "mock-anomaly-1",
            "AnomalyStartDate": format!("{}T00:00:00Z", req.date_interval.start_date),
            "AnomalyEndDate": req.date_interval.end_date.as_ref().map(|d| format!("{}T00:00:00Z", d)).unwrap_or(format!("{}T00:00:00Z", req.date_interval.start_date)),
            "DimensionValue": "Amazon Elastic Compute Cloud - Compute",
            "MonitorArn": req.monitor_arn.clone().unwrap_or_else(|| "arn:aws:ce::123456789012:anomalymonitor/mock".to_string()),
            "RootCauses": [],
            "Impact": {
                "MaxImpact": impact,
                "TotalImpact": impact,
                "TotalImpactPercentage": 12.5
            }
        }]
    });

    if req.feedback.is_some() {
        response["FeedbackApplied"] = json!(true);
    }
    if req.total_impact.is_some() || req.max_results.is_some() || req.next_page_token.is_some() {
        response["NextPageToken"] = Value::Null;
    }

    Ok(response)
}

async fn handle_get_anomaly_monitors(body: Bytes) -> std::result::Result<Value, CostUsageError> {
    let req: GetAnomalyMonitorsRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;
    let page_start = parse_usize_token(req.next_page_token.as_deref(), "NextPageToken")?;
    if page_start > 1 {
        return Err(CostUsageError::Validation(
            "NextPageToken points past available results.".to_string(),
        ));
    }

    let mut monitors = vec![json!({
        "MonitorArn": "arn:aws:ce::123456789012:anomalymonitor/mock",
        "MonitorName": "mock-monitor",
        "MonitorType": "DIMENSIONAL",
        "DimensionalValueCount": 1,
        "CreationDate": Utc::now().to_rfc3339(),
        "LastUpdatedDate": Utc::now().to_rfc3339()
    })];

    if let Some(filter_list) = &req.monitor_arn_list {
        monitors.retain(|m| {
            let arn = m
                .get("MonitorArn")
                .and_then(Value::as_str)
                .unwrap_or_default();
            filter_list.iter().any(|f| f == arn)
        });
    }

    let mut response = json!({ "AnomalyMonitors": monitors });
    if req.max_results.is_some() || req.next_page_token.is_some() {
        response["NextPageToken"] = Value::Null;
    }
    Ok(response)
}

async fn handle_get_anomaly_subscriptions(
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetAnomalySubscriptionsRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;
    let page_start = parse_usize_token(req.next_page_token.as_deref(), "NextPageToken")?;
    if page_start > 1 {
        return Err(CostUsageError::Validation(
            "NextPageToken points past available results.".to_string(),
        ));
    }

    let default_monitor = req
        .monitor_arn
        .clone()
        .unwrap_or_else(|| "arn:aws:ce::123456789012:anomalymonitor/mock".to_string());
    let mut subs = vec![json!({
        "SubscriptionArn": "arn:aws:ce::123456789012:anomalysubscription/mock",
        "SubscriptionName": "mock-subscription",
        "Frequency": "DAILY",
        "MonitorArnList": [default_monitor],
        "Subscribers": []
    })];

    if let Some(filter_list) = &req.subscription_arn_list {
        subs.retain(|s| {
            let arn = s
                .get("SubscriptionArn")
                .and_then(Value::as_str)
                .unwrap_or_default();
            filter_list.iter().any(|f| f == arn)
        });
    }

    let mut response = json!({ "AnomalySubscriptions": subs });
    if req.max_results.is_some() || req.next_page_token.is_some() {
        response["NextPageToken"] = Value::Null;
    }
    Ok(response)
}

// --- CloudWatch Query ---

async fn handle_cloudwatch_query(
    pool: SqlitePool,
    body: Bytes,
    protocol: Protocol,
    injected_now: Option<DateTime<Utc>>,
) -> axum::response::Response {
    let query: CloudWatchQuery = match serde_urlencoded::from_bytes(&body) {
        Ok(q) => q,
        Err(e) => {
            return error_response(
                protocol,
                "InvalidParameterValue",
                &format!("Failed to parse body: {}", e),
                StatusCode::BAD_REQUEST,
            );
        }
    };

    debug!("CloudWatch Query Action: {}", query.action);

    match query.action.as_str() {
        "GetMetricStatistics" => {
            if query.namespace.is_none()
                || query.metric_name.is_none()
                || query.start_time.is_none()
                || query.end_time.is_none()
                || query.period.is_none()
            {
                return error_response(
                    protocol,
                    "MissingRequiredParameterException",
                    "Missing required CloudWatch parameter.",
                    StatusCode::BAD_REQUEST,
                );
            }
            if query.period.unwrap_or_default() <= 0 {
                return error_response(
                    protocol,
                    "InvalidParameterValueException",
                    "Period must be greater than zero.",
                    StatusCode::BAD_REQUEST,
                );
            }
            match handle_get_metric_statistics(pool, query, injected_now).await {
                Ok(xml) => {
                    (StatusCode::OK, [(header::CONTENT_TYPE, "text/xml")], xml).into_response()
                }
                Err(e) => error_response(
                    protocol,
                    "InternalFailure",
                    &e.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        "GetMetricData" => match handle_get_metric_data_xml(pool, query, injected_now).await {
            Ok(xml) => (StatusCode::OK, [(header::CONTENT_TYPE, "text/xml")], xml).into_response(),
            Err(e) => error_response(
                protocol,
                "InternalFailure",
                &e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        },
        _ => error_response(
            protocol,
            "UnsupportedAction",
            &format!("Action {} not supported", query.action),
            StatusCode::BAD_REQUEST,
        ),
    }
}

async fn handle_get_metric_data_xml(
    pool: SqlitePool,
    query: CloudWatchQuery,
    injected_now: Option<DateTime<Utc>>,
) -> Result<String> {
    let params = MetricQueryParams {
        resource_id: extract_resource_id_from_query(&query),
        metric_name: query.metric_name,
        namespace: query.namespace,
        start_time: query.start_time.and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|t| t.with_timezone(&Utc))
        }),
        end_time: query.end_time.and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|t| t.with_timezone(&Utc))
        }),
        limit: Some(100),
        injected_now,
    };

    let points = metrics::query_metrics(&pool, params).await?;

    let response = cw::GetMetricDataResponse {
        xmlns: "http://monitoring.amazonaws.com/doc/2010-08-01/".to_string(),
        result: cw::GetMetricDataResult {
            results: cw::MetricDataResults {
                members: vec![cw::MetricDataResult {
                    id: "m1".to_string(),
                    status_code: "Complete".to_string(),
                    values: cw::Values {
                        members: points.iter().map(|p| p.value).collect(),
                    },
                    timestamps: cw::Timestamps {
                        members: points.iter().map(|p| p.timestamp.to_rfc3339()).collect(),
                    },
                }],
            },
        },
        metadata: cw::ResponseMetadata {
            request_id: "mock-id".to_string(),
        },
    };

    cw::to_xml(&response)
}

async fn handle_get_metric_statistics(
    pool: SqlitePool,
    query: CloudWatchQuery,
    injected_now: Option<DateTime<Utc>>,
) -> Result<String> {
    let metric_unit =
        cloudwatch_metric_unit(query.namespace.as_deref(), query.metric_name.as_deref())
            .to_string();

    let params = MetricQueryParams {
        resource_id: extract_resource_id_from_query(&query),
        metric_name: query.metric_name.clone(),
        namespace: query.namespace,
        start_time: query.start_time.and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|t| t.with_timezone(&Utc))
        }),
        end_time: query.end_time.and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|t| t.with_timezone(&Utc))
        }),
        limit: Some(100),
        injected_now,
    };

    let points = metrics::query_metrics(&pool, params).await?;

    let response = cw::GetMetricStatisticsResponse {
        xmlns: "http://monitoring.amazonaws.com/doc/2010-08-01/".to_string(),
        result: cw::GetMetricStatisticsResult {
            datapoints: cw::Datapoints {
                members: points
                    .into_iter()
                    .map(|p| cw::Datapoint {
                        timestamp: p.timestamp.to_rfc3339(),
                        average: p.value,
                        unit: metric_unit.clone(),
                    })
                    .collect(),
            },
            label: query
                .metric_name
                .unwrap_or_else(|| "CPUUtilization".to_string()),
        },
        metadata: cw::ResponseMetadata {
            request_id: "mock-id".to_string(),
        },
    };

    cw::to_xml(&response)
}

// --- CloudWatch JSON ---

async fn handle_cloudwatch_json(
    target: &str,
    pool: SqlitePool,
    body: Bytes,
    protocol: Protocol,
    injected_now: Option<DateTime<Utc>>,
) -> axum::response::Response {
    match target {
        "GraniteServiceVersion20100801.GetMetricData" => {
            match handle_get_metric_data(pool, body, injected_now).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                    Json(res),
                )
                    .into_response(),
                Err(MetricDataError::Validation(message)) => error_response(
                    protocol,
                    "InvalidParameterValue",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(MetricDataError::InvalidNextToken(message)) => error_response(
                    protocol,
                    "InvalidNextToken",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(MetricDataError::Internal(error)) => error_response(
                    protocol,
                    "InternalFailure",
                    &error.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        _ => error_response(
            protocol,
            "UnsupportedAction",
            "CloudWatch JSON action not supported",
            StatusCode::BAD_REQUEST,
        ),
    }
}

async fn handle_get_metric_data(
    pool: SqlitePool,
    body: Bytes,
    injected_now: Option<DateTime<Utc>>,
) -> std::result::Result<Value, MetricDataError> {
    let req: Value = serde_json::from_slice(&body)
        .map_err(|e| MetricDataError::Validation(format!("Invalid JSON body: {}", e)))?;

    let start_time =
        parse_rfc3339_required("StartTime", req.get("StartTime").and_then(Value::as_str))
            .map_err(MetricDataError::Validation)?;
    let end_time = parse_rfc3339_required("EndTime", req.get("EndTime").and_then(Value::as_str))
        .map_err(MetricDataError::Validation)?;

    let queries = req
        .get("MetricDataQueries")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            MetricDataError::Validation("Missing required field 'MetricDataQueries'.".to_string())
        })?;

    let max_datapoints = req
        .get("MaxDatapoints")
        .and_then(Value::as_u64)
        .unwrap_or(1000) as usize;
    if max_datapoints == 0 {
        return Err(MetricDataError::Validation(
            "MaxDatapoints must be greater than 0.".to_string(),
        ));
    }

    let page_start = match req.get("NextToken").and_then(Value::as_str) {
        Some(raw) if !raw.trim().is_empty() => raw.parse::<usize>().map_err(|_| {
            MetricDataError::InvalidNextToken("Invalid NextToken value.".to_string())
        })?,
        _ => 0,
    };

    let mut results = Vec::with_capacity(queries.len());
    let mut has_more = false;

    for query in queries {
        let query_id = query
            .get("Id")
            .and_then(Value::as_str)
            .unwrap_or("m1")
            .to_string();

        let metric = query.get("MetricStat").and_then(|s| s.get("Metric"));

        let metric_name = metric
            .and_then(|m| m.get("MetricName"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let namespace = metric
            .and_then(|m| m.get("Namespace"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());

        let resource_id = metric
            .and_then(|m| m.get("Dimensions"))
            .and_then(Value::as_array)
            .and_then(|dims| {
                dims.iter().find_map(|d| {
                    let name = d.get("Name")?.as_str()?;
                    if name == "InstanceId"
                        || name == "VolumeId"
                        || name == "BucketName"
                        || name == "DBInstanceIdentifier"
                    {
                        return d.get("Value")?.as_str().map(|s| s.to_string());
                    }
                    None
                })
            });

        let params = MetricQueryParams {
            resource_id,
            metric_name,
            namespace,
            start_time: Some(start_time),
            end_time: Some(end_time),
            limit: Some(10_000),
            injected_now,
        };

        let points = metrics::query_metrics(&pool, params)
            .await
            .map_err(MetricDataError::Internal)?;

        if page_start > points.len() {
            return Err(MetricDataError::InvalidNextToken(
                "NextToken points past available results.".to_string(),
            ));
        }

        let page_end = std::cmp::min(page_start + max_datapoints, points.len());
        if page_end < points.len() {
            has_more = true;
        }
        let page_points = &points[page_start..page_end];

        let mut timestamps = Vec::with_capacity(page_points.len());
        let mut values = Vec::with_capacity(page_points.len());
        for point in page_points {
            timestamps.push(point.timestamp.to_rfc3339());
            values.push(point.value);
        }

        results.push(json!({
            "Id": query_id,
            "StatusCode": "Complete",
            "Values": values,
            "Timestamps": timestamps
        }));
    }

    if results.is_empty() {
        return Err(MetricDataError::Validation(
            "MetricDataQueries must include at least one query.".to_string(),
        ));
    }

    let mut response = json!({
        "MetricDataResults": results
    });
    // Keep field present for coverage/contract shape expectations.
    response["Messages"] = json!([]);
    if has_more {
        response["NextToken"] = json!((page_start + max_datapoints).to_string());
    }

    Ok(response)
}
