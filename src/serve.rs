use anyhow::Result;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddr;
use tracing::{debug, info, warn};

use crate::cli::Scenario;
use crate::fixture;
use crate::generator;
use crate::handlers::{aws, cloudwatch as cw, cost_explorer as ce};
use crate::metrics::{self, MetricQueryParams};

const ADMIN_TOKEN_HEADER: &str = "x-mock-admin-token";

pub async fn run(pool: SqlitePool, address: String, port: u16) -> Result<()> {
    let app = build_app(pool);

    let addr: SocketAddr = format!("{}:{}", address, port).parse()?;
    info!("Starting AWS-compatible API server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

pub fn build_app(pool: SqlitePool) -> Router {
    Router::new()
        .route("/", post(aws_handler))
        .route("/_mock/status", get(status_handler))
        .route("/_mock/fixture/definition", get(fixture_definition_handler))
        .route("/_mock/fixture/realize", post(fixture_realize_handler))
        .route("/_mock/fixture/status", get(fixture_status_handler))
        .route("/_mock/fixture/manifest", get(fixture_manifest_handler))
        .route("/_mock/fixture/identities", get(fixture_identities_handler))
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
        .with_state(pool)
}

#[derive(Debug, Clone, Copy)]
enum Protocol {
    Json,
    Xml,
}

#[derive(Debug, Clone)]
struct CloudWatchQuery {
    action: String,
    namespace: Option<String>,
    metric_name: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
    period: Option<i32>,
    max_datapoints: Option<u64>,
    next_token: Option<String>,
    recently_active: Option<String>,
    dim_name_1: Option<String>,
    dim_value_1: Option<String>,
    dim_name_2: Option<String>,
    dim_value_2: Option<String>,
    statistics: Vec<String>,
    extended_statistics: Vec<String>,
}

#[derive(Deserialize)]
struct ScenarioRequest {
    scenario: Scenario,
    resource_id: Option<String>,
}

#[derive(Deserialize, Default)]
struct FixtureVersionQuery {
    version: Option<String>,
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
struct GetUsageForecastRequest {
    time_period: TimePeriod,
    metric: Option<String>,
    granularity: Option<String>,
    filter: Option<Value>,
    billing_view_arn: Option<String>,
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
struct GetTagsRequest {
    time_period: TimePeriod,
    tag_key: String,
    search_string: Option<String>,
    filter: Option<Value>,
    next_page_token: Option<String>,
    max_results: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetResourcesRequest {
    pagination_token: Option<String>,
    resources_per_page: Option<u64>,
    tags_per_page: Option<u64>,
    resource_arn_list: Option<Vec<String>>,
    resource_type_filters: Option<Vec<String>>,
    tag_filters: Option<Vec<TagFilterInput>>,
    include_compliance_details: Option<bool>,
    exclude_compliant_resources: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetTagKeysRequest {
    pagination_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetTagValuesRequest {
    key: String,
    pagination_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TagFilterInput {
    key: String,
    values: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetProductsRequest {
    service_code: String,
    filters: Option<Vec<PricingFilterInput>>,
    format_version: Option<String>,
    max_results: Option<u64>,
    next_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ComputeOptimizerRequest {
    next_token: Option<String>,
    max_results: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DescribeReportDefinitionsRequest {}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PricingFilterInput {
    field: String,
    #[serde(rename = "Type")]
    filter_type: String,
    value: String,
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

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetMetricDataRequest {
    start_time: CloudWatchDateTimeInput,
    end_time: CloudWatchDateTimeInput,
    metric_data_queries: Vec<GetMetricDataQuery>,
    next_token: Option<String>,
    max_datapoints: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetMetricStatisticsJsonRequest {
    namespace: Option<String>,
    metric_name: Option<String>,
    dimensions: Option<Vec<GetMetricDimension>>,
    start_time: Option<CloudWatchDateTimeInput>,
    end_time: Option<CloudWatchDateTimeInput>,
    period: Option<i64>,
    statistics: Option<Vec<String>>,
    extended_statistics: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListMetricsJsonRequest {
    namespace: Option<String>,
    metric_name: Option<String>,
    dimensions: Option<Vec<ListMetricsDimension>>,
    next_token: Option<String>,
    recently_active: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListMetricsDimension {
    name: String,
    value: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CloudWatchDateTimeInput {
    String(String),
    Integer(i64),
    Float(f64),
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetMetricDataQuery {
    id: String,
    metric_stat: Option<GetMetricStatRequest>,
    expression: Option<String>,
    label: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetMetricStatRequest {
    metric: GetMetricRequest,
    period: i64,
    stat: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetMetricRequest {
    namespace: String,
    metric_name: String,
    dimensions: Option<Vec<GetMetricDimension>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetMetricDimension {
    name: String,
    value: String,
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

enum MetricStatisticsError {
    MissingParameter(String),
    InvalidParameterCombination(String),
    Validation(String),
    Internal(anyhow::Error),
}

const GET_METRIC_STATISTICS_RAW_ROW_LIMIT: i64 = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StandardStatistic {
    SampleCount,
    Average,
    Sum,
    Minimum,
    Maximum,
}

impl StandardStatistic {
    fn parse_metric_data_stat(value: &str) -> std::result::Result<Self, MetricDataError> {
        match value.to_ascii_lowercase().as_str() {
            "average" => Ok(Self::Average),
            "sum" => Ok(Self::Sum),
            "minimum" => Ok(Self::Minimum),
            "maximum" => Ok(Self::Maximum),
            _ => Err(MetricDataError::Validation(format!(
                "Unsupported MetricStat.Stat '{}'.",
                value
            ))),
        }
    }

    fn parse_metric_statistics_stat(
        value: &str,
    ) -> std::result::Result<Self, MetricStatisticsError> {
        match value.to_ascii_lowercase().as_str() {
            "samplecount" => Ok(Self::SampleCount),
            "average" => Ok(Self::Average),
            "sum" => Ok(Self::Sum),
            "minimum" => Ok(Self::Minimum),
            "maximum" => Ok(Self::Maximum),
            _ => Err(MetricStatisticsError::Validation(format!(
                "Unsupported Statistics value '{}'.",
                value
            ))),
        }
    }

    fn value(self, point: &AggregatedMetricPoint) -> f64 {
        match self {
            Self::SampleCount => point.sample_count,
            Self::Average => point.average,
            Self::Sum => point.sum,
            Self::Minimum => point.minimum,
            Self::Maximum => point.maximum,
        }
    }
}

#[derive(Default, Clone)]
struct CeFilterCriteria {
    services: Option<BTreeSet<String>>,
    resource_ids: Option<BTreeSet<String>>,
    regions: Option<BTreeSet<String>>,
    tags: BTreeMap<String, BTreeSet<String>>,
}

struct CostRow {
    resource_id: String,
    resource_type: String,
    region: String,
    amount: f64,
    seconds_from_now: i64,
    tags_json: Option<String>,
}

#[derive(Clone, Copy, Default)]
struct CostUsageAmounts {
    unblended_cost: f64,
    usage_quantity: f64,
}

impl CostUsageAmounts {
    fn from_row(row: &CostRow) -> Self {
        Self {
            unblended_cost: row.amount,
            usage_quantity: row.amount / mock_usage_rate_for_resource_type(&row.resource_type),
        }
    }

    fn add(&mut self, other: Self) {
        self.unblended_cost += other.unblended_cost;
        self.usage_quantity += other.usage_quantity;
    }
}

impl From<CostUsageAmounts> for ce::CostUsageMetricAmounts {
    fn from(amounts: CostUsageAmounts) -> Self {
        Self {
            unblended_cost: amounts.unblended_cost,
            usage_quantity: amounts.usage_quantity,
        }
    }
}

#[derive(Clone)]
struct TaggedResource {
    arn: String,
    resource_type_filter: String,
    tags: Vec<(String, String)>,
}

#[derive(Clone)]
struct PricingProduct {
    service_code: &'static str,
    attributes: BTreeMap<&'static str, String>,
    rate_code: String,
    description: String,
    unit: &'static str,
    price_per_unit_usd: String,
}

struct MetricDataSeries {
    id: String,
    label: String,
    timestamps: Vec<String>,
    values: Vec<f64>,
}

struct PaginatedMetricDataSeries {
    results: Vec<MetricDataSeries>,
    next_token: Option<String>,
}

#[derive(Default)]
struct XmlMetricDataQueryBuilder {
    id: Option<String>,
    label: Option<String>,
    namespace: Option<String>,
    metric_name: Option<String>,
    period: Option<i64>,
    stat: Option<String>,
    expression: Option<String>,
    dimensions: BTreeMap<usize, XmlMetricDimensionBuilder>,
}

#[derive(Default)]
struct XmlMetricDimensionBuilder {
    name: Option<String>,
    value: Option<String>,
}

struct MetricDataSeriesRequest {
    id: String,
    label: String,
    namespace: String,
    metric_name: String,
    resource_id: Option<String>,
    stat: String,
    period: i64,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    injected_now: Option<DateTime<Utc>>,
}

struct ListMetricsPage {
    metrics: Vec<cw::Metric>,
    next_token: Option<String>,
}

struct GetMetricStatisticsRequest {
    namespace: String,
    metric_name: String,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    period: i64,
    resource_id: Option<String>,
    statistics: Vec<StandardStatistic>,
}

struct MetricStatisticsSeries {
    metric_name: String,
    metric_unit: String,
    statistics: Vec<StandardStatistic>,
    datapoints: Vec<AggregatedMetricPoint>,
}

struct MetricStatisticsRequestParts {
    namespace: String,
    metric_name: String,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    period: i64,
    resource_id: Option<String>,
    statistics: Vec<String>,
    extended_statistics: Vec<String>,
}

#[derive(Clone, Copy)]
struct AggregatedMetricPoint {
    timestamp: DateTime<Utc>,
    sample_count: f64,
    average: f64,
    sum: f64,
    minimum: f64,
    maximum: f64,
}

#[derive(Clone, Copy)]
struct MetricBucketAccumulator {
    sample_count: f64,
    sum: f64,
    minimum: f64,
    maximum: f64,
}

impl MetricBucketAccumulator {
    fn new(value: f64) -> Self {
        Self {
            sample_count: 1.0,
            sum: value,
            minimum: value,
            maximum: value,
        }
    }

    fn record(&mut self, value: f64) {
        self.sample_count += 1.0;
        self.sum += value;
        self.minimum = self.minimum.min(value);
        self.maximum = self.maximum.max(value);
    }

    fn into_point(self, timestamp: DateTime<Utc>) -> AggregatedMetricPoint {
        AggregatedMetricPoint {
            timestamp,
            sample_count: self.sample_count,
            average: self.sum / self.sample_count,
            sum: self.sum,
            minimum: self.minimum,
            maximum: self.maximum,
        }
    }
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

fn cost_explorer_api_entries(operation: &str) -> [DashboardApiEntry; 2] {
    [
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: operation.to_string(),
            protocol: "json-1.1".to_string(),
            target: Some(format!("AWSCostExplorer.{}", operation)),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cost-explorer".to_string(),
            operation: operation.to_string(),
            protocol: "json-1.1-alias".to_string(),
            target: Some(format!("AWSInsightsIndexService.{}", operation)),
            action: None,
            endpoint: None,
        },
    ]
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

#[derive(Debug, Clone)]
struct NormalizedDashboardQuery {
    scope: String,
    resource_type: Option<String>,
    resource_id: Option<String>,
    namespace: Option<String>,
    metric_name: Option<String>,
    top_n: usize,
    window_hours: i64,
    min_seconds: i64,
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

type MetricBucketMap = HashMap<i64, (f64, i64)>;
type MetricGroupMap = HashMap<String, (String, MetricBucketMap)>;
type CostBucketMap = HashMap<i64, f64>;
type CostGroupMap = HashMap<String, (String, CostBucketMap)>;

fn normalize_dashboard_query(query: DashboardDataQuery) -> NormalizedDashboardQuery {
    let scope = normalize_scope(query.scope);
    let resource_type = normalize_query_value(query.resource_type);
    let resource_id = normalize_query_value(query.resource_id);
    let namespace = normalize_query_value(query.namespace);
    let metric_name = normalize_query_value(query.metric_name);
    let top_n = query.top_n.unwrap_or(50).clamp(1, 500);
    let window_hours = query.window_hours.unwrap_or(24 * 14).clamp(24, 24 * 30);

    NormalizedDashboardQuery {
        scope,
        resource_type,
        resource_id,
        namespace,
        metric_name,
        top_n,
        window_hours,
        min_seconds: -window_hours * 3600,
    }
}

fn dashboard_applied_filters(query: &NormalizedDashboardQuery) -> DashboardAppliedFilters {
    DashboardAppliedFilters {
        scope: query.scope.clone(),
        resource_type: query.resource_type.clone(),
        resource_id: query.resource_id.clone(),
        namespace: query.namespace.clone(),
        metric_name: query.metric_name.clone(),
        top_n: query.top_n,
        window_hours: query.window_hours,
    }
}

fn dashboard_error_response() -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": "dashboard query failed"
        })),
    )
        .into_response()
}

async fn fetch_dashboard_summary(pool: &SqlitePool) -> Result<DashboardSummary> {
    Ok(DashboardSummary {
        resource_count: sqlx::query_scalar("SELECT COUNT(*) FROM resources")
            .fetch_one(pool)
            .await?,
        metric_count: sqlx::query_scalar("SELECT COUNT(*) FROM metrics")
            .fetch_one(pool)
            .await?,
        cost_record_count: sqlx::query_scalar("SELECT COUNT(*) FROM cost_records")
            .fetch_one(pool)
            .await?,
    })
}

async fn fetch_dashboard_resource_catalog(
    pool: &SqlitePool,
    query: &NormalizedDashboardQuery,
) -> Result<Vec<DashboardResourceEntry>> {
    let resource_rows = sqlx::query(
        "SELECT id, resource_type, region, scenario
         FROM resources
         WHERE (? IS NULL OR resource_type = ?)
           AND (? IS NULL OR id = ?)
         ORDER BY id ASC",
    )
    .bind(query.resource_type.as_deref())
    .bind(query.resource_type.as_deref())
    .bind(query.resource_id.as_deref())
    .bind(query.resource_id.as_deref())
    .fetch_all(pool)
    .await?;

    Ok(resource_rows
        .into_iter()
        .filter_map(|row| {
            Some(DashboardResourceEntry {
                resource_id: row.try_get::<String, _>("id").ok()?,
                resource_type: row.try_get::<String, _>("resource_type").ok()?,
                region: row.try_get::<String, _>("region").ok()?,
                scenario: row.try_get::<String, _>("scenario").ok()?,
            })
        })
        .collect())
}

async fn fetch_dashboard_cost_by_resource(
    pool: &SqlitePool,
    query: &NormalizedDashboardQuery,
) -> Result<Vec<DashboardContributor>> {
    let rows = sqlx::query(
        "SELECT c.resource_id AS resource_id,
                r.resource_type AS resource_type,
                SUM(c.amount) AS total_cost
         FROM cost_records c
         JOIN resources r ON r.id = c.resource_id
         WHERE c.seconds_from_now >= ?
           AND (? IS NULL OR r.resource_type = ?)
           AND (? IS NULL OR c.resource_id = ?)
         GROUP BY c.resource_id, r.resource_type
         ORDER BY total_cost DESC, c.resource_id ASC
         LIMIT ?",
    )
    .bind(query.min_seconds)
    .bind(query.resource_type.as_deref())
    .bind(query.resource_type.as_deref())
    .bind(query.resource_id.as_deref())
    .bind(query.resource_id.as_deref())
    .bind(query.top_n as i64)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(DashboardContributor {
                resource_id: row.try_get::<String, _>("resource_id").ok()?,
                resource_type: row.try_get::<String, _>("resource_type").ok()?,
                total_cost: row.try_get::<f64, _>("total_cost").ok()?,
                average_utilization: None,
            })
        })
        .collect())
}

async fn fetch_dashboard_low_utilization_resources(
    pool: &SqlitePool,
    query: &NormalizedDashboardQuery,
    cost_by_resource: &HashMap<String, f64>,
) -> Result<Vec<DashboardContributor>> {
    let rows = sqlx::query(
        "SELECT m.resource_id AS resource_id,
                r.resource_type AS resource_type,
                AVG(m.value) AS average_utilization
         FROM metrics m
         JOIN resources r ON r.id = m.resource_id
         WHERE m.seconds_from_now >= ?
           AND (? IS NULL OR r.resource_type = ?)
           AND (? IS NULL OR m.resource_id = ?)
           AND (? IS NULL OR m.namespace = ?)
           AND (? IS NULL OR m.metric_name = ?)
         GROUP BY m.resource_id, r.resource_type",
    )
    .bind(query.min_seconds)
    .bind(query.resource_type.as_deref())
    .bind(query.resource_type.as_deref())
    .bind(query.resource_id.as_deref())
    .bind(query.resource_id.as_deref())
    .bind(query.namespace.as_deref())
    .bind(query.namespace.as_deref())
    .bind(query.metric_name.as_deref())
    .bind(query.metric_name.as_deref())
    .fetch_all(pool)
    .await?;

    let mut contributors = rows
        .into_iter()
        .filter_map(|row| {
            let resource_id = row.try_get::<String, _>("resource_id").ok()?;
            Some(DashboardContributor {
                resource_type: row.try_get::<String, _>("resource_type").ok()?,
                total_cost: cost_by_resource.get(&resource_id).copied().unwrap_or(0.0),
                average_utilization: row.try_get::<f64, _>("average_utilization").ok(),
                resource_id,
            })
        })
        .collect::<Vec<_>>();

    contributors.sort_by(|a, b| {
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
    contributors.truncate(query.top_n);

    Ok(contributors)
}

async fn fetch_dashboard_metric_rows(
    pool: &SqlitePool,
    query: &NormalizedDashboardQuery,
) -> Result<Vec<sqlx::sqlite::SqliteRow>> {
    Ok(sqlx::query(
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
    .bind(query.min_seconds)
    .bind(query.resource_type.as_deref())
    .bind(query.resource_type.as_deref())
    .bind(query.resource_id.as_deref())
    .bind(query.resource_id.as_deref())
    .bind(query.namespace.as_deref())
    .bind(query.namespace.as_deref())
    .bind(query.metric_name.as_deref())
    .bind(query.metric_name.as_deref())
    .fetch_all(pool)
    .await?)
}

async fn fetch_dashboard_cost_rows(
    pool: &SqlitePool,
    query: &NormalizedDashboardQuery,
) -> Result<Vec<sqlx::sqlite::SqliteRow>> {
    Ok(sqlx::query(
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
    .bind(query.min_seconds)
    .bind(query.resource_type.as_deref())
    .bind(query.resource_type.as_deref())
    .bind(query.resource_id.as_deref())
    .bind(query.resource_id.as_deref())
    .fetch_all(pool)
    .await?)
}

fn build_coverage_scorecard(supported_apis: &[DashboardApiEntry]) -> DashboardCoverageScorecard {
    let implemented_api_entries = supported_apis.len() as i64;
    let cloudwatch_implemented_operations = supported_apis
        .iter()
        .filter(|entry| entry.service == "cloudwatch")
        .map(|entry| entry.operation.as_str())
        .collect::<BTreeSet<_>>()
        .len() as i64;
    let cost_explorer_implemented_operations = supported_apis
        .iter()
        .filter(|entry| entry.service == "cost-explorer")
        .map(|entry| entry.operation.as_str())
        .collect::<BTreeSet<_>>()
        .len() as i64;
    let cloudwatch_summary = DashboardCoverageServiceSummary {
        total_operations: 39,
        implemented_operations: cloudwatch_implemented_operations,
        unimplemented_operations: 39 - cloudwatch_implemented_operations,
    };
    let cost_explorer_summary = DashboardCoverageServiceSummary {
        total_operations: 46,
        implemented_operations: cost_explorer_implemented_operations,
        unimplemented_operations: 46 - cost_explorer_implemented_operations,
    };

    DashboardCoverageScorecard {
        implemented_api_entries,
        implemented_tested_entries: 0,
        cloudwatch: cloudwatch_summary,
        cost_explorer: cost_explorer_summary,
        benchmarks: DashboardParityBenchmarks {
            operation_coverage: 0.0,
            input_member_coverage: 0.0,
            output_member_coverage: 0.0,
            error_model_coverage: 0.0,
            behavioral_coverage_count: 0,
        },
    }
}

fn extract_resource_id_from_query(query: &CloudWatchQuery) -> Option<String> {
    if let Some(ref name) = query.dim_name_1
        && (name == "InstanceId"
            || name == "VolumeId"
            || name == "BucketName"
            || name == "DBInstanceIdentifier"
            || name == "CacheClusterId")
    {
        return query.dim_value_1.clone();
    }
    if let Some(ref name) = query.dim_name_2
        && (name == "InstanceId"
            || name == "VolumeId"
            || name == "BucketName"
            || name == "DBInstanceIdentifier"
            || name == "CacheClusterId")
    {
        return query.dim_value_2.clone();
    }
    // Fallback to dim_value_1 if it looks like an ID
    if let Some(ref val) = query.dim_value_1
        && (val.starts_with("i-") || val.starts_with("vol-"))
    {
        return Some(val.clone());
    }
    None
}

fn parse_cloudwatch_query_from_form(body: &[u8]) -> std::result::Result<CloudWatchQuery, String> {
    let pairs: Vec<(String, String)> =
        serde_urlencoded::from_bytes(body).map_err(|e| format!("Failed to parse body: {}", e))?;
    let mut action = None;
    let mut namespace = None;
    let mut metric_name = None;
    let mut start_time = None;
    let mut end_time = None;
    let mut period = None;
    let mut max_datapoints = None;
    let mut next_token = None;
    let mut recently_active = None;
    let mut dim_name_1 = None;
    let mut dim_value_1 = None;
    let mut dim_name_2 = None;
    let mut dim_value_2 = None;
    let mut statistics = BTreeMap::new();
    let mut extended_statistics = BTreeMap::new();

    for (key, value) in pairs {
        match key.as_str() {
            "Action" => action = Some(value),
            "Namespace" => namespace = Some(value),
            "MetricName" => metric_name = Some(value),
            "StartTime" => start_time = Some(value),
            "EndTime" => end_time = Some(value),
            "Period" => {
                period = Some(
                    value
                        .parse::<i32>()
                        .map_err(|e| format!("Failed to parse body: {}", e))?,
                )
            }
            "MaxDatapoints" => {
                max_datapoints = Some(
                    value
                        .parse::<u64>()
                        .map_err(|e| format!("Failed to parse body: {}", e))?,
                )
            }
            "NextToken" => next_token = Some(value),
            "RecentlyActive" => recently_active = Some(value),
            "Dimensions.member.1.Name" => dim_name_1 = Some(value),
            "Dimensions.member.1.Value" => dim_value_1 = Some(value),
            "Dimensions.member.2.Name" => dim_name_2 = Some(value),
            "Dimensions.member.2.Value" => dim_value_2 = Some(value),
            _ => {
                let parts = key.split('.').collect::<Vec<_>>();
                match parts.as_slice() {
                    ["Statistics", "member", index] => {
                        let index = index.parse::<usize>().map_err(|_| {
                            format!("Invalid Statistics member index in '{}'.", key)
                        })?;
                        statistics.insert(index, value);
                    }
                    ["ExtendedStatistics", "member", index] => {
                        let index = index.parse::<usize>().map_err(|_| {
                            format!("Invalid ExtendedStatistics member index in '{}'.", key)
                        })?;
                        extended_statistics.insert(index, value);
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(CloudWatchQuery {
        action: action.ok_or_else(|| "Missing required field 'Action'.".to_string())?,
        namespace,
        metric_name,
        start_time,
        end_time,
        period,
        max_datapoints,
        next_token,
        recently_active,
        dim_name_1,
        dim_value_1,
        dim_name_2,
        dim_value_2,
        statistics: statistics.into_values().collect(),
        extended_statistics: extended_statistics.into_values().collect(),
    })
}

fn build_get_metric_statistics_request(
    query: CloudWatchQuery,
) -> std::result::Result<GetMetricStatisticsRequest, MetricStatisticsError> {
    let resource_id = extract_resource_id_from_query(&query);
    let namespace = query.namespace.ok_or_else(|| {
        MetricStatisticsError::Validation("Missing required field 'Namespace'.".to_string())
    })?;
    let metric_name = query.metric_name.ok_or_else(|| {
        MetricStatisticsError::Validation("Missing required field 'MetricName'.".to_string())
    })?;
    let start_time = parse_rfc3339_required("StartTime", query.start_time.as_deref())
        .map_err(MetricStatisticsError::Validation)?;
    let end_time = parse_rfc3339_required("EndTime", query.end_time.as_deref())
        .map_err(MetricStatisticsError::Validation)?;
    let period = query.period.ok_or_else(|| {
        MetricStatisticsError::Validation("Missing required field 'Period'.".to_string())
    })? as i64;

    build_get_metric_statistics_request_from_parts(MetricStatisticsRequestParts {
        namespace,
        metric_name,
        start_time,
        end_time,
        period,
        resource_id,
        statistics: query.statistics,
        extended_statistics: query.extended_statistics,
    })
}

fn build_get_metric_statistics_request_from_parts(
    parts: MetricStatisticsRequestParts,
) -> std::result::Result<GetMetricStatisticsRequest, MetricStatisticsError> {
    if parts.statistics.is_empty() && parts.extended_statistics.is_empty() {
        return Err(MetricStatisticsError::MissingParameter(
            "GetMetricStatistics requires Statistics.member.N or ExtendedStatistics.member.N."
                .to_string(),
        ));
    }
    if !parts.statistics.is_empty() && !parts.extended_statistics.is_empty() {
        return Err(MetricStatisticsError::InvalidParameterCombination(
            "GetMetricStatistics does not allow both Statistics and ExtendedStatistics."
                .to_string(),
        ));
    }
    if !parts.extended_statistics.is_empty() {
        return Err(MetricStatisticsError::Validation(
            "ExtendedStatistics are not supported.".to_string(),
        ));
    }

    let statistics = parts
        .statistics
        .into_iter()
        .map(|value| StandardStatistic::parse_metric_statistics_stat(&value))
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(GetMetricStatisticsRequest {
        namespace: parts.namespace,
        metric_name: parts.metric_name,
        start_time: parts.start_time,
        end_time: parts.end_time,
        period: parts.period,
        resource_id: parts.resource_id,
        statistics,
    })
}

fn parse_metric_data_queries_from_form(
    body: &[u8],
) -> std::result::Result<Vec<GetMetricDataQuery>, MetricDataError> {
    let pairs: Vec<(String, String)> = serde_urlencoded::from_bytes(body)
        .map_err(|e| MetricDataError::Validation(format!("Failed to parse body: {}", e)))?;
    let mut builders: BTreeMap<usize, XmlMetricDataQueryBuilder> = BTreeMap::new();

    for (key, value) in pairs {
        let parts = key.split('.').collect::<Vec<_>>();
        if parts.len() < 4 || parts[0] != "MetricDataQueries" || parts[1] != "member" {
            continue;
        }

        let query_index = parts[2].parse::<usize>().map_err(|_| {
            MetricDataError::Validation(format!(
                "Invalid MetricDataQueries member index in '{}'.",
                key
            ))
        })?;
        let builder = builders.entry(query_index).or_default();

        match parts[3..].as_ref() {
            ["Id"] => builder.id = Some(value),
            ["Label"] => builder.label = Some(value),
            ["Expression"] => builder.expression = Some(value),
            ["MetricStat", "Metric", "Namespace"] => builder.namespace = Some(value),
            ["MetricStat", "Metric", "MetricName"] => builder.metric_name = Some(value),
            ["MetricStat", "Period"] => {
                builder.period = Some(value.parse::<i64>().map_err(|_| {
                    MetricDataError::Validation(format!(
                        "Invalid period for MetricDataQueries.member.{}.",
                        query_index
                    ))
                })?)
            }
            ["MetricStat", "Stat"] => builder.stat = Some(value),
            [
                "MetricStat",
                "Metric",
                "Dimensions",
                "member",
                dimension_index,
                "Name",
            ] => {
                let dimension_index = dimension_index.parse::<usize>().map_err(|_| {
                    MetricDataError::Validation(format!(
                        "Invalid dimension index for MetricDataQueries.member.{}.",
                        query_index
                    ))
                })?;
                builder.dimensions.entry(dimension_index).or_default().name = Some(value);
            }
            [
                "MetricStat",
                "Metric",
                "Dimensions",
                "member",
                dimension_index,
                "Value",
            ] => {
                let dimension_index = dimension_index.parse::<usize>().map_err(|_| {
                    MetricDataError::Validation(format!(
                        "Invalid dimension index for MetricDataQueries.member.{}.",
                        query_index
                    ))
                })?;
                builder.dimensions.entry(dimension_index).or_default().value = Some(value);
            }
            _ => {}
        }
    }

    if builders.is_empty() {
        return Err(MetricDataError::Validation(
            "MetricDataQueries must include at least one query.".to_string(),
        ));
    }
    if builders.len() > 50 {
        return Err(MetricDataError::Validation(
            "MetricDataQueries may contain at most 50 queries.".to_string(),
        ));
    }

    builders
        .into_iter()
        .map(|(query_index, builder)| {
            let id = builder.id.ok_or_else(|| {
                MetricDataError::Validation(format!(
                    "Missing required field 'MetricDataQueries.member.{}.Id'.",
                    query_index
                ))
            })?;
            let namespace = builder.namespace.ok_or_else(|| {
                MetricDataError::Validation(format!(
                    "Missing required field 'MetricDataQueries.member.{}.MetricStat.Metric.Namespace'.",
                    query_index
                ))
            })?;
            let metric_name = builder.metric_name.ok_or_else(|| {
                MetricDataError::Validation(format!(
                    "Missing required field 'MetricDataQueries.member.{}.MetricStat.Metric.MetricName'.",
                    query_index
                ))
            })?;
            let period = builder.period.ok_or_else(|| {
                MetricDataError::Validation(format!(
                    "Missing required field 'MetricDataQueries.member.{}.MetricStat.Period'.",
                    query_index
                ))
            })?;
            let stat = builder.stat.ok_or_else(|| {
                MetricDataError::Validation(format!(
                    "Missing required field 'MetricDataQueries.member.{}.MetricStat.Stat'.",
                    query_index
                ))
            })?;
            let dimensions = builder
                .dimensions
                .into_values()
                .filter_map(|dimension| match (dimension.name, dimension.value) {
                    (Some(name), Some(value)) => Some(GetMetricDimension { name, value }),
                    _ => None,
                })
                .collect::<Vec<_>>();

            Ok(GetMetricDataQuery {
                id,
                metric_stat: Some(GetMetricStatRequest {
                    metric: GetMetricRequest {
                        namespace,
                        metric_name,
                        dimensions: (!dimensions.is_empty()).then_some(dimensions),
                    },
                    period,
                    stat,
                }),
                expression: builder.expression,
                label: builder.label,
            })
        })
        .collect()
}

async fn build_metric_data_series_list(
    pool: &SqlitePool,
    metric_data_queries: Vec<GetMetricDataQuery>,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    injected_now: Option<DateTime<Utc>>,
) -> std::result::Result<Vec<MetricDataSeries>, MetricDataError> {
    let mut series_list = Vec::with_capacity(metric_data_queries.len());

    for query in metric_data_queries {
        if query.expression.is_some() {
            return Err(MetricDataError::Validation(
                "Metric math expressions are not supported.".to_string(),
            ));
        }
        let metric_stat = query.metric_stat.ok_or_else(|| {
            MetricDataError::Validation("MetricDataQueries must include MetricStat.".to_string())
        })?;
        let metric = metric_stat.metric;

        let series = build_metric_data_series(
            pool,
            MetricDataSeriesRequest {
                id: query.id,
                label: query.label.unwrap_or(metric.metric_name.clone()),
                namespace: metric.namespace,
                metric_name: metric.metric_name,
                resource_id: extract_resource_id_from_dimensions(metric.dimensions.as_deref()),
                stat: metric_stat.stat,
                period: metric_stat.period,
                start_time,
                end_time,
                injected_now,
            },
        )
        .await?;

        series_list.push(series);
    }

    Ok(series_list)
}

fn paginate_metric_data_series(
    series_list: Vec<MetricDataSeries>,
    page_start: usize,
    max_datapoints: usize,
) -> std::result::Result<PaginatedMetricDataSeries, MetricDataError> {
    let mut max_series_len = 0usize;
    for series in &series_list {
        max_series_len = max_series_len.max(series.timestamps.len());
    }

    if page_start > max_series_len {
        return Err(MetricDataError::InvalidNextToken(
            "NextToken points past available results.".to_string(),
        ));
    }

    let page_end = page_start.saturating_add(max_datapoints);
    let has_more = page_end < max_series_len;

    let results = series_list
        .into_iter()
        .map(|series| {
            let series_end = std::cmp::min(page_end, series.timestamps.len());
            let (timestamps, values) = if page_start >= series.timestamps.len() {
                (Vec::new(), Vec::new())
            } else {
                (
                    series.timestamps[page_start..series_end].to_vec(),
                    series.values[page_start..series_end].to_vec(),
                )
            };

            MetricDataSeries {
                id: series.id,
                label: series.label,
                timestamps,
                values,
            }
        })
        .collect::<Vec<_>>();

    Ok(PaginatedMetricDataSeries {
        results,
        next_token: has_more.then(|| page_end.to_string()),
    })
}

async fn build_metric_data_series(
    pool: &SqlitePool,
    request: MetricDataSeriesRequest,
) -> std::result::Result<MetricDataSeries, MetricDataError> {
    let points = metrics::query_metrics(
        pool,
        MetricQueryParams {
            resource_id: request.resource_id,
            metric_name: Some(request.metric_name),
            namespace: Some(request.namespace),
            start_time: Some(request.start_time),
            end_time: Some(request.end_time),
            limit: Some(10_000),
            injected_now: request.injected_now,
        },
    )
    .await
    .map_err(MetricDataError::Internal)?;

    let stat = StandardStatistic::parse_metric_data_stat(&request.stat)?;
    let aggregated = aggregate_metric_points(
        &points,
        request.start_time,
        request.end_time,
        request.period,
        stat,
    )?;

    Ok(MetricDataSeries {
        id: request.id,
        label: request.label,
        timestamps: aggregated
            .iter()
            .map(|point| point.timestamp.to_rfc3339())
            .collect(),
        values: aggregated.iter().map(|point| point.value).collect(),
    })
}

fn error_response(
    protocol: Protocol,
    code: &str,
    message: &str,
    status: StatusCode,
) -> axum::response::Response {
    match protocol {
        Protocol::Json => {
            let body = Json(aws::json_error(code, message));
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
            let body = aws::xml_error(code, message);
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

fn parse_cloudwatch_datetime_required(
    field_name: &str,
    value: Option<&CloudWatchDateTimeInput>,
) -> std::result::Result<DateTime<Utc>, String> {
    match value.ok_or_else(|| format!("Missing required field '{}'.", field_name))? {
        CloudWatchDateTimeInput::String(raw) => DateTime::parse_from_rfc3339(raw)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| format!("Invalid {}: {}", field_name, e)),
        CloudWatchDateTimeInput::Integer(epoch_seconds) => {
            DateTime::from_timestamp(*epoch_seconds, 0)
                .ok_or_else(|| format!("Invalid {} epoch seconds '{}'.", field_name, epoch_seconds))
        }
        CloudWatchDateTimeInput::Float(epoch_seconds) => {
            let whole_seconds = epoch_seconds.trunc() as i64;
            let nanos = ((epoch_seconds.fract().abs()) * 1_000_000_000.0).round() as u32;
            DateTime::from_timestamp(whole_seconds, nanos)
                .ok_or_else(|| format!("Invalid {} epoch seconds '{}'.", field_name, epoch_seconds))
        }
    }
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
        "elasticache" => "Amazon ElastiCache",
        "s3" => "Amazon Simple Storage Service",
        "elb" => "Elastic Load Balancing",
        _ => "AWS Service",
    }
}

fn ce_usage_type_from_resource_type(resource_type: &str) -> &'static str {
    match resource_type {
        "ec2" => "USE1-BoxUsage:m6i.xlarge",
        "rds" => "USE1-InstanceUsage:db.t3.medium",
        "elasticache" => "USE1-NodeUsage:cache.t3.micro",
        "elb" => "USE1-LoadBalancerUsage",
        "s3" => "TimedStorage-ByteHrs",
        _ => "Usage",
    }
}

fn cloudwatch_dimension_name_for_resource_type(resource_type: &str) -> Option<&'static str> {
    match resource_type {
        "ec2" => Some("InstanceId"),
        "s3" => Some("BucketName"),
        "rds" => Some("DBInstanceIdentifier"),
        "elb" => Some("LoadBalancer"),
        "elasticache" => Some("CacheClusterId"),
        _ => None,
    }
}

fn canonical_cost_explorer_operation(target: &str) -> Option<&str> {
    target
        .strip_prefix("AWSCostExplorer.")
        .or_else(|| target.strip_prefix("AWSInsightsIndexService."))
}

fn canonical_tagging_operation(target: &str) -> Option<&str> {
    target.strip_prefix("ResourceGroupsTaggingAPI_20170126.")
}

fn canonical_pricing_operation(target: &str) -> Option<&str> {
    target.strip_prefix("AWSPriceListService.")
}

fn canonical_compute_optimizer_operation(target: &str) -> Option<&str> {
    target.strip_prefix("ComputeOptimizerService.")
}

fn canonical_cur_operation(target: &str) -> Option<&str> {
    target.strip_prefix("AWSOrigamiServiceGatewayService.")
}

fn mock_account_id() -> &'static str {
    "123456789012"
}

fn resource_arn(resource_type: &str, region: &str, resource_id: &str) -> String {
    match resource_type {
        "ec2" => format!(
            "arn:aws:ec2:{region}:{}:instance/{resource_id}",
            mock_account_id()
        ),
        "rds" => format!(
            "arn:aws:rds:{region}:{}:db:{resource_id}",
            mock_account_id()
        ),
        "s3" => format!("arn:aws:s3:::{resource_id}"),
        "elb" => format!(
            "arn:aws:elasticloadbalancing:{region}:{}:loadbalancer/app/{resource_id}/mock",
            mock_account_id()
        ),
        "elasticache" => format!(
            "arn:aws:elasticache:{region}:{}:cluster:{resource_id}",
            mock_account_id()
        ),
        _ => format!(
            "arn:aws:{resource_type}:{region}:{}:{resource_id}",
            mock_account_id()
        ),
    }
}

fn tagging_resource_type_filter(resource_type: &str) -> &'static str {
    match resource_type {
        "ec2" => "ec2:instance",
        "rds" => "rds:db",
        "s3" => "s3:bucket",
        "elb" => "elasticloadbalancing:loadbalancer",
        "elasticache" => "elasticache:cluster",
        _ => "resource",
    }
}

fn resource_matches_type_filters(resource_type_filter: &str, filters: &[String]) -> bool {
    if filters.is_empty() {
        return true;
    }
    let normalized = resource_type_filter.to_ascii_lowercase();
    let normalized_service = normalized.split(':').next().unwrap_or(normalized.as_str());
    filters.iter().any(|filter| {
        let filter_lower = filter.to_ascii_lowercase();
        filter_lower == normalized
            || filter_lower == normalized_service
            || normalized.starts_with(&filter_lower)
    })
}

fn tagged_resource_matches_filters(resource: &TaggedResource, filters: &[TagFilterInput]) -> bool {
    if filters.is_empty() {
        return true;
    }

    let tags = resource
        .tags
        .iter()
        .cloned()
        .collect::<HashMap<String, String>>();

    filters.iter().all(|filter| {
        let Some(value) = tags.get(&filter.key) else {
            return false;
        };
        match filter.values.as_ref() {
            Some(values) if !values.is_empty() => values.iter().any(|candidate| candidate == value),
            _ => true,
        }
    })
}

fn merge_filter_values(
    slot: &mut Option<BTreeSet<String>>,
    incoming: BTreeSet<String>,
) -> std::result::Result<(), CostUsageError> {
    if incoming.is_empty() {
        return Ok(());
    }

    match slot {
        Some(existing) => {
            let intersection = existing
                .intersection(&incoming)
                .cloned()
                .collect::<BTreeSet<_>>();
            *existing = intersection;
        }
        None => {
            *slot = Some(incoming);
        }
    }

    Ok(())
}

fn merge_tag_filter_values(
    tags: &mut BTreeMap<String, BTreeSet<String>>,
    key: String,
    values: BTreeSet<String>,
) {
    if values.is_empty() {
        return;
    }

    if let Some(existing) = tags.get_mut(&key) {
        let intersection = existing
            .intersection(&values)
            .cloned()
            .collect::<BTreeSet<_>>();
        *existing = intersection;
    } else {
        tags.insert(key, values);
    }
}

fn parse_ce_filter_expr(
    expr: &Value,
    criteria: &mut CeFilterCriteria,
) -> std::result::Result<(), CostUsageError> {
    let Some(obj) = expr.as_object() else {
        return Err(CostUsageError::Validation(
            "Cost Explorer Filter must be a JSON object.".to_string(),
        ));
    };

    if let Some(and_values) = obj.get("And") {
        let items = and_values.as_array().ok_or_else(|| {
            CostUsageError::Validation("Filter.And must be an array.".to_string())
        })?;
        for item in items {
            parse_ce_filter_expr(item, criteria)?;
        }
        return Ok(());
    }

    if obj.contains_key("Or") || obj.contains_key("Not") {
        return Err(CostUsageError::Validation(
            "Only Filter.Dimensions, Filter.Tags, and Filter.And are supported.".to_string(),
        ));
    }

    if let Some(dimensions) = obj.get("Dimensions") {
        let dim_obj = dimensions.as_object().ok_or_else(|| {
            CostUsageError::Validation("Filter.Dimensions must be an object.".to_string())
        })?;
        let key = dim_obj.get("Key").and_then(Value::as_str).ok_or_else(|| {
            CostUsageError::Validation("Filter.Dimensions must include Key.".to_string())
        })?;
        let values = dim_obj
            .get("Values")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                CostUsageError::Validation("Filter.Dimensions must include Values.".to_string())
            })?
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect::<BTreeSet<_>>();

        match key {
            "SERVICE" => merge_filter_values(&mut criteria.services, values)?,
            "RESOURCE_ID" => merge_filter_values(&mut criteria.resource_ids, values)?,
            "REGION" => merge_filter_values(&mut criteria.regions, values)?,
            _ => {
                return Err(CostUsageError::Validation(format!(
                    "Unsupported Filter.Dimensions Key '{}'.",
                    key
                )));
            }
        }

        return Ok(());
    }

    if let Some(tags) = obj.get("Tags") {
        let tag_obj = tags.as_object().ok_or_else(|| {
            CostUsageError::Validation("Filter.Tags must be an object.".to_string())
        })?;
        let key = tag_obj.get("Key").and_then(Value::as_str).ok_or_else(|| {
            CostUsageError::Validation("Filter.Tags must include Key.".to_string())
        })?;
        let values = tag_obj
            .get("Values")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                CostUsageError::Validation("Filter.Tags must include Values.".to_string())
            })?
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect::<BTreeSet<_>>();
        merge_tag_filter_values(&mut criteria.tags, key.to_string(), values);
        return Ok(());
    }

    Err(CostUsageError::Validation(
        "Unsupported Cost Explorer filter expression.".to_string(),
    ))
}

fn parse_ce_filter(
    filter: Option<&Value>,
) -> std::result::Result<CeFilterCriteria, CostUsageError> {
    let mut criteria = CeFilterCriteria::default();
    if let Some(filter) = filter {
        parse_ce_filter_expr(filter, &mut criteria)?;
    }
    Ok(criteria)
}

fn parse_group_by_dimension(
    group_by: Option<&Vec<Value>>,
) -> std::result::Result<Option<&str>, CostUsageError> {
    let Some(group_by) = group_by else {
        return Ok(None);
    };
    if group_by.is_empty() {
        return Ok(None);
    }
    if group_by.len() > 1 {
        return Err(CostUsageError::Validation(
            "Only a single GroupBy entry is supported.".to_string(),
        ));
    }

    let entry = &group_by[0];
    let group_type = entry
        .get("Type")
        .and_then(Value::as_str)
        .unwrap_or("DIMENSION");
    let group_key = entry.get("Key").and_then(Value::as_str).ok_or_else(|| {
        CostUsageError::Validation("GroupBy entry must include a Key.".to_string())
    })?;

    if group_type != "DIMENSION" {
        return Err(CostUsageError::Validation(
            "Only GroupBy Type 'DIMENSION' is supported.".to_string(),
        ));
    }

    match group_key {
        "SERVICE" | "REGION" | "RESOURCE_ID" | "USAGE_TYPE" => Ok(Some(group_key)),
        _ => Err(CostUsageError::Validation(format!(
            "Unsupported GroupBy Key '{}'.",
            group_key
        ))),
    }
}

fn parse_resource_tags(tags_json: Option<&str>) -> HashMap<String, String> {
    tags_json
        .and_then(|raw| serde_json::from_str::<HashMap<String, String>>(raw).ok())
        .unwrap_or_default()
}

fn cost_row_matches_filter(row: &CostRow, criteria: &CeFilterCriteria) -> bool {
    if let Some(services) = criteria.services.as_ref()
        && !services.contains(ce_service_name_from_resource_type(&row.resource_type))
    {
        return false;
    }
    if let Some(resource_ids) = criteria.resource_ids.as_ref()
        && !resource_ids.contains(&row.resource_id)
    {
        return false;
    }
    if let Some(regions) = criteria.regions.as_ref()
        && !regions.contains(&row.region)
    {
        return false;
    }
    if !criteria.tags.is_empty() {
        let tags = parse_resource_tags(row.tags_json.as_deref());
        for (key, allowed_values) in &criteria.tags {
            let Some(value) = tags.get(key) else {
                return false;
            };
            if !allowed_values.contains(value) {
                return false;
            }
        }
    }
    true
}

async fn fetch_cost_rows_for_window(
    pool: &SqlitePool,
    start_offset: i64,
    end_offset: i64,
) -> std::result::Result<Vec<CostRow>, CostUsageError> {
    let rows = sqlx::query(
        "SELECT c.resource_id, c.seconds_from_now, c.amount, r.resource_type, r.region, r.tags
         FROM cost_records c
         JOIN resources r ON r.id = c.resource_id
         WHERE c.seconds_from_now >= ? AND c.seconds_from_now <= ?
         ORDER BY c.seconds_from_now ASC, c.resource_id ASC",
    )
    .bind(start_offset)
    .bind(end_offset)
    .fetch_all(pool)
    .await
    .map_err(|e| CostUsageError::Internal(e.into()))?;

    Ok(rows
        .into_iter()
        .map(|row| CostRow {
            resource_id: row.get::<String, _>("resource_id"),
            resource_type: row.get::<String, _>("resource_type"),
            region: row.get::<String, _>("region"),
            amount: row.get::<f64, _>("amount"),
            seconds_from_now: row.get::<i64, _>("seconds_from_now"),
            tags_json: row.try_get::<Option<String>, _>("tags").ok().flatten(),
        })
        .collect())
}

async fn fetch_tagged_resources(
    pool: &SqlitePool,
) -> std::result::Result<Vec<TaggedResource>, CostUsageError> {
    let rows = sqlx::query("SELECT id, resource_type, region, tags FROM resources ORDER BY id ASC")
        .fetch_all(pool)
        .await
        .map_err(|e| CostUsageError::Internal(e.into()))?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let resource_id = row.get::<String, _>("id");
            let resource_type = row.get::<String, _>("resource_type");
            let region = row.get::<String, _>("region");
            let tags = parse_resource_tags(
                row.try_get::<Option<String>, _>("tags")
                    .ok()
                    .flatten()
                    .as_deref(),
            )
            .into_iter()
            .collect::<Vec<_>>();

            TaggedResource {
                arn: resource_arn(&resource_type, &region, &resource_id),
                resource_type_filter: tagging_resource_type_filter(&resource_type).to_string(),
                tags,
            }
        })
        .collect())
}

fn pricing_catalog() -> Vec<PricingProduct> {
    let attributes = |pairs: &[(&'static str, &'static str)]| {
        pairs
            .iter()
            .map(|(key, value)| (*key, (*value).to_string()))
            .collect::<BTreeMap<&'static str, String>>()
    };

    vec![
        PricingProduct {
            service_code: "AmazonEC2",
            attributes: attributes(&[
                ("servicecode", "AmazonEC2"),
                ("location", "US East (N. Virginia)"),
                ("locationType", "AWS Region"),
                ("instanceType", "m6i.large"),
                ("operatingSystem", "Linux"),
                ("tenancy", "Shared"),
                ("capacitystatus", "Used"),
                ("preInstalledSw", "NA"),
                ("productFamily", "Compute Instance"),
            ]),
            rate_code: "AmazonEC2.m6i.large.ondemand".to_string(),
            description: "m6i.large On Demand Linux instance usage".to_string(),
            unit: "Hrs",
            price_per_unit_usd: "0.0960".to_string(),
        },
        PricingProduct {
            service_code: "AmazonEC2",
            attributes: attributes(&[
                ("servicecode", "AmazonEC2"),
                ("location", "US East (N. Virginia)"),
                ("locationType", "AWS Region"),
                ("instanceType", "m6i.xlarge"),
                ("operatingSystem", "Linux"),
                ("tenancy", "Shared"),
                ("capacitystatus", "Used"),
                ("preInstalledSw", "NA"),
                ("productFamily", "Compute Instance"),
            ]),
            rate_code: "AmazonEC2.m6i.xlarge.ondemand".to_string(),
            description: "m6i.xlarge On Demand Linux instance usage".to_string(),
            unit: "Hrs",
            price_per_unit_usd: "0.1920".to_string(),
        },
        PricingProduct {
            service_code: "AmazonEC2",
            attributes: attributes(&[
                ("servicecode", "AmazonEC2"),
                ("location", "US East (N. Virginia)"),
                ("locationType", "AWS Region"),
                ("instanceType", "t3.medium"),
                ("operatingSystem", "Linux"),
                ("tenancy", "Shared"),
                ("capacitystatus", "Used"),
                ("preInstalledSw", "NA"),
                ("productFamily", "Compute Instance"),
            ]),
            rate_code: "AmazonEC2.t3.medium.ondemand".to_string(),
            description: "t3.medium On Demand Linux instance usage".to_string(),
            unit: "Hrs",
            price_per_unit_usd: "0.0416".to_string(),
        },
        PricingProduct {
            service_code: "AmazonEC2",
            attributes: attributes(&[
                ("servicecode", "AmazonEC2"),
                ("location", "US East (N. Virginia)"),
                ("locationType", "AWS Region"),
                ("volumeType", "gp3"),
                ("storageMedia", "SSD-backed"),
                ("maxVolumeSize", "16384"),
                ("productFamily", "Storage"),
            ]),
            rate_code: "AmazonEC2.gp3.storage".to_string(),
            description: "gp3 Provisioned SSD storage".to_string(),
            unit: "GB-Mo",
            price_per_unit_usd: "0.0800".to_string(),
        },
        PricingProduct {
            service_code: "AmazonRDS",
            attributes: attributes(&[
                ("servicecode", "AmazonRDS"),
                ("location", "US East (N. Virginia)"),
                ("locationType", "AWS Region"),
                ("instanceType", "db.t3.medium"),
                ("databaseEngine", "PostgreSQL"),
                ("deploymentOption", "Single-AZ"),
                ("productFamily", "Database Instance"),
            ]),
            rate_code: "AmazonRDS.db.t3.medium.ondemand".to_string(),
            description: "db.t3.medium Single-AZ PostgreSQL instance usage".to_string(),
            unit: "Hrs",
            price_per_unit_usd: "0.0670".to_string(),
        },
        PricingProduct {
            service_code: "AmazonRDS",
            attributes: attributes(&[
                ("servicecode", "AmazonRDS"),
                ("location", "US East (N. Virginia)"),
                ("locationType", "AWS Region"),
                ("instanceType", "db.m6g.large"),
                ("databaseEngine", "MySQL"),
                ("deploymentOption", "Multi-AZ"),
                ("productFamily", "Database Instance"),
            ]),
            rate_code: "AmazonRDS.db.m6g.large.ondemand".to_string(),
            description: "db.m6g.large Multi-AZ MySQL instance usage".to_string(),
            unit: "Hrs",
            price_per_unit_usd: "0.3380".to_string(),
        },
        PricingProduct {
            service_code: "AmazonS3",
            attributes: attributes(&[
                ("servicecode", "AmazonS3"),
                ("location", "US East (N. Virginia)"),
                ("locationType", "AWS Region"),
                ("storageClass", "General Purpose"),
                ("volumeType", "Standard"),
                ("productFamily", "Storage"),
            ]),
            rate_code: "AmazonS3.standard.storage".to_string(),
            description: "Amazon S3 Standard storage usage".to_string(),
            unit: "GB-Mo",
            price_per_unit_usd: "0.0230".to_string(),
        },
        PricingProduct {
            service_code: "AmazonS3",
            attributes: attributes(&[
                ("servicecode", "AmazonS3"),
                ("location", "US East (N. Virginia)"),
                ("locationType", "AWS Region"),
                ("storageClass", "Infrequent Access"),
                ("volumeType", "Standard-IA"),
                ("productFamily", "Storage"),
            ]),
            rate_code: "AmazonS3.standard_ia.storage".to_string(),
            description: "Amazon S3 Standard-Infrequent Access storage usage".to_string(),
            unit: "GB-Mo",
            price_per_unit_usd: "0.0125".to_string(),
        },
        PricingProduct {
            service_code: "AWSELB",
            attributes: attributes(&[
                ("servicecode", "AWSELB"),
                ("location", "US East (N. Virginia)"),
                ("locationType", "AWS Region"),
                ("productFamily", "Load Balancer"),
                ("loadBalancerType", "Application"),
            ]),
            rate_code: "AWSELB.alb.usage".to_string(),
            description: "Application Load Balancer hourly usage".to_string(),
            unit: "Hrs",
            price_per_unit_usd: "0.0225".to_string(),
        },
        PricingProduct {
            service_code: "AWSELB",
            attributes: attributes(&[
                ("servicecode", "AWSELB"),
                ("location", "US East (N. Virginia)"),
                ("locationType", "AWS Region"),
                ("productFamily", "Load Balancer"),
                ("loadBalancerType", "Network"),
            ]),
            rate_code: "AWSELB.nlb.usage".to_string(),
            description: "Network Load Balancer hourly usage".to_string(),
            unit: "Hrs",
            price_per_unit_usd: "0.0225".to_string(),
        },
    ]
}

fn pricing_product_matches_filters(
    product: &PricingProduct,
    filters: &[PricingFilterInput],
) -> bool {
    filters.iter().all(|filter| {
        if !filter.filter_type.eq_ignore_ascii_case("TERM_MATCH") {
            return false;
        }
        if filter.field.eq_ignore_ascii_case("ServiceCode") {
            return product.service_code == filter.value;
        }
        product
            .attributes
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(filter.field.as_str()))
            .is_some_and(|(_, value)| value == &filter.value)
    })
}

fn pricing_product_to_value(product: &PricingProduct) -> Value {
    let sku = format!("sku-{}", product.rate_code);
    let attributes = product
        .attributes
        .iter()
        .map(|(k, v)| ((*k).to_string(), Value::String(v.clone())))
        .collect::<serde_json::Map<String, Value>>();

    json!({
        "product": {
            "sku": sku,
            "productFamily": product.attributes.get("productFamily").cloned().unwrap_or_else(|| "Service".to_string()),
            "attributes": Value::Object(attributes)
        },
        "serviceCode": product.service_code,
        "terms": {
            "OnDemand": {
                product.rate_code.clone(): {
                    "priceDimensions": {
                        format!("{}.dimension", product.rate_code): {
                            "unit": product.unit,
                            "description": product.description,
                            "pricePerUnit": {
                                "USD": product.price_per_unit_usd
                            }
                        }
                    }
                }
            }
        }
    })
}

fn mock_usage_rate_for_resource_type(resource_type: &str) -> f64 {
    match resource_type {
        "ec2" => 0.0960,
        "rds" => 0.0670,
        "elasticache" => 0.0340,
        "elb" => 0.0225,
        "s3" => 0.0230,
        _ => 1.0,
    }
}

fn resource_name_from_tags(tags_json: Option<&str>, fallback: &str) -> String {
    parse_resource_tags(tags_json)
        .get("Name")
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

fn clamp_page_size(requested: Option<u64>, default_size: usize, max_size: usize) -> usize {
    requested
        .unwrap_or(default_size as u64)
        .clamp(1, max_size as u64) as usize
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
        | (_, "CurrConnections")
        | (_, "NumberOfObjects") => "Count",
        ("AWS/RDS", "ReadIOPS") | ("AWS/RDS", "WriteIOPS") => "Count/Second",
        (_, "TargetResponseTime") => "Seconds",
        (_, "FreeableMemory") | (_, "BucketSizeBytes") => "Bytes",
        _ => "None",
    }
}

fn extract_resource_id_from_dimensions(
    dimensions: Option<&[GetMetricDimension]>,
) -> Option<String> {
    dimensions.and_then(|dims| {
        dims.iter().find_map(|dimension| {
            if dimension.name == "InstanceId"
                || dimension.name == "VolumeId"
                || dimension.name == "BucketName"
                || dimension.name == "DBInstanceIdentifier"
                || dimension.name == "CacheClusterId"
            {
                return Some(dimension.value.clone());
            }
            None
        })
    })
}

fn aggregate_metric_buckets(
    points: &[metrics::MetricPoint],
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    period_seconds: i64,
) -> std::result::Result<Vec<AggregatedMetricPoint>, MetricDataError> {
    if period_seconds <= 0 {
        return Err(MetricDataError::Validation(
            "MetricStat.Period must be greater than 0.".to_string(),
        ));
    }

    let mut buckets: BTreeMap<i64, MetricBucketAccumulator> = BTreeMap::new();
    for point in points {
        if point.timestamp < start_time || point.timestamp > end_time {
            continue;
        }
        let offset_seconds = (point.timestamp - start_time).num_seconds();
        if offset_seconds < 0 {
            continue;
        }
        let bucket_index = offset_seconds / period_seconds;
        buckets
            .entry(bucket_index)
            .and_modify(|bucket| bucket.record(point.value))
            .or_insert_with(|| MetricBucketAccumulator::new(point.value));
    }

    Ok(buckets
        .into_iter()
        .map(|(bucket_index, bucket)| {
            let timestamp = start_time + chrono::Duration::seconds(bucket_index * period_seconds);
            bucket.into_point(timestamp)
        })
        .collect())
}

fn aggregate_metric_points(
    points: &[metrics::MetricPoint],
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    period_seconds: i64,
    stat: StandardStatistic,
) -> std::result::Result<Vec<metrics::MetricPoint>, MetricDataError> {
    Ok(
        aggregate_metric_buckets(points, start_time, end_time, period_seconds)?
            .into_iter()
            .map(|point| metrics::MetricPoint {
                value: stat.value(&point),
                timestamp: point.timestamp,
            })
            .collect(),
    )
}

fn ensure_admin_authorized(
    headers: &HeaderMap,
) -> std::result::Result<(), Box<axum::response::Response>> {
    let expected = std::env::var("AWS_MOCK_ADMIN_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    ensure_admin_authorized_with_expected(headers, expected.as_deref())
}

fn ensure_admin_authorized_with_expected(
    headers: &HeaderMap,
    expected: Option<&str>,
) -> std::result::Result<(), Box<axum::response::Response>> {
    if let Some(token) = expected {
        let provided = headers
            .get(ADMIN_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::trim);

        if provided != Some(token) {
            return Err(Box::new(
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({
                        "error": "Unauthorized admin request"
                    })),
                )
                    .into_response(),
            ));
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
    } else if target.starts_with("ResourceGroupsTaggingAPI_20170126.") {
        handle_resource_groups_tagging(target, pool, body, protocol).await
    } else if target.starts_with("AWSPriceListService.") {
        handle_pricing(target, body, protocol).await
    } else if target.starts_with("ComputeOptimizerService.") {
        handle_compute_optimizer(target, pool, body, protocol).await
    } else if target.starts_with("AWSOrigamiServiceGatewayService.") {
        handle_cur(target, body, protocol).await
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

async fn handle_resource_groups_tagging(
    target: &str,
    pool: SqlitePool,
    body: Bytes,
    protocol: Protocol,
) -> axum::response::Response {
    match canonical_tagging_operation(target) {
        Some("GetResources") => match handle_get_resources(pool, body).await {
            Ok(res) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                Json(res),
            )
                .into_response(),
            Err(CostUsageError::Validation(message)) => error_response(
                protocol,
                "InvalidParameterException",
                &message,
                StatusCode::BAD_REQUEST,
            ),
            Err(CostUsageError::Internal(error)) => error_response(
                protocol,
                "InternalServiceException",
                &error.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        },
        Some("GetTagKeys") => match handle_get_tag_keys(pool, body).await {
            Ok(res) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                Json(res),
            )
                .into_response(),
            Err(CostUsageError::Validation(message)) => error_response(
                protocol,
                "InvalidParameterException",
                &message,
                StatusCode::BAD_REQUEST,
            ),
            Err(CostUsageError::Internal(error)) => error_response(
                protocol,
                "InternalServiceException",
                &error.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        },
        Some("GetTagValues") => match handle_get_tag_values(pool, body).await {
            Ok(res) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                Json(res),
            )
                .into_response(),
            Err(CostUsageError::Validation(message)) => error_response(
                protocol,
                "InvalidParameterException",
                &message,
                StatusCode::BAD_REQUEST,
            ),
            Err(CostUsageError::Internal(error)) => error_response(
                protocol,
                "InternalServiceException",
                &error.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        },
        _ => error_response(
            protocol,
            "UnknownAction",
            "The action is not supported",
            StatusCode::BAD_REQUEST,
        ),
    }
}

async fn handle_pricing(target: &str, body: Bytes, protocol: Protocol) -> axum::response::Response {
    match canonical_pricing_operation(target) {
        Some("GetProducts") => match handle_get_products(body).await {
            Ok(res) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-amz-json-1.1")],
                Json(res),
            )
                .into_response(),
            Err(CostUsageError::Validation(message)) => error_response(
                protocol,
                "InvalidParameterException",
                &message,
                StatusCode::BAD_REQUEST,
            ),
            Err(CostUsageError::Internal(error)) => error_response(
                protocol,
                "InternalErrorException",
                &error.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        },
        _ => error_response(
            protocol,
            "UnknownAction",
            "The action is not supported",
            StatusCode::BAD_REQUEST,
        ),
    }
}

async fn handle_compute_optimizer(
    target: &str,
    pool: SqlitePool,
    body: Bytes,
    protocol: Protocol,
) -> axum::response::Response {
    match canonical_compute_optimizer_operation(target) {
        Some("GetEC2InstanceRecommendations") => {
            match handle_get_ec2_instance_recommendations(pool, body).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.0")],
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
                    "InternalServerException",
                    &error.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        Some("GetEBSVolumeRecommendations") => {
            match handle_get_ebs_volume_recommendations(pool, body).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.0")],
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
                    "InternalServerException",
                    &error.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        _ => error_response(
            protocol,
            "UnknownAction",
            "The action is not supported",
            StatusCode::BAD_REQUEST,
        ),
    }
}

async fn handle_cur(target: &str, body: Bytes, protocol: Protocol) -> axum::response::Response {
    match canonical_cur_operation(target) {
        Some("DescribeReportDefinitions") => match handle_describe_report_definitions(body).await {
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
                "InternalErrorException",
                &error.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        },
        _ => error_response(
            protocol,
            "UnknownAction",
            "The action is not supported",
            StatusCode::BAD_REQUEST,
        ),
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

    let fixture_status = fixture::read_state(&pool)
        .await
        .ok()
        .and_then(|state| serde_json::from_slice::<Value>(&state.status_bytes).ok())
        .unwrap_or_else(|| json!({"status": "UNAVAILABLE"}));

    Json(json!({
        "status": "online",
        "resource_count": res_count,
        "metric_count": metric_count,
        "version": env!("CARGO_PKG_VERSION"),
        "fixture": fixture_status
    }))
}

fn fixture_document_response(status: StatusCode, bytes: Vec<u8>) -> axum::response::Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        bytes,
    )
        .into_response()
}

fn fixture_error_response(
    status: StatusCode,
    message: impl Into<String>,
) -> axum::response::Response {
    let payload = json!({
        "schema": "foxtail.release-fixture-error/v1",
        "error": "fixture_request_failed",
        "message": message.into()
    });
    fixture_document_response(
        status,
        fixture::canonical_bytes(&payload)
            .unwrap_or_else(|_| b"{\"error\":\"fixture_request_failed\"}".to_vec()),
    )
}

async fn fixture_definition_handler(
    Query(query): Query<FixtureVersionQuery>,
) -> axum::response::Response {
    if let Err(error) = fixture::validate_version(query.version.as_deref()) {
        return fixture_error_response(StatusCode::BAD_REQUEST, error.to_string());
    }
    match fixture::canonical_definition() {
        Ok((bytes, _)) => fixture_document_response(StatusCode::OK, bytes),
        Err(error) => fixture_error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn fixture_realize_handler(
    State(pool): State<SqlitePool>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    if let Err(response) = ensure_admin_authorized(&headers) {
        return *response;
    }
    fixture_realize_response(pool, body).await
}

async fn fixture_realize_response(pool: SqlitePool, body: Bytes) -> axum::response::Response {
    let request = match fixture::parse_json_request(&body) {
        Ok(request) => request,
        Err(error) => return fixture_error_response(StatusCode::BAD_REQUEST, error.to_string()),
    };
    if let Err(error) = fixture::validate_version(request.version.as_deref()) {
        return fixture_error_response(StatusCode::BAD_REQUEST, error.to_string());
    }
    match fixture::realize(&pool, request).await {
        Ok(snapshot) => match fixture::realization_response(&snapshot) {
            Ok(bytes) => fixture_document_response(StatusCode::OK, bytes),
            Err(error) => {
                fixture_error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            }
        },
        Err(error) => fixture_error_response(StatusCode::UNPROCESSABLE_ENTITY, error.to_string()),
    }
}

async fn fixture_status_handler(State(pool): State<SqlitePool>) -> axum::response::Response {
    match fixture::read_state(&pool).await {
        Ok(state) => fixture_document_response(StatusCode::OK, state.status_bytes),
        Err(error) => fixture_error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn fixture_manifest_handler(State(pool): State<SqlitePool>) -> axum::response::Response {
    match fixture::read_state(&pool).await {
        Ok(state) => match state.manifest_bytes {
            Some(bytes) => fixture_document_response(StatusCode::OK, bytes),
            None => fixture_error_response(StatusCode::NOT_FOUND, "fixture has not been realized"),
        },
        Err(error) => fixture_error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn fixture_identities_handler(State(pool): State<SqlitePool>) -> axum::response::Response {
    match fixture::read_state(&pool).await {
        Ok(state) => fixture_document_response(StatusCode::OK, state.identities_bytes),
        Err(error) => fixture_error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
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
        "elasticache" => "ElastiCache".to_string(),
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

fn build_metric_series_set(
    group_key: &str,
    metric_groups: &MetricGroupMap,
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
    cost_groups: &CostGroupMap,
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
) -> Result<DashboardDataResponse> {
    let now = Utc::now();
    let query = normalize_dashboard_query(query);
    let split_metric_dimension = query.metric_name.is_none();
    let summary = fetch_dashboard_summary(pool).await?;
    let resource_catalog = fetch_dashboard_resource_catalog(pool, &query).await?;
    let metric_rows = fetch_dashboard_metric_rows(pool, &query).await?;
    let cost_rows = fetch_dashboard_cost_rows(pool, &query).await?;

    let mut metric_groups: MetricGroupMap = HashMap::new();
    let mut utilization_by_resource: HashMap<String, (f64, i64)> = HashMap::new();
    let mut metric_score_by_group: HashMap<String, f64> = HashMap::new();

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
            &query.scope,
            &resource_type,
            &resource_id,
            &namespace,
            &metric_name,
            split_metric_dimension,
        );
        let (_, buckets) = metric_groups
            .entry(group_key.clone())
            .or_insert_with(|| (group_label, HashMap::new()));
        let bucket = buckets.entry(seconds_from_now).or_insert((0.0, 0));
        bucket.0 += value;
        bucket.1 += 1;
        let score_key = if split_metric_dimension {
            group_key
        } else {
            base_key
        };
        *metric_score_by_group.entry(score_key).or_insert(0.0) += value.abs();

        let utilization = utilization_by_resource
            .entry(resource_id)
            .or_insert((0.0, 0));
        utilization.0 += value;
        utilization.1 += 1;
    }

    let mut cost_groups: CostGroupMap = HashMap::new();
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

        let (group_key, group_label) =
            grouping_for_scope(&query.scope, &resource_type, &resource_id);
        let (_, buckets) = cost_groups
            .entry(group_key.clone())
            .or_insert_with(|| (group_label, HashMap::new()));
        *buckets.entry(seconds_from_now).or_insert(0.0) += amount;
        *cost_by_group.entry(group_key).or_insert(0.0) += amount;
    }

    let selected_cost_group_keys = if query.scope == "aggregate" {
        vec!["aggregate".to_string()]
    } else if query.scope == "resource" {
        if let Some(resource_id) = query.resource_id.clone() {
            vec![resource_id]
        } else {
            sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, query.top_n)
        }
    } else if query.scope == "service" {
        if let Some(resource_type) = query.resource_type.clone() {
            vec![resource_type]
        } else {
            sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, query.top_n)
        }
    } else {
        sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, query.top_n)
    };

    let mut selected_metric_group_keys = if !split_metric_dimension {
        if query.scope == "aggregate" {
            vec!["aggregate".to_string()]
        } else if query.scope == "resource" {
            if let Some(resource_id) = query.resource_id.clone() {
                vec![resource_id]
            } else {
                sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, query.top_n)
            }
        } else if query.scope == "service" {
            if let Some(resource_type) = query.resource_type.clone() {
                vec![resource_type]
            } else {
                sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, query.top_n)
            }
        } else {
            sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, query.top_n)
        }
    } else {
        let mut keys: Vec<String> = metric_groups.keys().cloned().collect();
        if let Some(resource_id) = query.resource_id.clone() {
            keys.retain(|key| key.starts_with(&format!("{}::", resource_id)));
        } else if query.scope == "service"
            && let Some(resource_type) = query.resource_type.clone()
        {
            keys.retain(|key| key.starts_with(&format!("{}::", resource_type)));
        }
        keys.sort_by(|a, b| {
            let a_score = metric_score_by_group.get(a).copied().unwrap_or(0.0);
            let b_score = metric_score_by_group.get(b).copied().unwrap_or(0.0);
            b_score
                .partial_cmp(&a_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        });
        keys.truncate(query.top_n);
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

    let mut top_cost_resources = fetch_dashboard_cost_by_resource(pool, &query).await?;
    for contributor in &mut top_cost_resources {
        contributor.average_utilization = utilization_by_resource
            .get(&contributor.resource_id)
            .and_then(|(sum, count)| {
                if *count > 0 {
                    Some(sum / *count as f64)
                } else {
                    None
                }
            });
    }
    let cost_map = top_cost_resources
        .iter()
        .map(|entry| (entry.resource_id.clone(), entry.total_cost))
        .collect::<HashMap<_, _>>();
    let top_low_utilization_resources =
        fetch_dashboard_low_utilization_resources(pool, &query, &cost_map).await?;

    let mut supported_apis = vec![
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
            protocol: "json-1.0".to_string(),
            target: Some("GraniteServiceVersion20100801.GetMetricStatistics".to_string()),
            action: None,
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
            service: "cloudwatch".to_string(),
            operation: "ListMetrics".to_string(),
            protocol: "json-1.0".to_string(),
            target: Some("GraniteServiceVersion20100801.ListMetrics".to_string()),
            action: None,
            endpoint: None,
        },
        DashboardApiEntry {
            service: "cloudwatch".to_string(),
            operation: "ListMetrics".to_string(),
            protocol: "query-xml".to_string(),
            target: None,
            action: Some("ListMetrics".to_string()),
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
    for operation in [
        "GetCostAndUsage",
        "GetCostAndUsageWithResources",
        "GetCostForecast",
        "GetUsageForecast",
        "GetDimensionValues",
        "GetTags",
        "GetReservationCoverage",
        "GetReservationUtilization",
        "GetSavingsPlansCoverage",
        "GetSavingsPlansUtilization",
        "GetRightsizingRecommendation",
        "GetAnomalies",
        "GetAnomalyMonitors",
        "GetAnomalySubscriptions",
    ] {
        supported_apis.extend(cost_explorer_api_entries(operation));
    }
    supported_apis.push(DashboardApiEntry {
        service: "resource-groups-tagging-api".to_string(),
        operation: "GetResources".to_string(),
        protocol: "json-1.1".to_string(),
        target: Some("ResourceGroupsTaggingAPI_20170126.GetResources".to_string()),
        action: None,
        endpoint: None,
    });
    supported_apis.push(DashboardApiEntry {
        service: "resource-groups-tagging-api".to_string(),
        operation: "GetTagKeys".to_string(),
        protocol: "json-1.1".to_string(),
        target: Some("ResourceGroupsTaggingAPI_20170126.GetTagKeys".to_string()),
        action: None,
        endpoint: None,
    });
    supported_apis.push(DashboardApiEntry {
        service: "resource-groups-tagging-api".to_string(),
        operation: "GetTagValues".to_string(),
        protocol: "json-1.1".to_string(),
        target: Some("ResourceGroupsTaggingAPI_20170126.GetTagValues".to_string()),
        action: None,
        endpoint: None,
    });
    supported_apis.push(DashboardApiEntry {
        service: "pricing".to_string(),
        operation: "GetProducts".to_string(),
        protocol: "json-1.1".to_string(),
        target: Some("AWSPriceListService.GetProducts".to_string()),
        action: None,
        endpoint: None,
    });
    supported_apis.push(DashboardApiEntry {
        service: "compute-optimizer".to_string(),
        operation: "GetEC2InstanceRecommendations".to_string(),
        protocol: "json-1.0".to_string(),
        target: Some("ComputeOptimizerService.GetEC2InstanceRecommendations".to_string()),
        action: None,
        endpoint: None,
    });
    supported_apis.push(DashboardApiEntry {
        service: "compute-optimizer".to_string(),
        operation: "GetEBSVolumeRecommendations".to_string(),
        protocol: "json-1.0".to_string(),
        target: Some("ComputeOptimizerService.GetEBSVolumeRecommendations".to_string()),
        action: None,
        endpoint: None,
    });
    supported_apis.push(DashboardApiEntry {
        service: "cur".to_string(),
        operation: "DescribeReportDefinitions".to_string(),
        protocol: "json-1.1".to_string(),
        target: Some("AWSOrigamiServiceGatewayService.DescribeReportDefinitions".to_string()),
        action: None,
        endpoint: None,
    });

    let coverage_scorecard = build_coverage_scorecard(&supported_apis);

    Ok(DashboardDataResponse {
        generated_at: now.to_rfc3339(),
        supported_apis,
        summary,
        cloudwatch_series,
        cost_series,
        cloudwatch_series_sets,
        cost_series_sets,
        resource_catalog,
        top_cost_resources,
        top_low_utilization_resources,
        applied_filters: dashboard_applied_filters(&query),
        coverage_scorecard,
    })
}

async fn dashboard_data_handler(
    State(pool): State<SqlitePool>,
    Query(query): Query<DashboardDataQuery>,
) -> axum::response::Response {
    match build_dashboard_data(&pool, query).await {
        Ok(data) => Json(data).into_response(),
        Err(_) => dashboard_error_response(),
    }
}

async fn dashboard_resources_handler(
    State(pool): State<SqlitePool>,
    Query(query): Query<DashboardDataQuery>,
) -> axum::response::Response {
    let now = Utc::now();
    let query = normalize_dashboard_query(query);
    let top_cost_resources = match fetch_dashboard_cost_by_resource(&pool, &query).await {
        Ok(value) => value,
        Err(_) => return dashboard_error_response(),
    };
    let cost_map = top_cost_resources
        .iter()
        .map(|entry| (entry.resource_id.clone(), entry.total_cost))
        .collect::<HashMap<_, _>>();
    let resource_catalog = match fetch_dashboard_resource_catalog(&pool, &query).await {
        Ok(value) => value,
        Err(_) => return dashboard_error_response(),
    };
    let top_low_utilization_resources =
        match fetch_dashboard_low_utilization_resources(&pool, &query, &cost_map).await {
            Ok(value) => value,
            Err(_) => return dashboard_error_response(),
        };

    Json(json!({
        "generated_at": now.to_rfc3339(),
        "applied_filters": dashboard_applied_filters(&query),
        "resource_catalog": resource_catalog,
        "top_cost_resources": top_cost_resources,
        "top_low_utilization_resources": top_low_utilization_resources
    }))
    .into_response()
}

async fn dashboard_cloudwatch_trends_handler(
    State(pool): State<SqlitePool>,
    Query(query): Query<DashboardDataQuery>,
) -> axum::response::Response {
    let now = Utc::now();
    let query = normalize_dashboard_query(query);
    let split_metric_dimension = query.metric_name.is_none();
    let metric_rows = match fetch_dashboard_metric_rows(&pool, &query).await {
        Ok(value) => value,
        Err(_) => return dashboard_error_response(),
    };

    let mut metric_groups: MetricGroupMap = HashMap::new();
    let mut metric_score_by_group: HashMap<String, f64> = HashMap::new();
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

        let (group_key, group_label, _) = metric_grouping_for_scope(
            &query.scope,
            &resource_type,
            &resource_id,
            &namespace,
            &metric_name,
            split_metric_dimension,
        );
        let (_, buckets) = metric_groups
            .entry(group_key.clone())
            .or_insert_with(|| (group_label, HashMap::new()));
        let bucket = buckets.entry(seconds_from_now).or_insert((0.0, 0));
        bucket.0 += value;
        bucket.1 += 1;
        *metric_score_by_group.entry(group_key).or_insert(0.0) += value.abs();
    }

    let mut selected_metric_group_keys: Vec<String> = metric_groups.keys().cloned().collect();
    selected_metric_group_keys.sort_by(|a, b| {
        let a_score = metric_score_by_group.get(a).copied().unwrap_or(0.0);
        let b_score = metric_score_by_group.get(b).copied().unwrap_or(0.0);
        b_score
            .partial_cmp(&a_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(b))
    });
    selected_metric_group_keys.truncate(query.top_n);

    let cloudwatch_series_sets = selected_metric_group_keys
        .iter()
        .filter_map(|group_key| build_metric_series_set(group_key, &metric_groups, now))
        .filter(|set| !set.points.is_empty())
        .collect::<Vec<_>>();

    Json(json!({
        "generated_at": now.to_rfc3339(),
        "applied_filters": dashboard_applied_filters(&query),
        "cloudwatch_series_sets": cloudwatch_series_sets
    }))
    .into_response()
}

async fn dashboard_cost_trends_handler(
    State(pool): State<SqlitePool>,
    Query(query): Query<DashboardDataQuery>,
) -> axum::response::Response {
    let now = Utc::now();
    let query = normalize_dashboard_query(query);
    let cost_rows = match fetch_dashboard_cost_rows(&pool, &query).await {
        Ok(value) => value,
        Err(_) => return dashboard_error_response(),
    };

    let mut cost_groups: HashMap<String, (String, HashMap<i64, f64>)> = HashMap::new();
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

        let (group_key, group_label) =
            grouping_for_scope(&query.scope, &resource_type, &resource_id);
        let (_, buckets) = cost_groups
            .entry(group_key.clone())
            .or_insert_with(|| (group_label, HashMap::new()));
        *buckets.entry(seconds_from_now).or_insert(0.0) += amount;
        *cost_by_group.entry(group_key).or_insert(0.0) += amount;
    }

    let selected_cost_group_keys = if query.scope == "aggregate" {
        vec!["aggregate".to_string()]
    } else if query.scope == "resource" {
        if let Some(resource_id) = query.resource_id.clone() {
            vec![resource_id]
        } else {
            sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, query.top_n)
        }
    } else if query.scope == "service" {
        if let Some(resource_type) = query.resource_type.clone() {
            vec![resource_type]
        } else {
            sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, query.top_n)
        }
    } else {
        sorted_cost_keys_with_top_n(&cost_groups, &cost_by_group, query.top_n)
    };

    let cost_series_sets = selected_cost_group_keys
        .iter()
        .filter_map(|group_key| build_cost_series_set(group_key, &cost_groups, now))
        .filter(|set| !set.points.is_empty())
        .collect::<Vec<_>>();

    Json(json!({
        "generated_at": now.to_rfc3339(),
        "applied_filters": dashboard_applied_filters(&query),
        "cost_series_sets": cost_series_sets
    }))
    .into_response()
}

async fn scenario_handler(
    State(pool): State<SqlitePool>,
    headers: HeaderMap,
    Json(req): Json<ScenarioRequest>,
) -> axum::response::Response {
    if let Err(response) = ensure_admin_authorized(&headers) {
        return *response;
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
    match canonical_cost_explorer_operation(target) {
        Some("GetCostAndUsage") => match handle_get_cost_and_usage(pool, body).await {
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
        Some("GetCostAndUsageWithResources") => {
            match handle_get_cost_and_usage_with_resources(pool, body).await {
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
        Some("GetCostForecast") => match handle_get_cost_forecast(pool, body).await {
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
        Some("GetUsageForecast") => match handle_get_usage_forecast(pool, body).await {
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
        Some("GetDimensionValues") => match handle_get_dimension_values(pool, body).await {
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
        Some("GetTags") => match handle_get_tags(pool, body).await {
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
        Some("GetReservationCoverage") => match handle_get_reservation_coverage(pool, body).await {
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
        Some("GetReservationUtilization") => {
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
        Some("GetSavingsPlansCoverage") => {
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
        Some("GetSavingsPlansUtilization") => {
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
        Some("GetRightsizingRecommendation") => {
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
        Some("GetAnomalies") => match handle_get_anomalies(pool, body).await {
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
        Some("GetAnomalyMonitors") => match handle_get_anomaly_monitors(body).await {
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
        Some("GetAnomalySubscriptions") => match handle_get_anomaly_subscriptions(body).await {
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

    build_cost_usage_response(&pool, &req, None).await
}

async fn handle_get_cost_and_usage_with_resources(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetCostAndUsageRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    build_cost_usage_response(&pool, &req, Some("RESOURCE_ID")).await
}

async fn build_cost_usage_response(
    pool: &SqlitePool,
    req: &GetCostAndUsageRequest,
    default_group_dimension: Option<&str>,
) -> std::result::Result<Value, CostUsageError> {
    let now = Utc::now();
    let start = parse_day_start_utc("TimePeriod.Start", &req.time_period.start)
        .map_err(CostUsageError::Validation)?;
    let end = parse_day_start_utc("TimePeriod.End", &req.time_period.end)
        .map_err(CostUsageError::Validation)?;
    let start_offset = (start - now).num_seconds();
    let end_offset = (end - now).num_seconds();
    let daily = req.granularity.as_deref() == Some("DAILY");
    let group_dimension =
        parse_group_by_dimension(req.group_by.as_ref())?.or(default_group_dimension);
    let filter = parse_ce_filter(req.filter.as_ref())?;
    let rows = fetch_cost_rows_for_window(pool, start_offset, end_offset).await?;
    let filtered_rows = rows
        .into_iter()
        .filter(|row| cost_row_matches_filter(row, &filter))
        .collect::<Vec<_>>();
    let total_amounts =
        filtered_rows
            .iter()
            .fold(CostUsageAmounts::default(), |mut accumulator, row| {
                accumulator.add(CostUsageAmounts::from_row(row));
                accumulator
            });

    let results_by_time = if let Some(group_dimension) = group_dimension {
        let mut bucket_groups: BTreeMap<String, BTreeMap<String, CostUsageAmounts>> =
            BTreeMap::new();
        let mut bucket_totals: BTreeMap<String, CostUsageAmounts> = BTreeMap::new();
        let mut bucket_ranges: BTreeMap<String, String> = BTreeMap::new();

        if daily {
            let mut cursor = start;
            while cursor < end {
                let bucket_start = cursor.format("%Y-%m-%d").to_string();
                let bucket_end = (cursor + chrono::Duration::days(1))
                    .format("%Y-%m-%d")
                    .to_string();
                bucket_groups.insert(bucket_start.clone(), BTreeMap::new());
                bucket_totals.insert(bucket_start.clone(), CostUsageAmounts::default());
                bucket_ranges.insert(bucket_start, bucket_end);
                cursor += chrono::Duration::days(1);
            }
        } else {
            bucket_groups.insert(req.time_period.start.clone(), BTreeMap::new());
            bucket_totals.insert(req.time_period.start.clone(), total_amounts);
            bucket_ranges.insert(req.time_period.start.clone(), req.time_period.end.clone());
        }

        for row in filtered_rows {
            let bucket_start = if daily {
                (now + chrono::Duration::seconds(row.seconds_from_now))
                    .format("%Y-%m-%d")
                    .to_string()
            } else {
                req.time_period.start.clone()
            };

            let group_key = match group_dimension {
                "SERVICE" => ce_service_name_from_resource_type(&row.resource_type).to_string(),
                "REGION" => row.region.clone(),
                "RESOURCE_ID" => row.resource_id.clone(),
                "USAGE_TYPE" => ce_usage_type_from_resource_type(&row.resource_type).to_string(),
                _ => unreachable!("group dimension validated above"),
            };
            let amounts = CostUsageAmounts::from_row(&row);

            if let Some(groups) = bucket_groups.get_mut(&bucket_start) {
                groups.entry(group_key).or_default().add(amounts);
                if daily {
                    bucket_totals.entry(bucket_start).or_default().add(amounts);
                }
            }
        }

        bucket_groups
            .into_iter()
            .map(|(bucket_start, groups)| {
                let bucket_total = bucket_totals
                    .get(&bucket_start)
                    .copied()
                    .unwrap_or_default();
                let bucket_end = bucket_ranges
                    .get(&bucket_start)
                    .cloned()
                    .unwrap_or_else(|| req.time_period.end.clone());
                let groups = groups
                    .into_iter()
                    .map(|(key, amount)| ce::CostUsageGroup {
                        key,
                        amounts: amount.into(),
                    })
                    .collect::<Vec<_>>();
                ce::time_bucket_json(
                    bucket_start,
                    bucket_end,
                    bucket_total.into(),
                    groups,
                    req.metrics.as_ref(),
                )
            })
            .collect::<Vec<Value>>()
    } else if daily {
        let mut bucket_totals: BTreeMap<String, CostUsageAmounts> = BTreeMap::new();
        let mut bucket_ranges: BTreeMap<String, String> = BTreeMap::new();

        let mut cursor = start;
        while cursor < end {
            let bucket_start = cursor.format("%Y-%m-%d").to_string();
            let bucket_end = (cursor + chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string();
            bucket_totals.insert(bucket_start.clone(), CostUsageAmounts::default());
            bucket_ranges.insert(bucket_start, bucket_end);
            cursor += chrono::Duration::days(1);
        }

        for row in &filtered_rows {
            let bucket_start = (now + chrono::Duration::seconds(row.seconds_from_now))
                .format("%Y-%m-%d")
                .to_string();
            if let Some(total) = bucket_totals.get_mut(&bucket_start) {
                total.add(CostUsageAmounts::from_row(row));
            }
        }

        bucket_totals
            .into_iter()
            .map(|(bucket_start, total)| {
                let bucket_end = bucket_ranges
                    .get(&bucket_start)
                    .cloned()
                    .unwrap_or_else(|| req.time_period.end.clone());
                ce::time_bucket_json(
                    bucket_start,
                    bucket_end,
                    total.into(),
                    Vec::new(),
                    req.metrics.as_ref(),
                )
            })
            .collect::<Vec<Value>>()
    } else {
        vec![ce::time_bucket_json(
            req.time_period.start.clone(),
            req.time_period.end.clone(),
            total_amounts.into(),
            Vec::new(),
            req.metrics.as_ref(),
        )]
    };

    let group_definitions = if let Some(group_by) = req.group_by.clone() {
        group_by
    } else if let Some(default_group_dimension) = default_group_dimension {
        vec![ce::dimension_group_definition(default_group_dimension)]
    } else {
        Vec::new()
    };

    let include_next_page_token =
        req.next_page_token.is_some() || req.billing_view_arn.is_some() || req.filter.is_some();

    Ok(ce::cost_and_usage_response(
        group_definitions,
        results_by_time,
        include_next_page_token,
        req.granularity.as_ref(),
        req.metrics.as_ref(),
    ))
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
    let granularity = req.granularity.clone().ok_or_else(|| {
        CostUsageError::Validation("Missing required field 'Granularity'.".to_string())
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

    response["Granularity"] = json!(granularity);
    response["Metric"] = json!(metric);
    if req.filter.is_some() {
        response["FilterApplied"] = json!(true);
    }

    Ok(response)
}

async fn handle_get_usage_forecast(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetUsageForecastRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let metric = req.metric.clone().ok_or_else(|| {
        CostUsageError::Validation("Missing required field 'Metric'.".to_string())
    })?;
    let granularity = req.granularity.clone().ok_or_else(|| {
        CostUsageError::Validation("Missing required field 'Granularity'.".to_string())
    })?;

    if !matches!(
        metric.as_str(),
        "USAGE_QUANTITY" | "NORMALIZED_USAGE_AMOUNT"
    ) {
        return Err(CostUsageError::Validation(format!(
            "Unsupported Metric '{}'.",
            metric
        )));
    }
    if !matches!(granularity.as_str(), "DAILY" | "MONTHLY") {
        return Err(CostUsageError::Validation(format!(
            "Unsupported Granularity '{}'.",
            granularity
        )));
    }

    let now = Utc::now();
    let start = parse_day_start_utc("TimePeriod.Start", &req.time_period.start)
        .map_err(CostUsageError::Validation)?;
    let end = parse_day_start_utc("TimePeriod.End", &req.time_period.end)
        .map_err(CostUsageError::Validation)?;
    let start_offset = (start - now).num_seconds();
    let end_offset = (end - now).num_seconds();
    let filter = parse_ce_filter(req.filter.as_ref())?;
    let rows = fetch_cost_rows_for_window(&pool, start_offset, end_offset).await?;
    let metric_multiplier = if metric == "NORMALIZED_USAGE_AMOUNT" {
        1.25
    } else {
        1.0
    };

    let mut bucket_usage: BTreeMap<String, f64> = BTreeMap::new();
    let mut bucket_end_dates: BTreeMap<String, String> = BTreeMap::new();
    let mut cursor = start;
    while cursor < end {
        let bucket_start = if granularity == "MONTHLY" {
            cursor.format("%Y-%m-01").to_string()
        } else {
            cursor.format("%Y-%m-%d").to_string()
        };
        let next_cursor = if granularity == "MONTHLY" {
            let first_of_month = cursor.with_day(1).unwrap_or(cursor);
            if first_of_month.month() == 12 {
                first_of_month
                    .with_year(first_of_month.year() + 1)
                    .and_then(|d| d.with_month(1))
                    .unwrap_or(first_of_month + chrono::Duration::days(31))
            } else {
                first_of_month
                    .with_month(first_of_month.month() + 1)
                    .unwrap_or(first_of_month + chrono::Duration::days(31))
            }
        } else {
            cursor + chrono::Duration::days(1)
        };
        bucket_usage.entry(bucket_start.clone()).or_insert(0.0);
        bucket_end_dates.insert(
            bucket_start,
            std::cmp::min(next_cursor, end)
                .format("%Y-%m-%d")
                .to_string(),
        );
        cursor = next_cursor;
    }

    for row in rows
        .into_iter()
        .filter(|row| cost_row_matches_filter(row, &filter))
    {
        let record_day = (now + chrono::Duration::seconds(row.seconds_from_now)).date_naive();
        let bucket_key = if granularity == "MONTHLY" {
            record_day.format("%Y-%m-01").to_string()
        } else {
            record_day.format("%Y-%m-%d").to_string()
        };
        let usage_amount = (row.amount / mock_usage_rate_for_resource_type(&row.resource_type))
            * metric_multiplier;
        *bucket_usage.entry(bucket_key).or_insert(0.0) += usage_amount;
    }

    let total_usage = bucket_usage.values().sum::<f64>();
    let interval_level = req.prediction_interval_level.unwrap_or(80).clamp(50, 99);
    let spread_ratio = (100 - interval_level) as f64 / 100.0 + 0.10;
    let results_by_time = bucket_usage
        .into_iter()
        .map(|(bucket_start, amount)| {
            let lower = (amount * (1.0 - spread_ratio)).max(0.0);
            let upper = amount * (1.0 + spread_ratio);
            json!({
                "TimePeriod": {
                    "Start": bucket_start,
                    "End": bucket_end_dates.get(&bucket_start).cloned().unwrap_or_else(|| req.time_period.end.clone())
                },
                "MeanValue": format!("{:.4}", amount),
                "PredictionIntervalLowerBound": format!("{:.4}", lower),
                "PredictionIntervalUpperBound": format!("{:.4}", upper)
            })
        })
        .collect::<Vec<_>>();

    let mut response = json!({
        "Total": {
            "Amount": format!("{:.4}", total_usage),
            "Unit": if metric == "NORMALIZED_USAGE_AMOUNT" { "N/A" } else { "UsageQuantity" }
        },
        "ForecastResultsByTime": results_by_time
    });
    if req.filter.is_some() {
        response["FilterApplied"] = json!(true);
    }
    if req.billing_view_arn.is_some() {
        response["BillingViewArn"] = json!(req.billing_view_arn);
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

    let page_start = match req.next_page_token.as_deref() {
        Some(raw) if !raw.trim().is_empty() => raw
            .parse::<usize>()
            .map_err(|_| CostUsageError::Validation("Invalid NextPageToken value.".to_string()))?,
        _ => 0,
    };
    let page_size = req.max_results.unwrap_or(100).clamp(1, 1000) as i64;

    let (mut values, total_size): (Vec<String>, usize) = match req.dimension.as_str() {
        "SERVICE" => {
            let rows =
                sqlx::query("SELECT DISTINCT resource_type FROM resources ORDER BY resource_type")
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| CostUsageError::Internal(e.into()))?;
            let values = rows
                .into_iter()
                .filter_map(|row| row.try_get::<String, _>("resource_type").ok())
                .map(|rt| ce_service_name_from_resource_type(&rt).to_string())
                .collect::<Vec<_>>();
            let total_size = values.len();
            (values, total_size)
        }
        "REGION" => {
            let search = req
                .search_string
                .as_ref()
                .map(|s| format!("%{}%", s.to_lowercase()));
            let total_size = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(DISTINCT region)
                 FROM resources
                 WHERE (? IS NULL OR lower(region) LIKE ?)",
            )
            .bind(search.as_deref())
            .bind(search.as_deref())
            .fetch_one(&pool)
            .await
            .map_err(|e| CostUsageError::Internal(e.into()))? as usize;
            let rows = sqlx::query(
                "SELECT DISTINCT region
                 FROM resources
                 WHERE (? IS NULL OR lower(region) LIKE ?)
                 ORDER BY region
                 LIMIT ? OFFSET ?",
            )
            .bind(search.as_deref())
            .bind(search.as_deref())
            .bind(page_size)
            .bind(page_start as i64)
            .fetch_all(&pool)
            .await
            .map_err(|e| CostUsageError::Internal(e.into()))?;
            (
                rows.into_iter()
                    .filter_map(|row| row.try_get::<String, _>("region").ok())
                    .collect(),
                total_size,
            )
        }
        "RESOURCE_ID" => {
            let search = req
                .search_string
                .as_ref()
                .map(|s| format!("%{}%", s.to_lowercase()));
            let total_size = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*)
                 FROM resources
                 WHERE (? IS NULL OR lower(id) LIKE ?)",
            )
            .bind(search.as_deref())
            .bind(search.as_deref())
            .fetch_one(&pool)
            .await
            .map_err(|e| CostUsageError::Internal(e.into()))? as usize;
            let rows = sqlx::query(
                "SELECT id
                 FROM resources
                 WHERE (? IS NULL OR lower(id) LIKE ?)
                 ORDER BY id
                 LIMIT ? OFFSET ?",
            )
            .bind(search.as_deref())
            .bind(search.as_deref())
            .bind(page_size)
            .bind(page_start as i64)
            .fetch_all(&pool)
            .await
            .map_err(|e| CostUsageError::Internal(e.into()))?;
            (
                rows.into_iter()
                    .filter_map(|row| row.try_get::<String, _>("id").ok())
                    .collect(),
                total_size,
            )
        }
        "LINKED_ACCOUNT" => (vec!["123456789012".to_string()], 1),
        _ => (Vec::new(), 0),
    };

    values.sort();
    values.dedup();

    if matches!(req.dimension.as_str(), "SERVICE")
        && let Some(search) = req.search_string.as_ref().map(|s| s.to_lowercase())
    {
        values.retain(|value| value.to_lowercase().contains(&search));
    }

    if page_start > values.len() {
        return Err(CostUsageError::Validation(
            "NextPageToken points past available results.".to_string(),
        ));
    }

    let page_size = page_size as usize;
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
        "TotalSize": total_size
    });

    if page_start + page_values.len() < total_size {
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

async fn handle_get_tags(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetTagsRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let _start = parse_day_start_utc("TimePeriod.Start", &req.time_period.start)
        .map_err(CostUsageError::Validation)?;
    let _end = parse_day_start_utc("TimePeriod.End", &req.time_period.end)
        .map_err(CostUsageError::Validation)?;
    let criteria = parse_ce_filter(req.filter.as_ref())?;
    let page_start = parse_usize_token(req.next_page_token.as_deref(), "NextPageToken")?;
    let page_size = req.max_results.unwrap_or(100).clamp(1, 1000) as usize;

    let rows = sqlx::query("SELECT id, resource_type, region, tags FROM resources ORDER BY id ASC")
        .fetch_all(&pool)
        .await
        .map_err(|e| CostUsageError::Internal(e.into()))?;

    let mut tag_values = rows
        .into_iter()
        .filter_map(|row| {
            let cost_row = CostRow {
                resource_id: row.get::<String, _>("id"),
                resource_type: row.get::<String, _>("resource_type"),
                region: row.get::<String, _>("region"),
                amount: 0.0,
                seconds_from_now: 0,
                tags_json: row.try_get::<Option<String>, _>("tags").ok().flatten(),
            };
            if !cost_row_matches_filter(&cost_row, &criteria) {
                return None;
            }
            let tags = parse_resource_tags(cost_row.tags_json.as_deref());
            tags.get(&req.tag_key).cloned()
        })
        .collect::<Vec<_>>();

    tag_values.sort();
    tag_values.dedup();

    if let Some(search) = req.search_string.as_ref().map(|s| s.to_lowercase()) {
        tag_values.retain(|value| value.to_lowercase().contains(&search));
    }

    if page_start > tag_values.len() {
        return Err(CostUsageError::Validation(
            "NextPageToken points past available results.".to_string(),
        ));
    }

    let total_size = tag_values.len();
    let page_end = std::cmp::min(page_start + page_size, total_size);
    let page_values = &tag_values[page_start..page_end];

    let mut response = json!({
        "Tags": page_values,
        "ReturnSize": page_values.len(),
        "TotalSize": total_size
    });

    if page_end < total_size {
        response["NextPageToken"] = json!(page_end.to_string());
    }
    if req.filter.is_some() {
        response["FilterApplied"] = json!(true);
    }
    if req.search_string.is_some() {
        response["SearchStringApplied"] = json!(true);
    }

    Ok(response)
}

async fn handle_get_resources(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetResourcesRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    if req.include_compliance_details.unwrap_or(false)
        || req.exclude_compliant_resources.unwrap_or(false)
    {
        return Err(CostUsageError::Validation(
            "Compliance detail options are not supported.".to_string(),
        ));
    }
    if let Some(resource_arn_list) = req.resource_arn_list.as_ref()
        && resource_arn_list.len() > 100
    {
        return Err(CostUsageError::Validation(
            "ResourceARNList may contain at most 100 entries.".to_string(),
        ));
    }

    let page_start = parse_usize_token(req.pagination_token.as_deref(), "PaginationToken")?;
    let page_size = req.resources_per_page.unwrap_or(50).clamp(1, 100) as usize;
    let tags_per_page = req.tags_per_page.unwrap_or(100).clamp(1, 500) as usize;
    let resource_type_filters = req.resource_type_filters.unwrap_or_default();
    let tag_filters = req.tag_filters.unwrap_or_default();
    let resource_arn_filter = req
        .resource_arn_list
        .unwrap_or_default()
        .into_iter()
        .collect::<BTreeSet<_>>();

    let resources = fetch_tagged_resources(&pool).await?;
    let mut filtered = resources
        .into_iter()
        .filter(|resource| {
            (resource_arn_filter.is_empty() || resource_arn_filter.contains(&resource.arn))
                && resource_matches_type_filters(
                    &resource.resource_type_filter,
                    &resource_type_filters,
                )
                && tagged_resource_matches_filters(resource, &tag_filters)
        })
        .collect::<Vec<_>>();

    if page_start > filtered.len() {
        return Err(CostUsageError::Validation(
            "PaginationToken points past available results.".to_string(),
        ));
    }

    filtered.sort_by(|a, b| a.arn.cmp(&b.arn));
    let total_size = filtered.len();
    let page_end = std::cmp::min(page_start + page_size, total_size);
    let page_values = filtered[page_start..page_end]
        .iter()
        .map(|resource| {
            let tags = resource
                .tags
                .iter()
                .take(tags_per_page)
                .map(|(key, value)| json!({ "Key": key, "Value": value }))
                .collect::<Vec<_>>();
            json!({
                "ResourceARN": resource.arn,
                "Tags": tags
            })
        })
        .collect::<Vec<_>>();

    let mut response = json!({
        "ResourceTagMappingList": page_values
    });
    if page_end < total_size {
        response["PaginationToken"] = json!(page_end.to_string());
    }

    Ok(response)
}

async fn handle_get_tag_keys(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetTagKeysRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let page_start = parse_usize_token(req.pagination_token.as_deref(), "PaginationToken")?;
    let rows = sqlx::query("SELECT tags FROM resources ORDER BY id ASC")
        .fetch_all(&pool)
        .await
        .map_err(|e| CostUsageError::Internal(e.into()))?;

    let mut keys = rows
        .into_iter()
        .flat_map(|row| {
            parse_resource_tags(
                row.try_get::<Option<String>, _>("tags")
                    .ok()
                    .flatten()
                    .as_deref(),
            )
            .into_keys()
            .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    keys.sort();
    keys.dedup();

    if page_start > keys.len() {
        return Err(CostUsageError::Validation(
            "PaginationToken points past available results.".to_string(),
        ));
    }

    let page_size = 100usize;
    let page_end = std::cmp::min(page_start + page_size, keys.len());
    let mut response = json!({
        "TagKeys": keys[page_start..page_end].to_vec()
    });
    if page_end < keys.len() {
        response["PaginationToken"] = json!(page_end.to_string());
    }

    Ok(response)
}

async fn handle_get_tag_values(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: GetTagValuesRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let page_start = parse_usize_token(req.pagination_token.as_deref(), "PaginationToken")?;
    let rows = sqlx::query("SELECT tags FROM resources ORDER BY id ASC")
        .fetch_all(&pool)
        .await
        .map_err(|e| CostUsageError::Internal(e.into()))?;

    let mut values = rows
        .into_iter()
        .filter_map(|row| {
            parse_resource_tags(
                row.try_get::<Option<String>, _>("tags")
                    .ok()
                    .flatten()
                    .as_deref(),
            )
            .get(&req.key)
            .cloned()
        })
        .collect::<Vec<_>>();

    values.sort();
    values.dedup();

    if page_start > values.len() {
        return Err(CostUsageError::Validation(
            "PaginationToken points past available results.".to_string(),
        ));
    }

    let page_size = 100usize;
    let page_end = std::cmp::min(page_start + page_size, values.len());
    let mut response = json!({
        "TagValues": values[page_start..page_end].to_vec()
    });
    if page_end < values.len() {
        response["PaginationToken"] = json!(page_end.to_string());
    }

    Ok(response)
}

async fn handle_get_products(body: Bytes) -> std::result::Result<Value, CostUsageError> {
    let req: GetProductsRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    if req
        .format_version
        .as_deref()
        .is_some_and(|format_version| format_version != "aws_v1")
    {
        return Err(CostUsageError::Validation(
            "FormatVersion must be 'aws_v1'.".to_string(),
        ));
    }

    let page_start = parse_usize_token(req.next_token.as_deref(), "NextToken")?;
    let page_size = req.max_results.unwrap_or(100).clamp(1, 100) as usize;
    let filters = req.filters.unwrap_or_default();
    let mut products = pricing_catalog()
        .into_iter()
        .filter(|product| product.service_code == req.service_code)
        .filter(|product| pricing_product_matches_filters(product, &filters))
        .collect::<Vec<_>>();

    if page_start > products.len() {
        return Err(CostUsageError::Validation(
            "NextToken points past available results.".to_string(),
        ));
    }

    products.sort_by(|a, b| a.rate_code.cmp(&b.rate_code));
    let total_size = products.len();
    let page_end = std::cmp::min(page_start + page_size, total_size);
    let price_list = products[page_start..page_end]
        .iter()
        .map(pricing_product_to_value)
        .map(|value| value.to_string())
        .collect::<Vec<_>>();

    let mut response = json!({
        "FormatVersion": "aws_v1",
        "PriceList": price_list
    });
    if page_end < total_size {
        response["NextToken"] = json!(page_end.to_string());
    }

    Ok(response)
}

async fn handle_get_ec2_instance_recommendations(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: ComputeOptimizerRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let page_start = parse_usize_token(req.next_token.as_deref(), "nextToken")?;
    let page_size = clamp_page_size(req.max_results, 50, 100);
    let rows = sqlx::query(
        "SELECT r.id, r.region, r.tags, AVG(m.value) AS avg_cpu
         FROM resources r
         LEFT JOIN metrics m
           ON m.resource_id = r.id
          AND m.namespace = 'AWS/EC2'
          AND m.metric_name = 'CPUUtilization'
         WHERE r.resource_type = 'ec2'
         GROUP BY r.id, r.region, r.tags
         ORDER BY r.id ASC",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| CostUsageError::Internal(e.into()))?;

    if page_start > rows.len() {
        return Err(CostUsageError::Validation(
            "nextToken points past available results.".to_string(),
        ));
    }

    let last_refresh = Utc::now().to_rfc3339();
    let page_end = std::cmp::min(page_start + page_size, rows.len());
    let recommendations = rows[page_start..page_end]
        .iter()
        .map(|row| {
            let instance_id = row.get::<String, _>("id");
            let region = row.get::<String, _>("region");
            let tags_json = row.try_get::<Option<String>, _>("tags").ok().flatten();
            let average_cpu = row
                .try_get::<Option<f64>, _>("avg_cpu")
                .ok()
                .flatten()
                .unwrap_or(0.0);
            let (finding, target_instance_type, projected_cpu, monthly_savings_pct) =
                if average_cpu < 15.0 {
                    (
                        "OVER_PROVISIONED",
                        "t3.medium",
                        (average_cpu * 1.8).min(85.0),
                        28.0,
                    )
                } else if average_cpu > 75.0 {
                    (
                        "UNDER_PROVISIONED",
                        "m6i.xlarge",
                        (average_cpu * 0.7).max(45.0),
                        0.0,
                    )
                } else {
                    ("OPTIMIZED", "m6i.large", average_cpu, 0.0)
                };
            let estimated_monthly_savings = if monthly_savings_pct > 0.0 {
                ((40.0 - average_cpu).max(5.0) * 3.2 * 100.0).round() / 100.0
            } else {
                0.0
            };

            json!({
                "accountId": mock_account_id(),
                "instanceArn": resource_arn("ec2", &region, &instance_id),
                "instanceName": resource_name_from_tags(tags_json.as_deref(), &instance_id),
                "currentInstanceType": "m6i.large",
                "finding": finding,
                "lookBackPeriodInDays": 14.0,
                "lastRefreshTimestamp": last_refresh,
                "utilizationMetrics": [{
                    "name": "Cpu",
                    "statistic": "Average",
                    "value": average_cpu
                }],
                "recommendationOptions": [{
                    "instanceType": target_instance_type,
                    "projectedUtilizationMetrics": [{
                        "name": "Cpu",
                        "statistic": "Average",
                        "value": projected_cpu
                    }],
                    "performanceRisk": if finding == "UNDER_PROVISIONED" { 3.0 } else { 1.0 },
                    "rank": 1,
                    "savingsOpportunity": {
                        "estimatedMonthlySavings": {
                            "currency": "USD",
                            "value": estimated_monthly_savings
                        },
                        "savingsOpportunityPercentage": monthly_savings_pct
                    }
                }]
            })
        })
        .collect::<Vec<_>>();

    let mut response = json!({
        "instanceRecommendations": recommendations
    });
    if page_end < rows.len() {
        response["nextToken"] = json!(page_end.to_string());
    }

    Ok(response)
}

async fn handle_get_ebs_volume_recommendations(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let req: ComputeOptimizerRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    let page_start = parse_usize_token(req.next_token.as_deref(), "nextToken")?;
    let page_size = clamp_page_size(req.max_results, 50, 100);
    let rows = sqlx::query(
        "SELECT r.id, r.region, r.tags,
                AVG(CASE WHEN m.metric_name = 'DiskReadBytes' THEN m.value END) AS avg_read_bytes,
                AVG(CASE WHEN m.metric_name = 'DiskWriteBytes' THEN m.value END) AS avg_write_bytes
         FROM resources r
         LEFT JOIN metrics m
           ON m.resource_id = r.id
          AND m.namespace = 'AWS/EC2'
          AND m.metric_name IN ('DiskReadBytes', 'DiskWriteBytes')
         WHERE r.resource_type = 'ec2'
         GROUP BY r.id, r.region, r.tags
         ORDER BY r.id ASC",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| CostUsageError::Internal(e.into()))?;

    if page_start > rows.len() {
        return Err(CostUsageError::Validation(
            "nextToken points past available results.".to_string(),
        ));
    }

    let last_refresh = Utc::now().to_rfc3339();
    let page_end = std::cmp::min(page_start + page_size, rows.len());
    let recommendations = rows[page_start..page_end]
        .iter()
        .map(|row| {
            let resource_id = row.get::<String, _>("id");
            let region = row.get::<String, _>("region");
            let tags_json = row.try_get::<Option<String>, _>("tags").ok().flatten();
            let avg_read = row
                .try_get::<Option<f64>, _>("avg_read_bytes")
                .ok()
                .flatten()
                .unwrap_or(0.0);
            let avg_write = row
                .try_get::<Option<f64>, _>("avg_write_bytes")
                .ok()
                .flatten()
                .unwrap_or(0.0);
            let throughput = avg_read + avg_write;
            let (finding, target_size_gb, savings_pct) = if throughput < 10_000_000.0 {
                ("OVER_PROVISIONED", 80, 20.0)
            } else if throughput > 120_000_000.0 {
                ("UNDER_PROVISIONED", 200, 0.0)
            } else {
                ("OPTIMIZED", 100, 0.0)
            };
            let estimated_monthly_savings = if savings_pct > 0.0 { 6.4 } else { 0.0 };
            let volume_id = format!("vol-{}", resource_id.trim_start_matches("i-"));

            json!({
                "accountId": mock_account_id(),
                "volumeArn": format!("arn:aws:ec2:{region}:{}:volume/{volume_id}", mock_account_id()),
                "volumeName": format!("{}-data", resource_name_from_tags(tags_json.as_deref(), &resource_id)),
                "currentConfiguration": {
                    "volumeType": "gp3",
                    "volumeSize": 100,
                    "volumeBaselineIOPS": 3000,
                    "volumeBaselineThroughput": 125
                },
                "finding": finding,
                "lastRefreshTimestamp": last_refresh,
                "utilizationMetrics": [{
                    "name": "VolumeReadWriteBytesPerSecond",
                    "statistic": "Average",
                    "value": throughput
                }],
                "volumeRecommendationOptions": [{
                    "configuration": {
                        "volumeType": "gp3",
                        "volumeSize": target_size_gb,
                        "volumeBaselineIOPS": 3000,
                        "volumeBaselineThroughput": if target_size_gb >= 200 { 250 } else { 125 }
                    },
                    "performanceRisk": if finding == "UNDER_PROVISIONED" { 3.0 } else { 1.0 },
                    "rank": 1,
                    "savingsOpportunity": {
                        "estimatedMonthlySavings": {
                            "currency": "USD",
                            "value": estimated_monthly_savings
                        },
                        "savingsOpportunityPercentage": savings_pct
                    }
                }]
            })
        })
        .collect::<Vec<_>>();

    let mut response = json!({
        "volumeRecommendations": recommendations
    });
    if page_end < rows.len() {
        response["nextToken"] = json!(page_end.to_string());
    }

    Ok(response)
}

async fn handle_describe_report_definitions(
    body: Bytes,
) -> std::result::Result<Value, CostUsageError> {
    let _req: DescribeReportDefinitionsRequest = serde_json::from_slice(&body)
        .map_err(|e| CostUsageError::Validation(format!("Invalid JSON body: {}", e)))?;

    Ok(json!({
        "ReportDefinitions": [{
            "ReportName": "foxtail-cur",
            "TimeUnit": "DAILY",
            "Format": "textORcsv",
            "Compression": "GZIP",
            "AdditionalSchemaElements": ["RESOURCES"],
            "S3Bucket": "mock-cur-bucket",
            "S3Prefix": "cur/foxtail",
            "S3Region": "us-east-1",
            "AdditionalArtifacts": ["ATHENA"],
            "RefreshClosedReports": true,
            "ReportVersioning": "CREATE_NEW_REPORT",
            "ReportStatus": {
                "lastDelivery": "2026-03-12T00:00:00Z",
                "lastStatus": "SUCCESS"
            }
        }]
    }))
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
    let query: CloudWatchQuery = match parse_cloudwatch_query_from_form(&body) {
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
        "ListMetrics" => match handle_list_metrics(pool, query).await {
            Ok(xml) => (StatusCode::OK, [(header::CONTENT_TYPE, "text/xml")], xml).into_response(),
            Err(message) => error_response(
                protocol,
                "InvalidParameterValueException",
                &message,
                StatusCode::BAD_REQUEST,
            ),
        },
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
                Err(MetricStatisticsError::Validation(message)) => error_response(
                    protocol,
                    "InvalidParameterValueException",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(MetricStatisticsError::MissingParameter(message)) => error_response(
                    protocol,
                    "MissingParameter",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(MetricStatisticsError::InvalidParameterCombination(message)) => error_response(
                    protocol,
                    "InvalidParameterCombination",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(MetricStatisticsError::Internal(e)) => error_response(
                    protocol,
                    "InternalFailure",
                    &e.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
        "GetMetricData" => match handle_get_metric_data_xml(pool, query, &body, injected_now).await
        {
            Ok(xml) => (StatusCode::OK, [(header::CONTENT_TYPE, "text/xml")], xml).into_response(),
            Err(MetricDataError::Validation(message)) => error_response(
                protocol,
                "InvalidParameterValueException",
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
        },
        _ => error_response(
            protocol,
            "UnsupportedAction",
            &format!("Action {} not supported", query.action),
            StatusCode::BAD_REQUEST,
        ),
    }
}

async fn handle_list_metrics(
    pool: SqlitePool,
    query: CloudWatchQuery,
) -> std::result::Result<String, String> {
    let page = list_metrics_page(&pool, query).await?;
    cw::list_metrics_xml(page.metrics, page.next_token).map_err(|e| e.to_string())
}

async fn list_metrics_page(
    pool: &SqlitePool,
    query: CloudWatchQuery,
) -> std::result::Result<ListMetricsPage, String> {
    if let Some(recently_active) = query.recently_active.as_deref()
        && recently_active != "PT3H"
    {
        return Err("RecentlyActive must be PT3H when provided.".to_string());
    }

    let page_start = match query.next_token.as_deref() {
        Some(raw) if !raw.trim().is_empty() => raw
            .parse::<usize>()
            .map_err(|_| "Invalid NextToken value.".to_string())?,
        _ => 0,
    };

    let rows = sqlx::query(
        "SELECT DISTINCT m.namespace, m.metric_name, r.resource_type, r.id
         FROM metrics m
         JOIN resources r ON r.id = m.resource_id
         ORDER BY m.namespace, m.metric_name, r.resource_type, r.id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let metrics = rows
        .into_iter()
        .filter_map(|row| {
            let namespace = row.try_get::<String, _>("namespace").ok()?;
            let metric_name = row.try_get::<String, _>("metric_name").ok()?;
            let resource_type = row.try_get::<String, _>("resource_type").ok()?;
            let resource_id = row.try_get::<String, _>("id").ok()?;
            let dimension_name = cloudwatch_dimension_name_for_resource_type(&resource_type)?;
            Some(cw::Metric {
                namespace,
                metric_name,
                dimensions: cw::Dimensions {
                    members: vec![cw::Dimension {
                        name: dimension_name.to_string(),
                        value: resource_id,
                    }],
                },
            })
        })
        .filter(|metric| {
            query
                .namespace
                .as_deref()
                .is_none_or(|namespace| metric.namespace == namespace)
                && query
                    .metric_name
                    .as_deref()
                    .is_none_or(|metric_name| metric.metric_name == metric_name)
                && query.dim_name_1.as_deref().is_none_or(|name| {
                    metric.dimensions.members.iter().any(|dimension| {
                        dimension.name == name
                            && query
                                .dim_value_1
                                .as_deref()
                                .is_none_or(|value| dimension.value == value)
                    })
                })
                && query.dim_name_2.as_deref().is_none_or(|name| {
                    metric.dimensions.members.iter().any(|dimension| {
                        dimension.name == name
                            && query
                                .dim_value_2
                                .as_deref()
                                .is_none_or(|value| dimension.value == value)
                    })
                })
        })
        .collect::<Vec<_>>();

    if page_start > metrics.len() {
        return Err("NextToken points past available results.".to_string());
    }

    let total_metrics = metrics.len();
    let page_end = std::cmp::min(page_start + 500, total_metrics);
    let next_token = (page_end < total_metrics).then(|| page_end.to_string());
    let metrics = metrics
        .into_iter()
        .skip(page_start)
        .take(page_end - page_start)
        .collect::<Vec<_>>();

    Ok(ListMetricsPage {
        metrics,
        next_token,
    })
}

async fn handle_get_metric_data_xml(
    pool: SqlitePool,
    query: CloudWatchQuery,
    body: &Bytes,
    injected_now: Option<DateTime<Utc>>,
) -> std::result::Result<String, MetricDataError> {
    let start_time = parse_rfc3339_required("StartTime", query.start_time.as_deref())
        .map_err(MetricDataError::Validation)?;
    let end_time = parse_rfc3339_required("EndTime", query.end_time.as_deref())
        .map_err(MetricDataError::Validation)?;
    let max_datapoints = query.max_datapoints.unwrap_or(1000) as usize;
    if max_datapoints == 0 {
        return Err(MetricDataError::Validation(
            "MaxDatapoints must be greater than 0.".to_string(),
        ));
    }
    let page_start = match query.next_token.as_deref() {
        Some(raw) if !raw.trim().is_empty() => raw.parse::<usize>().map_err(|_| {
            MetricDataError::InvalidNextToken("Invalid NextToken value.".to_string())
        })?,
        _ => 0,
    };
    let metric_data_queries = parse_metric_data_queries_from_form(body)?;
    let series_list = build_metric_data_series_list(
        &pool,
        metric_data_queries,
        start_time,
        end_time,
        injected_now,
    )
    .await?;
    let paginated = paginate_metric_data_series(series_list, page_start, max_datapoints)?;

    let series = paginated
        .results
        .into_iter()
        .map(|series| cw::MetricDataXmlSeries {
            id: series.id,
            values: series.values,
            timestamps: series.timestamps,
        })
        .collect::<Vec<_>>();

    cw::get_metric_data_xml(series, paginated.next_token).map_err(MetricDataError::Internal)
}

async fn handle_get_metric_statistics(
    pool: SqlitePool,
    query: CloudWatchQuery,
    injected_now: Option<DateTime<Utc>>,
) -> std::result::Result<String, MetricStatisticsError> {
    let request = build_get_metric_statistics_request(query)?;
    let MetricStatisticsSeries {
        metric_name,
        metric_unit,
        statistics,
        datapoints,
    } = build_metric_statistics_series(&pool, request, injected_now).await?;

    let datapoints = metric_statistics_datapoints(datapoints, &statistics, &metric_unit);
    cw::get_metric_statistics_xml(metric_name, datapoints).map_err(MetricStatisticsError::Internal)
}

fn metric_statistics_datapoints(
    datapoints: Vec<AggregatedMetricPoint>,
    statistics: &[StandardStatistic],
    metric_unit: &str,
) -> Vec<cw::JsonDatapoint> {
    datapoints
        .into_iter()
        .map(|point| cw::JsonDatapoint {
            timestamp: point.timestamp.to_rfc3339(),
            unit: metric_unit.to_string(),
            sample_count: statistics
                .contains(&StandardStatistic::SampleCount)
                .then_some(point.sample_count),
            average: statistics
                .contains(&StandardStatistic::Average)
                .then_some(point.average),
            sum: statistics
                .contains(&StandardStatistic::Sum)
                .then_some(point.sum),
            minimum: statistics
                .contains(&StandardStatistic::Minimum)
                .then_some(point.minimum),
            maximum: statistics
                .contains(&StandardStatistic::Maximum)
                .then_some(point.maximum),
        })
        .collect()
}

async fn build_metric_statistics_series(
    pool: &SqlitePool,
    request: GetMetricStatisticsRequest,
    injected_now: Option<DateTime<Utc>>,
) -> std::result::Result<MetricStatisticsSeries, MetricStatisticsError> {
    let GetMetricStatisticsRequest {
        namespace,
        metric_name,
        start_time,
        end_time,
        period,
        resource_id,
        statistics,
    } = request;
    let metric_unit =
        cloudwatch_metric_unit(Some(namespace.as_str()), Some(metric_name.as_str())).to_string();

    let params = MetricQueryParams {
        resource_id,
        metric_name: Some(metric_name.clone()),
        namespace: Some(namespace),
        start_time: Some(start_time),
        end_time: Some(end_time),
        limit: Some(GET_METRIC_STATISTICS_RAW_ROW_LIMIT + 1),
        injected_now,
    };

    let points = metrics::query_metrics(pool, params)
        .await
        .map_err(MetricStatisticsError::Internal)?;
    if points.len() as i64 > GET_METRIC_STATISTICS_RAW_ROW_LIMIT {
        return Err(MetricStatisticsError::Validation(format!(
            "GetMetricStatistics cannot aggregate more than {} raw metric rows without truncating results.",
            GET_METRIC_STATISTICS_RAW_ROW_LIMIT
        )));
    }
    let datapoints =
        aggregate_metric_buckets(&points, start_time, end_time, period).map_err(|e| match e {
            MetricDataError::Validation(message) | MetricDataError::InvalidNextToken(message) => {
                MetricStatisticsError::Validation(message)
            }
            MetricDataError::Internal(err) => MetricStatisticsError::Internal(err),
        })?;

    Ok(MetricStatisticsSeries {
        metric_name,
        metric_unit,
        statistics,
        datapoints,
    })
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
        "GraniteServiceVersion20100801.ListMetrics" => {
            match handle_list_metrics_json(pool, body).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.0")],
                    Json(res),
                )
                    .into_response(),
                Err(message) => error_response(
                    protocol,
                    "InvalidParameterValueException",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
            }
        }
        "GraniteServiceVersion20100801.GetMetricStatistics" => {
            match handle_get_metric_statistics_json(pool, body, injected_now).await {
                Ok(res) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/x-amz-json-1.0")],
                    Json(res),
                )
                    .into_response(),
                Err(MetricStatisticsError::Validation(message)) => error_response(
                    protocol,
                    "InvalidParameterValueException",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(MetricStatisticsError::MissingParameter(message)) => error_response(
                    protocol,
                    "MissingParameter",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(MetricStatisticsError::InvalidParameterCombination(message)) => error_response(
                    protocol,
                    "InvalidParameterCombination",
                    &message,
                    StatusCode::BAD_REQUEST,
                ),
                Err(MetricStatisticsError::Internal(e)) => error_response(
                    protocol,
                    "InternalFailure",
                    &e.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        }
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

async fn handle_list_metrics_json(
    pool: SqlitePool,
    body: Bytes,
) -> std::result::Result<Value, String> {
    let req: ListMetricsJsonRequest =
        serde_json::from_slice(&body).map_err(|e| format!("Invalid JSON body: {}", e))?;
    let mut query = CloudWatchQuery {
        action: "ListMetrics".to_string(),
        namespace: req.namespace,
        metric_name: req.metric_name,
        start_time: None,
        end_time: None,
        period: None,
        max_datapoints: None,
        next_token: req.next_token,
        recently_active: req.recently_active,
        dim_name_1: None,
        dim_value_1: None,
        dim_name_2: None,
        dim_value_2: None,
        statistics: Vec::new(),
        extended_statistics: Vec::new(),
    };
    if let Some(dimensions) = req.dimensions {
        if let Some(dimension) = dimensions.first() {
            query.dim_name_1 = Some(dimension.name.clone());
            query.dim_value_1 = dimension.value.clone();
        }
        if let Some(dimension) = dimensions.get(1) {
            query.dim_name_2 = Some(dimension.name.clone());
            query.dim_value_2 = dimension.value.clone();
        }
    }

    let page = list_metrics_page(&pool, query).await?;
    Ok(cw::list_metrics_json(page.metrics, page.next_token))
}

async fn handle_get_metric_statistics_json(
    pool: SqlitePool,
    body: Bytes,
    injected_now: Option<DateTime<Utc>>,
) -> std::result::Result<Value, MetricStatisticsError> {
    let req: GetMetricStatisticsJsonRequest = serde_json::from_slice(&body)
        .map_err(|e| MetricStatisticsError::Validation(format!("Invalid JSON body: {}", e)))?;
    let start_time = parse_cloudwatch_datetime_required("StartTime", req.start_time.as_ref())
        .map_err(MetricStatisticsError::Validation)?;
    let end_time = parse_cloudwatch_datetime_required("EndTime", req.end_time.as_ref())
        .map_err(MetricStatisticsError::Validation)?;
    let request = build_get_metric_statistics_request_from_parts(MetricStatisticsRequestParts {
        namespace: req.namespace.ok_or_else(|| {
            MetricStatisticsError::Validation("Missing required field 'Namespace'.".to_string())
        })?,
        metric_name: req.metric_name.ok_or_else(|| {
            MetricStatisticsError::Validation("Missing required field 'MetricName'.".to_string())
        })?,
        start_time,
        end_time,
        period: req.period.ok_or_else(|| {
            MetricStatisticsError::Validation("Missing required field 'Period'.".to_string())
        })?,
        resource_id: extract_resource_id_from_dimensions(req.dimensions.as_deref()),
        statistics: req.statistics.unwrap_or_default(),
        extended_statistics: req.extended_statistics.unwrap_or_default(),
    })?;
    if request.period <= 0 {
        return Err(MetricStatisticsError::Validation(
            "Period must be greater than zero.".to_string(),
        ));
    }

    let MetricStatisticsSeries {
        metric_name,
        metric_unit,
        statistics,
        datapoints,
    } = build_metric_statistics_series(&pool, request, injected_now).await?;
    let datapoints = metric_statistics_datapoints(datapoints, &statistics, &metric_unit);

    Ok(cw::get_metric_statistics_json(metric_name, datapoints))
}

async fn handle_get_metric_data(
    pool: SqlitePool,
    body: Bytes,
    injected_now: Option<DateTime<Utc>>,
) -> std::result::Result<Value, MetricDataError> {
    let req: GetMetricDataRequest = serde_json::from_slice(&body)
        .map_err(|e| MetricDataError::Validation(format!("Invalid JSON body: {}", e)))?;

    let start_time = parse_cloudwatch_datetime_required("StartTime", Some(&req.start_time))
        .map_err(MetricDataError::Validation)?;
    let end_time = parse_cloudwatch_datetime_required("EndTime", Some(&req.end_time))
        .map_err(MetricDataError::Validation)?;
    if req.metric_data_queries.is_empty() {
        return Err(MetricDataError::Validation(
            "MetricDataQueries must include at least one query.".to_string(),
        ));
    }
    if req.metric_data_queries.len() > 50 {
        return Err(MetricDataError::Validation(
            "MetricDataQueries may contain at most 50 queries.".to_string(),
        ));
    }

    let max_datapoints = req.max_datapoints.unwrap_or(1000) as usize;
    if max_datapoints == 0 {
        return Err(MetricDataError::Validation(
            "MaxDatapoints must be greater than 0.".to_string(),
        ));
    }

    let page_start = match req.next_token.as_deref() {
        Some(raw) if !raw.trim().is_empty() => raw.parse::<usize>().map_err(|_| {
            MetricDataError::InvalidNextToken("Invalid NextToken value.".to_string())
        })?,
        _ => 0,
    };

    let series_list = build_metric_data_series_list(
        &pool,
        req.metric_data_queries,
        start_time,
        end_time,
        injected_now,
    )
    .await?;
    let paginated = paginate_metric_data_series(series_list, page_start, max_datapoints)?;

    let results = paginated
        .results
        .into_iter()
        .map(|series| cw::MetricDataJsonSeries {
            id: series.id,
            label: series.label,
            values: series.values,
            timestamps: series.timestamps,
        })
        .collect::<Vec<_>>();

    Ok(cw::get_metric_data_json(results, paginated.next_token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::extract::Extension;
    use axum::http::Request;
    use serde_json::Value;
    use tower::ServiceExt;

    async fn test_pool() -> SqlitePool {
        let path = std::env::temp_dir().join(format!("foxtail-test-{}.db", uuid::Uuid::new_v4()));
        let database_url = format!("sqlite:{}", path.display());
        crate::db::init(&database_url)
            .await
            .expect("test database should initialize")
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should read");
        serde_json::from_slice(&body).expect("response body should be valid JSON")
    }

    fn xml_tag_value(xml: &str, tag: &str) -> Option<String> {
        let start_tag = format!("<{}>", tag);
        let end_tag = format!("</{}>", tag);
        let (_, rest) = xml.split_once(&start_tag)?;
        let (value, _) = rest.split_once(&end_tag)?;
        Some(value.to_string())
    }

    fn xml_error_code(xml: &str) -> Option<String> {
        xml_tag_value(xml, "Code")
    }

    #[tokio::test]
    async fn status_route_reports_counts() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-test', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
             VALUES ('i-test', 'AWS/EC2', 'CPUUtilization', -3600, 12.0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_mock/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["resource_count"], 1);
        assert_eq!(body["metric_count"], 1);
    }

    async fn seed_fixture_ec2_estate(pool: &SqlitePool, count: usize) {
        for index in 0..count {
            let resource_id = format!("i-fixture-{index}");
            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario, tags)
                 VALUES (?, 'ec2', 'us-east-1', 'Baseline', '{}')",
            )
            .bind(&resource_id)
            .execute(pool)
            .await
            .unwrap();
            for (offset, value) in [
                (-14 * 86400, 8.0 + index as f64),
                (-3600, 12.0 + index as f64),
            ] {
                sqlx::query(
                    "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
                     VALUES (?, 'AWS/EC2', 'CPUUtilization', ?, ?)",
                )
                .bind(&resource_id)
                .bind(offset)
                .bind(value)
                .execute(pool)
                .await
                .unwrap();
            }
            sqlx::query(
                "INSERT INTO cost_records (resource_id, seconds_from_now, amount)
                 VALUES (?, -86400, 1.25)",
            )
            .bind(&resource_id)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    async fn seed_empty_fixture_ec2_estate(pool: &SqlitePool, count: usize) {
        for index in 0..count {
            let resource_id = format!("i-empty-fixture-{index}");
            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario, tags)
                 VALUES (?, 'ec2', 'us-east-1', 'Baseline', '{}')",
            )
            .bind(resource_id)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    async fn fixture_realize_with_test_token(
        State(pool): State<SqlitePool>,
        Extension(expected): Extension<String>,
        headers: HeaderMap,
        body: Bytes,
    ) -> axum::response::Response {
        if let Err(response) = ensure_admin_authorized_with_expected(&headers, Some(&expected)) {
            return *response;
        }
        fixture_realize_response(pool, body).await
    }

    #[tokio::test]
    async fn fixture_definition_is_canonical_and_status_is_absent_before_realization() {
        let pool = test_pool().await;
        let app = build_app(pool);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/_mock/fixture/definition?version=release-qualification-v1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.as_ref(), fixture::canonical_definition().unwrap().0);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_mock/fixture/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["status"], "ABSENT");
        assert!(body["manifest_digest"].is_null());
    }

    #[tokio::test]
    async fn fixture_realization_persists_exact_manifest_and_public_identities() {
        let pool = test_pool().await;
        seed_fixture_ec2_estate(&pool, 5).await;
        let app = build_app(pool);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_mock/fixture/realize")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"version":"release-qualification-v1","clock_anchor":"2026-08-05T00:00:00Z"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let realization = response_json(response).await;
        assert_eq!(realization["manifest"]["schema"], fixture::MANIFEST_SCHEMA);
        let manifest_digest = realization["manifest_digest"].as_str().unwrap();
        assert_eq!(manifest_digest, realization["manifest"]["digest"]);
        assert_eq!(
            realization["manifest"]["resources"]
                .as_array()
                .unwrap()
                .len(),
            fixture::REALIZED_CONTROL_IDS.len()
        );

        let manifest_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/_mock/fixture/manifest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(manifest_response.status(), StatusCode::OK);
        let manifest = response_json(manifest_response).await;
        assert_eq!(manifest["digest"], manifest_digest);
        let roles = manifest["control_catalogue"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|control| control["role"].as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            roles,
            BTreeSet::from(["degraded", "mutation", "negative", "positive"])
        );
        for resource in manifest["resources"].as_array().unwrap() {
            let resource_id = resource["resource_id"].as_str().unwrap();
            assert!(resource_id.starts_with("i-fixture-"));
            assert!(
                resource["aws_identity"]
                    .as_str()
                    .unwrap()
                    .contains(resource_id)
            );
        }

        let identities = response_json(
            app.oneshot(
                Request::builder()
                    .uri("/_mock/fixture/identities")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(identities["manifest_digest"], manifest_digest);
        assert_eq!(
            identities["resource_ids"].as_array().unwrap().len(),
            fixture::REALIZED_CONTROL_IDS.len()
        );
    }

    #[tokio::test]
    async fn fixture_realization_materializes_and_validates_empty_ec2_rows() {
        let pool = test_pool().await;
        seed_empty_fixture_ec2_estate(&pool, 5).await;
        let app = build_app(pool.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_mock/fixture/realize")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"version":"release-qualification-v1","clock_anchor":"2026-08-05T00:00:00Z"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let realization = response_json(response).await;
        let resources = realization["manifest"]["resources"].as_array().unwrap();
        assert_eq!(resources.len(), fixture::REALIZED_CONTROL_IDS.len());
        for resource in resources {
            assert!(resource["observed"]["metric_count"].as_i64().unwrap() > 0);
            assert_eq!(resource["observed"]["cost_record_count"], 14);
            assert!(
                !resource["observed"]["metric_names"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }

        let degraded = resources
            .iter()
            .find(|resource| resource["control_id"] == "ec2-idle-degraded-001")
            .unwrap();
        assert_eq!(degraded["evidence"]["cloudwatch_complete_days"], 13);
        assert_eq!(
            degraded["evidence"]["cloudwatch_missing_offsets"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let metric_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM metrics WHERE resource_id = 'i-empty-fixture-0'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let cost_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_records WHERE resource_id = 'i-empty-fixture-0'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(metric_rows, 42);
        assert_eq!(cost_rows, 14);
    }

    #[tokio::test]
    async fn fixture_realization_requires_admin_token_when_configured() {
        let pool = test_pool().await;
        seed_empty_fixture_ec2_estate(&pool, 5).await;
        let app = Router::new()
            .route(
                "/_mock/fixture/realize",
                post(fixture_realize_with_test_token),
            )
            .layer(Extension("fixture-secret".to_string()))
            .with_state(pool.clone());
        let request_body = Body::from(
            r#"{"version":"release-qualification-v1","clock_anchor":"2026-08-05T00:00:00Z"}"#,
        );
        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_mock/fixture/realize")
                    .header("content-type", "application/json")
                    .body(request_body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let status = fixture::read_state(&pool).await.unwrap();
        assert_eq!(status.status, "ABSENT");

        let authorized = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_mock/fixture/realize")
                    .header("content-type", "application/json")
                    .header(ADMIN_TOKEN_HEADER, "fixture-secret")
                    .body(Body::from(
                        r#"{"version":"release-qualification-v1","clock_anchor":"2026-08-05T00:00:00Z"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn fixture_realization_rejects_incomplete_estate_without_partial_state() {
        let pool = test_pool().await;
        seed_fixture_ec2_estate(&pool, 4).await;
        let app = build_app(pool);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_mock/fixture/realize")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let status = response_json(
            app.clone()
                .oneshot(
                    Request::builder()
                        .uri("/_mock/fixture/status")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status["status"], "ABSENT");
        let manifest = app
            .oneshot(
                Request::builder()
                    .uri("/_mock/fixture/manifest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(manifest.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn fixture_unknown_version_fails_closed() {
        let pool = test_pool().await;
        let app = build_app(pool);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_mock/fixture/definition?version=release-qualification-v99")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn fixture_realization_rejects_unknown_input_without_writing_state() {
        let pool = test_pool().await;
        seed_fixture_ec2_estate(&pool, 5).await;
        let app = build_app(pool);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_mock/fixture/realize")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"version":"release-qualification-v1","unexpected":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let status = response_json(
            app.oneshot(
                Request::builder()
                    .uri("/_mock/fixture/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status["status"], "ABSENT");
    }

    #[tokio::test]
    async fn dashboard_data_reports_untested_scorecard() {
        let pool = test_pool().await;
        let app = build_app(pool);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_mock/dashboard/data")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let supported_cloudwatch_operations = body["supported_apis"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["service"] == "cloudwatch")
            .filter_map(|entry| entry["operation"].as_str())
            .collect::<BTreeSet<_>>()
            .len() as i64;
        let supported_targets = body["supported_apis"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["target"].as_str())
            .collect::<Vec<_>>();
        assert!(supported_targets.contains(&"GraniteServiceVersion20100801.ListMetrics"));
        assert!(supported_targets.contains(&"GraniteServiceVersion20100801.GetMetricStatistics"));
        assert_eq!(
            body["coverage_scorecard"]["cloudwatch"]["implemented_operations"],
            json!(supported_cloudwatch_operations)
        );
        assert_eq!(body["coverage_scorecard"]["implemented_tested_entries"], 0);
        assert_eq!(
            body["coverage_scorecard"]["benchmarks"]["operation_coverage"],
            0.0
        );
        assert_eq!(
            body["coverage_scorecard"]["benchmarks"]["behavioral_coverage_count"],
            0
        );
    }

    #[tokio::test]
    async fn dashboard_data_returns_500_when_pool_is_closed() {
        let pool = test_pool().await;
        let app = build_app(pool.clone());
        pool.close().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_mock/dashboard/data")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_json(response).await;
        assert_eq!(body["error"], "dashboard query failed");
    }

    #[tokio::test]
    async fn cloudwatch_metric_data_emits_next_token_when_truncated() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-test', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (offset, value) in [(-7200, 10.0), (-3600, 20.0), (0, 30.0)] {
            sqlx::query(
                "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
                 VALUES ('i-test', 'AWS/EC2', 'CPUUtilization', ?, ?)",
            )
            .bind(offset)
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(pool);
        let body = json!({
            "StartTime": "2026-03-11T10:00:00Z",
            "EndTime": "2026-03-11T12:00:00Z",
            "MaxDatapoints": 2,
            "MetricDataQueries": [{
                "Id": "m1",
                "MetricStat": {
                    "Metric": {
                        "Namespace": "AWS/EC2",
                        "MetricName": "CPUUtilization",
                        "Dimensions": [{
                            "Name": "InstanceId",
                            "Value": "i-test"
                        }]
                    },
                    "Period": 3600,
                    "Stat": "Average"
                }
            }]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header(
                        "x-amz-target",
                        "GraniteServiceVersion20100801.GetMetricData",
                    )
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["NextToken"], "2");
        assert_eq!(body["MetricDataResults"][0]["Id"], "m1");
        assert_eq!(
            body["MetricDataResults"][0]["Values"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn cloudwatch_metric_data_preserves_query_id_and_aggregates_average() {
        let pool = test_pool().await;
        for resource_id in ["i-test", "i-other"] {
            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario, tags)
                 VALUES (?, 'ec2', 'us-east-1', 'Baseline', '{}')",
            )
            .bind(resource_id)
            .execute(&pool)
            .await
            .unwrap();
        }
        for (resource_id, offset, value) in [
            ("i-test", -7200, 10.0),
            ("i-test", -7100, 30.0),
            ("i-test", -3600, 20.0),
            ("i-other", -7150, 100.0),
        ] {
            sqlx::query(
                "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
                 VALUES (?, 'AWS/EC2', 'CPUUtilization', ?, ?)",
            )
            .bind(resource_id)
            .bind(offset)
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(pool);
        let body = json!({
            "StartTime": "2026-03-11T10:00:00Z",
            "EndTime": "2026-03-11T12:00:00Z",
            "MetricDataQueries": [{
                "Id": "cpu",
                "MetricStat": {
                    "Metric": {
                        "Namespace": "AWS/EC2",
                        "MetricName": "CPUUtilization",
                        "Dimensions": [{
                            "Name": "InstanceId",
                            "Value": "i-test"
                        }]
                    },
                    "Period": 3600,
                    "Stat": "Average"
                }
            }]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header(
                        "x-amz-target",
                        "GraniteServiceVersion20100801.GetMetricData",
                    )
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["MetricDataResults"][0]["Id"], "cpu");
        assert_eq!(body["MetricDataResults"][0]["Label"], "CPUUtilization");
        assert_eq!(
            body["MetricDataResults"][0]["Timestamps"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            body["MetricDataResults"][0]["Timestamps"][0],
            "2026-03-11T10:00:00+00:00"
        );
        assert_eq!(body["MetricDataResults"][0]["Values"][0], json!(20.0));
        assert_eq!(body["MetricDataResults"][0]["Values"][1], json!(20.0));
    }

    #[tokio::test]
    async fn cloudwatch_metric_data_pagination_keeps_shorter_queries_empty_on_later_pages() {
        let pool = test_pool().await;
        for resource_id in ["i-long", "i-short"] {
            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario, tags)
                 VALUES (?, 'ec2', 'us-east-1', 'Baseline', '{}')",
            )
            .bind(resource_id)
            .execute(&pool)
            .await
            .unwrap();
        }
        for (resource_id, offset, value) in [
            ("i-long", -10800, 10.0),
            ("i-long", -7200, 20.0),
            ("i-long", -3600, 30.0),
            ("i-short", -3600, 40.0),
        ] {
            sqlx::query(
                "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
                 VALUES (?, 'AWS/EC2', 'CPUUtilization', ?, ?)",
            )
            .bind(resource_id)
            .bind(offset)
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(pool);
        let body = json!({
            "StartTime": "2026-03-11T09:00:00Z",
            "EndTime": "2026-03-11T12:00:00Z",
            "MaxDatapoints": 2,
            "NextToken": "2",
            "MetricDataQueries": [
                {
                    "Id": "long",
                    "MetricStat": {
                        "Metric": {
                            "Namespace": "AWS/EC2",
                            "MetricName": "CPUUtilization",
                            "Dimensions": [{
                                "Name": "InstanceId",
                                "Value": "i-long"
                            }]
                        },
                        "Period": 3600,
                        "Stat": "Average"
                    }
                },
                {
                    "Id": "short",
                    "MetricStat": {
                        "Metric": {
                            "Namespace": "AWS/EC2",
                            "MetricName": "CPUUtilization",
                            "Dimensions": [{
                                "Name": "InstanceId",
                                "Value": "i-short"
                            }]
                        },
                        "Period": 3600,
                        "Stat": "Average"
                    }
                }
            ]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header(
                        "x-amz-target",
                        "GraniteServiceVersion20100801.GetMetricData",
                    )
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert!(body.get("NextToken").is_none());
        assert_eq!(
            body["MetricDataResults"][0]["Timestamps"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            body["MetricDataResults"][1]["Timestamps"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            body["MetricDataResults"][1]["Values"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn cloudwatch_json_metric_statistics_returns_requested_standard_stats() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-test', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (offset, value) in [(-7200, 10.0), (-7100, 30.0), (-3600, 40.0)] {
            sqlx::query(
                "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
                 VALUES ('i-test', 'AWS/EC2', 'CPUUtilization', ?, ?)",
            )
            .bind(offset)
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(pool);
        let start_time = chrono::DateTime::parse_from_rfc3339("2026-03-11T10:00:00Z")
            .unwrap()
            .timestamp();
        let end_time = chrono::DateTime::parse_from_rfc3339("2026-03-11T12:00:00Z")
            .unwrap()
            .timestamp();
        let body = json!({
            "Namespace": "AWS/EC2",
            "MetricName": "CPUUtilization",
            "Dimensions": [{
                "Name": "InstanceId",
                "Value": "i-test"
            }],
            "StartTime": start_time,
            "EndTime": end_time,
            "Period": 3600,
            "Statistics": ["Average", "Maximum"]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.0")
                    .header(
                        "x-amz-target",
                        "GraniteServiceVersion20100801.GetMetricStatistics",
                    )
                    .header("x-amzn-query-mode", "true")
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let datapoints = body["Datapoints"].as_array().unwrap();
        assert_eq!(datapoints.len(), 2);
        assert_eq!(body["Label"], "CPUUtilization");
        assert_eq!(datapoints[0]["Average"], json!(20.0));
        assert_eq!(datapoints[0]["Maximum"], json!(30.0));
        assert!(datapoints[0].get("SampleCount").is_none());
        assert_eq!(datapoints[0]["Unit"], "Percent");
    }

    #[tokio::test]
    async fn cloudwatch_json_list_metrics_returns_seeded_elasticache_metric() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('cache-1', 'elasticache', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
             VALUES ('cache-1', 'AWS/ElastiCache', 'CurrConnections', -3600, 25.0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let body = json!({
            "Namespace": "AWS/ElastiCache",
            "MetricName": "CurrConnections",
            "Dimensions": [{
                "Name": "CacheClusterId"
            }]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.0")
                    .header("x-amz-target", "GraniteServiceVersion20100801.ListMetrics")
                    .header("x-amzn-query-mode", "true")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["Metrics"][0]["Namespace"], "AWS/ElastiCache");
        assert_eq!(body["Metrics"][0]["MetricName"], "CurrConnections");
        assert_eq!(
            body["Metrics"][0]["Dimensions"][0]["Name"],
            "CacheClusterId"
        );
        assert_eq!(body["Metrics"][0]["Dimensions"][0]["Value"], "cache-1");
    }

    #[tokio::test]
    async fn cost_explorer_dimension_values_paginates_resource_ids() {
        let pool = test_pool().await;
        for id in ["i-a", "i-b", "i-c"] {
            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario, tags)
                 VALUES (?, 'ec2', 'us-east-1', 'Baseline', '{}')",
            )
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(pool);
        let body = json!({
            "TimePeriod": {
                "Start": "2026-03-01",
                "End": "2026-03-11"
            },
            "Dimension": "RESOURCE_ID",
            "MaxResults": 2
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSCostExplorer.GetDimensionValues")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["ReturnSize"], 2);
        assert_eq!(body["NextPageToken"], "2");
    }

    #[tokio::test]
    async fn cost_explorer_alias_dimension_values_uses_cli_target_prefix() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-test', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let body = json!({
            "TimePeriod": {
                "Start": "2026-03-01",
                "End": "2026-03-11"
            },
            "Dimension": "SERVICE"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSInsightsIndexService.GetDimensionValues")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(
            body["DimensionValues"][0]["Value"],
            "Amazon Elastic Compute Cloud - Compute"
        );
    }

    #[tokio::test]
    async fn cost_explorer_group_by_service_returns_populated_groups() {
        let pool = test_pool().await;
        let end = Utc::now().date_naive();
        let start = end - chrono::Duration::days(1);
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-ec2', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('db-rds', 'rds', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cost_records (resource_id, seconds_from_now, amount)
             VALUES ('i-ec2', -86400, 12.5), ('db-rds', -86400, 7.5)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let body = json!({
            "TimePeriod": {
                "Start": start.format("%Y-%m-%d").to_string(),
                "End": end.format("%Y-%m-%d").to_string()
            },
            "Granularity": "DAILY",
            "Metrics": ["UnblendedCost"],
            "GroupBy": [{
                "Type": "DIMENSION",
                "Key": "SERVICE"
            }]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSInsightsIndexService.GetCostAndUsage")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let groups = body["ResultsByTime"][0]["Groups"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0]["Metrics"]["UnblendedCost"]["Unit"],
            Value::String("USD".to_string())
        );
    }

    #[tokio::test]
    async fn cost_explorer_group_by_usage_type_filters_ec2_and_returns_usage_quantity() {
        let pool = test_pool().await;
        let end = Utc::now().date_naive();
        let start = end - chrono::Duration::days(1);
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-ec2', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('app/lb', 'elb', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cost_records (resource_id, seconds_from_now, amount)
             VALUES ('i-ec2', -86400, 19.20), ('app/lb', -86400, 2.25)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let body = json!({
            "TimePeriod": {
                "Start": start.format("%Y-%m-%d").to_string(),
                "End": end.format("%Y-%m-%d").to_string()
            },
            "Granularity": "MONTHLY",
            "Metrics": ["UnblendedCost", "UsageQuantity"],
            "GroupBy": [{
                "Type": "DIMENSION",
                "Key": "USAGE_TYPE"
            }],
            "Filter": {
                "Dimensions": {
                    "Key": "SERVICE",
                    "Values": ["Amazon Elastic Compute Cloud - Compute"]
                }
            }
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSInsightsIndexService.GetCostAndUsage")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["GroupDefinitions"][0]["Key"], "USAGE_TYPE");
        let groups = body["ResultsByTime"][0]["Groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["Keys"][0], "USE1-BoxUsage:m6i.xlarge");
        assert_eq!(
            groups[0]["Metrics"]["UnblendedCost"]["Amount"],
            Value::String("19.20".to_string())
        );
        assert_eq!(
            groups[0]["Metrics"]["UsageQuantity"]["Amount"],
            Value::String("200.0000".to_string())
        );
        assert_eq!(
            body["ResultsByTime"][0]["Total"]["UsageQuantity"]["Unit"],
            Value::String("N/A".to_string())
        );
    }

    #[tokio::test]
    async fn cost_explorer_group_by_usage_type_filters_elb() {
        let pool = test_pool().await;
        let end = Utc::now().date_naive();
        let start = end - chrono::Duration::days(1);
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-ec2', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('app/lb', 'elb', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cost_records (resource_id, seconds_from_now, amount)
             VALUES ('i-ec2', -86400, 19.20), ('app/lb', -86400, 2.25)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let body = json!({
            "TimePeriod": {
                "Start": start.format("%Y-%m-%d").to_string(),
                "End": end.format("%Y-%m-%d").to_string()
            },
            "Granularity": "MONTHLY",
            "Metrics": ["UnblendedCost", "UsageQuantity"],
            "GroupBy": [{
                "Type": "DIMENSION",
                "Key": "USAGE_TYPE"
            }],
            "Filter": {
                "Dimensions": {
                    "Key": "SERVICE",
                    "Values": ["Elastic Load Balancing"]
                }
            }
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSInsightsIndexService.GetCostAndUsage")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let groups = body["ResultsByTime"][0]["Groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["Keys"][0], "USE1-LoadBalancerUsage");
        assert_eq!(
            groups[0]["Metrics"]["UsageQuantity"]["Amount"],
            Value::String("100.0000".to_string())
        );
    }

    #[tokio::test]
    async fn cost_explorer_with_resources_defaults_to_resource_id_groups() {
        let pool = test_pool().await;
        let end = Utc::now().date_naive();
        let start = end - chrono::Duration::days(1);
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-a', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"prod\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-b', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"dev\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cost_records (resource_id, seconds_from_now, amount)
             VALUES ('i-a', -86400, 12.5), ('i-b', -86400, 7.5)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let body = json!({
            "TimePeriod": {
                "Start": start.format("%Y-%m-%d").to_string(),
                "End": end.format("%Y-%m-%d").to_string()
            },
            "Granularity": "DAILY",
            "Metrics": ["UnblendedCost"]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header(
                        "x-amz-target",
                        "AWSInsightsIndexService.GetCostAndUsageWithResources",
                    )
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let groups = body["ResultsByTime"][0]["Groups"].as_array().unwrap();
        assert_eq!(body["GroupDefinitions"][0]["Key"], "RESOURCE_ID");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0]["Keys"][0], "i-a");
        assert_eq!(groups[1]["Keys"][0], "i-b");
    }

    #[tokio::test]
    async fn cost_explorer_get_tags_returns_distinct_tag_values_with_pagination() {
        let pool = test_pool().await;
        let end = Utc::now().date_naive();
        let start = end - chrono::Duration::days(10);
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES
             ('i-a', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"prod\",\"Name\":\"api-a\"}'),
             ('i-b', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"prod\",\"Name\":\"api-b\"}'),
             ('i-c', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"dev\",\"Name\":\"api-c\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let body = json!({
            "TimePeriod": {
                "Start": start.format("%Y-%m-%d").to_string(),
                "End": end.format("%Y-%m-%d").to_string()
            },
            "TagKey": "Environment",
            "MaxResults": 1
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSCostExplorer.GetTags")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["ReturnSize"], 1);
        assert_eq!(body["TotalSize"], 2);
        assert_eq!(body["Tags"][0], "dev");
        assert_eq!(body["NextPageToken"], "1");
    }

    #[tokio::test]
    async fn cost_forecast_requires_granularity() {
        let pool = test_pool().await;
        let app = build_app(pool);
        let body = json!({
            "TimePeriod": {
                "Start": "2026-03-01",
                "End": "2026-03-11"
            },
            "Metric": "UNBLENDED_COST"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSInsightsIndexService.GetCostForecast")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["__type"], "ValidationException");
    }

    #[tokio::test]
    async fn cost_explorer_alias_anomaly_monitors_returns_data() {
        let pool = test_pool().await;
        let app = build_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSInsightsIndexService.GetAnomalyMonitors")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["AnomalyMonitors"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn tagging_get_resources_returns_tagged_inventory_with_pagination() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES
             ('i-a', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"prod\",\"Name\":\"api-a\"}'),
             ('i-b', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"prod\",\"Name\":\"api-b\"}'),
             ('db-a', 'rds', 'us-east-1', 'Baseline', '{\"Environment\":\"dev\",\"Name\":\"db-a\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let body = json!({
            "ResourcesPerPage": 1,
            "ResourceTypeFilters": ["ec2"],
            "TagFilters": [{
                "Key": "Environment",
                "Values": ["prod"]
            }]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header(
                        "x-amz-target",
                        "ResourceGroupsTaggingAPI_20170126.GetResources",
                    )
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let mappings = body["ResourceTagMappingList"].as_array().unwrap();
        assert_eq!(mappings.len(), 1);
        assert!(
            mappings[0]["ResourceARN"]
                .as_str()
                .unwrap()
                .contains(":instance/")
        );
        assert_eq!(body["PaginationToken"], "1");
    }

    #[tokio::test]
    async fn pricing_get_products_returns_price_list_strings() {
        let pool = test_pool().await;
        let app = build_app(pool);
        let body = json!({
            "ServiceCode": "AmazonEC2",
            "FormatVersion": "aws_v1",
            "Filters": [{
                "Type": "TERM_MATCH",
                "Field": "instanceType",
                "Value": "m6i.large"
            }]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSPriceListService.GetProducts")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let price_list = body["PriceList"].as_array().unwrap();
        assert_eq!(body["FormatVersion"], "aws_v1");
        assert_eq!(price_list.len(), 1);
        assert!(price_list[0].as_str().unwrap().contains("AmazonEC2"));
        assert!(price_list[0].as_str().unwrap().contains("m6i.large"));
    }

    #[tokio::test]
    async fn pricing_get_products_defaults_missing_format_version_to_aws_v1() {
        let pool = test_pool().await;
        let app = build_app(pool);
        let body = json!({
            "ServiceCode": "AmazonEC2",
            "Filters": [{
                "Type": "TERM_MATCH",
                "Field": "instanceType",
                "Value": "m6i.large"
            }]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSPriceListService.GetProducts")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["FormatVersion"], "aws_v1");
        assert_eq!(body["PriceList"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn pricing_get_products_rejects_unknown_format_version() {
        let pool = test_pool().await;
        let app = build_app(pool);
        let body = json!({
            "ServiceCode": "AmazonEC2",
            "FormatVersion": "aws_v2"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSPriceListService.GetProducts")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["__type"], "InvalidParameterException");
        assert_eq!(body["Message"], "FormatVersion must be 'aws_v1'.");
    }

    #[tokio::test]
    async fn pricing_get_products_supports_storage_filters_and_pagination() {
        let pool = test_pool().await;
        let app = build_app(pool.clone());

        let first_page_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSPriceListService.GetProducts")
                    .body(Body::from(
                        json!({
                            "ServiceCode": "AmazonEC2",
                            "FormatVersion": "aws_v1",
                            "MaxResults": 1
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(first_page_response.status(), StatusCode::OK);
        let first_page = response_json(first_page_response).await;
        assert_eq!(first_page["PriceList"].as_array().unwrap().len(), 1);
        assert_eq!(first_page["NextToken"], "1");

        let storage_filter_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSPriceListService.GetProducts")
                    .body(Body::from(
                        json!({
                            "ServiceCode": "AmazonEC2",
                            "FormatVersion": "aws_v1",
                            "Filters": [{
                                "Type": "TERM_MATCH",
                                "Field": "volumeType",
                                "Value": "gp3"
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(storage_filter_response.status(), StatusCode::OK);
        let storage_page = response_json(storage_filter_response).await;
        let price_list = storage_page["PriceList"].as_array().unwrap();
        assert_eq!(price_list.len(), 1);
        assert!(price_list[0].as_str().unwrap().contains("gp3"));
    }

    #[tokio::test]
    async fn tagging_get_tag_keys_returns_distinct_values() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES
             ('i-a', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"prod\",\"Name\":\"api-a\"}'),
             ('i-b', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"dev\",\"Owner\":\"platform\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header(
                        "x-amz-target",
                        "ResourceGroupsTaggingAPI_20170126.GetTagKeys",
                    )
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["TagKeys"], json!(["Environment", "Name", "Owner"]));
    }

    #[tokio::test]
    async fn tagging_get_tag_values_returns_distinct_values_for_key() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES
             ('i-a', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"prod\",\"Name\":\"api-a\"}'),
             ('i-b', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"dev\",\"Name\":\"api-b\"}'),
             ('i-c', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"prod\",\"Name\":\"api-c\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let body = json!({ "Key": "Environment" });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header(
                        "x-amz-target",
                        "ResourceGroupsTaggingAPI_20170126.GetTagValues",
                    )
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["TagValues"], json!(["dev", "prod"]));
    }

    #[tokio::test]
    async fn compute_optimizer_ec2_recommendations_reflect_average_cpu() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES
             ('i-low', 'ec2', 'us-east-1', 'Baseline', '{\"Name\":\"idle-node\"}'),
             ('i-high', 'ec2', 'us-east-1', 'Baseline', '{\"Name\":\"busy-node\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (resource_id, value) in [("i-low", 8.0), ("i-low", 12.0), ("i-high", 82.0)] {
            sqlx::query(
                "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
                 VALUES (?, 'AWS/EC2', 'CPUUtilization', -3600, ?)",
            )
            .bind(resource_id)
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(pool);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.0")
                    .header(
                        "x-amz-target",
                        "ComputeOptimizerService.GetEC2InstanceRecommendations",
                    )
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let recs = body["instanceRecommendations"].as_array().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0]["instanceName"], "busy-node");
        assert_eq!(recs[0]["finding"], "UNDER_PROVISIONED");
        assert_eq!(recs[1]["instanceName"], "idle-node");
        assert_eq!(recs[1]["finding"], "OVER_PROVISIONED");
    }

    #[tokio::test]
    async fn compute_optimizer_ebs_recommendations_reflect_disk_activity() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-a', 'ec2', 'us-east-1', 'Baseline', '{\"Name\":\"api-a\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (metric_name, value) in [
            ("DiskReadBytes", 2_000_000.0),
            ("DiskWriteBytes", 3_000_000.0),
        ] {
            sqlx::query(
                "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
                 VALUES ('i-a', 'AWS/EC2', ?, -3600, ?)",
            )
            .bind(metric_name)
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(pool);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.0")
                    .header(
                        "x-amz-target",
                        "ComputeOptimizerService.GetEBSVolumeRecommendations",
                    )
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let recs = body["volumeRecommendations"].as_array().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["finding"], "OVER_PROVISIONED");
        assert!(
            recs[0]["volumeArn"]
                .as_str()
                .unwrap()
                .contains("volume/vol-a")
        );
    }

    #[tokio::test]
    async fn cost_explorer_usage_forecast_returns_usage_quantity_series() {
        let pool = test_pool().await;
        let end = Utc::now().date_naive();
        let start = end - chrono::Duration::days(2);
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-a', 'ec2', 'us-east-1', 'Baseline', '{\"Environment\":\"prod\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cost_records (resource_id, seconds_from_now, amount)
             VALUES ('i-a', -172800, 9.60), ('i-a', -86400, 19.20)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let body = json!({
            "TimePeriod": {
                "Start": start.format("%Y-%m-%d").to_string(),
                "End": end.format("%Y-%m-%d").to_string()
            },
            "Metric": "USAGE_QUANTITY",
            "Granularity": "DAILY"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header("x-amz-target", "AWSInsightsIndexService.GetUsageForecast")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["Total"]["Unit"], "UsageQuantity");
        assert_eq!(body["ForecastResultsByTime"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn cur_describe_report_definitions_returns_mock_report() {
        let pool = test_pool().await;
        let app = build_app(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-amz-json-1.1")
                    .header(
                        "x-amz-target",
                        "AWSOrigamiServiceGatewayService.DescribeReportDefinitions",
                    )
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["ReportDefinitions"][0]["ReportName"], "foxtail-cur");
        assert_eq!(
            body["ReportDefinitions"][0]["ReportStatus"]["lastStatus"],
            "SUCCESS"
        );
    }

    #[tokio::test]
    async fn cloudwatch_query_xml_returns_requested_standard_statistics() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-test', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (offset, value) in [(-3590, 10.0), (-3500, 20.0), (-3400, 30.0)] {
            sqlx::query(
                "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
                 VALUES ('i-test', 'AWS/EC2', 'CPUUtilization', ?, ?)",
            )
            .bind(offset)
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(pool);
        let form = "Action=GetMetricStatistics&Namespace=AWS%2FEC2&MetricName=CPUUtilization&StartTime=2026-03-11T11%3A00%3A00Z&EndTime=2026-03-11T12%3A00%3A00Z&Period=3600&Statistics.member.1=SampleCount&Statistics.member.2=Average&Statistics.member.3=Sum&Statistics.member.4=Minimum&Statistics.member.5=Maximum&Dimensions.member.1.Name=InstanceId&Dimensions.member.1.Value=i-test";

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let xml = String::from_utf8(body.to_vec()).unwrap();
        assert!(xml.contains("GetMetricStatisticsResponse"));
        assert!(xml.contains("CPUUtilization"));
        assert!(xml.contains("2026-03-11T11:00:00+00:00"));
        assert_eq!(
            xml_tag_value(&xml, "SampleCount")
                .unwrap()
                .parse::<f64>()
                .unwrap(),
            3.0
        );
        assert_eq!(
            xml_tag_value(&xml, "Average")
                .unwrap()
                .parse::<f64>()
                .unwrap(),
            20.0
        );
        assert_eq!(
            xml_tag_value(&xml, "Sum").unwrap().parse::<f64>().unwrap(),
            60.0
        );
        assert_eq!(
            xml_tag_value(&xml, "Minimum")
                .unwrap()
                .parse::<f64>()
                .unwrap(),
            10.0
        );
        assert_eq!(
            xml_tag_value(&xml, "Maximum")
                .unwrap()
                .parse::<f64>()
                .unwrap(),
            30.0
        );
    }

    #[test]
    fn cloudwatch_query_parser_collects_metric_statistics_members_once() {
        let form = b"Action=GetMetricStatistics&Namespace=AWS%2FEC2&MetricName=CPUUtilization&StartTime=2026-03-11T11%3A00%3A00Z&EndTime=2026-03-11T12%3A00%3A00Z&Period=3600&Statistics.member.1=SampleCount&Statistics.member.2=Average&ExtendedStatistics.member.1=p99&Dimensions.member.1.Name=InstanceId&Dimensions.member.1.Value=i-test";

        let query = parse_cloudwatch_query_from_form(form).unwrap();

        assert_eq!(query.action, "GetMetricStatistics");
        assert_eq!(query.namespace.as_deref(), Some("AWS/EC2"));
        assert_eq!(query.metric_name.as_deref(), Some("CPUUtilization"));
        assert_eq!(query.statistics, vec!["SampleCount", "Average"]);
        assert_eq!(query.extended_statistics, vec!["p99"]);
    }

    #[test]
    fn aggregate_metric_buckets_streams_standard_statistics() {
        let start_time = chrono::DateTime::parse_from_rfc3339("2026-03-11T11:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let end_time = chrono::DateTime::parse_from_rfc3339("2026-03-11T13:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let points = vec![
            metrics::MetricPoint {
                value: 20.0,
                timestamp: start_time + chrono::Duration::seconds(3599),
            },
            metrics::MetricPoint {
                value: 10.0,
                timestamp: start_time + chrono::Duration::seconds(5),
            },
            metrics::MetricPoint {
                value: 30.0,
                timestamp: start_time + chrono::Duration::seconds(20),
            },
            metrics::MetricPoint {
                value: 50.0,
                timestamp: start_time + chrono::Duration::seconds(3601),
            },
        ];

        let buckets = aggregate_metric_buckets(&points, start_time, end_time, 3600)
            .unwrap_or_else(|_| panic!("bucket aggregation should succeed"));

        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].timestamp, start_time);
        assert_eq!(buckets[0].sample_count, 3.0);
        assert_eq!(buckets[0].average, 20.0);
        assert_eq!(buckets[0].sum, 60.0);
        assert_eq!(buckets[0].minimum, 10.0);
        assert_eq!(buckets[0].maximum, 30.0);
        assert_eq!(
            buckets[1].timestamp,
            start_time + chrono::Duration::seconds(3600)
        );
        assert_eq!(buckets[1].sample_count, 1.0);
        assert_eq!(buckets[1].average, 50.0);
        assert_eq!(buckets[1].sum, 50.0);
        assert_eq!(buckets[1].minimum, 50.0);
        assert_eq!(buckets[1].maximum, 50.0);
    }

    #[tokio::test]
    async fn cloudwatch_query_xml_metric_statistics_requires_statistics() {
        let pool = test_pool().await;
        let app = build_app(pool);
        let form = "Action=GetMetricStatistics&Namespace=AWS%2FEC2&MetricName=CPUUtilization&StartTime=2026-03-11T11%3A00%3A00Z&EndTime=2026-03-11T12%3A00%3A00Z&Period=3600";

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let xml = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(xml_error_code(&xml).as_deref(), Some("MissingParameter"));
        assert!(xml.contains(
            "GetMetricStatistics requires Statistics.member.N or ExtendedStatistics.member.N."
        ));
    }

    #[tokio::test]
    async fn cloudwatch_query_xml_metric_statistics_rejects_mixed_stat_inputs() {
        let pool = test_pool().await;
        let app = build_app(pool);
        let form = "Action=GetMetricStatistics&Namespace=AWS%2FEC2&MetricName=CPUUtilization&StartTime=2026-03-11T11%3A00%3A00Z&EndTime=2026-03-11T12%3A00%3A00Z&Period=3600&Statistics.member.1=Average&ExtendedStatistics.member.1=p99";

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let xml = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(
            xml_error_code(&xml).as_deref(),
            Some("InvalidParameterCombination")
        );
        assert!(xml.contains(
            "GetMetricStatistics does not allow both Statistics and ExtendedStatistics."
        ));
    }

    #[tokio::test]
    async fn cloudwatch_query_xml_metric_statistics_rejects_truncated_raw_rows() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-test', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();

        for _ in 0..100 {
            let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
                "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value) ",
            );
            builder.push_values(0..100, |mut row, _| {
                row.push_bind("i-test")
                    .push_bind("AWS/EC2")
                    .push_bind("CPUUtilization")
                    .push_bind(-3600)
                    .push_bind(1.0f64);
            });
            builder.build().execute(&pool).await.unwrap();
        }

        sqlx::query(
            "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
             VALUES ('i-test', 'AWS/EC2', 'CPUUtilization', -3600, 1.0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let form = "Action=GetMetricStatistics&Namespace=AWS%2FEC2&MetricName=CPUUtilization&StartTime=2026-03-11T11%3A00%3A00Z&EndTime=2026-03-11T12%3A00%3A00Z&Period=3600&Statistics.member.1=SampleCount&Statistics.member.2=Average&Dimensions.member.1.Name=InstanceId&Dimensions.member.1.Value=i-test";

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let xml = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(
            xml_error_code(&xml).as_deref(),
            Some("InvalidParameterValueException")
        );
        assert!(xml.contains(
            "GetMetricStatistics cannot aggregate more than 10000 raw metric rows without truncating results."
        ));
    }

    #[tokio::test]
    async fn cloudwatch_query_xml_metric_statistics_rejects_unsupported_statistic() {
        let pool = test_pool().await;
        let app = build_app(pool);
        let form = "Action=GetMetricStatistics&Namespace=AWS%2FEC2&MetricName=CPUUtilization&StartTime=2026-03-11T11%3A00%3A00Z&EndTime=2026-03-11T12%3A00%3A00Z&Period=3600&Statistics.member.1=Median";

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let xml = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(
            xml_error_code(&xml).as_deref(),
            Some("InvalidParameterValueException")
        );
        assert!(xml.contains("Unsupported Statistics value 'Median'."));
    }

    #[tokio::test]
    async fn cloudwatch_query_xml_metric_data_preserves_query_id_and_aggregates() {
        let pool = test_pool().await;
        for resource_id in ["i-test", "i-other"] {
            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario, tags)
                 VALUES (?, 'ec2', 'us-east-1', 'Baseline', '{}')",
            )
            .bind(resource_id)
            .execute(&pool)
            .await
            .unwrap();
        }
        for (resource_id, offset, value) in [
            ("i-test", -7200, 10.0),
            ("i-test", -7100, 30.0),
            ("i-test", -3600, 20.0),
            ("i-other", -7150, 100.0),
        ] {
            sqlx::query(
                "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
                 VALUES (?, 'AWS/EC2', 'CPUUtilization', ?, ?)",
            )
            .bind(resource_id)
            .bind(offset)
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(pool);
        let form = "Action=GetMetricData&Version=2010-08-01&MetricDataQueries.member.1.Id=cpu&MetricDataQueries.member.1.MetricStat.Metric.Namespace=AWS%2FEC2&MetricDataQueries.member.1.MetricStat.Metric.MetricName=CPUUtilization&MetricDataQueries.member.1.MetricStat.Metric.Dimensions.member.1.Name=InstanceId&MetricDataQueries.member.1.MetricStat.Metric.Dimensions.member.1.Value=i-test&MetricDataQueries.member.1.MetricStat.Period=3600&MetricDataQueries.member.1.MetricStat.Stat=Average&StartTime=2026-03-11T10%3A00%3A00Z&EndTime=2026-03-11T12%3A00%3A00Z";

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let xml = String::from_utf8(body.to_vec()).unwrap();
        assert!(xml.contains("GetMetricDataResponse"));
        assert!(xml.contains("<Id>cpu</Id>"));
        assert_eq!(xml.matches("2026-03-11T10:00:00+00:00").count(), 1);
        assert_eq!(xml.matches("2026-03-11T11:00:00+00:00").count(), 1);
    }

    #[tokio::test]
    async fn cloudwatch_query_xml_metric_data_supports_network_in_and_out_queries() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-test', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();

        for (metric_name, offset, value) in [
            ("NetworkIn", -7200, 12_000_000.0),
            ("NetworkIn", -3600, 14_000_000.0),
            ("NetworkOut", -7200, 9_000_000.0),
            ("NetworkOut", -3600, 11_000_000.0),
        ] {
            sqlx::query(
                "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
                 VALUES ('i-test', 'AWS/EC2', ?, ?, ?)",
            )
            .bind(metric_name)
            .bind(offset)
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(pool);
        let form = "Action=GetMetricData&Version=2010-08-01&MetricDataQueries.member.1.Id=netin&MetricDataQueries.member.1.MetricStat.Metric.Namespace=AWS%2FEC2&MetricDataQueries.member.1.MetricStat.Metric.MetricName=NetworkIn&MetricDataQueries.member.1.MetricStat.Metric.Dimensions.member.1.Name=InstanceId&MetricDataQueries.member.1.MetricStat.Metric.Dimensions.member.1.Value=i-test&MetricDataQueries.member.1.MetricStat.Period=3600&MetricDataQueries.member.1.MetricStat.Stat=Average&MetricDataQueries.member.2.Id=netout&MetricDataQueries.member.2.MetricStat.Metric.Namespace=AWS%2FEC2&MetricDataQueries.member.2.MetricStat.Metric.MetricName=NetworkOut&MetricDataQueries.member.2.MetricStat.Metric.Dimensions.member.1.Name=InstanceId&MetricDataQueries.member.2.MetricStat.Metric.Dimensions.member.1.Value=i-test&MetricDataQueries.member.2.MetricStat.Period=3600&MetricDataQueries.member.2.MetricStat.Stat=Average&StartTime=2026-03-11T10%3A00%3A00Z&EndTime=2026-03-11T12%3A00%3A00Z";

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let xml = String::from_utf8(body.to_vec()).unwrap();
        assert!(xml.contains("<Id>netin</Id>"));
        assert!(xml.contains("<Id>netout</Id>"));
        assert!(xml.contains("12000000"));
        assert!(xml.contains("14000000"));
        assert!(xml.contains("9000000"));
        assert!(xml.contains("11000000"));
        assert_eq!(xml.matches("2026-03-11T10:00:00+00:00").count(), 2);
        assert_eq!(xml.matches("2026-03-11T11:00:00+00:00").count(), 2);
    }

    #[tokio::test]
    async fn cloudwatch_query_xml_metric_data_emits_next_token_when_paginated() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-test', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();

        for (offset, value) in [(-7200, 10.0), (-3600, 20.0)] {
            sqlx::query(
                "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
                 VALUES ('i-test', 'AWS/EC2', 'CPUUtilization', ?, ?)",
            )
            .bind(offset)
            .bind(value)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(pool);
        let first_page = "Action=GetMetricData&Version=2010-08-01&MetricDataQueries.member.1.Id=cpu&MetricDataQueries.member.1.MetricStat.Metric.Namespace=AWS%2FEC2&MetricDataQueries.member.1.MetricStat.Metric.MetricName=CPUUtilization&MetricDataQueries.member.1.MetricStat.Metric.Dimensions.member.1.Name=InstanceId&MetricDataQueries.member.1.MetricStat.Metric.Dimensions.member.1.Value=i-test&MetricDataQueries.member.1.MetricStat.Period=3600&MetricDataQueries.member.1.MetricStat.Stat=Average&MaxDatapoints=1&StartTime=2026-03-11T10%3A00%3A00Z&EndTime=2026-03-11T12%3A00%3A00Z";

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(first_page))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let xml = String::from_utf8(body.to_vec()).unwrap();
        assert!(xml.contains("<NextToken>1</NextToken>"));
        assert_eq!(xml.matches("<member>").count(), 3);
        assert_eq!(xml.matches("2026-03-11T10:00:00+00:00").count(), 1);

        let second_page = "Action=GetMetricData&Version=2010-08-01&MetricDataQueries.member.1.Id=cpu&MetricDataQueries.member.1.MetricStat.Metric.Namespace=AWS%2FEC2&MetricDataQueries.member.1.MetricStat.Metric.MetricName=CPUUtilization&MetricDataQueries.member.1.MetricStat.Metric.Dimensions.member.1.Name=InstanceId&MetricDataQueries.member.1.MetricStat.Metric.Dimensions.member.1.Value=i-test&MetricDataQueries.member.1.MetricStat.Period=3600&MetricDataQueries.member.1.MetricStat.Stat=Average&MaxDatapoints=1&NextToken=1&StartTime=2026-03-11T10%3A00%3A00Z&EndTime=2026-03-11T12%3A00%3A00Z";

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("x-mock-now", "2026-03-11T12:00:00Z")
                    .body(Body::from(second_page))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let xml = String::from_utf8(body.to_vec()).unwrap();
        assert!(!xml.contains("<NextToken>"));
        assert_eq!(xml.matches("2026-03-11T11:00:00+00:00").count(), 1);
        assert!(!xml.contains("2026-03-11T10:00:00+00:00"));
    }

    #[tokio::test]
    async fn cloudwatch_query_xml_list_metrics_returns_seeded_metric() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-test', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
             VALUES ('i-test', 'AWS/EC2', 'CPUUtilization', -3600, 12.0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool);
        let form =
            "Action=ListMetrics&Version=2010-08-01&Namespace=AWS%2FEC2&MetricName=CPUUtilization";

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let xml = String::from_utf8(body.to_vec()).unwrap();
        assert!(xml.contains("ListMetricsResponse"));
        assert!(xml.contains("CPUUtilization"));
        assert!(xml.contains("InstanceId"));
        assert!(xml.contains("i-test"));
    }

    #[tokio::test]
    async fn scenario_endpoint_updates_resource_scenario() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('i-test', 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_mock/scenario")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "scenario": "Spike",
                            "resource_id": "i-test"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let scenario: String =
            sqlx::query_scalar("SELECT scenario FROM resources WHERE id = 'i-test'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(scenario, "Spike");
    }

    #[tokio::test]
    async fn scenario_endpoint_generates_elasticache_metrics() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES ('cache-1', 'elasticache', 'us-east-1', 'Baseline', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(pool.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_mock/scenario")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "scenario": "Spike",
                            "resource_id": "cache-1"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let metric_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM metrics
             WHERE resource_id = 'cache-1'
               AND namespace = 'AWS/ElastiCache'
               AND metric_name IN ('CPUUtilization', 'CurrConnections')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(metric_count, 672);
    }
}
