use anyhow::Result;
use quick_xml::se::to_string;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename = "ErrorResponse")]
pub struct ErrorResponse {
    #[serde(rename = "Error")]
    pub error: ErrorDetails,
    #[serde(rename = "RequestId")]
    pub request_id: String,
}

#[derive(Serialize)]
pub struct ErrorDetails {
    #[serde(rename = "Code")]
    pub code: String,
    #[serde(rename = "Message")]
    pub message: String,
}

#[derive(Serialize)]
#[serde(rename = "GetMetricStatisticsResponse")]
pub struct GetMetricStatisticsResponse {
    #[serde(rename = "@xmlns")]
    pub xmlns: String,
    #[serde(rename = "GetMetricStatisticsResult")]
    pub result: GetMetricStatisticsResult,
    #[serde(rename = "ResponseMetadata")]
    pub metadata: ResponseMetadata,
}

#[derive(Serialize)]
pub struct GetMetricStatisticsResult {
    #[serde(rename = "Datapoints")]
    pub datapoints: Datapoints,
    #[serde(rename = "Label")]
    pub label: String,
}

#[derive(Serialize)]
pub struct Datapoints {
    #[serde(rename = "member")]
    pub members: Vec<Datapoint>,
}

#[derive(Serialize)]
pub struct Datapoint {
    #[serde(rename = "Timestamp")]
    pub timestamp: String,
    #[serde(rename = "Average")]
    pub average: f64,
    #[serde(rename = "Unit")]
    pub unit: String,
}

#[derive(Serialize)]
#[serde(rename = "GetMetricDataResponse")]
pub struct GetMetricDataResponse {
    #[serde(rename = "@xmlns")]
    pub xmlns: String,
    #[serde(rename = "GetMetricDataResult")]
    pub result: GetMetricDataResult,
    #[serde(rename = "ResponseMetadata")]
    pub metadata: ResponseMetadata,
}

#[derive(Serialize)]
pub struct GetMetricDataResult {
    #[serde(rename = "MetricDataResults")]
    pub results: MetricDataResults,
}

#[derive(Serialize)]
pub struct MetricDataResults {
    #[serde(rename = "member")]
    pub members: Vec<MetricDataResult>,
}

#[derive(Serialize)]
pub struct MetricDataResult {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "StatusCode")]
    pub status_code: String,
    #[serde(rename = "Values")]
    pub values: Values,
    #[serde(rename = "Timestamps")]
    pub timestamps: Timestamps,
}

#[derive(Serialize)]
pub struct Values {
    #[serde(rename = "member")]
    pub members: Vec<f64>,
}

#[derive(Serialize)]
pub struct Timestamps {
    #[serde(rename = "member")]
    pub members: Vec<String>,
}

#[derive(Serialize)]
pub struct ResponseMetadata {
    #[serde(rename = "RequestId")]
    pub request_id: String,
}

#[derive(Serialize)]
#[serde(rename = "ListMetricsResponse")]
pub struct ListMetricsResponse {
    #[serde(rename = "@xmlns")]
    pub xmlns: String,
    #[serde(rename = "ListMetricsResult")]
    pub result: ListMetricsResult,
    #[serde(rename = "ResponseMetadata")]
    pub metadata: ResponseMetadata,
}

#[derive(Serialize)]
pub struct ListMetricsResult {
    #[serde(rename = "Metrics")]
    pub metrics: Metrics,
    #[serde(rename = "NextToken", skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

#[derive(Serialize)]
pub struct Metrics {
    #[serde(rename = "member")]
    pub members: Vec<Metric>,
}

#[derive(Serialize)]
pub struct Metric {
    #[serde(rename = "Namespace")]
    pub namespace: String,
    #[serde(rename = "MetricName")]
    pub metric_name: String,
    #[serde(rename = "Dimensions")]
    pub dimensions: Dimensions,
}

#[derive(Serialize)]
pub struct Dimensions {
    #[serde(rename = "member")]
    pub members: Vec<Dimension>,
}

#[derive(Serialize)]
pub struct Dimension {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Value")]
    pub value: String,
}

pub fn to_xml<T: Serialize>(val: &T) -> Result<String> {
    Ok(to_string(val)?)
}
