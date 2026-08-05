use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use foxtail::cli::{FixtureCommands, FixtureMutationAuthorityArgs};
use foxtail::fixture::{
    self, DEFAULT_ACCOUNT_ID, DEFAULT_REGION, FaultRequest, MutationAuthority, RealizeRequest,
};
use foxtail::mutation::{self, SetupFaultKind};
use serde_json::Value;
use sqlx::SqlitePool;
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
async fn failed_external_realize_is_durable_and_fail_closed() {
    fixture::with_isolated_qualification(async {
        let pool = seeded_pool().await;
        let error = fixture::realize(
            &pool,
            RealizeRequest {
                endpoint_url: Some("http://127.0.0.1:1".to_string()),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("mutation") || error.contains("EC2"));
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

        let absent: Value = serde_json::from_slice(
            &fixture::execute_fixture_cli_command(&pool, FixtureCommands::MutationStatus)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(absent["status"], "ABSENT");
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
