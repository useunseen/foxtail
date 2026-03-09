use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use sqlx::{Row, SqlitePool};
use tracing::debug;

#[derive(Debug, Default)]
pub struct MetricQueryParams {
    pub resource_id: Option<String>,
    pub metric_name: Option<String>,
    pub namespace: Option<String>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    pub injected_now: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct MetricPoint {
    pub value: f64,
    pub timestamp: DateTime<Utc>,
}

pub async fn query_metrics(
    pool: &SqlitePool,
    params: MetricQueryParams,
) -> Result<Vec<MetricPoint>> {
    let now = params.injected_now.unwrap_or_else(Utc::now);
    let start_offset = params
        .start_time
        .map(|t| (t - now).num_seconds())
        .unwrap_or(-86400 * 14);
    let end_offset = params
        .end_time
        .map(|t| (t - now).num_seconds())
        .unwrap_or(86400); // Allow slightly in future if needed
    let limit = params.limit.unwrap_or(10000);

    let mut query_str = String::from(
        "SELECT value, seconds_from_now FROM metrics WHERE seconds_from_now >= ? AND seconds_from_now <= ?",
    );

    if params.resource_id.is_some() {
        query_str.push_str(" AND resource_id = ?");
    }
    if params.metric_name.is_some() {
        query_str.push_str(" AND metric_name = ?");
    }
    if params.namespace.is_some() {
        query_str.push_str(" AND namespace = ?");
    }

    query_str.push_str(" ORDER BY seconds_from_now ASC LIMIT ?");

    debug!(
        "Querying metrics with: {} (params: {:?})",
        query_str, params
    );

    let mut query = sqlx::query(&query_str).bind(start_offset).bind(end_offset);

    if let Some(ref rid) = params.resource_id {
        query = query.bind(rid);
    }
    if let Some(ref mname) = params.metric_name {
        query = query.bind(mname);
    }
    if let Some(ref ns) = params.namespace {
        query = query.bind(ns);
    }

    query = query.bind(limit);

    let rows = query.fetch_all(pool).await?;

    let mut points = Vec::with_capacity(rows.len());
    for row in rows {
        let value: f64 = row.get(0);
        let seconds_from_now: i64 = row.get::<i32, _>(1) as i64;
        points.push(MetricPoint {
            value,
            timestamp: now + Duration::seconds(seconds_from_now),
        });
    }

    Ok(points)
}
