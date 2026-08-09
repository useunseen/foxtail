use anyhow::{Context, Result};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::str::FromStr;
use tracing::info;

#[derive(Debug, Clone, PartialEq)]
pub struct Ec2ComputeOptimizerRow {
    pub resource_id: String,
    pub region: String,
    pub tags: Option<String>,
    pub average_cpu: f64,
    pub latest_metric_offset: i64,
}

pub async fn fetch_ec2_compute_optimizer_rows(
    pool: &SqlitePool,
) -> Result<Vec<Ec2ComputeOptimizerRow>> {
    let rows = sqlx::query(
        "SELECT r.id, r.region, r.tags, AVG(m.value) AS avg_cpu,
                MAX(m.seconds_from_now) AS latest_metric_offset
         FROM resources r
         LEFT JOIN metrics m
           ON m.resource_id = r.id
          AND m.namespace = 'AWS/EC2'
          AND m.metric_name = 'CPUUtilization'
         WHERE r.resource_type = 'ec2'
         GROUP BY r.id, r.region, r.tags
         ORDER BY r.id ASC",
    )
    .fetch_all(pool)
    .await
    .context("load EC2 Compute Optimizer evidence")?;

    rows.into_iter()
        .map(|row| {
            Ok(Ec2ComputeOptimizerRow {
                resource_id: row.try_get("id")?,
                region: row.try_get("region")?,
                tags: row.try_get("tags")?,
                average_cpu: row.try_get::<Option<f64>, _>("avg_cpu")?.unwrap_or(0.0),
                latest_metric_offset: row
                    .try_get::<Option<i64>, _>("latest_metric_offset")?
                    .unwrap_or(0),
            })
        })
        .collect()
}

pub async fn init(database_url: &str) -> Result<SqlitePool> {
    info!("Initializing database at {}", database_url);

    let connection_options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connection_options)
        .await?;

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}
