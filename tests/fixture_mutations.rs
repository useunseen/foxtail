use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use foxtail::cli::{FixtureCommands, FixtureMutationAuthorityArgs};
use foxtail::fixture::{
    self, DEFAULT_ACCOUNT_ID, DEFAULT_REGION, DestroyRequest, FaultRequest, MutationAuthority,
    RealizeRequest,
};
use foxtail::mutation::{self, SetupFaultKind};
use serde_json::Value;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::borrow::Cow;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use tower::ServiceExt;

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn seeded_pool() -> SqlitePool {
    let path =
        std::env::temp_dir().join(format!("foxtail-mutation-it-{}.db", uuid::Uuid::new_v4()));
    let pool = foxtail::db::init(&format!("sqlite:{}", path.display()))
        .await
        .unwrap();
    for index in 0..5 {
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES (?, 'ec2', 'us-east-1', 'Baseline', '{}')",
        )
        .bind(format!("i-it-source-{index}"))
        .execute(&pool)
        .await
        .unwrap();
    }
    pool
}

async fn target_state(pool: &SqlitePool, target_id: &str) -> (String, String) {
    sqlx::query_as::<_, (String, String)>(
        "SELECT instance_state, instance_type
         FROM fixture_mutation_resources WHERE resource_id = ?",
    )
    .bind(target_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn intent_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM fixture_mutation_intents")
        .fetch_one(pool)
        .await
        .unwrap()
}

fn authority_from_snapshot(snapshot: &fixture::FixtureSnapshot) -> MutationAuthority {
    let manifest: Value = serde_json::from_slice(&snapshot.manifest_bytes).unwrap();
    MutationAuthority {
        version: Some(fixture::FIXTURE_VERSION.to_string()),
        generation: Some(snapshot.generation),
        manifest_digest: Some(snapshot.manifest_digest.clone()),
        mutation_generation: manifest["mutation_generation"].as_i64(),
        mutation_generation_id: manifest["mutation_generation_id"]
            .as_str()
            .map(str::to_string),
    }
}

fn cli_authority(authority: &MutationAuthority) -> FixtureMutationAuthorityArgs {
    FixtureMutationAuthorityArgs {
        version: authority.version.clone().unwrap(),
        generation: authority.generation.unwrap(),
        manifest_digest: authority.manifest_digest.clone().unwrap(),
        mutation_generation: authority.mutation_generation.unwrap(),
        mutation_generation_id: authority.mutation_generation_id.clone().unwrap(),
    }
}

fn authority_from_state(state: &fixture::FixtureState) -> MutationAuthority {
    let manifest: Value = serde_json::from_slice(state.manifest_bytes.as_ref().unwrap()).unwrap();
    MutationAuthority {
        version: Some(fixture::FIXTURE_VERSION.to_string()),
        generation: state.generation,
        manifest_digest: state.manifest_digest.clone(),
        mutation_generation: manifest["mutation_generation"].as_i64(),
        mutation_generation_id: manifest["mutation_generation_id"]
            .as_str()
            .map(str::to_string),
    }
}

fn validate_emitted_schema(kind: &str, value: &Value) {
    let path = std::env::temp_dir().join(format!(
        "foxtail-mutation-schema-{}-{}.json",
        kind,
        uuid::Uuid::new_v4()
    ));
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    let argument = match kind {
        "status" => "--mutation-status",
        "receipt" => "--receipt",
        other => panic!("unknown schema kind {other}"),
    };
    let output = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/validate_release_fixture.py"
        ))
        .arg(argument)
        .arg(&path)
        .output()
        .unwrap();
    fs::remove_file(&path).unwrap();
    assert!(
        output.status.success(),
        "schema validation failed for {kind}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_schema_rejects(kind: &str, value: &Value) {
    let path = std::env::temp_dir().join(format!(
        "foxtail-mutation-schema-negative-{}-{}.json",
        kind,
        uuid::Uuid::new_v4()
    ));
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    let argument = if kind == "status" {
        "--mutation-status"
    } else {
        "--receipt"
    };
    let output = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/validate_release_fixture.py"
        ))
        .arg(argument)
        .arg(&path)
        .output()
        .unwrap();
    fs::remove_file(&path).unwrap();
    assert!(
        !output.status.success(),
        "schema unexpectedly accepted {kind}"
    );
}

#[tokio::test]
async fn ordinary_realize_keeps_mutation_controls_declared_only() {
    let pool = seeded_pool().await;
    let snapshot = fixture::realize(
        &pool,
        RealizeRequest {
            account_id: Some(DEFAULT_ACCOUNT_ID.to_string()),
            region: Some(DEFAULT_REGION.to_string()),
            endpoint_url: Some("mock://ordinary-read-only".to_string()),
            ..RealizeRequest::default()
        },
    )
    .await
    .unwrap();
    let manifest: Value = serde_json::from_slice(&snapshot.manifest_bytes).unwrap();
    assert_eq!(manifest["mutation_generation"], Value::Null);
    assert_eq!(manifest["mutation_generation_id"], Value::Null);
    assert_eq!(manifest["mutation_resources"], serde_json::json!([]));
    assert!(
        manifest["control_catalogue"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| {
                entry["role"] != "mutation" || entry["realization_status"] == "declared-only"
            })
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fixture_mutation_generations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    pool.close().await;
}

#[tokio::test]
async fn mock_backend_reconciles_all_four_scenarios_and_public_absence() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        let endpoint = format!("mock://integration-{}", uuid::Uuid::new_v4());
        let first = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some(endpoint),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap();
        assert!(
            fixture::realize(
                &pool,
                RealizeRequest {
                    endpoint_url: Some("mock://a-different-endpoint".to_string()),
                    clock_anchor: Some("2026-08-06T01:00:00Z".to_string()),
                    ..RealizeRequest::default()
                },
            )
            .await
            .is_err()
        );
        let manifest: Value = serde_json::from_slice(&first.manifest_bytes).unwrap();
        let targets = manifest["mutation_resources"].as_array().unwrap();
        assert_eq!(targets.len(), mutation::CATALOGUE.len());
        assert_eq!(
            targets
                .iter()
                .map(|target| target["target_kind"].as_str().unwrap())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );

        let authority = MutationAuthority {
            version: Some(fixture::FIXTURE_VERSION.to_string()),
            generation: Some(first.generation),
            manifest_digest: Some(first.manifest_digest.clone()),
            mutation_generation: manifest["mutation_generation"].as_i64(),
            mutation_generation_id: manifest["mutation_generation_id"]
                .as_str()
                .map(str::to_string),
        };
        let resize_restoration = targets
            .iter()
            .find(|target| target["target_kind"] == "resize-restoration")
            .unwrap();
        let receipt: Value = serde_json::from_slice(
            &fixture::apply_fault(
                &pool,
                FaultRequest {
                    authority: authority.clone(),
                    control_id: resize_restoration["control_id"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                    target_id: resize_restoration["resource_id"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                    scope: "target".to_string(),
                    fault_kind: "resize".to_string(),
                    application_time: None,
                },
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(receipt["status"], "APPLIED");
        fixture::reset_fault(
            &pool,
            fixture::ResetRequest {
                authority: authority.clone(),
                receipt_id: receipt["receipt_id"].as_str().unwrap().to_string(),
                reset_token: receipt["reset_token"].as_str().unwrap().to_string(),
            },
        )
        .await
        .unwrap();
        let destroy: Value = serde_json::from_slice(
            &fixture::destroy(&pool, fixture::DestroyRequest { authority })
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(destroy["public_inventory_absence"]["all_absent"], true);
        let external_termination = destroy["external_ec2_termination"].as_array().unwrap();
        assert_eq!(external_termination.len(), mutation::CATALOGUE.len());
        assert!(
            external_termination
                .iter()
                .all(|target| target["state"] == "not-found")
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM resources WHERE scenario = 'QualificationMutation'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        pool.close().await;
    })
    .await;
}

#[tokio::test]
async fn pre_dispatch_realize_failure_is_durable_and_fail_closed() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        let error = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some("mock://pre-dispatch".to_string()),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("before dispatch"));
        assert_eq!(fixture::read_state(&pool).await.unwrap().status, "ABSENT");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_mutation_generations WHERE state = 'ACTIVE'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_mutation_intents WHERE status = 'FAILED'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        pool.close().await;
    })
    .await;
}

#[tokio::test]
async fn external_termination_requires_terminal_state_or_not_found() {
    let endpoint = format!("mock://termination-evidence-{}", uuid::Uuid::new_v4());
    let backend =
        mutation::Ec2MutationBackend::connect(&endpoint, DEFAULT_REGION, DEFAULT_ACCOUNT_ID)
            .await
            .unwrap();
    let targets = backend
        .provision_generation(77, &mutation::generation_id(77))
        .await
        .unwrap();
    let running_id = targets
        .iter()
        .find(|(scenario, _)| scenario.target_kind == mutation::TargetKind::Stop)
        .map(|(_, observed)| observed.resource_id.clone())
        .unwrap();
    let stopped_id = targets
        .iter()
        .find(|(scenario, _)| scenario.target_kind == mutation::TargetKind::Resize)
        .map(|(_, observed)| observed.resource_id.clone())
        .unwrap();
    assert_eq!(backend.verify_destroyed(&running_id).await.unwrap(), None);
    assert_eq!(backend.verify_destroyed(&stopped_id).await.unwrap(), None);
    assert_eq!(
        backend
            .verify_destroyed("i-missing-termination-evidence")
            .await
            .unwrap(),
        Some(mutation::ExternalTerminationState::NotFound)
    );
    let describe_error = mutation::Ec2MutationBackend::connect(
        "mock://describe-error",
        DEFAULT_REGION,
        DEFAULT_ACCOUNT_ID,
    )
    .await
    .unwrap();
    assert!(
        describe_error
            .verify_destroyed("i-describe-error")
            .await
            .is_err()
    );
    backend
        .terminate_all(
            &targets
                .iter()
                .map(|(_, observed)| observed.resource_id.clone())
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn setup_failure_records_current_returned_id_and_proves_cleanup() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        let endpoint = "mock://fail-setup-2".to_string();
        let error = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some(endpoint.clone()),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("external_ec2_termination_proven"));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_mutation_intents WHERE status = 'FAILED'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fixture_mutation_generations",)
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        let backend =
            mutation::Ec2MutationBackend::connect(&endpoint, DEFAULT_REGION, DEFAULT_ACCOUNT_ID)
                .await
                .unwrap();
        for target_kind in mutation::TargetKind::ALL {
            assert_eq!(
                backend
                    .verify_destroyed(&mutation::resource_id_hint(1, target_kind))
                    .await
                    .unwrap(),
                Some(mutation::ExternalTerminationState::NotFound)
            );
        }
        pool.close().await;
    })
    .await;
}

#[tokio::test]
async fn cleanup_failure_quarantines_every_returned_id_as_ambiguous() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        let endpoint = "mock://fail-cleanup-setup-2".to_string();
        let error = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some(endpoint.clone()),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("cleanup ambiguous"));
        let resource_ids: String = sqlx::query_scalar(
            "SELECT resource_ids FROM fixture_mutation_generations WHERE external_status = 'AMBIGUOUS'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let resource_ids: Vec<String> = serde_json::from_str(&resource_ids).unwrap();
        assert_eq!(resource_ids.len(), 2);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_mutation_intents WHERE status = 'AMBIGUOUS'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        let status: Value = serde_json::from_slice(&fixture::mutation_status(&pool).await.unwrap())
            .unwrap();
        assert_eq!(status["status"], "QUARANTINED");
        assert_eq!(
            status["resource_ids"].as_array().unwrap().len(),
            2
        );
        assert_eq!(status["intents"].as_array().unwrap().len(), 1);
        validate_emitted_schema("status", &status);
        let generation_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM fixture_mutation_generations",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let intent_count_before_retry = intent_count(&pool).await;
        let retry_error = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some(endpoint.clone()),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(retry_error.contains("globally blocked"));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_mutation_generations",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            generation_count
        );
        assert_eq!(intent_count(&pool).await, intent_count_before_retry);
        let backend = mutation::Ec2MutationBackend::connect(
            &endpoint,
            DEFAULT_REGION,
            DEFAULT_ACCOUNT_ID,
        )
        .await
        .unwrap();
        for resource_id in resource_ids {
            assert!(backend.describe_instance(&resource_id).await.is_ok());
        }
        pool.close().await;
    })
    .await;
}

#[tokio::test]
async fn orphan_intent_only_quarantine_status_is_schema_valid() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        sqlx::query(
            "INSERT INTO fixture_mutation_intents
             (intent_id, operation, request_bytes, status, error, created_at, updated_at)
             VALUES ('orphan-intent-001', 'realize', ?, 'AMBIGUOUS',
                     'orphaned mutation intent requires reconciliation',
                     '2026-08-06T00:00:00Z', '2026-08-06T00:00:00Z')",
        )
        .bind(b"{}".as_slice())
        .execute(&pool)
        .await
        .unwrap();

        let status: Value =
            serde_json::from_slice(&fixture::mutation_status(&pool).await.unwrap()).unwrap();
        assert_eq!(status["status"], "QUARANTINED");
        assert!(
            status["quarantined_generations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(status["intents"].as_array().unwrap().len(), 1);
        assert_eq!(status["resource_ids"].as_array().unwrap().len(), 0);
        validate_emitted_schema("status", &status);
        pool.close().await;
    })
    .await;
}

#[tokio::test]
async fn lost_or_empty_run_instances_identity_reconciles_without_duplicate_replay() {
    for (endpoint, generation) in [
        ("mock://lost-response-1", 901_i64),
        ("mock://empty-id-1", 902_i64),
    ] {
        let backend =
            mutation::Ec2MutationBackend::connect(endpoint, DEFAULT_REGION, DEFAULT_ACCOUNT_ID)
                .await
                .unwrap();
        let generation_id = mutation::generation_id(generation);
        let first = backend
            .provision_generation(generation, &generation_id)
            .await
            .unwrap();
        let replay = backend
            .provision_generation(generation, &generation_id)
            .await
            .unwrap();
        let first_ids = first
            .iter()
            .map(|(_, observed)| observed.resource_id.clone())
            .collect::<Vec<_>>();
        let replay_ids = replay
            .iter()
            .map(|(_, observed)| observed.resource_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(first_ids, replay_ids);
        assert_eq!(first_ids.len(), mutation::CATALOGUE.len());
        for resource_id in first_ids {
            assert!(backend.describe_instance(&resource_id).await.is_ok());
        }
    }
}

#[tokio::test]
async fn post_provision_database_failure_compensates_public_targets() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        sqlx::query(
            "CREATE TRIGGER inject_fixture_realization_failure
             BEFORE INSERT ON fixture_realizations
             BEGIN SELECT RAISE(ABORT, 'injected fixture finalization failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let endpoint = format!("mock://finalization-failure-{}", uuid::Uuid::new_v4());
        let error = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some(endpoint.clone()),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("persist fixture realization atomically"));
        sqlx::query("DROP TRIGGER inject_fixture_realization_failure")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_mutation_intents WHERE status = 'FAILED'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM resources WHERE scenario = 'QualificationMutation'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        let backend =
            mutation::Ec2MutationBackend::connect(&endpoint, DEFAULT_REGION, DEFAULT_ACCOUNT_ID)
                .await
                .unwrap();
        for target_kind in mutation::TargetKind::ALL {
            assert_eq!(
                backend
                    .verify_destroyed(&mutation::resource_id_hint(1, target_kind))
                    .await
                    .unwrap(),
                Some(mutation::ExternalTerminationState::NotFound)
            );
        }
        pool.close().await;
    })
    .await;
}

#[tokio::test]
async fn upgrade_from_pre_boundary_mutation_schema_quarantines_legacy_rows() {
    let path = std::env::temp_dir().join(format!(
        "foxtail-mutation-upgrade-{}.db",
        uuid::Uuid::new_v4()
    ));
    let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))
        .unwrap()
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .unwrap();
    let full =
        sqlx::migrate::Migrator::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"))
            .await
            .unwrap();
    let legacy_migrations = full
        .iter()
        .filter(|migration| migration.version <= 20260805120000)
        .cloned()
        .collect::<Vec<_>>();
    let legacy = sqlx::migrate::Migrator {
        migrations: Cow::Owned(legacy_migrations),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    legacy.run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO fixture_mutation_generations
         (mutation_generation, generation_id, fixture_generation, manifest_digest,
          complete_estate_fingerprint, state, resource_ids, public_absence, created_at)
         VALUES (1, 'mg-0001', 1, '', '', 'ACTIVE', ?, NULL, '2026-08-06T00:00:00Z')",
    )
    .bind(
        serde_json::json!([
            "i-legacy-stop",
            "i-legacy-resize",
            "i-legacy-recovery",
            "i-legacy-restoration"
        ])
        .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();
    for (
        resource_id,
        control_id,
        target_kind,
        initial_state,
        initial_type,
        terminal_state,
        terminal_type,
        restored_state,
        restored_type,
    ) in [
        (
            "i-legacy-stop",
            "ec2-mutation-stop-001",
            "stop",
            "running",
            "m6i.large",
            "stopped",
            "m6i.large",
            "running",
            "m6i.large",
        ),
        (
            "i-legacy-resize",
            "ec2-mutation-resize-001",
            "resize",
            "running",
            "m6i.large",
            "running",
            "m6i.medium",
            "running",
            "m6i.large",
        ),
        (
            "i-legacy-recovery",
            "ec2-mutation-recovery-001",
            "recovery",
            "stopped",
            "m6i.large",
            "running",
            "m6i.large",
            "stopped",
            "m6i.large",
        ),
        (
            "i-legacy-restoration",
            "ec2-mutation-restoration-001",
            "restoration",
            "stopped",
            "m6i.medium",
            "running",
            "m6i.large",
            "stopped",
            "m6i.medium",
        ),
    ] {
        sqlx::query(
            "INSERT INTO fixture_mutation_resources
             (resource_id, mutation_generation, generation_id, control_id, target_kind,
              instance_state, instance_type, initial_state, initial_type,
              terminal_state, terminal_type, restored_state, restored_type, created_at)
             VALUES (?, 1, 'mg-0001', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, '2026-08-06T00:00:00Z')",
        )
        .bind(resource_id)
        .bind(control_id)
        .bind(target_kind)
        .bind(initial_state)
        .bind(initial_type)
        .bind(initial_state)
        .bind(initial_type)
        .bind(terminal_state)
        .bind(terminal_type)
        .bind(restored_state)
        .bind(restored_type)
        .execute(&pool)
        .await
        .unwrap();
    }
    full.run(&pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT external_status FROM fixture_mutation_generations WHERE mutation_generation = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "AMBIGUOUS"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM fixture_mutation_intents
             WHERE intent_id = 'upgrade-quarantine-1' AND status = 'AMBIGUOUS'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM fixture_mutation_resources
             WHERE mutation_generation = 1 AND external_identity_verified = 0",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        4
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'idx_fixture_mutation_intents_one_inflight_generation'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    pool.close().await;
}

#[tokio::test]
async fn migration_quarantines_duplicate_nonterminal_intents_before_unique_index() {
    let path = std::env::temp_dir().join(format!(
        "foxtail-mutation-duplicate-intents-{}.db",
        uuid::Uuid::new_v4()
    ));
    let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))
        .unwrap()
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .unwrap();
    let full =
        sqlx::migrate::Migrator::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"))
            .await
            .unwrap();
    let intermediate_migrations = full
        .iter()
        .filter(|migration| migration.version <= 20260806100000)
        .cloned()
        .collect::<Vec<_>>();
    let intermediate = sqlx::migrate::Migrator {
        migrations: Cow::Owned(intermediate_migrations),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    intermediate.run(&pool).await.unwrap();
    for (intent_id, status, created_at) in [
        ("duplicate-old", "INTENT", "2026-08-06T00:00:00Z"),
        ("duplicate-new", "DISPATCHED", "2026-08-06T00:00:01Z"),
    ] {
        sqlx::query(
            "INSERT INTO fixture_mutation_intents
             (intent_id, operation, mutation_generation, generation_id, fixture_generation,
              request_bytes, status, created_at, updated_at)
             VALUES (?, 'fault', 7, 'mg-0007', 7, ?, ?, ?, ?)",
        )
        .bind(intent_id)
        .bind(b"{}".as_slice())
        .bind(status)
        .bind(created_at)
        .bind(created_at)
        .execute(&pool)
        .await
        .unwrap();
    }
    full.run(&pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM fixture_mutation_intents WHERE intent_id = 'duplicate-old'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "INTENT"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM fixture_mutation_intents WHERE intent_id = 'duplicate-new'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "AMBIGUOUS"
    );
    assert!(
        sqlx::query_scalar::<_, String>(
            "SELECT error FROM fixture_mutation_intents WHERE intent_id = 'duplicate-new'",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .contains("duplicate nonterminal")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'idx_fixture_mutation_intents_one_inflight_generation'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    pool.close().await;
}

#[test]
fn mutation_parsers_reject_malformed_and_unknown_fields() {
    assert!(fixture::parse_fault_request(b"not-json").is_err());
    assert!(fixture::parse_reset_request(b"{").is_err());
    assert!(fixture::parse_recreate_request(b"[]").is_err());
    assert!(fixture::parse_destroy_request(b"null").is_err());

    assert!(fixture::parse_fault_request(
        br#"{"control_id":"c","target_id":"t","scope":"target","fault_kind":"stop","unknown":true}"#,
    )
    .is_err());
    assert!(
        fixture::parse_reset_request(br#"{"receipt_id":"r","reset_token":"t","unknown":true}"#,)
            .is_err()
    );
    assert!(fixture::parse_recreate_request(br#"{"unknown":true}"#).is_err());
    assert!(fixture::parse_destroy_request(br#"{"unknown":true}"#).is_err());
}

#[tokio::test]
async fn authority_and_one_use_guards_leave_state_unchanged() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        let endpoint = format!("mock://guards-{}", uuid::Uuid::new_v4());
        let snapshot = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some(endpoint),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap();
        let authority = authority_from_snapshot(&snapshot);
        let manifest: Value = serde_json::from_slice(&snapshot.manifest_bytes).unwrap();
        let target = manifest["mutation_resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["target_kind"] == "stop")
            .unwrap();
        let target_id = target["resource_id"].as_str().unwrap().to_string();
        let control_id = target["control_id"].as_str().unwrap().to_string();
        let initial_state = target_state(&pool, &target_id).await;
        let initial_intents = intent_count(&pool).await;

        let mut stale_generation = authority.clone();
        stale_generation.generation = Some(authority.generation.unwrap() + 1);
        assert!(
            fixture::apply_fault(
                &pool,
                FaultRequest {
                    authority: stale_generation,
                    control_id: control_id.clone(),
                    target_id: target_id.clone(),
                    scope: "target".to_string(),
                    fault_kind: "stop".to_string(),
                    application_time: None,
                },
            )
            .await
            .is_err()
        );

        let mut wrong_manifest = authority.clone();
        wrong_manifest.manifest_digest = Some("sha256:wrong".to_string());
        assert!(
            fixture::apply_fault(
                &pool,
                FaultRequest {
                    authority: wrong_manifest,
                    control_id: control_id.clone(),
                    target_id: target_id.clone(),
                    scope: "target".to_string(),
                    fault_kind: "stop".to_string(),
                    application_time: None,
                },
            )
            .await
            .is_err()
        );

        for (wrong_control, wrong_target) in [
            ("wrong-control".to_string(), target_id.clone()),
            (control_id.clone(), "i-not-a-mutation-target".to_string()),
        ] {
            assert!(
                fixture::apply_fault(
                    &pool,
                    FaultRequest {
                        authority: authority.clone(),
                        control_id: wrong_control,
                        target_id: wrong_target,
                        scope: "target".to_string(),
                        fault_kind: "stop".to_string(),
                        application_time: None,
                    },
                )
                .await
                .is_err()
            );
        }
        assert_eq!(target_state(&pool, &target_id).await, initial_state);
        assert_eq!(intent_count(&pool).await, initial_intents);

        let fault_request = FaultRequest {
            authority: authority.clone(),
            control_id: control_id.clone(),
            target_id: target_id.clone(),
            scope: "target".to_string(),
            fault_kind: "stop".to_string(),
            application_time: Some("2026-08-06T00:00:00Z".to_string()),
        };
        let fault: Value = serde_json::from_slice(
            &fixture::apply_fault(&pool, fault_request.clone())
                .await
                .unwrap(),
        )
        .unwrap();
        let fault_state = target_state(&pool, &target_id).await;
        assert_ne!(fault_state, initial_state);
        let after_fault_intents = intent_count(&pool).await;
        assert!(fixture::apply_fault(&pool, fault_request).await.is_err());
        assert_eq!(target_state(&pool, &target_id).await, fault_state);
        assert_eq!(intent_count(&pool).await, after_fault_intents);

        let reset_request = fixture::ResetRequest {
            authority: authority.clone(),
            receipt_id: fault["receipt_id"].as_str().unwrap().to_string(),
            reset_token: fault["reset_token"].as_str().unwrap().to_string(),
        };
        fixture::reset_fault(&pool, reset_request.clone())
            .await
            .unwrap();
        let reset_state = target_state(&pool, &target_id).await;
        assert_eq!(reset_state, initial_state);
        let after_reset_intents = intent_count(&pool).await;
        assert!(fixture::reset_fault(&pool, reset_request).await.is_err());
        assert_eq!(target_state(&pool, &target_id).await, reset_state);
        assert_eq!(intent_count(&pool).await, after_reset_intents);
        pool.close().await;
    })
    .await;
}

#[tokio::test]
async fn external_failure_is_ambiguous_and_blocks_replay() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        let endpoint = format!("mock://ambiguous-{}", uuid::Uuid::new_v4());
        let snapshot = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some(endpoint),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap();
        let authority = authority_from_snapshot(&snapshot);
        let manifest: Value = serde_json::from_slice(&snapshot.manifest_bytes).unwrap();
        let target = manifest["mutation_resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["target_kind"] == "stop")
            .unwrap();
        let target_id = target["resource_id"].as_str().unwrap().to_string();
        let control_id = target["control_id"].as_str().unwrap().to_string();
        let before = target_state(&pool, &target_id).await;

        sqlx::query(
            "UPDATE fixture_mutation_generations
             SET endpoint_url = 'http://127.0.0.1:1'
             WHERE mutation_generation = ? AND generation_id = ?",
        )
        .bind(authority.mutation_generation.unwrap())
        .bind(authority.mutation_generation_id.as_deref().unwrap())
        .execute(&pool)
        .await
        .unwrap();

        let error = fixture::apply_fault(
            &pool,
            FaultRequest {
                authority: authority.clone(),
                control_id,
                target_id: target_id.clone(),
                scope: "target".to_string(),
                fault_kind: "stop".to_string(),
                application_time: None,
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(!error.is_empty());
        assert_eq!(target_state(&pool, &target_id).await, before);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_mutation_intents WHERE status = 'AMBIGUOUS'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        let intents_after_ambiguity = intent_count(&pool).await;
        assert!(
            fixture::apply_fault(
                &pool,
                FaultRequest {
                    authority,
                    control_id: target["control_id"].as_str().unwrap().to_string(),
                    target_id,
                    scope: "target".to_string(),
                    fault_kind: "stop".to_string(),
                    application_time: None,
                },
            )
            .await
            .is_err()
        );
        assert_eq!(intent_count(&pool).await, intents_after_ambiguity);
        pool.close().await;
    })
    .await;
}

#[tokio::test]
async fn concurrent_recreate_allows_one_winner_and_no_duplicate_active_generation() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        let endpoint = format!("mock://recreate-{}", uuid::Uuid::new_v4());
        let snapshot = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some(endpoint),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap();
        let authority = authority_from_snapshot(&snapshot);
        let request = fixture::RecreateRequest {
            authority: authority.clone(),
            clock_anchor: Some("2026-08-06T00:00:00Z".to_string()),
        };
        let (first, second) = tokio::join!(
            fixture::recreate(&pool, request.clone()),
            fixture::recreate(&pool, request),
        );
        let results = [first, second];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_mutation_generations WHERE state = 'ACTIVE'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_mutation_intents
                 WHERE operation = 'recreate' AND status IN ('INTENT', 'DISPATCHED')",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        let repeated = fixture::recreate(
            &pool,
            fixture::RecreateRequest {
                authority,
                clock_anchor: Some("2026-08-06T00:00:00Z".to_string()),
            },
        )
        .await;
        assert!(repeated.is_err());
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_mutation_generations WHERE state = 'ACTIVE'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        pool.close().await;
    })
    .await;
}

#[tokio::test]
async fn recreate_blocks_new_authority_until_recreated_receipt_commits() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        let endpoint = "mock://slow-cleanup-500".to_string();
        let snapshot = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some(endpoint.clone()),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap();
        let old_authority = authority_from_snapshot(&snapshot);
        let old_generation = old_authority.generation.unwrap();
        let recreate_pool = pool.clone();
        let recreate_task = tokio::spawn(async move {
            fixture::with_isolated_qualification(async move {
                fixture::recreate(
                    &recreate_pool,
                    fixture::RecreateRequest {
                        authority: old_authority,
                        clock_anchor: Some("2026-08-06T00:00:00Z".to_string()),
                    },
                )
                .await
            })
            .await
        });

        let mut replacement_authority = None;
        for _ in 0..200 {
            let state = fixture::read_state(&pool).await.unwrap();
            if state.generation.unwrap_or_default() > old_generation {
                replacement_authority = Some(authority_from_state(&state));
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let replacement_authority = match replacement_authority {
            Some(authority) => authority,
            None => {
                let recreate_result = recreate_task.await.unwrap();
                panic!("replacement generation published: {recreate_result:?}");
            }
        };
        let pending_status: Value =
            serde_json::from_slice(&fixture::mutation_status(&pool).await.unwrap()).unwrap();
        assert_eq!(pending_status["status"], "QUARANTINED");
        assert_eq!(
            pending_status["quarantined_generations"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        validate_emitted_schema("status", &pending_status);
        let replacement_manifest: Value = serde_json::from_slice(
            fixture::read_state(&pool)
                .await
                .unwrap()
                .manifest_bytes
                .as_ref()
                .unwrap(),
        )
        .unwrap();
        let target = replacement_manifest["mutation_resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["target_kind"] == "stop")
            .unwrap();
        let blocked_fault = fixture::apply_fault(
            &pool,
            FaultRequest {
                authority: replacement_authority.clone(),
                control_id: target["control_id"].as_str().unwrap().to_string(),
                target_id: target["resource_id"].as_str().unwrap().to_string(),
                scope: "target".to_string(),
                fault_kind: "stop".to_string(),
                application_time: None,
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(blocked_fault.contains("globally blocked"));
        assert!(
            fixture::destroy(
                &pool,
                DestroyRequest {
                    authority: replacement_authority,
                },
            )
            .await
            .is_err()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_operation_receipts WHERE operation IN ('fault', 'destroy')",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        let receipt = recreate_task.await.unwrap().unwrap();
        let receipt: Value = serde_json::from_slice(&receipt).unwrap();
        assert_eq!(receipt["status"], "RECREATED");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM fixture_mutation_intents
                 WHERE operation = 'recreate' AND status IN ('INTENT', 'DISPATCHED')",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        pool.close().await;
    })
    .await;
}

#[tokio::test]
async fn http_lifecycle_success_and_error_envelopes_match_domain_contract() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        let endpoint = format!("mock://http-{}", uuid::Uuid::new_v4());
        let snapshot = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some(endpoint),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap();
        let authority = authority_from_snapshot(&snapshot);
        let manifest: Value = serde_json::from_slice(&snapshot.manifest_bytes).unwrap();
        let target = manifest["mutation_resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["target_kind"] == "stop")
            .unwrap();
        let fault_request = FaultRequest {
            authority: authority.clone(),
            control_id: target["control_id"].as_str().unwrap().to_string(),
            target_id: target["resource_id"].as_str().unwrap().to_string(),
            scope: "target".to_string(),
            fault_kind: "stop".to_string(),
            application_time: Some("2026-08-06T00:00:00Z".to_string()),
        };
        let app = foxtail::serve::build_app(pool.clone());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_mock/fixture/fault")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&fault_request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let http_receipt = response_json(response).await;
        assert_eq!(http_receipt["operation"], "fault");
        assert_eq!(http_receipt["status"], "APPLIED");

        let malformed = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_mock/fixture/fault")
                    .header("content-type", "application/json")
                    .body(Body::from(br#"{"unknown":true}"#.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        let http_error = response_json(malformed).await;
        assert_eq!(http_error["error"], "fixture_request_failed");
        assert!(
            http_error["message"]
                .as_str()
                .unwrap()
                .contains("invalid fixture fault JSON")
        );
        assert!(fixture::parse_fault_request(br#"{"unknown":true}"#).is_err());

        let reset_request = fixture::ResetRequest {
            authority,
            receipt_id: http_receipt["receipt_id"].as_str().unwrap().to_string(),
            reset_token: http_receipt["reset_token"].as_str().unwrap().to_string(),
        };
        let domain_receipt = fixture::reset_fault(&pool, reset_request).await.unwrap();
        let domain_receipt: Value = serde_json::from_slice(&domain_receipt).unwrap();
        assert_eq!(domain_receipt["operation"], "reset");
        assert_eq!(domain_receipt["status"], "RESET");
        pool.close().await;
    })
    .await;
}

#[tokio::test]
async fn native_cli_dispatch_covers_mutation_lifecycle_and_stale_failure() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        let endpoint = format!("mock://cli-{}", uuid::Uuid::new_v4());
        let snapshot = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some(endpoint),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap();
        let authority = authority_from_snapshot(&snapshot);
        let manifest: Value = serde_json::from_slice(&snapshot.manifest_bytes).unwrap();
        let target = manifest["mutation_resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["target_kind"] == "stop")
            .unwrap();
        let target_id = target["resource_id"].as_str().unwrap().to_string();
        let control_id = target["control_id"].as_str().unwrap().to_string();

        let status: Value = serde_json::from_slice(
            &fixture::execute_fixture_cli_command(&pool, FixtureCommands::MutationStatus)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(status["status"], "ACTIVE");
        validate_emitted_schema("status", &status);
        let mut invalid_status = status.clone();
        invalid_status["unknown"] = true.into();
        assert_schema_rejects("status", &invalid_status);

        let before_error_intents = intent_count(&pool).await;
        let mut stale = cli_authority(&authority);
        stale.generation += 1;
        let stale_error = fixture::execute_fixture_cli_command(
            &pool,
            FixtureCommands::Fault {
                authority: stale,
                control_id: control_id.clone(),
                target_id: target_id.clone(),
                scope: "target".to_string(),
                fault_kind: "stop".to_string(),
                application_time: None,
            },
        )
        .await;
        assert!(stale_error.is_err());
        assert_eq!(intent_count(&pool).await, before_error_intents);
        assert_eq!(target_state(&pool, &target_id).await.0, "running");

        let fault: Value = serde_json::from_slice(
            &fixture::execute_fixture_cli_command(
                &pool,
                FixtureCommands::Fault {
                    authority: cli_authority(&authority),
                    control_id,
                    target_id,
                    scope: "target".to_string(),
                    fault_kind: "stop".to_string(),
                    application_time: Some("2026-08-06T00:00:00Z".to_string()),
                },
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(fault["schema"], "foxtail.release-fixture-fault-receipt/v1");
        assert_eq!(fault["status"], "APPLIED");
        validate_emitted_schema("receipt", &fault);
        let mut invalid_fault = fault.clone();
        invalid_fault["unknown"] = true.into();
        assert_schema_rejects("receipt", &invalid_fault);

        let reset: Value = serde_json::from_slice(
            &fixture::execute_fixture_cli_command(
                &pool,
                FixtureCommands::Reset {
                    authority: cli_authority(&authority),
                    receipt_id: fault["receipt_id"].as_str().unwrap().to_string(),
                    reset_token: fault["reset_token"].as_str().unwrap().to_string(),
                },
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(reset["schema"], "foxtail.release-fixture-reset-receipt/v1");
        assert_eq!(reset["status"], "RESET");
        validate_emitted_schema("receipt", &reset);

        let recreated: Value = serde_json::from_slice(
            &fixture::execute_fixture_cli_command(
                &pool,
                FixtureCommands::Recreate {
                    authority: cli_authority(&authority),
                    clock_anchor: Some("2026-08-06T00:00:00Z".to_string()),
                },
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            recreated["schema"],
            "foxtail.release-fixture-recreate-receipt/v1"
        );
        assert_eq!(recreated["status"], "RECREATED");
        assert_eq!(
            recreated["prior"]["external_ec2_termination"]
                .as_array()
                .unwrap()
                .len(),
            mutation::CATALOGUE.len()
        );
        validate_emitted_schema("receipt", &recreated);
        let mut missing_termination_proof = recreated.clone();
        missing_termination_proof["prior"]
            .as_object_mut()
            .unwrap()
            .remove("external_ec2_termination");
        assert_schema_rejects("receipt", &missing_termination_proof);

        let current_authority = authority_from_state(&fixture::read_state(&pool).await.unwrap());
        let destroyed: Value = serde_json::from_slice(
            &fixture::execute_fixture_cli_command(
                &pool,
                FixtureCommands::Destroy {
                    authority: cli_authority(&current_authority),
                },
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            destroyed["schema"],
            "foxtail.release-fixture-destroy-receipt/v1"
        );
        assert_eq!(destroyed["status"], "DESTROYED");
        assert_eq!(destroyed["public_inventory_absence"]["all_absent"], true);
        assert_eq!(
            destroyed["external_ec2_termination"]
                .as_array()
                .unwrap()
                .len(),
            mutation::CATALOGUE.len()
        );
        validate_emitted_schema("receipt", &destroyed);
        let mut duplicate_termination_evidence = destroyed.clone();
        let termination_evidence = duplicate_termination_evidence["external_ec2_termination"]
            .as_array()
            .unwrap()
            .clone();
        duplicate_termination_evidence["external_ec2_termination"] = serde_json::json!([
            termination_evidence[0].clone(),
            termination_evidence[0].clone(),
            termination_evidence[2].clone(),
            termination_evidence[3].clone()
        ]);
        assert_schema_rejects("receipt", &duplicate_termination_evidence);
        let mut contradictory_absence_count = destroyed.clone();
        contradictory_absence_count["public_inventory_absence"]["absent_count"] = 0.into();
        assert_schema_rejects("receipt", &contradictory_absence_count);

        let absent: Value = serde_json::from_slice(
            &fixture::execute_fixture_cli_command(&pool, FixtureCommands::MutationStatus)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(absent["status"], "ABSENT");
        validate_emitted_schema("status", &absent);
        pool.close().await;
    })
    .await;
}

#[test]
fn canonical_catalogue_keeps_setup_fault_separate_from_target_scenario() {
    let recovery = mutation::scenario_for_target_kind("stop-recovery").unwrap();
    assert_eq!(recovery.setup_fault_kind, SetupFaultKind::Stop);
    assert_ne!(
        recovery.target_kind.as_str(),
        recovery.setup_fault_kind.as_str()
    );
    let restoration = mutation::scenario_for_target_kind("resize-restoration").unwrap();
    assert_eq!(restoration.setup_fault_kind, SetupFaultKind::Resize);
    assert_ne!(
        restoration.target_kind.as_str(),
        restoration.setup_fault_kind.as_str()
    );
}
