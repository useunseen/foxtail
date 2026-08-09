use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{DateTime, Timelike, Utc};
use foxtail::{db, fixture, serve};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};
use tower::ServiceExt;

async fn test_pool(name: &str) -> SqlitePool {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("foxtail-{name}-{nonce}.db"));
    db::init(&format!("sqlite:{}", path.display()))
        .await
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn response_text(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn seed_metric_series(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO resources (id, resource_type, region, scenario, tags)
         VALUES ('i-api', 'ec2', 'us-east-1', 'Baseline', '{}')",
    )
    .execute(pool)
    .await
    .unwrap();

    for (offset, value) in [(-7200, 10.0), (-7100, 30.0), (-3600, 40.0)] {
        sqlx::query(
            "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
             VALUES ('i-api', 'AWS/EC2', 'CPUUtilization', ?, ?)",
        )
        .bind(offset)
        .bind(value)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn seed_fixture_resources(pool: &SqlitePool) {
    for index in 0..5 {
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES (?, 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .bind(format!("i-ec2-query-{index}"))
        .execute(pool)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn ec2_query_describe_instances_returns_exact_manifest_rows() {
    let pool = test_pool("ec2-query").await;
    seed_fixture_resources(&pool).await;
    let snapshot = fixture::realize(
        &pool,
        fixture::RealizeRequest {
            clock_anchor: Some("2026-08-05T00:00:00Z".to_string()),
            ..fixture::RealizeRequest::default()
        },
    )
    .await
    .unwrap();
    let manifest: Value = serde_json::from_slice(&snapshot.manifest_bytes).unwrap();
    let expected_ids = manifest["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| resource["resource_id"].as_str().unwrap().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    let app = serve::build_app(pool);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("Action=DescribeInstances&Version=2016-11-15"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    let mut reader = quick_xml::Reader::from_str(&body);
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(error) => panic!("EC2 XML response is malformed: {error}"),
        }
    }
    assert!(body.contains("<DescribeInstancesResponse"));
    assert!(body.contains("<instanceState><code>16</code><name>running</name>"));
    assert!(body.contains("<instanceType>m6i.large</instanceType>"));
    assert!(body.contains("<rootDeviceType>ebs</rootDeviceType>"));
    assert!(body.contains("<architecture>x86_64</architecture>"));
    assert!(body.contains("<virtualizationType>hvm</virtualizationType>"));
    assert!(body.contains("<enaSupport>true</enaSupport>"));
    assert!(body.contains("<availabilityZone>us-east-1a</availabilityZone>"));
    for resource_id in &expected_ids {
        assert!(body.contains(&format!("<instanceId>{resource_id}</instanceId>")));
        assert!(body.contains(&format!("<value>{resource_id}</value>")));
    }
    assert_eq!(body.matches("<instanceId>").count(), 5);
    assert_eq!(body.matches("<item><instanceId>").count(), 5);
}

#[tokio::test]
async fn ec2_oracle_observation_routes_bind_exact_fixture_facts_and_fail_closed() {
    let pool = test_pool("ec2-oracle-observations").await;
    seed_fixture_resources(&pool).await;
    sqlx::query("UPDATE resources SET disable_api_termination = 1 WHERE id = 'i-ec2-query-0'")
        .execute(&pool)
        .await
        .unwrap();
    let snapshot = fixture::realize(
        &pool,
        fixture::RealizeRequest {
            clock_anchor: Some("2026-08-05T00:00:00Z".to_string()),
            ..fixture::RealizeRequest::default()
        },
    )
    .await
    .unwrap();
    let manifest: Value = serde_json::from_slice(&snapshot.manifest_bytes).unwrap();
    let app = serve::build_app(pool.clone());
    let manifest_digest_before = snapshot.manifest_digest.clone();

    let attribute_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "Action=DescribeInstanceAttribute&Version=2016-11-15&InstanceId=i-ec2-query-0&Attribute=disableApiTermination",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(attribute_response.status(), StatusCode::OK);
    let attribute_body = response_text(attribute_response).await;
    assert!(attribute_body.contains("<instanceId>i-ec2-query-0</instanceId>"));
    assert!(attribute_body.contains("<disableApiTermination><value>true</value>"));

    let type_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "Action=DescribeInstanceTypes&Version=2016-11-15&InstanceType.1=t3.medium&InstanceType.2=m6i.xlarge",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(type_response.status(), StatusCode::OK);
    let type_body = response_text(type_response).await;
    for field in [
        "<instanceType>t3.medium</instanceType>",
        "<instanceType>m6i.xlarge</instanceType>",
        "<supportedRootDeviceTypes><item>ebs</item>",
        "<supportedVirtualizationTypes><item>hvm</item>",
        "<supportedArchitectures><item>x86_64</item>",
        "<enaSupport>required</enaSupport>",
    ] {
        assert!(type_body.contains(field), "missing {field}");
    }
    assert_eq!(type_body.matches("<instanceType>").count(), 2);

    for body in [
        "Action=DescribeInstanceAttribute&InstanceId=i-unknown&Attribute=disableApiTermination",
        "Action=DescribeInstanceAttribute&InstanceId=i-ec2-query-0&Attribute=instanceType",
        "Action=DescribeInstanceAttribute&InstanceId=i-ec2-query-0&InstanceId=i-ec2-query-1&Attribute=disableApiTermination",
        "Action=DescribeInstanceTypes&InstanceType.1=t3.unknown",
        "Action=DescribeInstanceTypes&InstanceType.1=t3.medium&InstanceType.2=t3.medium",
        "Action=DescribeInstanceTypes&InstanceType.1=t3.medium&NextToken=unsupported",
        "Action=DescribeInstanceTypes&InstanceTypes.1=m6i.large",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            !response.status().is_success(),
            "unexpected success for {body}"
        );
        let response_body = response_text(response).await;
        if body.contains("t3.unknown") {
            assert!(response_body.contains("InvalidInstanceType"));
            assert!(!response_body.contains("InvalidInstanceType.NotFound"));
        }
    }
    let state_after = fixture::read_state(&pool).await.unwrap();
    assert_eq!(
        state_after.manifest_digest.as_deref(),
        Some(manifest_digest_before.as_str())
    );

    let bound_resource = manifest["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|resource| resource["resource_id"] == "i-ec2-query-0")
        .unwrap();
    assert_eq!(
        bound_resource["observed"]["disable_api_termination"],
        json!(true)
    );
    assert_eq!(
        manifest["ec2_instance_type_catalogue"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn fixture_idle_positive_cloudwatch_peak_is_complete_and_deterministic() {
    let pool = test_pool("fixture-idle-positive-cloudwatch").await;
    seed_fixture_resources(&pool).await;
    fixture::realize(
        &pool,
        fixture::RealizeRequest {
            clock_anchor: Some("2026-08-05T00:00:00Z".to_string()),
            ..fixture::RealizeRequest::default()
        },
    )
    .await
    .unwrap();

    let now = Utc::now().with_nanosecond(0).unwrap();
    let body = json!({
        "Namespace": "AWS/EC2",
        "MetricName": "CPUUtilization",
        "Dimensions": [{"Name": "InstanceId", "Value": "i-ec2-query-0"}],
        "StartTime": (now - chrono::Duration::days(15)).to_rfc3339(),
        "EndTime": now.to_rfc3339(),
        "Period": 3600,
        "Statistics": ["Maximum"]
    });
    let response = serve::build_app(pool)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/x-amz-json-1.0")
                .header(
                    "x-amz-target",
                    "GraniteServiceVersion20100801.GetMetricStatistics",
                )
                .header("x-mock-now", now.to_rfc3339())
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    let datapoints = payload["Datapoints"].as_array().unwrap();
    assert_eq!(datapoints.len(), 14);
    assert!(
        datapoints
            .iter()
            .all(|datapoint| datapoint["Maximum"] == json!(4.0))
    );
}

#[tokio::test]
async fn tagging_inventory_excludes_fixture_mutation_resources() {
    let pool = test_pool("tagging-fixture-boundary").await;
    sqlx::query(
        "INSERT INTO resources (id, resource_type, region, scenario, tags)
         VALUES
         ('i-read-only', 'ec2', 'us-east-1', 'Baseline', '{\"Name\":\"read-only\"}'),
         ('i-mutation', 'ec2', 'us-east-1', 'QualificationMutation', '{\"Name\":\"mutation\"}')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO fixture_mutation_generations
         (mutation_generation, generation_id, fixture_generation, manifest_digest,
          complete_estate_fingerprint, state, resource_ids, created_at, endpoint_url,
          region, account_id, external_status)
         VALUES (1, 'mg-0001', 1, 'sha256:manifest', 'sha256:estate', 'ACTIVE',
                 '[\"i-mutation\"]', '2026-08-08T00:00:00Z', 'http://127.0.0.1:4666',
                 'us-east-1', '123456789012', 'ACTIVE')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO fixture_mutation_resources
         (resource_id, mutation_generation, generation_id, control_id, target_kind,
          setup_fault_kind, instance_state, instance_type, initial_state, initial_type,
          terminal_state, terminal_type, restored_state, restored_type, external_status,
          external_identity_verified, created_at)
         VALUES ('i-mutation', 1, 'mg-0001', 'ec2-mutation-stop-001', 'stop', 'stop',
                 'running', 'm6i.large', 'running', 'm6i.large', 'stopped', 'm6i.large',
                 'running', 'm6i.large', 'ACTIVE', 1, '2026-08-08T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let response = serve::build_app(pool)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/x-amz-json-1.1")
                .header(
                    "x-amz-target",
                    "ResourceGroupsTaggingAPI_20170126.GetResources",
                )
                .body(Body::from(
                    json!({"ResourceTypeFilters": ["ec2:instance"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body["ResourceTagMappingList"],
        json!([{
            "ResourceARN": "arn:aws:ec2:us-east-1:123456789012:instance/i-read-only",
            "Tags": [{"Key": "Name", "Value": "read-only"}]
        }])
    );
}

#[tokio::test]
async fn cloudwatch_metric_statistics_json_and_xml_emit_requested_stats() {
    let pool = test_pool("cloudwatch-statistics").await;
    seed_metric_series(&pool).await;
    let app = serve::build_app(pool);
    let start_time = DateTime::parse_from_rfc3339("2026-03-11T10:00:00Z")
        .unwrap()
        .timestamp();
    let end_time = DateTime::parse_from_rfc3339("2026-03-11T12:00:00Z")
        .unwrap()
        .timestamp();

    let json_body = json!({
        "Namespace": "AWS/EC2",
        "MetricName": "CPUUtilization",
        "Dimensions": [{
            "Name": "InstanceId",
            "Value": "i-api"
        }],
        "StartTime": start_time,
        "EndTime": end_time,
        "Period": 3600,
        "Statistics": ["Average", "Maximum"]
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/x-amz-json-1.0")
                .header(
                    "x-amz-target",
                    "GraniteServiceVersion20100801.GetMetricStatistics",
                )
                .header("x-mock-now", "2026-03-11T12:00:00Z")
                .body(Body::from(json_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["Datapoints"][0]["Average"], json!(20.0));
    assert_eq!(body["Datapoints"][0]["Maximum"], json!(30.0));
    assert!(body["Datapoints"][0].get("SampleCount").is_none());

    let xml_body = "Action=GetMetricStatistics\
        &Namespace=AWS%2FEC2\
        &MetricName=CPUUtilization\
        &Dimensions.member.1.Name=InstanceId\
        &Dimensions.member.1.Value=i-api\
        &StartTime=2026-03-11T10%3A00%3A00Z\
        &EndTime=2026-03-11T12%3A00%3A00Z\
        &Period=3600\
        &Statistics.member.1=Average\
        &Statistics.member.2=Maximum";
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("x-mock-now", "2026-03-11T12:00:00Z")
                .body(Body::from(xml_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    assert!(body.contains("<Average>20</Average>"));
    assert!(body.contains("<Maximum>30</Maximum>"));
    assert!(!body.contains("<SampleCount>"));
}

#[tokio::test]
async fn cost_explorer_usage_type_grouping_returns_usage_quantity() {
    let pool = test_pool("ce-usage-type").await;
    sqlx::query(
        "INSERT INTO resources (id, resource_type, region, scenario, tags)
         VALUES ('i-api', 'ec2', 'us-east-1', 'Baseline', '{}')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO cost_records (resource_id, seconds_from_now, amount)
         VALUES ('i-api', -86400, 19.20)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let app = serve::build_app(pool);
    let today = Utc::now().date_naive();
    let start = today - chrono::Duration::days(2);
    let end = today;
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
    let group = &body["ResultsByTime"][0]["Groups"][0];
    assert_eq!(group["Keys"][0], "USE1-BoxUsage:m6i.xlarge");
    assert_eq!(group["Metrics"]["UnblendedCost"]["Amount"], "19.20");
    assert_eq!(group["Metrics"]["UsageQuantity"]["Amount"], "200.0000");
}
