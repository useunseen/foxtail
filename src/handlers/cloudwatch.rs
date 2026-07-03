use anyhow::Result;
use quick_xml::se::to_string;
use serde::Serialize;
use serde_json::{Value, json};

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
    #[serde(rename = "SampleCount", skip_serializing_if = "Option::is_none")]
    pub sample_count: Option<f64>,
    #[serde(rename = "Average", skip_serializing_if = "Option::is_none")]
    pub average: Option<f64>,
    #[serde(rename = "Sum", skip_serializing_if = "Option::is_none")]
    pub sum: Option<f64>,
    #[serde(rename = "Minimum", skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(rename = "Maximum", skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
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
    #[serde(rename = "NextToken", skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
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

pub struct JsonDatapoint {
    pub timestamp: String,
    pub unit: String,
    pub sample_count: Option<f64>,
    pub average: Option<f64>,
    pub sum: Option<f64>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
}

pub struct MetricDataXmlSeries {
    pub id: String,
    pub values: Vec<f64>,
    pub timestamps: Vec<String>,
}

pub fn to_xml<T: Serialize>(val: &T) -> Result<String> {
    Ok(to_string(val)?)
}

pub fn list_metrics_xml(metrics: Vec<Metric>, next_token: Option<String>) -> Result<String> {
    let response = ListMetricsResponse {
        xmlns: "http://monitoring.amazonaws.com/doc/2010-08-01/".to_string(),
        result: ListMetricsResult {
            metrics: Metrics { members: metrics },
            next_token,
        },
        metadata: ResponseMetadata {
            request_id: "mock-id".to_string(),
        },
    };

    to_xml(&response)
}

pub fn get_metric_statistics_xml(label: String, datapoints: Vec<JsonDatapoint>) -> Result<String> {
    let response = GetMetricStatisticsResponse {
        xmlns: "http://monitoring.amazonaws.com/doc/2010-08-01/".to_string(),
        result: GetMetricStatisticsResult {
            datapoints: Datapoints {
                members: datapoints
                    .into_iter()
                    .map(|point| Datapoint {
                        timestamp: point.timestamp,
                        sample_count: point.sample_count,
                        average: point.average,
                        sum: point.sum,
                        minimum: point.minimum,
                        maximum: point.maximum,
                        unit: point.unit,
                    })
                    .collect(),
            },
            label,
        },
        metadata: ResponseMetadata {
            request_id: "mock-id".to_string(),
        },
    };

    to_xml(&response)
}

pub fn get_metric_data_xml(
    series: Vec<MetricDataXmlSeries>,
    next_token: Option<String>,
) -> Result<String> {
    let response = GetMetricDataResponse {
        xmlns: "http://monitoring.amazonaws.com/doc/2010-08-01/".to_string(),
        result: GetMetricDataResult {
            results: MetricDataResults {
                members: series
                    .into_iter()
                    .map(|series| MetricDataResult {
                        id: series.id,
                        status_code: "Complete".to_string(),
                        values: Values {
                            members: series.values,
                        },
                        timestamps: Timestamps {
                            members: series.timestamps,
                        },
                    })
                    .collect(),
            },
            next_token,
        },
        metadata: ResponseMetadata {
            request_id: "mock-id".to_string(),
        },
    };

    to_xml(&response)
}

pub fn list_metrics_json(metrics: Vec<Metric>, next_token: Option<String>) -> Value {
    let metrics = metrics
        .into_iter()
        .map(|metric| {
            json!({
                "Namespace": metric.namespace,
                "MetricName": metric.metric_name,
                "Dimensions": metric.dimensions.members.into_iter().map(|dimension| {
                    json!({
                        "Name": dimension.name,
                        "Value": dimension.value
                    })
                }).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();

    let mut response = json!({
        "Metrics": metrics
    });
    if let Some(next_token) = next_token {
        response["NextToken"] = json!(next_token);
    }
    response
}

pub fn get_metric_statistics_json(label: String, datapoints: Vec<JsonDatapoint>) -> Value {
    let datapoints = datapoints
        .into_iter()
        .map(|point| {
            let mut datapoint = json!({
                "Timestamp": point.timestamp,
                "Unit": point.unit
            });
            if let Some(sample_count) = point.sample_count {
                datapoint["SampleCount"] = json!(sample_count);
            }
            if let Some(average) = point.average {
                datapoint["Average"] = json!(average);
            }
            if let Some(sum) = point.sum {
                datapoint["Sum"] = json!(sum);
            }
            if let Some(minimum) = point.minimum {
                datapoint["Minimum"] = json!(minimum);
            }
            if let Some(maximum) = point.maximum {
                datapoint["Maximum"] = json!(maximum);
            }
            datapoint
        })
        .collect::<Vec<_>>();

    json!({
        "Label": label,
        "Datapoints": datapoints
    })
}
