//! The versioned release-qualification fixture contract.
//!
//! This module deliberately keeps fixture intent separate from the ordinary
//! AWS-compatible observation surface.  The definition is a stable declaration
//! of intent; a manifest is one canonical realization of that definition
//! against the currently discovered EC2 estate.

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::collections::{BTreeMap, BTreeSet};

pub const FIXTURE_VERSION: &str = "release-qualification-v1";
pub const DEFINITION_SCHEMA: &str = "foxtail.release-fixture-definition/v1";
pub const MANIFEST_SCHEMA: &str = "foxtail.release-fixture-manifest/v1";
pub const DEFINITION_REVISION: &str = "1.0.0";
pub const DEFAULT_ACCOUNT_ID: &str = "123456789012";
pub const DEFAULT_REGION: &str = "us-east-1";
pub const DEFAULT_LOCALSTACK_ENDPOINT: &str = "http://localhost:4566";
pub const CONTROL_IDS: [&str; 7] = [
    "ec2-idle-positive-001",
    "ec2-idle-negative-001",
    "ec2-idle-degraded-001",
    "ec2-resize-positive-001",
    "ec2-resize-negative-001",
    "ec2-mutation-stop-001",
    "ec2-mutation-resize-001",
];
pub const REALIZED_CONTROL_IDS: [&str; 5] = [
    "ec2-idle-positive-001",
    "ec2-idle-negative-001",
    "ec2-idle-degraded-001",
    "ec2-resize-positive-001",
    "ec2-resize-negative-001",
];
pub const MUTATION_CONTROL_IDS: [&str; 2] = ["ec2-mutation-stop-001", "ec2-mutation-resize-001"];

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub struct RealizeRequest {
    #[serde(alias = "Version")]
    pub version: Option<String>,
    #[serde(alias = "ClockAnchor")]
    pub clock_anchor: Option<String>,
    #[serde(alias = "AccountId")]
    pub account_id: Option<String>,
    #[serde(alias = "Region")]
    pub region: Option<String>,
    #[serde(alias = "EndpointUrl")]
    pub endpoint_url: Option<String>,
    #[serde(alias = "LocalStackVersion", alias = "LocalstackVersion")]
    pub localstack_version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FixtureSnapshot {
    pub definition_bytes: Vec<u8>,
    pub definition_digest: String,
    pub manifest_bytes: Vec<u8>,
    pub manifest_digest: String,
    pub status_bytes: Vec<u8>,
    pub identities_bytes: Vec<u8>,
    pub generation: i64,
}

#[derive(Debug, Clone)]
pub struct FixtureState {
    pub status: &'static str,
    pub definition_bytes: Vec<u8>,
    pub definition_digest: String,
    pub manifest_bytes: Option<Vec<u8>>,
    pub manifest_digest: Option<String>,
    pub status_bytes: Vec<u8>,
    pub identities_bytes: Vec<u8>,
    pub generation: Option<i64>,
}

/// Produce canonical UTF-8 JSON with recursively sorted object keys and no
/// insignificant whitespace.
pub fn canonical_bytes(value: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec(&sort_json(value)).context("serialize canonical JSON")
}

/// Return a `sha256:<hex>` digest of canonical JSON, excluding only the
/// document's own top-level `digest` field.
pub fn canonical_digest(value: &Value) -> Result<String> {
    let mut payload = value.clone();
    if let Some(object) = payload.as_object_mut() {
        object.remove("digest");
    }
    let bytes = canonical_bytes(&payload)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

/// Validate a persisted/document value without silently rewriting its digest.
pub fn validate_document(value: &Value, digest_field: &str) -> Result<(Vec<u8>, String)> {
    let actual = canonical_digest(value)?;
    if let Some(declared) = value.get(digest_field).and_then(Value::as_str)
        && declared != actual
    {
        bail!(
            "{} does not match canonical document digest (declared {}, computed {})",
            digest_field,
            declared,
            actual
        );
    }
    let bytes = canonical_bytes(value)?;
    Ok((bytes, actual))
}

pub fn definition_value() -> Value {
    let controls = vec![
        control_definition(
            "ec2-idle-positive-001",
            "positive",
            "ec2.idle.complete-history",
            "idle utilization with complete public history",
            json!({
                "cloudwatch": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization", "required_history_days": 14},
                "cost_explorer": {"metric": "UnblendedCost", "required_history_days": 14},
                "topology": "independently-observable"
            }),
        ),
        control_definition(
            "ec2-idle-negative-001",
            "negative",
            "ec2.busy.complete-history",
            "busy utilization is not an idle candidate",
            json!({
                "cloudwatch": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization", "required_history_days": 14},
                "cost_explorer": {"metric": "UnblendedCost", "required_history_days": 14},
                "topology": "independently-observable"
            }),
        ),
        control_definition(
            "ec2-idle-degraded-001",
            "degraded",
            "ec2.idle.scoped-missing-day",
            "idle utilization with a declared incomplete evidence window",
            json!({
                "cloudwatch": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization", "required_history_days": 14, "degradation": "scoped-missing-day"},
                "cost_explorer": {"metric": "UnblendedCost", "required_history_days": 14},
                "topology": "independently-observable"
            }),
        ),
        control_definition(
            "ec2-resize-positive-001",
            "positive",
            "ec2.resize.fresh-compatible-recommendation",
            "current instance identity has fresh resize evidence",
            json!({
                "cloudwatch": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization", "required_history_days": 14},
                "compute_optimizer": {"service": "ec2", "fresh": true, "identity_bound": true}
            }),
        ),
        control_definition(
            "ec2-resize-negative-001",
            "negative",
            "ec2.resize.no-compatible-recommendation",
            "current instance identity has no compatible resize action",
            json!({
                "cloudwatch": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization", "required_history_days": 14},
                "compute_optimizer": {"service": "ec2", "fresh": true, "identity_bound": true}
            }),
        ),
        control_definition(
            "ec2-mutation-stop-001",
            "mutation",
            "ec2.mutation.disposable-stop",
            "disposable stop control is declared for a later lifecycle lane",
            json!({
                "lifecycle": "deferred",
                "allowed_operation": "stop_instance",
                "initial_state": "running",
                "terminal_state": "stopped",
                "restored_state": "running"
            }),
        ),
        control_definition(
            "ec2-mutation-resize-001",
            "mutation",
            "ec2.mutation.disposable-resize",
            "disposable resize control is declared for a later lifecycle lane",
            json!({
                "lifecycle": "deferred",
                "allowed_operation": "resize_instance",
                "initial_type": "m6i.large",
                "terminal_type": "m6i.medium",
                "restored_type": "m6i.large"
            }),
        ),
    ];

    json!({
        "schema": DEFINITION_SCHEMA,
        "name": FIXTURE_VERSION,
        "revision": DEFINITION_REVISION,
        "namespace": "unseen:release-qualification:v1",
        "clock_contract": {
            "timezone": "UTC",
            "bucket": "complete-utc-day",
            "required_history_days": 14,
            "reuse_ttl_hours": 24
        },
        "generation_rules": {
            "resource_source": "LocalStack EC2 inventory",
            "resource_order": "stable resource id order with deterministic CPU-role assignment",
            "metric_surface": ["AWS/EC2/CPUUtilization", "AWS/EC2/NetworkIn", "AWS/EC2/NetworkOut"],
            "cost_surface": ["CostExplorer.UnblendedCost", "CostExplorer.UsageQuantity"],
            "recommendation_surface": ["ComputeOptimizer.GetEC2InstanceRecommendations"],
            "history_days": 14,
            "required_ec2_resources": 5
        },
        "control_ids": CONTROL_IDS,
        "controls": controls
    })
}

fn control_definition(
    control_id: &str,
    role: &str,
    scenario_intent: &str,
    realization_intent: &str,
    evidence: Value,
) -> Value {
    json!({
        "control_id": control_id,
        "role": role,
        "service": "ec2",
        "scenario_intent": scenario_intent,
        "realization_intent": realization_intent,
        "evidence": evidence
    })
}

pub fn canonical_definition() -> Result<(Vec<u8>, String)> {
    let (mut bytes, digest) = validate_document(&definition_value(), "digest")?;
    let mut value: Value = serde_json::from_slice(&bytes)?;
    value["digest"] = Value::String(digest.clone());
    bytes = canonical_bytes(&value)?;
    Ok((bytes, digest))
}

pub fn definition_with_digest() -> Result<Value> {
    let (bytes, _) = canonical_definition()?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn validate_version(version: Option<&str>) -> Result<()> {
    if let Some(version) = version
        && version != FIXTURE_VERSION
    {
        bail!("unsupported fixture version '{version}'")
    }
    Ok(())
}

pub async fn read_state(pool: &SqlitePool) -> Result<FixtureState> {
    let (definition_bytes, definition_digest) = canonical_definition()?;
    let row = sqlx::query(
        "SELECT definition_bytes, definition_digest, manifest_bytes, manifest_digest, generation
         FROM fixture_realizations WHERE singleton_id = 1",
    )
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        let status_bytes =
            canonical_status_bytes("ABSENT", &definition_digest, None, None, &[], None)?;
        let identities_bytes = canonical_identities_bytes("ABSENT", None, &[])?;
        return Ok(FixtureState {
            status: "ABSENT",
            definition_bytes,
            definition_digest,
            manifest_bytes: None,
            manifest_digest: None,
            status_bytes,
            identities_bytes,
            generation: None,
        });
    };

    let stored_definition: Vec<u8> = row.try_get("definition_bytes")?;
    let stored_definition_digest: String = row.try_get("definition_digest")?;
    let manifest_bytes: Vec<u8> = row.try_get("manifest_bytes")?;
    let manifest_digest: String = row.try_get("manifest_digest")?;
    let generation: i64 = row.try_get("generation")?;

    let definition_value: Value = serde_json::from_slice(&stored_definition)
        .context("persisted fixture definition is not JSON")?;
    let (canonical_definition_bytes, computed_definition_digest) =
        validate_document(&definition_value, "digest")?;
    if canonical_definition_bytes != stored_definition
        || computed_definition_digest != stored_definition_digest
        || stored_definition != definition_bytes
        || stored_definition_digest != definition_digest
    {
        bail!("persisted fixture definition bytes or digest are inconsistent")
    }

    let manifest_value: Value = serde_json::from_slice(&manifest_bytes)
        .context("persisted fixture manifest is not JSON")?;
    let (canonical_manifest_bytes, computed_manifest_digest) =
        validate_document(&manifest_value, "digest")?;
    if canonical_manifest_bytes != manifest_bytes || computed_manifest_digest != manifest_digest {
        bail!("persisted fixture manifest bytes or digest are inconsistent")
    }
    if manifest_value.get("schema").and_then(Value::as_str) != Some(MANIFEST_SCHEMA)
        || manifest_value
            .pointer("/definition/digest")
            .and_then(Value::as_str)
            != Some(stored_definition_digest.as_str())
        || manifest_value.get("generation").and_then(Value::as_i64) != Some(generation)
        || generation < 1
    {
        bail!("persisted fixture manifest is not bound to the active definition and generation")
    }
    let identities = identity_values(&manifest_value)?;
    let status_bytes = canonical_status_bytes(
        "REALIZED",
        &stored_definition_digest,
        Some(&manifest_digest),
        Some(generation),
        &identities,
        Some(&manifest_value),
    )?;
    let identities_bytes =
        canonical_identities_bytes("REALIZED", Some(&manifest_digest), &identities)?;

    Ok(FixtureState {
        status: "REALIZED",
        definition_bytes: stored_definition,
        definition_digest: stored_definition_digest,
        manifest_bytes: Some(manifest_bytes),
        manifest_digest: Some(manifest_digest),
        status_bytes,
        identities_bytes,
        generation: Some(generation),
    })
}

pub async fn realize(pool: &SqlitePool, request: RealizeRequest) -> Result<FixtureSnapshot> {
    validate_version(request.version.as_deref())?;
    let (definition_bytes, definition_digest) = canonical_definition()?;
    let anchor = parse_anchor(request.clock_anchor.as_deref())?;

    let rows = sqlx::query(
        "SELECT id, region, scenario,
                (SELECT AVG(m.value) FROM metrics m
                  WHERE m.resource_id = r.id
                    AND m.namespace = 'AWS/EC2'
                    AND m.metric_name = 'CPUUtilization') AS avg_cpu,
                (SELECT COUNT(*) FROM metrics m WHERE m.resource_id = r.id) AS metric_count,
                (SELECT COUNT(*) FROM cost_records c WHERE c.resource_id = r.id) AS cost_record_count
         FROM resources r
         WHERE r.resource_type = 'ec2'
         ORDER BY r.id ASC",
    )
    .fetch_all(pool)
    .await
    .context("read EC2 estate for fixture realization")?;

    if rows.len() < REALIZED_CONTROL_IDS.len() {
        bail!(
            "fixture realization requires at least {} EC2 resources; found {}",
            REALIZED_CONTROL_IDS.len(),
            rows.len()
        )
    }

    let mut resources = rows
        .into_iter()
        .map(|row| {
            Ok(EstateResource {
                id: row.try_get("id")?,
                region: row.try_get("region")?,
                scenario: row.try_get("scenario")?,
                avg_cpu: row.try_get("avg_cpu")?,
                metric_count: row.try_get("metric_count")?,
                cost_record_count: row.try_get("cost_record_count")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let estate_region = resources
        .iter()
        .map(|resource| resource.region.as_str())
        .collect::<BTreeSet<_>>();
    if estate_region.len() != 1 {
        bail!("fixture realization requires one AWS region; found multiple regions")
    }
    let discovered_region = estate_region
        .iter()
        .next()
        .copied()
        .unwrap_or(DEFAULT_REGION);
    let region = request
        .region
        .as_deref()
        .unwrap_or(discovered_region)
        .to_string();
    if region != discovered_region {
        bail!(
            "requested region '{}' does not match discovered EC2 region '{}'",
            region,
            discovered_region
        )
    }

    resources.sort_by(|left, right| {
        left.avg_cpu
            .partial_cmp(&right.avg_cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
    });

    let assigned = assign_realized_resources(&resources);
    let account_id = request
        .account_id
        .unwrap_or_else(|| DEFAULT_ACCOUNT_ID.to_string());
    if account_id.trim().is_empty() {
        bail!("account_id must not be empty")
    }
    let endpoint_url = request
        .endpoint_url
        .or_else(|| std::env::var("AWS_ENDPOINT_URL").ok())
        .unwrap_or_else(|| DEFAULT_LOCALSTACK_ENDPOINT.to_string());
    let localstack_version = request
        .localstack_version
        .or_else(|| std::env::var("LOCALSTACK_VERSION").ok())
        .unwrap_or_else(|| "unknown".to_string());
    let source_revision =
        std::env::var("FOXTAIL_SOURCE_REVISION").unwrap_or_else(|_| "unknown".to_string());

    let generation = sqlx::query_scalar::<_, i64>(
        "SELECT generation FROM fixture_realizations WHERE singleton_id = 1",
    )
    .fetch_optional(pool)
    .await?
    .unwrap_or(0)
        + 1;

    let read_only_fingerprint = estate_fingerprint(&assigned, &region, &account_id, false)?;
    let complete_map = resources
        .iter()
        .map(|resource| (resource.id.clone(), resource.clone()))
        .collect::<BTreeMap<_, _>>();
    let complete_fingerprint = estate_fingerprint(&complete_map, &region, &account_id, true)?;

    let manifest_without_digest = build_manifest(ManifestContext {
        definition_digest: &definition_digest,
        assigned: &assigned,
        complete_resources: &resources,
        region: &region,
        account_id: &account_id,
        endpoint_url: &endpoint_url,
        localstack_version: &localstack_version,
        source_revision: &source_revision,
        anchor,
        generation,
        read_only_fingerprint: &read_only_fingerprint,
        complete_fingerprint: &complete_fingerprint,
    })?;
    let (manifest_bytes, manifest_digest) = with_digest(&manifest_without_digest)?;

    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO fixture_realizations
           (singleton_id, definition_bytes, definition_digest, manifest_bytes, manifest_digest,
            generation, created_at, updated_at)
         VALUES (1, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(singleton_id) DO UPDATE SET
           definition_bytes = excluded.definition_bytes,
           definition_digest = excluded.definition_digest,
           manifest_bytes = excluded.manifest_bytes,
           manifest_digest = excluded.manifest_digest,
           generation = excluded.generation,
           updated_at = excluded.updated_at",
    )
    .bind(&definition_bytes)
    .bind(&definition_digest)
    .bind(&manifest_bytes)
    .bind(&manifest_digest)
    .bind(generation)
    .bind(anchor.to_rfc3339_opts(SecondsFormat::Secs, true))
    .bind(anchor.to_rfc3339_opts(SecondsFormat::Secs, true))
    .execute(&mut *tx)
    .await
    .context("persist fixture realization atomically")?;
    tx.commit().await?;

    let identities = identity_values(&manifest_without_digest)?;
    let status_bytes = canonical_status_bytes(
        "REALIZED",
        &definition_digest,
        Some(&manifest_digest),
        Some(generation),
        &identities,
        Some(&manifest_without_digest),
    )?;
    let identities_bytes =
        canonical_identities_bytes("REALIZED", Some(&manifest_digest), &identities)?;

    Ok(FixtureSnapshot {
        definition_bytes,
        definition_digest,
        manifest_bytes,
        manifest_digest,
        status_bytes,
        identities_bytes,
        generation,
    })
}

pub fn realization_response(snapshot: &FixtureSnapshot) -> Result<Vec<u8>> {
    let definition: Value = serde_json::from_slice(&snapshot.definition_bytes)?;
    let manifest: Value = serde_json::from_slice(&snapshot.manifest_bytes)?;
    let status: Value = serde_json::from_slice(&snapshot.status_bytes)?;
    let identities: Value = serde_json::from_slice(&snapshot.identities_bytes)?;
    let value = json!({
        "status": status,
        "definition": definition,
        "manifest": manifest,
        "identities": identities,
        "definition_digest": snapshot.definition_digest,
        "manifest_digest": snapshot.manifest_digest,
        "generation": snapshot.generation
    });
    canonical_bytes(&value)
}

fn parse_anchor(raw: Option<&str>) -> Result<DateTime<Utc>> {
    let anchor = match raw {
        Some(value) => DateTime::parse_from_rfc3339(value)
            .with_context(|| format!("invalid clock_anchor '{value}'"))?
            .with_timezone(&Utc),
        None => {
            let now = Utc::now();
            now - Duration::seconds(now.timestamp() % 3600)
        }
    };
    Ok(anchor)
}

fn with_digest(value: &Value) -> Result<(Vec<u8>, String)> {
    let digest = canonical_digest(value)?;
    let mut document = value.clone();
    document["digest"] = Value::String(digest.clone());
    Ok((canonical_bytes(&document)?, digest))
}

fn sort_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut sorted = Map::new();
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (key, child) in entries {
                sorted.insert(key.clone(), sort_json(child));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(sort_json).collect()),
        _ => value.clone(),
    }
}

#[derive(Debug, Clone)]
struct EstateResource {
    id: String,
    region: String,
    scenario: String,
    avg_cpu: Option<f64>,
    metric_count: i64,
    cost_record_count: i64,
}

struct ManifestContext<'a> {
    definition_digest: &'a str,
    assigned: &'a BTreeMap<String, EstateResource>,
    complete_resources: &'a [EstateResource],
    region: &'a str,
    account_id: &'a str,
    endpoint_url: &'a str,
    localstack_version: &'a str,
    source_revision: &'a str,
    anchor: DateTime<Utc>,
    generation: i64,
    read_only_fingerprint: &'a str,
    complete_fingerprint: &'a str,
}

fn assign_realized_resources(resources: &[EstateResource]) -> BTreeMap<String, EstateResource> {
    let indices = [
        0usize,
        resources.len() - 1,
        resources.len().saturating_sub(2),
        1,
        2,
    ];
    REALIZED_CONTROL_IDS
        .iter()
        .zip(indices)
        .map(|(control_id, index)| ((*control_id).to_string(), resources[index].clone()))
        .collect()
}

fn resource_arn(region: &str, account_id: &str, resource_id: &str) -> String {
    format!("arn:aws:ec2:{region}:{account_id}:instance/{resource_id}")
}

fn estate_fingerprint(
    resources: &BTreeMap<String, EstateResource>,
    region: &str,
    account_id: &str,
    complete: bool,
) -> Result<String> {
    let rows = resources
        .iter()
        .map(|(control_id, resource)| {
            json!({
                "control_id": control_id,
                "resource_id": resource.id,
                "resource_type": "ec2",
                "region": resource.region,
                "scenario": resource.scenario,
                "metric_count": resource.metric_count,
                "cost_record_count": resource.cost_record_count,
                "complete": complete
            })
        })
        .collect::<Vec<_>>();
    canonical_digest(&json!({
        "account_id": account_id,
        "region": region,
        "resources": rows
    }))
}

fn build_manifest(context: ManifestContext<'_>) -> Result<Value> {
    let ManifestContext {
        definition_digest,
        assigned,
        complete_resources,
        region,
        account_id,
        endpoint_url,
        localstack_version,
        source_revision,
        anchor,
        generation,
        read_only_fingerprint,
        complete_fingerprint,
    } = context;
    let resource_entries = assigned
        .iter()
        .map(|(control_id, resource)| {
            let (role, scenario_intent) = role_and_intent(control_id);
            json!({
                "control_id": control_id,
                "role": role,
                "resource_id": resource.id,
                "resource_type": "ec2",
                "aws_identity": resource_arn(region, account_id, &resource.id),
                "scenario": scenario_intent,
                "evidence": evidence_declaration(control_id, resource),
                "observed": {
                    "metric_count": resource.metric_count,
                    "cost_record_count": resource.cost_record_count,
                    "average_cpu": resource.avg_cpu
                }
            })
        })
        .collect::<Vec<_>>();

    let evidence_declarations = assigned
        .iter()
        .map(|(control_id, resource)| {
            json!({
                "control_id": control_id,
                "resource_id": resource.id,
                "surfaces": [
                    "ec2.describe-instances",
                    "cloudwatch.list-metrics",
                    "cloudwatch.get-metric-statistics",
                    "cost-explorer.get-cost-and-usage",
                    "compute-optimizer.get-ec2-instance-recommendations"
                ],
                "clock": {"anchor": anchor.to_rfc3339_opts(SecondsFormat::Secs, true), "required_history_days": 14},
                "metric": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization"},
                "cost": {"metric": "UnblendedCost"}
            })
        })
        .collect::<Vec<_>>();

    let mut control_catalogue = resource_entries.clone();
    for control_id in MUTATION_CONTROL_IDS {
        let (role, scenario_intent) = role_and_intent(control_id);
        control_catalogue.push(json!({
            "control_id": control_id,
            "role": role,
            "service": "ec2",
            "scenario": scenario_intent,
            "realization_status": "declared-only",
            "realization": {"lifecycle": "deferred"}
        }));
    }
    control_catalogue.sort_by(|left, right| {
        left.get("control_id")
            .and_then(Value::as_str)
            .cmp(&right.get("control_id").and_then(Value::as_str))
    });

    let reusable_until = anchor + Duration::hours(24);
    let complete_resource_count = complete_resources.len();
    Ok(json!({
        "schema": MANIFEST_SCHEMA,
        "definition": {
            "name": FIXTURE_VERSION,
            "revision": DEFINITION_REVISION,
            "digest": definition_digest
        },
        "generator": {
            "foxtail_version": env!("CARGO_PKG_VERSION"),
            "source_revision": source_revision,
            "fixture_contract": "release-qualification-v1"
        },
        "environment": {
            "account_id": account_id,
            "region": region,
            "aws_endpoint_url": endpoint_url,
            "localstack_version": localstack_version,
            "read_only_estate_fingerprint": read_only_fingerprint,
            "estate_fingerprint": complete_fingerprint,
            "complete_resource_count": complete_resource_count
        },
        "clock": {
            "anchor": anchor.to_rfc3339_opts(SecondsFormat::Secs, true),
            "bucket": "complete-utc-day",
            "required_history_days": 14,
            "reusable_until": reusable_until.to_rfc3339_opts(SecondsFormat::Secs, true)
        },
        "generation": generation,
        "resources": resource_entries,
        "evidence_declarations": evidence_declarations,
        "control_catalogue": control_catalogue,
        "fault_profiles": [],
        "mutation_generation": 0
    }))
}

fn role_and_intent(control_id: &str) -> (&'static str, &'static str) {
    match control_id {
        "ec2-idle-positive-001" => ("positive", "ec2.idle.complete-history"),
        "ec2-idle-negative-001" => ("negative", "ec2.busy.complete-history"),
        "ec2-idle-degraded-001" => ("degraded", "ec2.idle.scoped-missing-day"),
        "ec2-resize-positive-001" => ("positive", "ec2.resize.fresh-compatible-recommendation"),
        "ec2-resize-negative-001" => ("negative", "ec2.resize.no-compatible-recommendation"),
        "ec2-mutation-stop-001" => ("mutation", "ec2.mutation.disposable-stop"),
        "ec2-mutation-resize-001" => ("mutation", "ec2.mutation.disposable-resize"),
        _ => ("degraded", "ec2.unknown"),
    }
}

fn evidence_declaration(control_id: &str, resource: &EstateResource) -> Value {
    let mut evidence = json!({
        "cloudwatch_complete_days": 14,
        "cost_complete_days": 14,
        "topology": "independently-observable"
    });
    if control_id == "ec2-idle-degraded-001" {
        evidence["cloudwatch_complete_days"] = json!(13);
        evidence["degradation"] = json!("scoped-missing-day");
    }
    if control_id.starts_with("ec2-resize") {
        evidence["recommendation_bound_to_current_type"] = json!(true);
        evidence["recommendation_fresh"] = json!(true);
    }
    evidence["observed_metric_count"] = json!(resource.metric_count);
    evidence["observed_cost_record_count"] = json!(resource.cost_record_count);
    evidence
}

fn identity_values(manifest: &Value) -> Result<Vec<Value>> {
    manifest
        .get("resources")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("manifest resources must be an array"))
        .map(|resources| {
            resources
                .iter()
                .map(|resource| {
                    json!({
                        "control_id": resource.get("control_id").cloned().unwrap_or(Value::Null),
                        "role": resource.get("role").cloned().unwrap_or(Value::Null),
                        "resource_id": resource.get("resource_id").cloned().unwrap_or(Value::Null),
                        "aws_identity": resource.get("aws_identity").cloned().unwrap_or(Value::Null)
                    })
                })
                .collect()
        })
}

fn canonical_status_bytes(
    status: &str,
    definition_digest: &str,
    manifest_digest: Option<&str>,
    generation: Option<i64>,
    identities: &[Value],
    manifest: Option<&Value>,
) -> Result<Vec<u8>> {
    let mut value = json!({
        "schema": "foxtail.release-fixture-status/v1",
        "fixture": FIXTURE_VERSION,
        "status": status,
        "definition_digest": definition_digest,
        "manifest_digest": manifest_digest,
        "generation": generation,
        "control_ids": CONTROL_IDS,
        "realized_control_ids": REALIZED_CONTROL_IDS,
        "identities": identities
    });
    if let Some(manifest) = manifest {
        value["clock"] = manifest.get("clock").cloned().unwrap_or(Value::Null);
        value["environment"] = manifest.get("environment").cloned().unwrap_or(Value::Null);
    }
    canonical_bytes(&value)
}

fn canonical_identities_bytes(
    status: &str,
    manifest_digest: Option<&str>,
    identities: &[Value],
) -> Result<Vec<u8>> {
    canonical_bytes(&json!({
        "schema": "foxtail.release-fixture-identities/v1",
        "fixture": FIXTURE_VERSION,
        "status": status,
        "manifest_digest": manifest_digest,
        "identities": identities,
        "resource_ids": identities.iter().filter_map(|value| value.get("resource_id").and_then(Value::as_str)).collect::<Vec<_>>()
    }))
}

pub fn parse_json_request(body: &[u8]) -> Result<RealizeRequest> {
    if body.is_empty() {
        return Ok(RealizeRequest::default());
    }
    serde_json::from_slice(body).context("invalid fixture realization JSON")
}

pub fn cli_bytes_to_string(bytes: &[u8]) -> Result<String> {
    String::from_utf8(bytes.to_vec()).context("fixture document is not UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_sorts_keys_and_removes_whitespace() {
        let value = json!({"z": [json!({"b": 2, "a": 1})], "a": true});
        assert_eq!(
            canonical_bytes(&value).unwrap(),
            br#"{"a":true,"z":[{"a":1,"b":2}]}"#
        );
    }

    #[test]
    fn digest_excludes_only_own_digest_field() {
        let mut value = json!({"b": 2, "a": 1});
        let digest = canonical_digest(&value).unwrap();
        value["digest"] = json!(digest);
        assert_eq!(canonical_digest(&value).unwrap(), digest);
        value["other"] = json!(true);
        assert_ne!(canonical_digest(&value).unwrap(), digest);
    }

    #[test]
    fn definition_contains_all_roles_without_policy_maturity_or_findings() {
        let definition = definition_with_digest().unwrap();
        let controls = definition["controls"].as_array().unwrap();
        let roles = controls
            .iter()
            .filter_map(|control| control["role"].as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            roles,
            BTreeSet::from(["degraded", "mutation", "negative", "positive"])
        );
        let text = definition.to_string().to_ascii_lowercase();
        for forbidden in [
            "\"required\"",
            "\"stretch\"",
            "expected_finding",
            "expected_findings",
        ] {
            assert!(!text.contains(forbidden), "definition contains {forbidden}");
        }
    }

    #[test]
    fn definition_digest_is_sensitive_to_intent_clock_controls_and_generation_rules() {
        let definition = definition_with_digest().unwrap();
        let baseline = canonical_digest(&definition).unwrap();
        for (path, replacement) in [
            ("/controls/0/scenario_intent", json!("ec2.idle.changed")),
            ("/clock_contract/reuse_ttl_hours", json!(48)),
            ("/control_ids/0", json!("ec2-other-control")),
            ("/generation_rules/history_days", json!(30)),
        ] {
            let mut changed = definition.clone();
            set_pointer(&mut changed, path, replacement);
            assert_ne!(canonical_digest(&changed).unwrap(), baseline, "{path}");
        }
    }

    fn set_pointer(value: &mut Value, pointer: &str, replacement: Value) {
        let mut current = value;
        let parts = pointer
            .trim_start_matches('/')
            .split('/')
            .collect::<Vec<_>>();
        for part in &parts[..parts.len() - 1] {
            current = if let Ok(index) = part.parse::<usize>() {
                &mut current[index]
            } else {
                &mut current[*part]
            };
        }
        let last = parts[parts.len() - 1];
        if let Ok(index) = last.parse::<usize>() {
            current[index] = replacement;
        } else {
            current[last] = replacement;
        }
    }

    #[test]
    fn validation_rejects_mismatched_digest_without_rewriting() {
        let mut definition = definition_with_digest().unwrap();
        definition["digest"] = json!("sha256:wrong");
        let error = validate_document(&definition, "digest")
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not match canonical document digest"));
    }

    #[test]
    fn definition_matches_checked_in_canonical_golden() {
        let (bytes, _) = canonical_definition().unwrap();
        let golden = include_bytes!("../tests/fixtures/release-qualification-v1.definition.json");
        let golden = golden.strip_suffix(b"\n").unwrap_or(golden);
        assert_eq!(bytes, golden);
    }

    #[test]
    fn manifest_golden_has_a_self_consistent_canonical_digest() {
        let bytes = include_bytes!("../tests/fixtures/release-qualification-v1.manifest.json");
        let value: Value = serde_json::from_slice(bytes).unwrap();
        let (canonical, digest) = validate_document(&value, "digest").unwrap();
        assert_eq!(canonical, bytes.strip_suffix(b"\n").unwrap_or(bytes));
        assert_eq!(value["digest"], digest);
        assert_eq!(value["resources"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn checked_in_schemas_are_valid_json_and_pin_the_v1_contract() {
        let definition: Value = serde_json::from_slice(include_bytes!(
            "../schemas/release-fixture-definition-v1.schema.json"
        ))
        .unwrap();
        let manifest: Value = serde_json::from_slice(include_bytes!(
            "../schemas/release-fixture-manifest-v1.schema.json"
        ))
        .unwrap();
        assert_eq!(
            definition["properties"]["schema"]["const"],
            DEFINITION_SCHEMA
        );
        assert_eq!(manifest["properties"]["schema"]["const"], MANIFEST_SCHEMA);
        assert_eq!(definition["properties"]["name"]["const"], FIXTURE_VERSION);
        assert_eq!(
            manifest["properties"]["definition"]["properties"]["name"]["const"],
            FIXTURE_VERSION
        );
    }
}
