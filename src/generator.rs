use anyhow::Result;
use aws_config::BehaviorVersion;
use aws_sdk_cloudwatch::config::Credentials;
use aws_sdk_cloudwatch::config::Region;
use aws_sdk_ec2::Client as Ec2Client;
use aws_sdk_ec2::types::{InstanceStateName, InstanceType};
use aws_sdk_elasticache::Client as ElastiCacheClient;
use aws_sdk_elasticloadbalancingv2::Client as ElbClient;
use aws_sdk_rds::Client as RdsClient;
use aws_sdk_s3::Client as S3Client;
use aws_smithy_http_client::Builder as HttpClientBuilder;
use serde_json::json;
use sqlx::{Row, SqlitePool};
use tracing::info;

use crate::cli::Scenario;

fn stable_hash_u64(parts: &[&str]) -> u64 {
    // FNV-1a 64-bit hash for deterministic per-resource variability.
    let mut hash: u64 = 0xcbf29ce484222325;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[derive(Clone, Copy)]
struct MetricShape {
    baseline: f64,
    spike: f64,
    idle_heavy: f64,
    min_value: f64,
    max_value: f64,
    wave_ratio: f64,
    noise_ratio: f64,
    long_wave_ratio: f64,
}

fn metric_shape(namespace: &str, metric_name: &str) -> MetricShape {
    match (namespace, metric_name) {
        ("AWS/EC2", "CPUUtilization") => MetricShape {
            baseline: 18.0,
            spike: 82.0,
            idle_heavy: 4.0,
            min_value: 0.0,
            max_value: 100.0,
            wave_ratio: 0.28,
            noise_ratio: 0.04,
            long_wave_ratio: 0.45,
        },
        ("AWS/EC2", "NetworkIn") => MetricShape {
            baseline: 22_000_000.0,
            spike: 72_000_000.0,
            idle_heavy: 4_500_000.0,
            min_value: 100_000.0,
            max_value: 160_000_000.0,
            wave_ratio: 0.22,
            noise_ratio: 0.03,
            long_wave_ratio: 0.35,
        },
        ("AWS/EC2", "NetworkOut") => MetricShape {
            baseline: 18_000_000.0,
            spike: 66_000_000.0,
            idle_heavy: 3_800_000.0,
            min_value: 80_000.0,
            max_value: 150_000_000.0,
            wave_ratio: 0.24,
            noise_ratio: 0.03,
            long_wave_ratio: 0.38,
        },
        ("AWS/EC2", "DiskReadOps") => MetricShape {
            baseline: 170.0,
            spike: 520.0,
            idle_heavy: 20.0,
            min_value: 0.0,
            max_value: 2000.0,
            wave_ratio: 0.34,
            noise_ratio: 0.05,
            long_wave_ratio: 0.30,
        },
        ("AWS/EC2", "DiskWriteOps") => MetricShape {
            baseline: 160.0,
            spike: 480.0,
            idle_heavy: 18.0,
            min_value: 0.0,
            max_value: 2000.0,
            wave_ratio: 0.34,
            noise_ratio: 0.05,
            long_wave_ratio: 0.30,
        },
        ("AWS/EC2", "DiskReadBytes") => MetricShape {
            baseline: 12_000_000.0,
            spike: 38_000_000.0,
            idle_heavy: 1_400_000.0,
            min_value: 50_000.0,
            max_value: 95_000_000.0,
            wave_ratio: 0.27,
            noise_ratio: 0.03,
            long_wave_ratio: 0.34,
        },
        ("AWS/EC2", "DiskWriteBytes") => MetricShape {
            baseline: 10_000_000.0,
            spike: 34_000_000.0,
            idle_heavy: 1_200_000.0,
            min_value: 50_000.0,
            max_value: 85_000_000.0,
            wave_ratio: 0.27,
            noise_ratio: 0.03,
            long_wave_ratio: 0.34,
        },
        ("AWS/EC2", "StatusCheckFailed") => MetricShape {
            baseline: 0.0,
            spike: 0.0,
            idle_heavy: 0.0,
            min_value: 0.0,
            max_value: 1.0,
            wave_ratio: 0.0,
            noise_ratio: 0.0,
            long_wave_ratio: 0.0,
        },
        ("AWS/RDS", "CPUUtilization") => MetricShape {
            baseline: 20.0,
            spike: 76.0,
            idle_heavy: 8.0,
            min_value: 0.0,
            max_value: 100.0,
            wave_ratio: 0.25,
            noise_ratio: 0.04,
            long_wave_ratio: 0.40,
        },
        ("AWS/RDS", "DatabaseConnections") => MetricShape {
            baseline: 42.0,
            spike: 140.0,
            idle_heavy: 6.0,
            min_value: 0.0,
            max_value: 500.0,
            wave_ratio: 0.28,
            noise_ratio: 0.04,
            long_wave_ratio: 0.35,
        },
        ("AWS/RDS", "ReadIOPS") => MetricShape {
            baseline: 120.0,
            spike: 420.0,
            idle_heavy: 12.0,
            min_value: 0.0,
            max_value: 2500.0,
            wave_ratio: 0.33,
            noise_ratio: 0.05,
            long_wave_ratio: 0.32,
        },
        ("AWS/RDS", "WriteIOPS") => MetricShape {
            baseline: 110.0,
            spike: 390.0,
            idle_heavy: 10.0,
            min_value: 0.0,
            max_value: 2500.0,
            wave_ratio: 0.33,
            noise_ratio: 0.05,
            long_wave_ratio: 0.32,
        },
        ("AWS/RDS", "FreeableMemory") => MetricShape {
            baseline: 6_000_000_000.0,
            spike: 2_300_000_000.0,
            idle_heavy: 8_200_000_000.0,
            min_value: 500_000_000.0,
            max_value: 16_000_000_000.0,
            wave_ratio: 0.18,
            noise_ratio: 0.02,
            long_wave_ratio: 0.50,
        },
        ("AWS/ElastiCache", "CPUUtilization") => MetricShape {
            baseline: 16.0,
            spike: 70.0,
            idle_heavy: 5.0,
            min_value: 0.0,
            max_value: 100.0,
            wave_ratio: 0.24,
            noise_ratio: 0.04,
            long_wave_ratio: 0.36,
        },
        ("AWS/ElastiCache", "CurrConnections") => MetricShape {
            baseline: 85.0,
            spike: 420.0,
            idle_heavy: 9.0,
            min_value: 0.0,
            max_value: 2000.0,
            wave_ratio: 0.31,
            noise_ratio: 0.05,
            long_wave_ratio: 0.34,
        },
        ("AWS/S3", "BucketSizeBytes") => MetricShape {
            baseline: 240_000_000_000.0,
            spike: 340_000_000_000.0,
            idle_heavy: 220_000_000_000.0,
            min_value: 50_000_000_000.0,
            max_value: 1_500_000_000_000.0,
            wave_ratio: 0.08,
            noise_ratio: 0.01,
            long_wave_ratio: 0.65,
        },
        ("AWS/S3", "NumberOfObjects") => MetricShape {
            baseline: 4_200_000.0,
            spike: 6_800_000.0,
            idle_heavy: 3_600_000.0,
            min_value: 100_000.0,
            max_value: 30_000_000.0,
            wave_ratio: 0.10,
            noise_ratio: 0.01,
            long_wave_ratio: 0.60,
        },
        ("AWS/ApplicationELB", "RequestCount") => MetricShape {
            baseline: 560.0,
            spike: 2_200.0,
            idle_heavy: 90.0,
            min_value: 0.0,
            max_value: 8_000.0,
            wave_ratio: 0.30,
            noise_ratio: 0.06,
            long_wave_ratio: 0.35,
        },
        ("AWS/ApplicationELB", "TargetResponseTime") => MetricShape {
            baseline: 0.18,
            spike: 0.95,
            idle_heavy: 0.07,
            min_value: 0.01,
            max_value: 5.0,
            wave_ratio: 0.22,
            noise_ratio: 0.08,
            long_wave_ratio: 0.40,
        },
        ("AWS/ApplicationELB", "HTTPCode_Target_5XX_Count") => MetricShape {
            baseline: 2.0,
            spike: 28.0,
            idle_heavy: 0.4,
            min_value: 0.0,
            max_value: 250.0,
            wave_ratio: 0.60,
            noise_ratio: 0.12,
            long_wave_ratio: 0.25,
        },
        _ => MetricShape {
            baseline: 15.0,
            spike: 85.0,
            idle_heavy: 2.0,
            min_value: 0.0,
            max_value: 100.0,
            wave_ratio: 0.25,
            noise_ratio: 0.04,
            long_wave_ratio: 0.40,
        },
    }
}

fn metric_base_for_scenario(shape: MetricShape, scenario: Scenario) -> f64 {
    match scenario {
        Scenario::Baseline => shape.baseline,
        Scenario::Spike => shape.spike,
        Scenario::IdleHeavy => shape.idle_heavy,
    }
}

pub async fn run(
    pool: SqlitePool,
    endpoint_url: String,
    region: String,
    scenario: Scenario,
    prune: bool,
    json_output: bool,
) -> Result<()> {
    info!(
        "Starting resource discovery at {} (region: {}, scenario: {}, prune: {})",
        endpoint_url, region, scenario, prune
    );

    let mut config_loader = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(region.clone()))
        .endpoint_url(&endpoint_url)
        .credentials_provider(Credentials::new("test", "test", None, None, "static"));

    if endpoint_url.starts_with("http://") {
        config_loader = config_loader.http_client(HttpClientBuilder::new().build_http());
    }

    let config = config_loader.load().await;

    let ec2_client = Ec2Client::new(&config);
    let rds_client = RdsClient::new(&config);
    let s3_client = S3Client::new(&config);
    let elb_client = ElbClient::new(&config);
    let elasticache_client = ElastiCacheClient::new(&config);

    let mut discovered_ids = Vec::new();
    let mut stats = json!({
        "ec2": 0,
        "rds": 0,
        "s3": 0,
        "elb": 0,
        "elasticache": 0,
    });

    let mut tx = pool.begin().await?;

    // 1. Discover EC2 Instances
    info!("Discovering EC2 instances...");
    if let Ok(instances) = ec2_client.describe_instances().send().await {
        for reservation in instances.reservations() {
            for instance in reservation.instances() {
                let id = instance.instance_id().unwrap_or_default().to_string();
                if id.is_empty() {
                    continue;
                }

                discovered_ids.push(id.clone());
                stats["ec2"] = json!(stats["ec2"].as_i64().unwrap_or_default() + 1);

                let tags = instance.tags();
                let tags_json = json!(
                    tags.iter()
                        .map(|t| (t.key().unwrap_or(""), t.value().unwrap_or("")))
                        .collect::<std::collections::HashMap<_, _>>()
                )
                .to_string();
                let instance_state = instance
                    .state()
                    .and_then(|state| state.name())
                    .map(InstanceStateName::as_str)
                    .unwrap_or("running")
                    .to_string();
                let instance_type = instance
                    .instance_type()
                    .map(InstanceType::as_str)
                    .unwrap_or("m6i.large")
                    .to_string();
                let availability_zone = instance
                    .placement()
                    .and_then(|placement| placement.availability_zone())
                    .map(str::to_string);

                sqlx::query(
                    "INSERT INTO resources
                        (id, resource_type, region, scenario, tags,
                         instance_state, instance_type, availability_zone)
                     VALUES (?, 'ec2', ?, ?, ?, ?, ?, ?)
                     ON CONFLICT(id) DO UPDATE SET
                        resource_type='ec2', region=?, scenario=?, tags=?,
                        instance_state=?, instance_type=?, availability_zone=?",
                )
                .bind(&id)
                .bind(&region)
                .bind(scenario.to_string())
                .bind(&tags_json)
                .bind(&instance_state)
                .bind(&instance_type)
                .bind(&availability_zone)
                .bind(&region)
                .bind(scenario.to_string())
                .bind(&tags_json)
                .bind(&instance_state)
                .bind(&instance_type)
                .bind(&availability_zone)
                .execute(&mut *tx)
                .await?;

                regenerate_resource_data_tx(&mut tx, &id, "ec2", scenario).await?;
            }
        }
    }

    // 2. Discover RDS Instances
    info!("Discovering RDS instances...");
    if let Ok(dbs) = rds_client.describe_db_instances().send().await {
        for db in dbs.db_instances() {
            let id = db.db_instance_identifier().unwrap_or_default().to_string();
            if id.is_empty() {
                continue;
            }

            discovered_ids.push(id.clone());
            stats["rds"] = json!(stats["rds"].as_i64().unwrap_or_default() + 1);

            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario)
                 VALUES (?, 'rds', ?, ?)
                 ON CONFLICT(id) DO UPDATE SET resource_type='rds', region=?, scenario=?",
            )
            .bind(&id)
            .bind(&region)
            .bind(scenario.to_string())
            .bind(&region)
            .bind(scenario.to_string())
            .execute(&mut *tx)
            .await?;

            regenerate_resource_data_tx(&mut tx, &id, "rds", scenario).await?;
        }
    }

    // 3. Discover S3 Buckets
    info!("Discovering S3 buckets...");
    if let Ok(buckets) = s3_client.list_buckets().send().await {
        for bucket in buckets.buckets() {
            let id = bucket.name().unwrap_or_default().to_string();
            if id.is_empty() {
                continue;
            }

            discovered_ids.push(id.clone());
            stats["s3"] = json!(stats["s3"].as_i64().unwrap_or_default() + 1);

            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario)
                 VALUES (?, 's3', ?, ?)
                 ON CONFLICT(id) DO UPDATE SET resource_type='s3', region=?, scenario=?",
            )
            .bind(&id)
            .bind(&region)
            .bind(scenario.to_string())
            .bind(&region)
            .bind(scenario.to_string())
            .execute(&mut *tx)
            .await?;

            regenerate_resource_data_tx(&mut tx, &id, "s3", scenario).await?;
        }
    }

    // 4. Discover ALBs
    info!("Discovering Load Balancers...");
    if let Ok(elbs) = elb_client.describe_load_balancers().send().await {
        for lb in elbs.load_balancers() {
            let id = lb.load_balancer_name().unwrap_or_default().to_string();
            if id.is_empty() {
                continue;
            }

            discovered_ids.push(id.clone());
            stats["elb"] = json!(stats["elb"].as_i64().unwrap_or_default() + 1);

            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario)
                 VALUES (?, 'elb', ?, ?)
                 ON CONFLICT(id) DO UPDATE SET resource_type='elb', region=?, scenario=?",
            )
            .bind(&id)
            .bind(&region)
            .bind(scenario.to_string())
            .bind(&region)
            .bind(scenario.to_string())
            .execute(&mut *tx)
            .await?;

            regenerate_resource_data_tx(&mut tx, &id, "elb", scenario).await?;
        }
    }

    // 5. Discover ElastiCache clusters
    info!("Discovering ElastiCache clusters...");
    if let Ok(clusters) = elasticache_client.describe_cache_clusters().send().await {
        for cluster in clusters.cache_clusters() {
            let id = cluster.cache_cluster_id().unwrap_or_default().to_string();
            if id.is_empty() {
                continue;
            }

            discovered_ids.push(id.clone());
            stats["elasticache"] = json!(stats["elasticache"].as_i64().unwrap_or_default() + 1);

            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario)
                 VALUES (?, 'elasticache', ?, ?)
                 ON CONFLICT(id) DO UPDATE SET resource_type='elasticache', region=?, scenario=?",
            )
            .bind(&id)
            .bind(&region)
            .bind(scenario.to_string())
            .bind(&region)
            .bind(scenario.to_string())
            .execute(&mut *tx)
            .await?;

            regenerate_resource_data_tx(&mut tx, &id, "elasticache", scenario).await?;
        }
    }

    if prune {
        info!("Pruning resources no longer in LocalStack...");
        if !discovered_ids.is_empty() {
            let placeholders = discovered_ids
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let query_str = format!("DELETE FROM resources WHERE id NOT IN ({})", placeholders);
            let mut q = sqlx::query(&query_str);
            for id in &discovered_ids {
                q = q.bind(id);
            }
            q.execute(&mut *tx).await?;
        } else {
            sqlx::query("DELETE FROM resources")
                .execute(&mut *tx)
                .await?;
        }
    }

    tx.commit().await?;

    if json_output {
        println!(
            "{}",
            json!({
                "status": "success",
                "discovered_ids": discovered_ids,
                "stats": stats,
                "scenario": scenario
            })
        );
    }

    info!("Generation complete.");
    Ok(())
}

pub async fn apply_scenario(
    pool: &SqlitePool,
    scenario: Scenario,
    resource_id: Option<&str>,
) -> Result<u64> {
    let mut tx = pool.begin().await?;

    let resources = if let Some(id) = resource_id {
        sqlx::query("SELECT id, resource_type FROM resources WHERE id = ?")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?
    } else {
        sqlx::query("SELECT id, resource_type FROM resources")
            .fetch_all(&mut *tx)
            .await?
    };

    let updated_count = resources.len() as u64;

    for row in resources {
        let id: String = row.get(0);
        let resource_type: String = row.get(1);

        sqlx::query("UPDATE resources SET scenario = ? WHERE id = ?")
            .bind(scenario.to_string())
            .bind(&id)
            .execute(&mut *tx)
            .await?;

        regenerate_resource_data_tx(&mut tx, &id, &resource_type, scenario).await?;
    }

    tx.commit().await?;
    Ok(updated_count)
}

async fn regenerate_resource_data_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    resource_id: &str,
    resource_type: &str,
    scenario: Scenario,
) -> Result<()> {
    clear_resource_data_tx(tx, resource_id).await?;

    match resource_type {
        "ec2" => {
            for (namespace, metric_name) in [
                ("AWS/EC2", "CPUUtilization"),
                ("AWS/EC2", "NetworkIn"),
                ("AWS/EC2", "NetworkOut"),
                ("AWS/EC2", "DiskReadOps"),
                ("AWS/EC2", "DiskWriteOps"),
                ("AWS/EC2", "DiskReadBytes"),
                ("AWS/EC2", "DiskWriteBytes"),
                ("AWS/EC2", "StatusCheckFailed"),
            ] {
                generate_metric_series_tx(tx, resource_id, namespace, metric_name, scenario)
                    .await?;
            }
        }
        "rds" => {
            for (namespace, metric_name) in [
                ("AWS/RDS", "CPUUtilization"),
                ("AWS/RDS", "DatabaseConnections"),
                ("AWS/RDS", "ReadIOPS"),
                ("AWS/RDS", "WriteIOPS"),
                ("AWS/RDS", "FreeableMemory"),
            ] {
                generate_metric_series_tx(tx, resource_id, namespace, metric_name, scenario)
                    .await?;
            }
        }
        "elasticache" => {
            for (namespace, metric_name) in [
                ("AWS/ElastiCache", "CPUUtilization"),
                ("AWS/ElastiCache", "CurrConnections"),
            ] {
                generate_metric_series_tx(tx, resource_id, namespace, metric_name, scenario)
                    .await?;
            }
        }
        "s3" => {
            for (namespace, metric_name) in
                [("AWS/S3", "BucketSizeBytes"), ("AWS/S3", "NumberOfObjects")]
            {
                generate_metric_series_tx(tx, resource_id, namespace, metric_name, scenario)
                    .await?;
            }
        }
        "elb" => {
            for (namespace, metric_name) in [
                ("AWS/ApplicationELB", "RequestCount"),
                ("AWS/ApplicationELB", "TargetResponseTime"),
                ("AWS/ApplicationELB", "HTTPCode_Target_5XX_Count"),
            ] {
                generate_metric_series_tx(tx, resource_id, namespace, metric_name, scenario)
                    .await?;
            }
        }
        _ => {}
    }

    generate_cost_records_tx(tx, resource_id, scenario).await?;

    Ok(())
}

async fn clear_resource_data_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    resource_id: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM metrics WHERE resource_id = ?")
        .bind(resource_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM cost_records WHERE resource_id = ?")
        .bind(resource_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn generate_metric_series_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    resource_id: &str,
    namespace: &str,
    metric_name: &str,
    scenario: Scenario,
) -> Result<()> {
    // Generate 14 days of hourly metrics (336 points)
    let shape = metric_shape(namespace, metric_name);
    let base_value = metric_base_for_scenario(shape, scenario);
    let seed = stable_hash_u64(&[resource_id, namespace, metric_name, &scenario.to_string()]);
    let bias = base_value * (((seed % 161) as f64 / 1000.0) - 0.08);
    let seeded_amplitude_factor = 0.7 + ((seed >> 10) % 100) as f64 / 100.0;
    let amplitude = (base_value * shape.wave_ratio * seeded_amplitude_factor)
        .max((shape.max_value - shape.min_value) * 0.002);
    let phase = ((seed >> 18) % 360) as f64;
    let short_period_hours = 6.0 + ((seed >> 27) % 12) as f64;
    let long_period_hours = 36.0 + ((seed >> 36) % 84) as f64;
    let phase_radians = phase.to_radians();
    let status_stride = match scenario {
        Scenario::Spike => 24 + ((seed % 30) as i32),
        Scenario::Baseline => 140 + ((seed % 80) as i32),
        Scenario::IdleHeavy => 180 + ((seed % 100) as i32),
    };
    let status_phase = ((seed >> 17) % (status_stride as u64)) as i32;
    let status_secondary_stride = (status_stride / 3).max(8);
    let status_secondary_phase = ((seed >> 9) % (status_secondary_stride as u64)) as i32;

    for hour in 0..336 {
        let offset = -(hour * 3600);
        let hour_i = hour;
        let hour_f = hour_i as f64;

        let value = if metric_name == "StatusCheckFailed" {
            let primary_event = ((hour_i + status_phase) % status_stride) == 0;
            let secondary_event = matches!(scenario, Scenario::Spike)
                && ((hour_i + status_secondary_phase) % status_secondary_stride == 0);
            if primary_event || secondary_event {
                1.0
            } else {
                0.0
            }
        } else {
            let short_wave = amplitude * ((hour_f / short_period_hours) + phase_radians).sin();
            let long_wave = (amplitude * shape.long_wave_ratio)
                * ((hour_f / long_period_hours) + phase_radians * 0.5).cos();
            let noise_seed = ((seed % 23) + 3) as i32;
            let deterministic_noise =
                (((hour_i * noise_seed) % 13) as f64 - 6.0) * (base_value * shape.noise_ratio);
            (base_value + bias + short_wave + long_wave + deterministic_noise)
                .clamp(shape.min_value, shape.max_value)
        };

        sqlx::query(
            "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(resource_id)
        .bind(namespace)
        .bind(metric_name)
        .bind(offset)
        .bind(value)
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

async fn generate_cost_records_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    resource_id: &str,
    scenario: Scenario,
) -> Result<()> {
    // Generate daily cost records for 30 days
    let base_daily_cost = match scenario {
        Scenario::IdleHeavy => 10.0, // Expensive but idle
        Scenario::Spike => 5.0,
        Scenario::Baseline => 1.0,
    };
    let seed = stable_hash_u64(&[resource_id, "cost", &scenario.to_string()]);
    let resource_multiplier = 0.6 + ((seed % 170) as f64 / 100.0); // 0.60x .. 2.29x
    let weekly_amplitude = 0.08 + (((seed >> 11) % 18) as f64 / 100.0); // 8% .. 25%
    let trend_per_day = ((seed >> 20) % 11) as f64 / 1000.0 - 0.005; // -0.5% .. +0.5%
    let phase = ((seed >> 31) % 360) as f64;
    let phase_radians = phase.to_radians();

    for day in 0..30 {
        let offset = -(day * 86400);
        let day_f = day as f64;
        let trend_factor = (1.0 + trend_per_day * day_f).max(0.4);
        let weekly_factor = 1.0 + weekly_amplitude * ((day_f / 7.0) + phase_radians).sin();
        let deterministic_noise = (((day * (((seed % 17) + 5) as i32)) % 9) as f64 - 4.0) * 0.03;
        let amount = (base_daily_cost * resource_multiplier * trend_factor * weekly_factor)
            + deterministic_noise;

        sqlx::query(
            "INSERT INTO cost_records (resource_id, seconds_from_now, amount)
             VALUES (?, ?, ?)",
        )
        .bind(resource_id)
        .bind(offset)
        .bind(amount.max(0.05))
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}
