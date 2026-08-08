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
use std::future::Future;

use crate::mutation;

const HISTORY_DAYS: i64 = 14;
const DAY_SECONDS: i64 = 86_400;
const HOUR_SECONDS: i64 = 3_600;
const DEGRADED_MISSING_DAY: i64 = 6;
const NETWORK_IN_BASE: f64 = 10_000.0;
const NETWORK_OUT_BASE: f64 = 20_000.0;
const NETWORK_PER_DAY_INCREMENT: f64 = 100.0;
const LOW_CPU_MAX_EXCLUSIVE: f64 = 15.0;
const BUSY_CPU_MIN_EXCLUSIVE: f64 = 75.0;
const OPTIMIZED_CPU_MIN_INCLUSIVE: f64 = 15.0;
const OPTIMIZED_CPU_MAX_INCLUSIVE: f64 = 75.0;
const FORBIDDEN_POLICY_KEYS: [&str; 9] = [
    "Required",
    "Stretch",
    "required",
    "stretch",
    "expected_finding",
    "expected_findings",
    "authoritative_expected_finding",
    "authoritative_finding",
    "finding",
];

pub const FIXTURE_VERSION: &str = "release-qualification-v1";
pub const DEFINITION_SCHEMA: &str = "foxtail.release-fixture-definition/v1";
pub const MANIFEST_SCHEMA: &str = "foxtail.release-fixture-manifest/v1";
pub const DEFINITION_REVISION: &str = "1.0.0";
pub const DEFAULT_ACCOUNT_ID: &str = "123456789012";
pub const DEFAULT_REGION: &str = "us-east-1";
pub const DEFAULT_LOCALSTACK_ENDPOINT: &str = "http://localhost:4566";
/// Mutating fixture controls are deliberately opt-in. A caller must set this
/// to `isolated` before a mutation can affect fixture-owned rows.
pub const ISOLATED_QUALIFICATION_ENV: &str = "FOXTAIL_QUALIFICATION_ENV";
pub const ISOLATED_QUALIFICATION_VALUE: &str = "isolated";
pub const CONTROL_IDS: [&str; 9] = [
    "ec2-idle-positive-001",
    "ec2-idle-negative-001",
    "ec2-idle-degraded-001",
    "ec2-resize-positive-001",
    "ec2-resize-negative-001",
    "ec2-mutation-stop-001",
    "ec2-mutation-resize-001",
    "ec2-mutation-stop-recovery-001",
    "ec2-mutation-resize-restoration-001",
];
pub const REALIZED_CONTROL_IDS: [&str; 5] = [
    "ec2-idle-positive-001",
    "ec2-idle-negative-001",
    "ec2-idle-degraded-001",
    "ec2-resize-positive-001",
    "ec2-resize-negative-001",
];
pub const MUTATION_CONTROL_IDS: [&str; 4] = [
    "ec2-mutation-stop-001",
    "ec2-mutation-resize-001",
    "ec2-mutation-stop-recovery-001",
    "ec2-mutation-resize-restoration-001",
];

pub const MUTATION_TARGET_KINDS: [&str; 4] =
    ["stop", "resize", "stop-recovery", "resize-restoration"];

#[derive(Debug, Clone, Copy)]
struct MaterializationProfile {
    control_id: &'static str,
    cpu_value: f64,
    cost_amount: f64,
    missing_cpu_day: Option<i64>,
}

const MATERIALIZATION_PROFILES: [MaterializationProfile; 5] = [
    MaterializationProfile {
        control_id: "ec2-idle-positive-001",
        cpu_value: 5.0,
        cost_amount: 1.0,
        missing_cpu_day: None,
    },
    MaterializationProfile {
        control_id: "ec2-idle-negative-001",
        cpu_value: 85.0,
        cost_amount: 1.1,
        missing_cpu_day: None,
    },
    MaterializationProfile {
        control_id: "ec2-idle-degraded-001",
        cpu_value: 7.0,
        cost_amount: 1.2,
        missing_cpu_day: Some(DEGRADED_MISSING_DAY),
    },
    MaterializationProfile {
        control_id: "ec2-resize-positive-001",
        cpu_value: 6.0,
        cost_amount: 1.3,
        missing_cpu_day: None,
    },
    MaterializationProfile {
        control_id: "ec2-resize-negative-001",
        cpu_value: 40.0,
        cost_amount: 1.4,
        missing_cpu_day: None,
    },
];

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
    /// Internal lifecycle control used by `recreate`; it is never accepted
    /// from serialized CLI/HTTP input.
    #[serde(skip)]
    pub force_new: bool,
    /// Internal reservation used by `recreate` while the replacement
    /// generation is being provisioned and retired.
    #[serde(skip)]
    pub reuse_intent_id: Option<String>,
    /// Keep the replacement-generation reservation nonterminal until the
    /// outer recreate receipt and old-generation retirement commit.
    #[serde(skip)]
    pub defer_intent_finalization: bool,
    /// Internal allow-list for the two recreate reservations. Other global
    /// quarantine/nonterminal blockers remain fail-closed.
    #[serde(skip)]
    pub allowed_intent_ids: Vec<String>,
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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub struct MutationAuthority {
    #[serde(alias = "Version")]
    pub version: Option<String>,
    #[serde(alias = "Generation")]
    pub generation: Option<i64>,
    #[serde(alias = "ManifestDigest")]
    pub manifest_digest: Option<String>,
    #[serde(alias = "MutationGeneration")]
    pub mutation_generation: Option<i64>,
    #[serde(alias = "MutationGenerationId")]
    pub mutation_generation_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub struct FaultRequest {
    #[serde(flatten)]
    pub authority: MutationAuthority,
    #[serde(alias = "ControlId")]
    pub control_id: String,
    #[serde(alias = "TargetId")]
    pub target_id: String,
    #[serde(alias = "Scope")]
    pub scope: String,
    #[serde(alias = "FaultKind", alias = "Kind")]
    pub fault_kind: String,
    #[serde(alias = "ApplicationTime")]
    pub application_time: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub struct ResetRequest {
    #[serde(flatten)]
    pub authority: MutationAuthority,
    #[serde(alias = "ReceiptId")]
    pub receipt_id: String,
    #[serde(alias = "ResetToken")]
    pub reset_token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub struct RecreateRequest {
    #[serde(flatten)]
    pub authority: MutationAuthority,
    #[serde(alias = "ClockAnchor")]
    pub clock_anchor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub struct DestroyRequest {
    #[serde(flatten)]
    pub authority: MutationAuthority,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct MutationSnapshot {
    pub status: String,
    pub fixture_generation: Option<i64>,
    pub mutation_generation: Option<i64>,
    pub mutation_generation_id: Option<String>,
    pub manifest_digest: Option<String>,
    pub targets: Vec<Value>,
    pub active_faults: Vec<Value>,
}

async fn ensure_no_global_mutation_blockers(
    pool: &SqlitePool,
    allowed_intent_ids: &[String],
) -> Result<()> {
    let quarantined_generations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fixture_mutation_generations
         WHERE state = 'ACTIVE' AND external_status <> 'ACTIVE'",
    )
    .fetch_one(pool)
    .await?;
    let intent_rows = sqlx::query(
        "SELECT intent_id FROM fixture_mutation_intents
         WHERE status IN ('INTENT', 'DISPATCHED', 'AMBIGUOUS')",
    )
    .fetch_all(pool)
    .await?;
    let blocked_intents = intent_rows
        .iter()
        .filter_map(|row| row.try_get::<String, _>("intent_id").ok())
        .filter(|intent_id| {
            !allowed_intent_ids
                .iter()
                .any(|allowed| allowed == intent_id)
        })
        .count();
    if quarantined_generations != 0 || blocked_intents != 0 {
        bail!(
            "mutation authority is globally blocked by quarantined or nonterminal external state"
        );
    }
    Ok(())
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
    validate_policy_fields(value, "$")?;
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

fn validate_policy_fields(value: &Value, path: &str) -> Result<()> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = format!("{path}.{key}");
                if FORBIDDEN_POLICY_KEYS.contains(&key.as_str()) {
                    bail!("forbidden policy field at {child_path}");
                }
                validate_policy_fields(child, &child_path)?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                validate_policy_fields(child, &format!("{path}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn definition_value() -> Value {
    let controls = vec![
        control_definition(
            "ec2-idle-positive-001",
            "positive",
            "ec2.idle.complete-history",
            "idle utilization with complete public history",
            json!({
                "cloudwatch": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization", "required_history_days": HISTORY_DAYS},
                "cost_explorer": {"metric": "UnblendedCost", "required_history_days": HISTORY_DAYS},
                "topology": "independently-observable"
            }),
        ),
        control_definition(
            "ec2-idle-negative-001",
            "negative",
            "ec2.busy.complete-history",
            "busy utilization is not an idle candidate",
            json!({
                "cloudwatch": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization", "required_history_days": HISTORY_DAYS},
                "cost_explorer": {"metric": "UnblendedCost", "required_history_days": HISTORY_DAYS},
                "topology": "independently-observable"
            }),
        ),
        control_definition(
            "ec2-idle-degraded-001",
            "degraded",
            "ec2.idle.scoped-missing-day",
            "idle utilization with a declared incomplete evidence window",
            json!({
                "cloudwatch": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization", "required_history_days": HISTORY_DAYS, "degradation": "scoped-missing-day"},
                "cost_explorer": {"metric": "UnblendedCost", "required_history_days": HISTORY_DAYS},
                "topology": "independently-observable"
            }),
        ),
        control_definition(
            "ec2-resize-positive-001",
            "positive",
            "ec2.resize.fresh-compatible-recommendation",
            "current instance identity has fresh resize evidence",
            json!({
                "cloudwatch": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization", "required_history_days": HISTORY_DAYS},
                "compute_optimizer": {"service": "ec2", "fresh": true, "identity_bound": true}
            }),
        ),
        control_definition(
            "ec2-resize-negative-001",
            "negative",
            "ec2.resize.no-compatible-recommendation",
            "current instance identity has no compatible resize action",
            json!({
                "cloudwatch": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization", "required_history_days": HISTORY_DAYS},
                "compute_optimizer": {"service": "ec2", "fresh": true, "identity_bound": true}
            }),
        ),
        control_definition(
            "ec2-mutation-stop-001",
            "mutation",
            "ec2.mutation.disposable-stop",
            "disposable stop target is provisioned per isolated mutation generation",
            json!({
                "lifecycle": "qualification-only",
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
            "disposable resize target is provisioned per isolated mutation generation",
            json!({
                "lifecycle": "qualification-only",
                "setup_fault_kind": "resize",
                "allowed_operation": "resize_instance",
                "initial_state": "stopped",
                "initial_type": "m6i.large",
                "terminal_state": "stopped",
                "terminal_type": "m6i.medium",
                "restored_state": "stopped",
                "restored_type": "m6i.large"
            }),
        ),
        control_definition(
            "ec2-mutation-stop-recovery-001",
            "mutation",
            "ec2.mutation.disposable-stop-recovery",
            "disposable stop-recovery target is provisioned per isolated mutation generation",
            json!({
                "lifecycle": "qualification-only",
                "setup_fault_kind": "stop",
                "allowed_operation": "recover_instance",
                "initial_state": "running",
                "terminal_state": "stopped",
                "restored_state": "running"
            }),
        ),
        control_definition(
            "ec2-mutation-resize-restoration-001",
            "mutation",
            "ec2.mutation.disposable-resize-restoration",
            "disposable resize-restoration target is provisioned per isolated mutation generation",
            json!({
                "lifecycle": "qualification-only",
                "setup_fault_kind": "resize",
                "allowed_operation": "restore_instance",
                "initial_state": "stopped",
                "initial_type": "m6i.medium",
                "terminal_state": "stopped",
                "terminal_type": "m6i.large",
                "restored_state": "stopped",
                "restored_type": "m6i.medium"
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
            "required_history_days": HISTORY_DAYS,
            "reuse_ttl_hours": 24
        },
        "generation_rules": generation_rules_value(),
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

fn materialization_profile(control_id: &str) -> Result<&'static MaterializationProfile> {
    MATERIALIZATION_PROFILES
        .iter()
        .find(|profile| profile.control_id == control_id)
        .ok_or_else(|| anyhow!("unknown realized control '{control_id}'"))
}

fn generation_rules_value() -> Value {
    let control_order = MATERIALIZATION_PROFILES
        .iter()
        .map(|profile| profile.control_id)
        .collect::<Vec<_>>();
    json!({
        "resource_source": "LocalStack EC2 inventory",
        "resource_order": "ascending resource id order",
        "assignment": {
            "selection": "first five EC2 resources after ascending id sort",
            "control_order": control_order,
            "mapping": "one resource per control in control order"
        },
        "metric_surface": ["AWS/EC2/CPUUtilization", "AWS/EC2/NetworkIn", "AWS/EC2/NetworkOut"],
        "cost_surface": ["CostExplorer.UnblendedCost", "CostExplorer.UsageQuantity"],
        "recommendation_surface": ["ComputeOptimizer.GetEC2InstanceRecommendations"],
        "history_days": HISTORY_DAYS,
        "history": {
            "days": HISTORY_DAYS,
            "offset_formula": "-(day * 86400 + 3600)",
            "offset_seconds": history_offsets().into_iter().collect::<Vec<_>>()
        },
        "evidence_profiles": MATERIALIZATION_PROFILES.iter().map(|profile| json!({
            "control_id": profile.control_id,
            "cpu_value": profile.cpu_value,
            "cost_amount": profile.cost_amount,
            "missing_cpu_day": profile.missing_cpu_day
        })).collect::<Vec<_>>(),
        "network_profile": {
            "network_in_base": NETWORK_IN_BASE,
            "network_out_base": NETWORK_OUT_BASE,
            "per_day_increment": NETWORK_PER_DAY_INCREMENT
        },
        "cpu_predicates": {
            "low_max_exclusive": LOW_CPU_MAX_EXCLUSIVE,
            "busy_min_exclusive": BUSY_CPU_MIN_EXCLUSIVE,
            "optimized_min_inclusive": OPTIMIZED_CPU_MIN_INCLUSIVE,
            "optimized_max_inclusive": OPTIMIZED_CPU_MAX_INCLUSIVE
        },
        "required_ec2_resources": MATERIALIZATION_PROFILES.len()
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

/// Account identity used by both the fixture manifest and the public AWS-like
/// handlers.  A caller may repeat this scope explicitly, but cannot publish a
/// fixture under an identity the public surfaces will not return.
pub fn authoritative_account_id() -> &'static str {
    DEFAULT_ACCOUNT_ID
}

pub fn resolve_account_id(requested: Option<&str>) -> Result<String> {
    let configured = authoritative_account_id();
    match requested.map(str::trim) {
        None => Ok(configured.to_string()),
        Some("") => bail!("account_id must not be empty"),
        Some(value) if value != configured => bail!(
            "requested account_id '{}' does not match public AWS account '{}'",
            value,
            configured
        ),
        Some(value) => Ok(value.to_string()),
    }
}

pub fn validate_version(version: Option<&str>) -> Result<()> {
    if let Some(version) = version
        && version != FIXTURE_VERSION
    {
        bail!("unsupported fixture version '{version}'")
    }
    Ok(())
}

/// Mutating fixture controls are intentionally unavailable in an ordinary
/// development database.  The caller must opt into an isolated qualification
/// environment explicitly; HTTP callers still pass through the admin-token
/// guard in `serve.rs` as a separate authorization boundary.
pub fn ensure_isolated_qualification() -> Result<()> {
    if !is_isolated_qualification() {
        bail!("fixture mutation requires {ISOLATED_QUALIFICATION_ENV}=isolated")
    }
    Ok(())
}

/// Whether this process explicitly opted into disposable external mutation.
/// Any value other than the exact opt-in is treated as ordinary read-only
/// fixture realization. Mutation endpoints still call
/// [`ensure_isolated_qualification`] and fail closed.
pub fn is_isolated_qualification() -> bool {
    if let Ok(value) = ISOLATED_QUALIFICATION_OVERRIDE.try_with(|value| *value) {
        return value;
    }
    std::env::var(ISOLATED_QUALIFICATION_ENV).ok().as_deref() == Some(ISOLATED_QUALIFICATION_VALUE)
}

tokio::task_local! {
    static ISOLATED_QUALIFICATION_OVERRIDE: bool;
}

/// Test/embedded-runner hook that scopes qualification mode to one async task
/// without mutating the process environment. Production callers must use the
/// explicit environment gate above.
pub async fn with_isolated_qualification<F>(future: F) -> F::Output
where
    F: Future,
{
    ISOLATED_QUALIFICATION_OVERRIDE.scope(true, future).await
}

fn mutation_generation_id(mutation_generation: i64) -> String {
    mutation::generation_id(mutation_generation)
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn non_empty(value: Option<&str>, name: &str) -> Result<String> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("{name} is required"))?;
    Ok(value.to_string())
}

fn parse_resource_tags(raw: Option<&str>) -> BTreeMap<String, String> {
    raw.and_then(|raw| serde_json::from_str::<Map<String, Value>>(raw).ok())
        .map(|tags| {
            tags.into_iter()
                .filter_map(|(key, value)| value.as_str().map(|value| (key, value.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

async fn validate_mutation_authority(
    pool: &SqlitePool,
    authority: &MutationAuthority,
) -> Result<(i64, String, i64, String)> {
    ensure_isolated_qualification()?;
    ensure_no_global_mutation_blockers(pool, &[]).await?;
    validate_version(authority.version.as_deref())?;
    let generation = authority
        .generation
        .ok_or_else(|| anyhow!("generation is required"))?;
    let manifest_digest = non_empty(authority.manifest_digest.as_deref(), "manifest_digest")?;
    let mutation_generation = authority
        .mutation_generation
        .ok_or_else(|| anyhow!("mutation_generation is required"))?;
    let generation_id = non_empty(
        authority.mutation_generation_id.as_deref(),
        "mutation_generation_id",
    )?;
    let row = sqlx::query(
        "SELECT generation, manifest_digest, mutation_generation, mutation_generation_id
         FROM fixture_realizations WHERE singleton_id = 1",
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow!("fixture has not been realized"))?;
    let stored_generation: i64 = row.try_get("generation")?;
    let stored_manifest: String = row.try_get("manifest_digest")?;
    let stored_mutation_generation: i64 = row.try_get("mutation_generation")?;
    let stored_generation_id: Option<String> = row.try_get("mutation_generation_id")?;
    if generation != stored_generation
        || manifest_digest != stored_manifest
        || mutation_generation != stored_mutation_generation
        || stored_generation_id.as_deref() != Some(generation_id.as_str())
    {
        bail!("stale or mismatched fixture authority")
    }
    let active: Option<i64> = sqlx::query_scalar(
        "SELECT mutation_generation FROM fixture_mutation_generations
         WHERE mutation_generation = ? AND generation_id = ? AND state = 'ACTIVE'",
    )
    .bind(mutation_generation)
    .bind(&generation_id)
    .fetch_optional(pool)
    .await?;
    if active != Some(mutation_generation) {
        bail!("mutation generation is not active")
    }
    let nonterminal_or_ambiguous: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fixture_mutation_intents
         WHERE mutation_generation = ? AND status IN ('INTENT', 'DISPATCHED', 'AMBIGUOUS')",
    )
    .bind(mutation_generation)
    .fetch_one(pool)
    .await?;
    if nonterminal_or_ambiguous != 0 {
        bail!(
            "mutation generation has a nonterminal or ambiguous external operation; reconcile it before retrying"
        )
    }
    validate_dispatchable_generation(pool, mutation_generation, &generation_id).await?;
    Ok((
        generation,
        manifest_digest,
        mutation_generation,
        generation_id,
    ))
}

async fn validate_dispatchable_generation(
    pool: &SqlitePool,
    mutation_generation: i64,
    generation_id: &str,
) -> Result<()> {
    let generation = sqlx::query(
        "SELECT external_status
         FROM fixture_mutation_generations
         WHERE mutation_generation = ? AND generation_id = ? AND state = 'ACTIVE'",
    )
    .bind(mutation_generation)
    .bind(generation_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow!("mutation generation is not active"))?;
    let external_status: String = generation.try_get("external_status")?;
    if external_status != "ACTIVE" {
        bail!(
            "mutation generation external status is '{}'; public readiness is not dispatchable",
            external_status
        );
    }

    let rows = sqlx::query(
        "SELECT control_id, target_kind, setup_fault_kind, retired_at,
                external_identity_verified
         FROM fixture_mutation_resources
         WHERE mutation_generation = ? AND generation_id = ?",
    )
    .bind(mutation_generation)
    .bind(generation_id)
    .fetch_all(pool)
    .await?;
    if rows.len() != mutation::CATALOGUE.len()
        || rows.iter().any(|row| {
            row.try_get::<Option<String>, _>("retired_at")
                .ok()
                .flatten()
                .is_some()
                || row
                    .try_get::<i64, _>("external_identity_verified")
                    .unwrap_or(0)
                    != 1
        })
    {
        bail!("mutation generation requires exactly four non-retired externally verified targets");
    }
    let observed = rows
        .iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("control_id")?,
                row.try_get::<String, _>("target_kind")?,
                row.try_get::<String, _>("setup_fault_kind")?,
            ))
        })
        .collect::<Result<std::collections::BTreeSet<_>>>()?;
    let expected = mutation::CATALOGUE
        .iter()
        .map(|scenario| {
            (
                scenario.control_id.to_string(),
                scenario.target_kind.as_str().to_string(),
                scenario.setup_fault_kind.as_str().to_string(),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    if observed != expected {
        bail!("mutation generation targets do not match the canonical catalogue");
    }
    Ok(())
}

async fn mutation_target_values(pool: &SqlitePool, mutation_generation: i64) -> Result<Vec<Value>> {
    let rows = sqlx::query(
        "SELECT m.resource_id, m.control_id, m.target_kind, m.setup_fault_kind,
                m.instance_state, m.instance_type,
                initial_state, initial_type, terminal_state, terminal_type,
                restored_state, restored_type, r.region, m.external_status,
                m.external_identity_verified
         FROM fixture_mutation_resources m
         JOIN resources r ON r.id = m.resource_id
         WHERE m.mutation_generation = ?
         ORDER BY m.control_id ASC",
    )
    .bind(mutation_generation)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let resource_id: String = row.try_get("resource_id")?;
            let target_kind: String = row.try_get("target_kind")?;
            let region: String = row.try_get("region")?;
            Ok(json!({
                "control_id": row.try_get::<String, _>("control_id")?,
                "target_kind": target_kind,
                "setup_fault_kind": row.try_get::<String, _>("setup_fault_kind")?,
                "resource_id": resource_id,
                "aws_identity": resource_arn(&region, authoritative_account_id(), &resource_id),
                "instance_state": row.try_get::<String, _>("instance_state")?,
                "instance_type": row.try_get::<String, _>("instance_type")?,
                "initial_state": row.try_get::<String, _>("initial_state")?,
                "initial_type": row.try_get::<String, _>("initial_type")?,
                "terminal_state": row.try_get::<String, _>("terminal_state")?,
                "terminal_type": row.try_get::<String, _>("terminal_type")?,
                "restored_state": row.try_get::<String, _>("restored_state")?,
                "restored_type": row.try_get::<String, _>("restored_type")?,
                "external_status": row.try_get::<String, _>("external_status")?,
                "external_identity_verified": row.try_get::<i64, _>("external_identity_verified")? != 0
            }))
        })
        .collect()
}

async fn mutation_backend_for_generation(
    pool: &SqlitePool,
    mutation_generation: i64,
    generation_id: &str,
) -> Result<mutation::Ec2MutationBackend> {
    let row = sqlx::query(
        "SELECT endpoint_url, region, account_id, external_status
         FROM fixture_mutation_generations
         WHERE mutation_generation = ? AND generation_id = ? AND state = 'ACTIVE'",
    )
    .bind(mutation_generation)
    .bind(generation_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow!("mutation generation is not active"))?;
    let endpoint_url: String = row.try_get("endpoint_url")?;
    let region: String = row.try_get("region")?;
    let account_id: String = row.try_get("account_id")?;
    let external_status: String = row.try_get("external_status")?;
    if external_status != "ACTIVE" {
        bail!(
            "mutation generation external status is '{}'; public readiness is not dispatchable",
            external_status
        )
    }
    validate_dispatchable_generation(pool, mutation_generation, generation_id).await?;
    mutation::Ec2MutationBackend::connect(&endpoint_url, &region, &account_id).await
}

struct MutationGenerationContext<'a> {
    fixture_generation: i64,
    mutation_generation: i64,
    generation_id: &'a str,
    region: &'a str,
    account_id: &'a str,
    endpoint_url: &'a str,
    anchor: &'a str,
}

async fn create_mutation_generation(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    context: MutationGenerationContext<'_>,
    provisioned: &[(mutation::MutationScenario, mutation::ObservedInstance)],
) -> Result<Vec<Value>> {
    if provisioned.len() != mutation::CATALOGUE.len() {
        bail!(
            "mutation generation requires {} provisioned targets; found {}",
            mutation::CATALOGUE.len(),
            provisioned.len()
        );
    }
    let resource_ids = provisioned
        .iter()
        .map(|(_, observed)| observed.resource_id.clone())
        .collect::<Vec<_>>();
    sqlx::query(
        "INSERT INTO fixture_mutation_generations
         (mutation_generation, generation_id, fixture_generation, manifest_digest,
          complete_estate_fingerprint, state, resource_ids, public_absence,
          endpoint_url, region, account_id, external_status, created_at)
         VALUES (?, ?, ?, '', '', 'ACTIVE', ?, NULL, ?, ?, ?, 'PROVISIONED', ?)",
    )
    .bind(context.mutation_generation)
    .bind(context.generation_id)
    .bind(context.fixture_generation)
    .bind(serde_json::to_string(&resource_ids)?)
    .bind(context.endpoint_url)
    .bind(context.region)
    .bind(context.account_id)
    .bind(context.anchor)
    .execute(&mut **tx)
    .await?;

    let mut targets = Vec::with_capacity(provisioned.len());
    for (scenario, observed) in provisioned {
        let resource_id = &observed.resource_id;
        let tags = serde_json::to_string(&json!({
            "Name": resource_id,
            "FoxtailFixture": FIXTURE_VERSION,
            "FoxtailMutationGeneration": context.mutation_generation,
            "FoxtailMutationGenerationId": context.generation_id,
            "FoxtailMutationControl": scenario.control_id,
            "FoxtailMutationTarget": scenario.target_kind.as_str(),
            "InstanceState": observed.instance_state,
            "InstanceType": observed.instance_type
        }))?;
        sqlx::query(
            "INSERT INTO resources (id, resource_type, region, scenario, tags)
             VALUES (?, 'ec2', ?, 'QualificationMutation', ?)",
        )
        .bind(resource_id)
        .bind(context.region)
        .bind(tags)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("persist mutation resource {resource_id}"))?;
        sqlx::query(
            "INSERT INTO fixture_mutation_resources
             (resource_id, mutation_generation, generation_id, control_id, target_kind, setup_fault_kind,
              instance_state, instance_type, initial_state, initial_type,
              terminal_state, terminal_type, restored_state, restored_type, external_status,
              external_identity_verified, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'PROVISIONED', 1, ?)",
        )
        .bind(resource_id)
        .bind(context.mutation_generation)
        .bind(context.generation_id)
        .bind(scenario.control_id)
        .bind(scenario.target_kind.as_str())
        .bind(scenario.setup_fault_kind.as_str())
        .bind(&observed.instance_state)
        .bind(&observed.instance_type)
        .bind(scenario.initial_state)
        .bind(scenario.initial_type)
        .bind(scenario.terminal_state)
        .bind(scenario.terminal_type)
        .bind(scenario.restored_state)
        .bind(scenario.restored_type)
        .bind(context.anchor)
        .execute(&mut **tx)
        .await?;
        targets.push(json!({
            "control_id": scenario.control_id,
            "target_kind": scenario.target_kind.as_str(),
            "setup_fault_kind": scenario.setup_fault_kind.as_str(),
            "resource_id": resource_id,
            "aws_identity": resource_arn(context.region, context.account_id, resource_id),
            "instance_state": observed.instance_state,
            "instance_type": observed.instance_type,
            "initial_state": scenario.initial_state,
            "initial_type": scenario.initial_type,
            "terminal_state": scenario.terminal_state,
            "terminal_type": scenario.terminal_type,
            "restored_state": scenario.restored_state,
            "restored_type": scenario.restored_type,
            "external_identity_verified": true
        }));
    }
    Ok(targets)
}

async fn quarantine_provisioned_generation(
    pool: &SqlitePool,
    context: MutationGenerationContext<'_>,
    returned_ids: &[String],
    error: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO fixture_mutation_generations
         (mutation_generation, generation_id, fixture_generation, manifest_digest,
          complete_estate_fingerprint, state, resource_ids, public_absence,
          endpoint_url, region, account_id, external_status, created_at)
         VALUES (?, ?, ?, '', '', 'ACTIVE', ?, NULL, ?, ?, ?, 'AMBIGUOUS', ?)",
    )
    .bind(context.mutation_generation)
    .bind(context.generation_id)
    .bind(context.fixture_generation)
    .bind(serde_json::to_string(returned_ids)?)
    .bind(context.endpoint_url)
    .bind(context.region)
    .bind(context.account_id)
    .bind(context.anchor)
    .execute(pool)
    .await
    .context("persist quarantined mutation generation")?;
    sqlx::query(
        "UPDATE fixture_mutation_intents
         SET error = ?, updated_at = ?
         WHERE mutation_generation = ? AND generation_id = ?
           AND status IN ('INTENT', 'DISPATCHED')",
    )
    .bind(error)
    .bind(now_rfc3339())
    .bind(context.mutation_generation)
    .bind(context.generation_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn compensate_realization_failure(
    pool: &SqlitePool,
    intent_id: &str,
    backend: &mutation::Ec2MutationBackend,
    context: MutationGenerationContext<'_>,
    provisioned: &[(mutation::MutationScenario, mutation::ObservedInstance)],
    original_error: &anyhow::Error,
) {
    let ids = provisioned
        .iter()
        .map(|(_, observed)| observed.resource_id.clone())
        .collect::<Vec<_>>();
    let cleanup_result = backend.terminate_all(&ids).await;
    if let Err(cleanup_error) = cleanup_result {
        let error = format!(
            "post-provision finalization ambiguous; returned_ids={ids:?}; original_error={original_error}; cleanup_error={cleanup_error}"
        );
        let quarantine = quarantine_provisioned_generation(pool, context, &ids, &error).await;
        let intent_error = match quarantine {
            Ok(()) => error,
            Err(quarantine_error) => format!("{error}; quarantine_error={quarantine_error}"),
        };
        let _ = update_mutation_intent(
            pool,
            intent_id,
            "AMBIGUOUS",
            Some(&intent_error),
            &now_rfc3339(),
        )
        .await;
        return;
    }
    let _ = update_mutation_intent(
        pool,
        intent_id,
        "FAILED",
        Some(&original_error.to_string()),
        &now_rfc3339(),
    )
    .await;
}

async fn active_fault_values(
    pool: &SqlitePool,
    mutation_generation: Option<i64>,
) -> Result<Vec<Value>> {
    let rows = if let Some(generation) = mutation_generation {
        sqlx::query(
            "SELECT receipt_id, mutation_generation, generation_id, manifest_digest,
                    control_id, target_id, scope, fault_kind, applied_at,
                    prior_state, terminal_state, status, reset_at, reset_receipt_id
             FROM fixture_faults
             WHERE mutation_generation = ? AND status = 'ACTIVE'
             ORDER BY receipt_id ASC",
        )
        .bind(generation)
        .fetch_all(pool)
        .await?
    } else {
        Vec::new()
    };
    rows.into_iter()
        .map(|row| {
            Ok(json!({
                "receipt_id": row.try_get::<String, _>("receipt_id")?,
                "mutation_generation": row.try_get::<i64, _>("mutation_generation")?,
                "generation_id": row.try_get::<String, _>("generation_id")?,
                "manifest_digest": row.try_get::<String, _>("manifest_digest")?,
                "control_id": row.try_get::<String, _>("control_id")?,
                "target_id": row.try_get::<String, _>("target_id")?,
                "scope": row.try_get::<String, _>("scope")?,
                "fault_kind": row.try_get::<String, _>("fault_kind")?,
                "applied_at": row.try_get::<String, _>("applied_at")?,
                "prior_state": row.try_get::<String, _>("prior_state")?,
                "terminal_state": row.try_get::<String, _>("terminal_state")?,
                "status": row.try_get::<String, _>("status")?,
                "reset_at": row.try_get::<Option<String>, _>("reset_at")?,
                "reset_receipt_id": row.try_get::<Option<String>, _>("reset_receipt_id")?
            }))
        })
        .collect()
}

fn receipt_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

fn reset_token() -> String {
    format!("rt-{}", uuid::Uuid::new_v4())
}

struct IntentContext<'a> {
    mutation_generation: Option<i64>,
    generation_id: Option<&'a str>,
    fixture_generation: Option<i64>,
    target_id: Option<&'a str>,
}

async fn begin_mutation_intent(
    pool: &SqlitePool,
    operation: &str,
    context: IntentContext<'_>,
    request: &Value,
    created_at: &str,
) -> Result<String> {
    let intent_id = receipt_id("intent");
    let request_bytes = canonical_bytes(request)?;
    let mut tx = pool.begin().await?;
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fixture_mutation_intents
         WHERE status IN ('INTENT', 'DISPATCHED', 'AMBIGUOUS')
           AND COALESCE(mutation_generation, -1) = COALESCE(?, -1)",
    )
    .bind(context.mutation_generation)
    .fetch_one(&mut *tx)
    .await?;
    if active != 0 {
        bail!("mutation operation '{operation}' is already in progress")
    }
    let insert = sqlx::query(
        "INSERT INTO fixture_mutation_intents
         (intent_id, operation, mutation_generation, generation_id, fixture_generation,
          target_id, request_bytes, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'INTENT', ?, ?)",
    )
    .bind(&intent_id)
    .bind(operation)
    .bind(context.mutation_generation)
    .bind(context.generation_id)
    .bind(context.fixture_generation)
    .bind(context.target_id)
    .bind(request_bytes)
    .bind(created_at)
    .bind(created_at)
    .execute(&mut *tx)
    .await;
    if let Err(error) = insert {
        if error.to_string().to_ascii_lowercase().contains("unique") {
            bail!("mutation generation already has a nonterminal operation");
        }
        return Err(error.into());
    }
    tx.commit().await?;
    Ok(intent_id)
}

async fn update_mutation_intent(
    pool: &SqlitePool,
    intent_id: &str,
    status: &str,
    error: Option<&str>,
    updated_at: &str,
) -> Result<()> {
    let affected = sqlx::query(
        "UPDATE fixture_mutation_intents
         SET status = ?, error = ?, updated_at = ?
         WHERE intent_id = ? AND status IN ('INTENT', 'DISPATCHED')",
    )
    .bind(status)
    .bind(error)
    .bind(updated_at)
    .bind(intent_id)
    .execute(pool)
    .await?
    .rows_affected();
    if affected != 1 {
        bail!("mutation intent {intent_id} was already finalized")
    }
    Ok(())
}

async fn mark_intent_dispatched(
    pool: &SqlitePool,
    intent_id: &str,
    updated_at: &str,
) -> Result<()> {
    let affected = sqlx::query(
        "UPDATE fixture_mutation_intents SET status = 'DISPATCHED', updated_at = ?
         WHERE intent_id = ? AND status = 'INTENT'",
    )
    .bind(updated_at)
    .bind(intent_id)
    .execute(pool)
    .await?
    .rows_affected();
    if affected != 1 {
        bail!("mutation intent {intent_id} is not dispatchable")
    }
    Ok(())
}

async fn finalize_intent_ambiguous(pool: &SqlitePool, intent_id: &str, error: &anyhow::Error) {
    let _ = update_mutation_intent(
        pool,
        intent_id,
        "AMBIGUOUS",
        Some(&error.to_string()),
        &now_rfc3339(),
    )
    .await;
}

async fn mark_generation_ambiguous(
    pool: &SqlitePool,
    mutation_generation: i64,
    generation_id: &str,
) {
    let _ = sqlx::query(
        "UPDATE fixture_mutation_generations
         SET external_status = 'AMBIGUOUS'
         WHERE mutation_generation = ? AND generation_id = ? AND state = 'ACTIVE'",
    )
    .bind(mutation_generation)
    .bind(generation_id)
    .execute(pool)
    .await;
}

fn parse_application_time(raw: Option<&str>) -> Result<String> {
    match raw {
        Some(value) => Ok(DateTime::parse_from_rfc3339(value)
            .with_context(|| format!("invalid application_time '{value}'"))?
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Secs, true)),
        None => Ok(now_rfc3339()),
    }
}

fn canonical_receipt(mut value: Value) -> Result<Vec<u8>> {
    if value.get("schema").is_none() {
        value["schema"] = json!("foxtail.release-fixture-receipt/v1");
    }
    canonical_bytes(&value)
}

struct ReceiptContext<'a> {
    mutation_generation: Option<i64>,
    generation_id: Option<&'a str>,
    manifest_digest: Option<&'a str>,
}

async fn persist_receipt(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    operation: &str,
    receipt_id: &str,
    context: ReceiptContext<'_>,
    receipt_bytes: &[u8],
    created_at: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO fixture_operation_receipts
         (receipt_id, operation, mutation_generation, generation_id, manifest_digest,
          receipt_bytes, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(receipt_id)
    .bind(operation)
    .bind(context.mutation_generation)
    .bind(context.generation_id)
    .bind(context.manifest_digest)
    .bind(receipt_bytes)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn read_state(pool: &SqlitePool) -> Result<FixtureState> {
    let (definition_bytes, definition_digest) = canonical_definition()?;
    let row = sqlx::query(
        "SELECT definition_bytes, definition_digest, manifest_bytes, manifest_digest, generation,
                mutation_generation
         FROM fixture_realizations WHERE singleton_id = 1",
    )
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        let status_bytes =
            canonical_status_bytes("ABSENT", &definition_digest, None, None, &[], None, &[])?;
        let identities_bytes = canonical_identities_bytes("ABSENT", None, &[], &[])?;
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
    let mutation_generation: i64 = row.try_get("mutation_generation")?;

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
    let mutation_identities = mutation_identity_values(&manifest_value)?;
    let active_faults = active_fault_values(pool, Some(mutation_generation)).await?;
    let status_bytes = canonical_status_bytes(
        "REALIZED",
        &stored_definition_digest,
        Some(&manifest_digest),
        Some(generation),
        &identities,
        Some(&manifest_value),
        &active_faults,
    )?;
    let identities_bytes = canonical_identities_bytes(
        "REALIZED",
        Some(&manifest_digest),
        &identities,
        &mutation_identities,
    )?;

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
    let isolated = is_isolated_qualification();
    let request_value = serde_json::to_value(&request)?;
    if isolated {
        ensure_no_global_mutation_blockers(pool, &request.allowed_intent_ids).await?;
    }

    // Ordinary realization remains a read-only lookup.  An isolated active
    // generation is different: every subsequent realization request, even
    // one with changed options, must use the authority-bound recreate path.
    if let Some(row) = sqlx::query(
        "SELECT manifest_digest, generation, mutation_generation_id
         FROM fixture_realizations WHERE singleton_id = 1",
    )
    .fetch_optional(pool)
    .await?
    {
        let manifest_digest: String = row.try_get("manifest_digest")?;
        let current_mutation_id: Option<String> = row.try_get("mutation_generation_id")?;
        if isolated && current_mutation_id.is_some() && !request.force_new {
            bail!(
                "an isolated mutation generation is active; use authority-bound recreate instead of realize"
            );
        }
        if !isolated
            && !request.force_new
            && request.clock_anchor.is_none()
            && request.account_id.is_none()
            && request.region.is_none()
            && request.endpoint_url.is_none()
            && request.localstack_version.is_none()
        {
            let state = read_state(pool).await?;
            if let Some(manifest_bytes) = state.manifest_bytes {
                return Ok(FixtureSnapshot {
                    definition_bytes: state.definition_bytes,
                    definition_digest: state.definition_digest,
                    manifest_bytes,
                    manifest_digest: state.manifest_digest.unwrap_or(manifest_digest),
                    status_bytes: state.status_bytes,
                    identities_bytes: state.identities_bytes,
                    generation: row.try_get("generation")?,
                });
            }
        }
    }

    if !isolated {
        let active_mutations: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM fixture_mutation_generations WHERE state = 'ACTIVE'",
        )
        .fetch_one(pool)
        .await?;
        if active_mutations != 0 {
            bail!(
                "an isolated mutation generation is active; set {ISOLATED_QUALIFICATION_ENV}=isolated to manage it"
            );
        }
    }

    let rows = sqlx::query(
        "SELECT id, region, scenario, instance_state, instance_type, availability_zone, tags
         FROM resources
         WHERE resource_type = 'ec2'
           AND id NOT IN (
             SELECT resource_id FROM fixture_mutation_resources WHERE retired_at IS NULL
           )
         ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await
    .context("read EC2 estate for fixture realization")?;

    if rows.len() < MATERIALIZATION_PROFILES.len() {
        bail!(
            "fixture realization requires at least {} EC2 resources; found {}",
            MATERIALIZATION_PROFILES.len(),
            rows.len()
        )
    }

    let resources = rows
        .into_iter()
        .map(|row| {
            Ok(ResourceIdentity {
                id: row.try_get("id")?,
                region: row.try_get("region")?,
                scenario: row.try_get("scenario")?,
                instance_state: row.try_get("instance_state")?,
                instance_type: row.try_get("instance_type")?,
                availability_zone: row.try_get("availability_zone")?,
                tags: parse_resource_tags(row.try_get::<Option<String>, _>("tags")?.as_deref()),
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

    let assigned = assign_realized_resources(&resources);
    let account_id = resolve_account_id(request.account_id.as_deref())?;
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

    let active_faults: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM fixture_faults WHERE status = 'ACTIVE'")
            .fetch_one(pool)
            .await?;
    if active_faults != 0 {
        bail!("cannot realize or recreate while a mutation fault is active; reset it first");
    }

    // Allocate identities before dispatch. The intent is committed before any
    // EC2 call so a crash or ambiguous SDK response cannot be mistaken for a
    // successful fixture mutation.
    let generation: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(value), 0) + 1 FROM (
            SELECT generation AS value FROM fixture_realizations
            UNION ALL SELECT fixture_generation AS value FROM fixture_mutation_generations
        )",
    )
    .fetch_one(pool)
    .await?;
    let mutation_generation: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(mutation_generation), 0) + 1
         FROM fixture_mutation_generations",
    )
    .fetch_one(pool)
    .await?;
    let mutation_generation_id = isolated.then(|| mutation_generation_id(mutation_generation));
    // Account identity is verified by STS before the intent is persisted. A
    // mismatch, transport failure, or malformed identity therefore leaves no
    // partial mutation ledger behind and cannot reach EC2 RunInstances.
    let mutation_backend = if isolated {
        Some(
            mutation::Ec2MutationBackend::connect(&endpoint_url, &region, &account_id)
                .await
                .with_context(|| format!("connect to public EC2 endpoint {endpoint_url}"))?,
        )
    } else {
        None
    };
    let intent_id = if isolated {
        let id = if let Some(reuse_intent_id) = request.reuse_intent_id.clone() {
            reuse_intent_id
        } else {
            begin_mutation_intent(
                pool,
                "realize",
                IntentContext {
                    mutation_generation: Some(mutation_generation),
                    generation_id: mutation_generation_id.as_deref(),
                    fixture_generation: Some(generation),
                    target_id: None,
                },
                &request_value,
                &anchor.to_rfc3339_opts(SecondsFormat::Secs, true),
            )
            .await?
        };
        if let Err(error) = mark_intent_dispatched(pool, &id, &now_rfc3339()).await {
            finalize_intent_ambiguous(pool, &id, &error).await;
            return Err(error);
        }
        Some(id)
    } else {
        None
    };

    let provisioned = if let Some(backend) = mutation_backend {
        match backend
            .provision_generation(generation, mutation_generation_id.as_deref().unwrap())
            .await
        {
            Ok(targets) => Some((backend, targets)),
            Err(error) => {
                if let Some(intent_id) = intent_id.as_deref() {
                    if let Some(failure) = error.downcast_ref::<mutation::ProvisionFailure>() {
                        let quarantine = quarantine_provisioned_generation(
                            pool,
                            MutationGenerationContext {
                                fixture_generation: generation,
                                mutation_generation,
                                generation_id: mutation_generation_id.as_deref().unwrap(),
                                region: &region,
                                account_id: &account_id,
                                endpoint_url: &endpoint_url,
                                anchor: &anchor.to_rfc3339_opts(SecondsFormat::Secs, true),
                            },
                            &failure.returned_ids,
                            &error.to_string(),
                        )
                        .await;
                        let intent_error = match quarantine {
                            Ok(()) => error.to_string(),
                            Err(quarantine_error) => {
                                format!("{error}; quarantine_error={quarantine_error}")
                            }
                        };
                        let _ = update_mutation_intent(
                            pool,
                            intent_id,
                            "AMBIGUOUS",
                            Some(&intent_error),
                            &now_rfc3339(),
                        )
                        .await;
                    } else {
                        let _ = update_mutation_intent(
                            pool,
                            intent_id,
                            "FAILED",
                            Some(&error.to_string()),
                            &now_rfc3339(),
                        )
                        .await;
                    }
                }
                return Err(error);
            }
        }
    } else {
        None
    };

    let persistence = async {
        let mut tx = pool.begin().await?;
        for (control_id, resource) in &assigned {
            materialize_control_evidence(&mut tx, control_id, &resource.id).await?;
        }

        let observed_resources = load_estate_resources(&mut tx, &resources).await?;
        let observed_by_id = observed_resources
            .iter()
            .map(|resource| (resource.id.clone(), resource.clone()))
            .collect::<BTreeMap<_, _>>();
        let assigned_observed = assigned
            .iter()
            .map(|(control_id, resource)| {
                let observed = observed_by_id.get(&resource.id).cloned().ok_or_else(|| {
                    anyhow!("materialized fixture resource {} disappeared", resource.id)
                })?;
                Ok((control_id.clone(), observed))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        validate_realized_controls(&assigned_observed)?;

        let mutation_targets = if let Some((_, provisioned)) = &provisioned {
            create_mutation_generation(
                &mut tx,
                MutationGenerationContext {
                    fixture_generation: generation,
                    mutation_generation,
                    generation_id: mutation_generation_id.as_deref().unwrap(),
                    region: &region,
                    account_id: &account_id,
                    endpoint_url: &endpoint_url,
                    anchor: &anchor.to_rfc3339_opts(SecondsFormat::Secs, true),
                },
                provisioned,
            )
            .await?
        } else {
            Vec::new()
        };

        let read_only_fingerprint = estate_fingerprint(&assigned_observed, &region, &account_id)?;
        let complete_map = observed_resources
            .into_iter()
            .map(|resource| (resource.id.clone(), resource))
            .collect::<BTreeMap<_, _>>();
        let manifest_mutation_generation = isolated.then_some(mutation_generation);
        let manifest_mutation_generation_id =
            isolated.then(|| mutation_generation_id.clone()).flatten();
        let complete_fingerprint = canonical_digest(&json!({
            "mutation_generation": manifest_mutation_generation,
            "mutation_generation_id": &manifest_mutation_generation_id,
            "read_only_estate_fingerprint": &read_only_fingerprint,
            "mutation_targets": &mutation_targets
        }))?;

        let manifest_without_digest = build_manifest(ManifestContext {
            definition_digest: &definition_digest,
            assigned: &assigned_observed,
            complete_resources: &complete_map,
            region: &region,
            account_id: &account_id,
            endpoint_url: &endpoint_url,
            localstack_version: &localstack_version,
            source_revision: &source_revision,
            anchor,
            generation,
            read_only_fingerprint: &read_only_fingerprint,
            complete_fingerprint: &complete_fingerprint,
            mutation_generation: manifest_mutation_generation,
            mutation_generation_id: manifest_mutation_generation_id.as_deref(),
            mutation_targets: &mutation_targets,
        })?;
        let (manifest_bytes, manifest_digest) = with_digest(&manifest_without_digest)?;

        if isolated {
            sqlx::query(
                "UPDATE fixture_mutation_generations
             SET manifest_digest = ?, complete_estate_fingerprint = ?, external_status = 'ACTIVE'
             WHERE mutation_generation = ? AND generation_id = ? AND state = 'ACTIVE'",
            )
            .bind(&manifest_digest)
            .bind(&complete_fingerprint)
            .bind(mutation_generation)
            .bind(mutation_generation_id.as_deref())
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query(
            "INSERT INTO fixture_realizations
           (singleton_id, definition_bytes, definition_digest, manifest_bytes, manifest_digest,
            generation, mutation_generation, mutation_generation_id,
            complete_estate_fingerprint, created_at, updated_at)
         VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(singleton_id) DO UPDATE SET
           definition_bytes = excluded.definition_bytes,
           definition_digest = excluded.definition_digest,
           manifest_bytes = excluded.manifest_bytes,
           manifest_digest = excluded.manifest_digest,
           generation = excluded.generation,
           mutation_generation = excluded.mutation_generation,
           mutation_generation_id = excluded.mutation_generation_id,
           complete_estate_fingerprint = excluded.complete_estate_fingerprint,
           updated_at = excluded.updated_at",
        )
        .bind(&definition_bytes)
        .bind(&definition_digest)
        .bind(&manifest_bytes)
        .bind(&manifest_digest)
        .bind(generation)
        .bind(if isolated { mutation_generation } else { 0 })
        .bind(mutation_generation_id.as_deref())
        .bind(&complete_fingerprint)
        .bind(anchor.to_rfc3339_opts(SecondsFormat::Secs, true))
        .bind(anchor.to_rfc3339_opts(SecondsFormat::Secs, true))
        .execute(&mut *tx)
        .await
        .context("persist fixture realization atomically")?;
        tx.commit().await?;
        Ok::<(Vec<u8>, String, Value), anyhow::Error>((
            manifest_bytes,
            manifest_digest,
            manifest_without_digest,
        ))
    }
    .await;

    let (manifest_bytes, manifest_digest, manifest_without_digest) = match persistence {
        Ok(value) => value,
        Err(error) => {
            if let (Some(intent_id), Some((backend, provisioned))) =
                (intent_id.as_deref(), provisioned.as_ref())
            {
                compensate_realization_failure(
                    pool,
                    intent_id,
                    backend,
                    MutationGenerationContext {
                        fixture_generation: generation,
                        mutation_generation,
                        generation_id: mutation_generation_id.as_deref().unwrap(),
                        region: &region,
                        account_id: &account_id,
                        endpoint_url: &endpoint_url,
                        anchor: &anchor.to_rfc3339_opts(SecondsFormat::Secs, true),
                    },
                    provisioned,
                    &error,
                )
                .await;
            }
            return Err(error);
        }
    };

    let response_bytes = (|| {
        let identities = identity_values(&manifest_without_digest)?;
        let mutation_identities = mutation_identity_values(&manifest_without_digest)?;
        let status_bytes = canonical_status_bytes(
            "REALIZED",
            &definition_digest,
            Some(&manifest_digest),
            Some(generation),
            &identities,
            Some(&manifest_without_digest),
            &[],
        )?;
        let identities_bytes = canonical_identities_bytes(
            "REALIZED",
            Some(&manifest_digest),
            &identities,
            &mutation_identities,
        )?;
        Ok::<(Vec<u8>, Vec<u8>), anyhow::Error>((status_bytes, identities_bytes))
    })();
    let (status_bytes, identities_bytes) = match response_bytes {
        Ok(bytes) => bytes,
        Err(error) => {
            if let Some(intent_id) = intent_id.as_deref() {
                finalize_intent_ambiguous(pool, intent_id, &error).await;
            }
            return Err(error);
        }
    };

    if !request.defer_intent_finalization
        && let Some(intent_id) = intent_id.as_deref()
        && let Err(error) =
            update_mutation_intent(pool, intent_id, "SUCCEEDED", None, &now_rfc3339()).await
    {
        // The external estate, manifest, and response bytes are already
        // durable. Keep the intent visibly fail-closed rather than claiming
        // a green receipt when finalization itself was ambiguous.
        finalize_intent_ambiguous(pool, intent_id, &error).await;
        return Err(error);
    }

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

async fn quarantined_mutation_status(pool: &SqlitePool) -> Result<Option<Value>> {
    let generation_rows = sqlx::query(
        "SELECT mutation_generation, generation_id, fixture_generation,
                manifest_digest, resource_ids, external_status
         FROM fixture_mutation_generations
         WHERE state = 'ACTIVE'
           AND (external_status <> 'ACTIVE' OR EXISTS (
               SELECT 1
               FROM fixture_mutation_intents i
               WHERE i.mutation_generation = fixture_mutation_generations.mutation_generation
                 AND i.generation_id = fixture_mutation_generations.generation_id
                 AND i.status IN ('INTENT', 'DISPATCHED', 'AMBIGUOUS')
           ))
         ORDER BY mutation_generation",
    )
    .fetch_all(pool)
    .await?;
    let intent_rows = sqlx::query(
        "SELECT intent_id, operation, mutation_generation, generation_id,
                fixture_generation, status, error, created_at, updated_at
         FROM fixture_mutation_intents
         WHERE status IN ('INTENT', 'DISPATCHED', 'AMBIGUOUS')
         ORDER BY created_at, intent_id",
    )
    .fetch_all(pool)
    .await?;
    if generation_rows.is_empty() && intent_rows.is_empty() {
        return Ok(None);
    }

    let intents = intent_rows
        .iter()
        .map(|row| {
            Ok(json!({
                "intent_id": row.try_get::<String, _>("intent_id")?,
                "operation": row.try_get::<String, _>("operation")?,
                "mutation_generation": row.try_get::<Option<i64>, _>("mutation_generation")?,
                "generation_id": row.try_get::<Option<String>, _>("generation_id")?,
                "fixture_generation": row.try_get::<Option<i64>, _>("fixture_generation")?,
                "status": row.try_get::<String, _>("status")?,
                "error": row.try_get::<Option<String>, _>("error")?,
                "created_at": row.try_get::<String, _>("created_at")?,
                "updated_at": row.try_get::<String, _>("updated_at")?
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut resource_ids = BTreeSet::new();
    let quarantined_generations = generation_rows
        .iter()
        .map(|row| {
            let raw_ids: String = row.try_get("resource_ids")?;
            let ids = serde_json::from_str::<Vec<String>>(&raw_ids).unwrap_or_default();
            resource_ids.extend(ids.iter().cloned());
            let mutation_generation: i64 = row.try_get("mutation_generation")?;
            let generation_id: String = row.try_get("generation_id")?;
            let generation_intents = intent_rows
                .iter()
                .filter(|intent| {
                    intent
                        .try_get::<Option<i64>, _>("mutation_generation")
                        .ok()
                        .flatten()
                        == Some(mutation_generation)
                        && intent
                            .try_get::<Option<String>, _>("generation_id")
                            .ok()
                            .flatten()
                            .as_deref()
                            == Some(generation_id.as_str())
                })
                .map(|intent| {
                    Ok(json!({
                        "intent_id": intent.try_get::<String, _>("intent_id")?,
                        "operation": intent.try_get::<String, _>("operation")?,
                        "status": intent.try_get::<String, _>("status")?,
                        "error": intent.try_get::<Option<String>, _>("error")?,
                        "created_at": intent.try_get::<String, _>("created_at")?,
                        "updated_at": intent.try_get::<String, _>("updated_at")?
                    }))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(json!({
                "mutation_generation": mutation_generation,
                "generation_id": generation_id,
                "fixture_generation": row.try_get::<i64, _>("fixture_generation")?,
                "manifest_digest": row.try_get::<String, _>("manifest_digest")?,
                "external_status": row.try_get::<String, _>("external_status")?,
                "resource_ids": ids,
                "intents": generation_intents
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    let first_generation = generation_rows.first();
    Ok(Some(json!({
        "schema": "foxtail.release-fixture-mutation-status/v1",
        "status": "QUARANTINED",
        "fixture_generation": first_generation
            .map(|row| row.try_get::<i64, _>("fixture_generation"))
            .transpose()?,
        "mutation_generation": first_generation
            .map(|row| row.try_get::<i64, _>("mutation_generation"))
            .transpose()?,
        "mutation_generation_id": first_generation
            .map(|row| row.try_get::<String, _>("generation_id"))
            .transpose()?,
        "manifest_digest": first_generation
            .and_then(|row| row.try_get::<String, _>("manifest_digest").ok())
            .filter(|digest| !digest.is_empty()),
        "targets": [],
        "active_faults": [],
        "resource_ids": resource_ids.into_iter().collect::<Vec<_>>(),
        "intents": intents,
        "quarantined_generations": quarantined_generations
    })))
}

/// Return the qualification-only mutation state.  This intentionally has a
/// separate gate from the read-only fixture status route.
pub async fn mutation_status(pool: &SqlitePool) -> Result<Vec<u8>> {
    ensure_isolated_qualification()?;
    if let Some(quarantined) = quarantined_mutation_status(pool).await? {
        return canonical_bytes(&quarantined);
    }
    let row = sqlx::query(
        "SELECT generation, manifest_digest, mutation_generation, mutation_generation_id
         FROM fixture_realizations WHERE singleton_id = 1",
    )
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return canonical_bytes(&json!({
            "schema": "foxtail.release-fixture-mutation-status/v1",
            "status": "ABSENT",
            "fixture_generation": Value::Null,
            "mutation_generation": Value::Null,
            "mutation_generation_id": Value::Null,
            "manifest_digest": Value::Null,
            "targets": [],
            "active_faults": []
        }));
    };
    let mutation_generation: i64 = row.try_get("mutation_generation")?;
    let mutation_generation_id: Option<String> = row.try_get("mutation_generation_id")?;
    if mutation_generation < 1 || mutation_generation_id.is_none() {
        return canonical_bytes(&json!({
            "schema": "foxtail.release-fixture-mutation-status/v1",
            "status": "ABSENT",
            "fixture_generation": row.try_get::<i64, _>("generation")?,
            "mutation_generation": Value::Null,
            "mutation_generation_id": Value::Null,
            "manifest_digest": Value::Null,
            "targets": [],
            "active_faults": []
        }));
    }
    let status = MutationSnapshot {
        status: "ACTIVE".to_string(),
        fixture_generation: Some(row.try_get("generation")?),
        mutation_generation: Some(mutation_generation),
        mutation_generation_id,
        manifest_digest: Some(row.try_get("manifest_digest")?),
        targets: mutation_target_values(pool, mutation_generation).await?,
        active_faults: active_fault_values(pool, Some(mutation_generation)).await?,
    };
    canonical_bytes(&json!({
        "schema": "foxtail.release-fixture-mutation-status/v1",
        "status": status.status,
        "fixture_generation": status.fixture_generation,
        "mutation_generation": status.mutation_generation,
        "mutation_generation_id": status.mutation_generation_id,
        "manifest_digest": status.manifest_digest,
        "targets": status.targets,
        "active_faults": status.active_faults
    }))
}

pub fn parse_fault_request(body: &[u8]) -> Result<FaultRequest> {
    serde_json::from_slice(body).context("invalid fixture fault JSON")
}

pub fn parse_reset_request(body: &[u8]) -> Result<ResetRequest> {
    serde_json::from_slice(body).context("invalid fixture reset JSON")
}

pub fn parse_recreate_request(body: &[u8]) -> Result<RecreateRequest> {
    if body.is_empty() {
        return Ok(RecreateRequest::default());
    }
    serde_json::from_slice(body).context("invalid fixture recreate JSON")
}

pub fn parse_destroy_request(body: &[u8]) -> Result<DestroyRequest> {
    serde_json::from_slice(body).context("invalid fixture destroy JSON")
}

pub async fn apply_fault(pool: &SqlitePool, request: FaultRequest) -> Result<Vec<u8>> {
    let (_, manifest_digest, mutation_generation, generation_id) =
        validate_mutation_authority(pool, &request.authority).await?;
    let control_id = non_empty(Some(&request.control_id), "control_id")?;
    let target_id = non_empty(Some(&request.target_id), "target_id")?;
    let scope = non_empty(Some(&request.scope), "scope")?;
    let fault_kind = non_empty(Some(&request.fault_kind), "fault_kind")?.to_ascii_lowercase();
    if scope != "target" {
        bail!("scope must be exactly 'target'")
    }
    if !matches!(fault_kind.as_str(), "stop" | "resize") {
        bail!("unknown fault_kind '{fault_kind}'")
    }
    let applied_at = parse_application_time(request.application_time.as_deref())?;
    let receipt_id = receipt_id("fault");
    let reset_token = reset_token();
    let target = sqlx::query(
        "SELECT control_id, target_kind, setup_fault_kind, instance_state, instance_type,
                initial_state, initial_type, terminal_state, terminal_type
         FROM fixture_mutation_resources
         WHERE resource_id = ? AND mutation_generation = ? AND generation_id = ?
           AND retired_at IS NULL",
    )
    .bind(&target_id)
    .bind(mutation_generation)
    .bind(&generation_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow!("target is not part of the active mutation generation"))?;
    let stored_control: String = target.try_get("control_id")?;
    let target_kind: String = target.try_get("target_kind")?;
    let setup_fault_kind: String = target.try_get("setup_fault_kind")?;
    if stored_control != control_id || setup_fault_kind != fault_kind {
        bail!("control_id, target_id, and fault_kind do not match the manifest-bound setup fault")
    }
    let prior_state: String = target.try_get("instance_state")?;
    let prior_type: String = target.try_get("instance_type")?;
    let initial_state: String = target.try_get("initial_state")?;
    let initial_type: String = target.try_get("initial_type")?;
    if prior_state != initial_state || prior_type != initial_type {
        bail!("target is not in its one-use initial state")
    }
    let duplicate: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fixture_faults
         WHERE mutation_generation = ? AND target_id = ? AND status = 'ACTIVE'",
    )
    .bind(mutation_generation)
    .bind(&target_id)
    .fetch_one(pool)
    .await?;
    if duplicate != 0 {
        bail!("target already has an active fault")
    }
    let terminal_state: String = target.try_get("terminal_state")?;
    let terminal_type: String = target.try_get("terminal_type")?;
    let scenario = mutation::scenario_for_control(&control_id)?;
    if scenario.target_kind.as_str() != target_kind
        || scenario.setup_fault_kind.as_str() != fault_kind
    {
        bail!("target metadata is inconsistent with the canonical mutation catalogue")
    }
    let intent_id = begin_mutation_intent(
        pool,
        "fault",
        IntentContext {
            mutation_generation: Some(mutation_generation),
            generation_id: Some(&generation_id),
            fixture_generation: request.authority.generation,
            target_id: Some(&target_id),
        },
        &serde_json::to_value(&request)?,
        &applied_at,
    )
    .await?;
    if let Err(error) = mark_intent_dispatched(pool, &intent_id, &now_rfc3339()).await {
        finalize_intent_ambiguous(pool, &intent_id, &error).await;
        return Err(error);
    }
    let operation_result = async {
        let backend = match mutation_backend_for_generation(
            pool,
            mutation_generation,
            &generation_id,
        )
        .await
        {
            Ok(backend) => backend,
            Err(error) => {
                finalize_intent_ambiguous(pool, &intent_id, &error).await;
                return Err(error);
            }
        };
        let observed = match backend
            .apply_setup_fault(&target_id, scenario, scenario.setup_fault_kind)
            .await
        {
            Ok(observed) => observed,
            Err(error) => {
                finalize_intent_ambiguous(pool, &intent_id, &error).await;
                return Err(error);
            }
        };
        if observed.instance_state != terminal_state || observed.instance_type != terminal_type {
            let error = anyhow!(
                "public EC2 state after fault was {}:{}, expected {}:{}",
                observed.instance_state,
                observed.instance_type,
                terminal_state,
                terminal_type
            );
            finalize_intent_ambiguous(pool, &intent_id, &error).await;
            return Err(error);
        }
        let mut tx = pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE fixture_mutation_resources SET instance_state = ?, instance_type = ?,
                    external_status = 'FAULTED', external_identity_verified = 1
             WHERE resource_id = ? AND mutation_generation = ? AND generation_id = ?
               AND instance_state = ? AND instance_type = ? AND retired_at IS NULL",
        )
        .bind(&terminal_state)
        .bind(&terminal_type)
        .bind(&target_id)
        .bind(mutation_generation)
        .bind(&generation_id)
        .bind(&prior_state)
        .bind(&prior_type)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated != 1 {
            let error = anyhow!("fault finalization lost the target row to a concurrent operation");
            finalize_intent_ambiguous(pool, &intent_id, &error).await;
            return Err(error);
        }
        sqlx::query(
            "INSERT INTO fixture_faults
             (receipt_id, mutation_generation, generation_id, manifest_digest, control_id,
              target_id, scope, fault_kind, applied_at, reset_token, prior_state,
              terminal_state, status)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'ACTIVE')",
        )
        .bind(&receipt_id)
        .bind(mutation_generation)
        .bind(&generation_id)
        .bind(&manifest_digest)
        .bind(&control_id)
        .bind(&target_id)
        .bind(&scope)
        .bind(&fault_kind)
        .bind(&applied_at)
        .bind(&reset_token)
        .bind(format!("{prior_state}:{prior_type}"))
        .bind(format!("{terminal_state}:{terminal_type}"))
        .execute(&mut *tx)
        .await?;
        let receipt = json!({
            "schema": "foxtail.release-fixture-fault-receipt/v1",
            "operation": "fault",
            "status": "APPLIED",
            "receipt_id": receipt_id,
            "generation": request.authority.generation,
            "manifest_digest": manifest_digest,
            "mutation_generation": mutation_generation,
            "mutation_generation_id": generation_id,
            "control_id": control_id,
            "target_id": target_id,
            "scope": scope,
            "fault_kind": fault_kind,
            "applied_at": applied_at,
            "prior_state": format!("{prior_state}:{prior_type}"),
            "terminal_state": format!("{terminal_state}:{terminal_type}"),
            "reset_token": reset_token,
            "reset_token_use": "one-use"
        });
        let receipt_bytes = canonical_receipt(receipt)?;
        persist_receipt(
            &mut tx,
            "fault",
            &receipt_id,
            ReceiptContext {
                mutation_generation: Some(mutation_generation),
                generation_id: Some(&generation_id),
                manifest_digest: Some(&manifest_digest),
            },
            &receipt_bytes,
            &applied_at,
        )
        .await?;
        tx.commit().await?;
        Ok::<Vec<u8>, anyhow::Error>(receipt_bytes)
    }
    .await;
    match operation_result {
        Ok(receipt_bytes) => {
            match update_mutation_intent(pool, &intent_id, "SUCCEEDED", None, &now_rfc3339()).await
            {
                Ok(()) => Ok(receipt_bytes),
                Err(error) => {
                    finalize_intent_ambiguous(pool, &intent_id, &error).await;
                    Err(error)
                }
            }
        }
        Err(error) => {
            finalize_intent_ambiguous(pool, &intent_id, &error).await;
            Err(error)
        }
    }
}

pub async fn reset_fault(pool: &SqlitePool, request: ResetRequest) -> Result<Vec<u8>> {
    let (_, manifest_digest, mutation_generation, generation_id) =
        validate_mutation_authority(pool, &request.authority).await?;
    let fault_receipt_id = non_empty(Some(&request.receipt_id), "receipt_id")?;
    let reset_token = non_empty(Some(&request.reset_token), "reset_token")?;
    let reset_receipt_id = receipt_id("reset");
    let reset_at = now_rfc3339();
    let fault = sqlx::query(
        "SELECT control_id, target_id, scope, fault_kind, applied_at, prior_state,
                terminal_state, reset_token
         FROM fixture_faults
         WHERE receipt_id = ? AND mutation_generation = ? AND generation_id = ?
           AND manifest_digest = ? AND status = 'ACTIVE'",
    )
    .bind(&fault_receipt_id)
    .bind(mutation_generation)
    .bind(&generation_id)
    .bind(&manifest_digest)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow!("fault receipt is stale, already reset, or unknown"))?;
    let stored_token: String = fault.try_get("reset_token")?;
    if stored_token != reset_token {
        bail!("reset token does not match the fault receipt")
    }
    let target_id: String = fault.try_get("target_id")?;
    let target = sqlx::query(
        "SELECT control_id, instance_state, instance_type, initial_state, initial_type
         FROM fixture_mutation_resources
         WHERE resource_id = ? AND mutation_generation = ? AND generation_id = ?
           AND retired_at IS NULL",
    )
    .bind(&target_id)
    .bind(mutation_generation)
    .bind(&generation_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow!("fault target is absent from the active generation"))?;
    let prior_terminal_state = format!(
        "{}:{}",
        target.try_get::<String, _>("instance_state")?,
        target.try_get::<String, _>("instance_type")?
    );
    let recorded_terminal_state: String = fault.try_get("terminal_state")?;
    if prior_terminal_state != recorded_terminal_state {
        bail!("fault target state changed; reset requires the recorded terminal state")
    }
    let control_id: String = target.try_get("control_id")?;
    let scenario = mutation::scenario_for_control(&control_id)?;
    let initial_state: String = target.try_get("initial_state")?;
    let initial_type: String = target.try_get("initial_type")?;
    let intent_id = begin_mutation_intent(
        pool,
        "reset",
        IntentContext {
            mutation_generation: Some(mutation_generation),
            generation_id: Some(&generation_id),
            fixture_generation: request.authority.generation,
            target_id: Some(&target_id),
        },
        &serde_json::to_value(&request)?,
        &reset_at,
    )
    .await?;
    if let Err(error) = mark_intent_dispatched(pool, &intent_id, &now_rfc3339()).await {
        finalize_intent_ambiguous(pool, &intent_id, &error).await;
        return Err(error);
    }
    let operation_result = async {
        let backend =
            match mutation_backend_for_generation(pool, mutation_generation, &generation_id).await {
                Ok(backend) => backend,
                Err(error) => {
                    finalize_intent_ambiguous(pool, &intent_id, &error).await;
                    return Err(error);
                }
            };
        let observed = match backend.reset_setup_fault(&target_id, scenario).await {
            Ok(observed) => observed,
            Err(error) => {
                finalize_intent_ambiguous(pool, &intent_id, &error).await;
                return Err(error);
            }
        };
        if observed.instance_state != initial_state || observed.instance_type != initial_type {
            let error = anyhow!(
                "public EC2 state after reset was {}:{}, expected {}:{}",
                observed.instance_state,
                observed.instance_type,
                initial_state,
                initial_type
            );
            finalize_intent_ambiguous(pool, &intent_id, &error).await;
            return Err(error);
        }
        let mut tx = pool.begin().await?;
        let target_updated = sqlx::query(
            "UPDATE fixture_mutation_resources SET instance_state = ?, instance_type = ?,
                    external_status = 'RESTORED', external_identity_verified = 1
             WHERE resource_id = ? AND mutation_generation = ? AND generation_id = ?
               AND instance_state = ? AND instance_type = ? AND retired_at IS NULL",
        )
        .bind(&initial_state)
        .bind(&initial_type)
        .bind(&target_id)
        .bind(mutation_generation)
        .bind(&generation_id)
        .bind(target.try_get::<String, _>("instance_state")?)
        .bind(target.try_get::<String, _>("instance_type")?)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let fault_updated = sqlx::query(
            "UPDATE fixture_faults SET status = 'RESET', reset_at = ?, reset_receipt_id = ?
             WHERE receipt_id = ? AND mutation_generation = ? AND generation_id = ? AND status = 'ACTIVE'",
        )
        .bind(&reset_at)
        .bind(&reset_receipt_id)
        .bind(&fault_receipt_id)
        .bind(mutation_generation)
        .bind(&generation_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if target_updated != 1 || fault_updated != 1 {
            let error =
                anyhow!("reset finalization lost the fault or target row to a concurrent operation");
            finalize_intent_ambiguous(pool, &intent_id, &error).await;
            return Err(error);
        }
        let receipt = json!({
            "schema": "foxtail.release-fixture-reset-receipt/v1",
            "operation": "reset",
            "status": "RESET",
            "receipt_id": reset_receipt_id,
            "fault_receipt_id": fault_receipt_id,
            "generation": request.authority.generation,
            "manifest_digest": manifest_digest,
            "mutation_generation": mutation_generation,
            "mutation_generation_id": generation_id,
            "control_id": control_id,
            "target_id": target_id,
            "scope": fault.try_get::<String, _>("scope")?,
            "fault_kind": fault.try_get::<String, _>("fault_kind")?,
            "prior_state": prior_terminal_state,
            "terminal_state": format!("{initial_state}:{initial_type}"),
            "reset_at": reset_at,
            "reset_token_consumed": true
        });
        let receipt_bytes = canonical_receipt(receipt)?;
        persist_receipt(
            &mut tx,
            "reset",
            &reset_receipt_id,
            ReceiptContext {
                mutation_generation: Some(mutation_generation),
                generation_id: Some(&generation_id),
                manifest_digest: Some(&manifest_digest),
            },
            &receipt_bytes,
            &reset_at,
        )
        .await?;
        tx.commit().await?;
        Ok::<Vec<u8>, anyhow::Error>(receipt_bytes)
    }
    .await;
    match operation_result {
        Ok(receipt_bytes) => {
            match update_mutation_intent(pool, &intent_id, "SUCCEEDED", None, &now_rfc3339()).await
            {
                Ok(()) => Ok(receipt_bytes),
                Err(error) => {
                    finalize_intent_ambiguous(pool, &intent_id, &error).await;
                    Err(error)
                }
            }
        }
        Err(error) => {
            finalize_intent_ambiguous(pool, &intent_id, &error).await;
            Err(error)
        }
    }
}

pub async fn recreate(pool: &SqlitePool, request: RecreateRequest) -> Result<Vec<u8>> {
    let (_, old_manifest_digest, old_mutation_generation, old_generation_id) =
        validate_mutation_authority(pool, &request.authority).await?;
    let old_targets = mutation_target_values(pool, old_mutation_generation).await?;
    let old_manifest_bytes = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT manifest_bytes FROM fixture_realizations WHERE singleton_id = 1",
    )
    .fetch_one(pool)
    .await?;
    let old_manifest: Value = serde_json::from_slice(&old_manifest_bytes)?;
    let old_environment = old_manifest
        .get("environment")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("active fixture manifest has no environment"))?;
    let old_clock_anchor = old_manifest
        .pointer("/clock/anchor")
        .and_then(Value::as_str)
        .map(str::to_string);
    let anchor = request.clock_anchor.clone().or(old_clock_anchor);
    let active_faults: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fixture_faults
         WHERE mutation_generation = ? AND generation_id = ? AND status = 'ACTIVE'",
    )
    .bind(old_mutation_generation)
    .bind(&old_generation_id)
    .fetch_one(pool)
    .await?;
    if active_faults != 0 {
        bail!("cannot realize or recreate while a mutation fault is active; reset it first");
    }
    let predicted_generation: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(value), 0) + 1 FROM (
            SELECT generation AS value FROM fixture_realizations
            UNION ALL SELECT fixture_generation AS value FROM fixture_mutation_generations
        )",
    )
    .fetch_one(pool)
    .await?;
    let predicted_mutation_generation: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(mutation_generation), 0) + 1 FROM fixture_mutation_generations",
    )
    .fetch_one(pool)
    .await?;
    let predicted_generation_id = mutation_generation_id(predicted_mutation_generation);
    let old_endpoint = old_environment
        .get("aws_endpoint_url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("active fixture manifest endpoint is missing"))?;
    let old_account = old_environment
        .get("account_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("active fixture manifest account is missing"))?;
    let old_region = old_environment
        .get("region")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("active fixture manifest region is missing"))?;
    // Do the account-bound STS preflight before either recreate intent is
    // persisted. A changed endpoint/account therefore cannot leave a partial
    // recreation ledger behind.
    let _preflight_backend =
        mutation::Ec2MutationBackend::connect(old_endpoint, old_region, old_account)
            .await
            .with_context(|| format!("verify recreate account through {old_endpoint}"))?;
    let created_at = now_rfc3339();
    let intent_id = begin_mutation_intent(
        pool,
        "recreate",
        IntentContext {
            mutation_generation: Some(old_mutation_generation),
            generation_id: Some(&old_generation_id),
            fixture_generation: Some(predicted_generation),
            target_id: None,
        },
        &serde_json::to_value(&request)?,
        &created_at,
    )
    .await?;
    if let Err(error) = mark_intent_dispatched(pool, &intent_id, &now_rfc3339()).await {
        finalize_intent_ambiguous(pool, &intent_id, &error).await;
        return Err(error);
    }
    let new_intent_id = match begin_mutation_intent(
        pool,
        "recreate",
        IntentContext {
            mutation_generation: Some(predicted_mutation_generation),
            generation_id: Some(&predicted_generation_id),
            fixture_generation: Some(predicted_generation),
            target_id: None,
        },
        &serde_json::to_value(&request)?,
        &created_at,
    )
    .await
    {
        Ok(intent_id) => intent_id,
        Err(error) => {
            finalize_intent_ambiguous(pool, &intent_id, &error).await;
            return Err(error);
        }
    };
    let mut new_identity = None;
    let operation_result = async {
        let snapshot = realize(
            pool,
            RealizeRequest {
                version: request.authority.version.clone(),
                clock_anchor: anchor,
                account_id: old_environment
                    .get("account_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                region: old_environment
                    .get("region")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                endpoint_url: old_environment
                    .get("aws_endpoint_url")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                localstack_version: old_environment
                    .get("localstack_version")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                force_new: true,
                reuse_intent_id: Some(new_intent_id.clone()),
                defer_intent_finalization: true,
                allowed_intent_ids: vec![intent_id.clone(), new_intent_id.clone()],
            },
        )
        .await;
        let snapshot = match snapshot {
            Ok(snapshot) => snapshot,
            Err(error) => {
                finalize_intent_ambiguous(pool, &intent_id, &error).await;
                return Err(error);
            }
        };
        let new_manifest: Value = serde_json::from_slice(&snapshot.manifest_bytes)?;
        let new_targets = new_manifest
            .get("mutation_resources")
            .cloned()
            .unwrap_or_else(|| json!([]));
        let new_mutation_generation = new_manifest
            .get("mutation_generation")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("recreate did not produce an isolated mutation generation"))?;
        let new_generation_id = new_manifest
            .get("mutation_generation_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("recreate did not produce a mutation generation id"))?;
        new_identity = Some((new_mutation_generation, new_generation_id.to_string()));
        if new_mutation_generation != predicted_mutation_generation
            || new_generation_id != predicted_generation_id
        {
            let error =
                anyhow!("concurrent realization changed the mutation generation allocation");
            finalize_intent_ambiguous(pool, &intent_id, &error).await;
            return Err(error);
        }
        let old_ids = old_targets
            .iter()
            .filter_map(|target| target.get("resource_id").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let old_backend =
            mutation_backend_for_generation(pool, old_mutation_generation, &old_generation_id)
                .await?;
        let external_ec2_termination = match old_backend.terminate_all(&old_ids).await {
            Ok(evidence) => evidence,
            Err(error) => {
                let error =
                    anyhow!("recreate cleanup ambiguous; returned_ids={old_ids:?}; {error}");
                mark_generation_ambiguous(pool, old_mutation_generation, &old_generation_id).await;
                mark_generation_ambiguous(pool, new_mutation_generation, new_generation_id).await;
                finalize_intent_ambiguous(pool, &intent_id, &error).await;
                return Err(error);
            }
        };
        let receipt_id = receipt_id("recreate");
        let receipt = json!({
            "schema": "foxtail.release-fixture-recreate-receipt/v1",
            "operation": "recreate",
            "status": "RECREATED",
            "receipt_id": receipt_id,
            "prior": {
                "manifest_digest": old_manifest_digest,
                "mutation_generation": old_mutation_generation,
                "mutation_generation_id": old_generation_id,
                "targets": old_targets,
                "external_ec2_termination": external_ec2_termination
            },
            "terminal": {
                "manifest_digest": snapshot.manifest_digest,
                "mutation_generation": new_manifest.get("mutation_generation"),
                "mutation_generation_id": new_manifest.get("mutation_generation_id"),
                "targets": new_targets
            },
            "created_at": created_at,
            "identities_replaced": true
        });
        let receipt_bytes = canonical_receipt(receipt)?;
        let mut tx = pool.begin().await?;
        let old_retired = sqlx::query(
            "UPDATE fixture_mutation_resources SET retired_at = ?, external_status = 'DESTROYED'
         WHERE mutation_generation = ? AND generation_id = ? AND retired_at IS NULL",
        )
        .bind(&created_at)
        .bind(old_mutation_generation)
        .bind(&old_generation_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if old_retired != old_ids.len() as u64 {
            let error = anyhow!("recreate lost an old mutation target row during retirement");
            finalize_intent_ambiguous(pool, &intent_id, &error).await;
            return Err(error);
        }
        for id in &old_ids {
            sqlx::query("DELETE FROM metrics WHERE resource_id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM cost_records WHERE resource_id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM resources WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
        "UPDATE fixture_mutation_generations SET state = 'DESTROYED', external_status = 'DESTROYED',
                public_absence = ?, destroyed_at = ?
         WHERE mutation_generation = ? AND generation_id = ? AND state = 'ACTIVE'",
    )
    .bind(serde_json::to_string(&json!({
        "checked_at": created_at,
        "resource_ids": &old_ids,
        "all_absent": true,
        "absent_count": old_ids.len()
    }))?)
    .bind(&created_at)
    .bind(old_mutation_generation)
    .bind(&old_generation_id)
    .execute(&mut *tx)
    .await?;
        persist_receipt(
            &mut tx,
            "recreate",
            &receipt_id,
            ReceiptContext {
                mutation_generation: new_manifest
                    .get("mutation_generation")
                    .and_then(Value::as_i64),
                generation_id: new_manifest
                    .get("mutation_generation_id")
                    .and_then(Value::as_str),
                manifest_digest: Some(snapshot.manifest_digest.as_str()),
            },
            &receipt_bytes,
            &created_at,
        )
        .await?;
        tx.commit().await?;
        Ok::<Vec<u8>, anyhow::Error>(receipt_bytes)
    }
    .await;
    match operation_result {
        Ok(receipt_bytes) => {
            if let Err(error) =
                update_mutation_intent(pool, &intent_id, "SUCCEEDED", None, &now_rfc3339()).await
            {
                if let Some((new_mutation_generation, new_generation_id)) = &new_identity {
                    mark_generation_ambiguous(pool, old_mutation_generation, &old_generation_id)
                        .await;
                    mark_generation_ambiguous(pool, *new_mutation_generation, new_generation_id)
                        .await;
                }
                finalize_intent_ambiguous(pool, &intent_id, &error).await;
                finalize_intent_ambiguous(pool, &new_intent_id, &error).await;
                return Err(error);
            }
            if let Err(error) =
                update_mutation_intent(pool, &new_intent_id, "SUCCEEDED", None, &now_rfc3339())
                    .await
            {
                if let Some((new_mutation_generation, new_generation_id)) = &new_identity {
                    mark_generation_ambiguous(pool, *new_mutation_generation, new_generation_id)
                        .await;
                }
                finalize_intent_ambiguous(pool, &new_intent_id, &error).await;
                return Err(error);
            }
            Ok(receipt_bytes)
        }
        Err(error) => {
            if let Some((new_mutation_generation, new_generation_id)) = &new_identity {
                mark_generation_ambiguous(pool, old_mutation_generation, &old_generation_id).await;
                mark_generation_ambiguous(pool, *new_mutation_generation, new_generation_id).await;
            }
            finalize_intent_ambiguous(pool, &intent_id, &error).await;
            finalize_intent_ambiguous(pool, &new_intent_id, &error).await;
            Err(error)
        }
    }
}

pub async fn destroy(pool: &SqlitePool, request: DestroyRequest) -> Result<Vec<u8>> {
    let (_, manifest_digest, mutation_generation, generation_id) =
        validate_mutation_authority(pool, &request.authority).await?;
    let destroyed_at = now_rfc3339();
    let destroy_receipt_id = receipt_id("destroy");
    let target_rows = sqlx::query(
        "SELECT resource_id, control_id, target_kind, instance_state, instance_type,
                restored_state, restored_type
         FROM fixture_mutation_resources
         WHERE mutation_generation = ? AND generation_id = ? AND retired_at IS NULL
         ORDER BY resource_id",
    )
    .bind(mutation_generation)
    .bind(&generation_id)
    .fetch_all(pool)
    .await?;
    let target_records = target_rows
        .iter()
        .map(|row| {
            Ok(json!({
                "resource_id": row.try_get::<String, _>("resource_id")?,
                "control_id": row.try_get::<String, _>("control_id")?,
                "target_kind": row.try_get::<String, _>("target_kind")?,
                "prior_state": format!(
                    "{}:{}",
                    row.try_get::<String, _>("instance_state")?,
                    row.try_get::<String, _>("instance_type")?
                ),
                "terminal_state": format!(
                    "{}:{}",
                    row.try_get::<String, _>("restored_state")?,
                    row.try_get::<String, _>("restored_type")?
                )
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    let target_ids = target_records
        .iter()
        .filter_map(|record| record.get("resource_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let active_faults = sqlx::query(
        "SELECT receipt_id, target_id, control_id, scope, fault_kind, applied_at,
                prior_state, terminal_state, reset_token FROM fixture_faults
         WHERE mutation_generation = ? AND generation_id = ? AND status = 'ACTIVE'
         ORDER BY receipt_id",
    )
    .bind(mutation_generation)
    .bind(&generation_id)
    .fetch_all(pool)
    .await?;
    let intent_id = begin_mutation_intent(
        pool,
        "destroy",
        IntentContext {
            mutation_generation: Some(mutation_generation),
            generation_id: Some(&generation_id),
            fixture_generation: request.authority.generation,
            target_id: None,
        },
        &serde_json::to_value(&request)?,
        &destroyed_at,
    )
    .await?;
    if let Err(error) = mark_intent_dispatched(pool, &intent_id, &now_rfc3339()).await {
        finalize_intent_ambiguous(pool, &intent_id, &error).await;
        return Err(error);
    }
    let operation_result = async {
        let backend =
            match mutation_backend_for_generation(pool, mutation_generation, &generation_id).await {
                Ok(backend) => backend,
                Err(error) => {
                    finalize_intent_ambiguous(pool, &intent_id, &error).await;
                    return Err(error);
                }
            };
    let mut reset_receipts = Vec::new();
    for fault in &active_faults {
        let fault_receipt_id: String = fault.try_get("receipt_id")?;
        let target_id: String = fault.try_get("target_id")?;
        let target = sqlx::query(
            "SELECT control_id, initial_state, initial_type FROM fixture_mutation_resources
             WHERE resource_id = ? AND mutation_generation = ?",
        )
        .bind(&target_id)
        .bind(mutation_generation)
        .fetch_one(pool)
        .await?;
        let control_id: String = target.try_get("control_id")?;
        let initial_state: String = target.try_get("initial_state")?;
        let initial_type: String = target.try_get("initial_type")?;
        let scenario = mutation::scenario_for_control(&control_id)?;
        let public_before = match backend.describe_instance(&target_id).await {
            Ok(observed) => observed,
            Err(error) => {
                let _ = update_mutation_intent(
                    pool,
                    &intent_id,
                    "AMBIGUOUS",
                    Some(&error.to_string()),
                    &now_rfc3339(),
                )
                .await;
                return Err(error);
            }
        };
        let recorded_terminal_state: String = fault.try_get("terminal_state")?;
        if format!(
            "{}:{}",
            public_before.instance_state, public_before.instance_type
        ) != recorded_terminal_state
        {
            let error = anyhow!(
                "destroy reset observed public state {}:{} but receipt recorded {}",
                public_before.instance_state,
                public_before.instance_type,
                recorded_terminal_state
            );
            let _ = update_mutation_intent(
                pool,
                &intent_id,
                "AMBIGUOUS",
                Some(&error.to_string()),
                &now_rfc3339(),
            )
            .await;
            return Err(error);
        }
        let observed = match backend.reset_setup_fault(&target_id, scenario).await {
            Ok(observed) => observed,
            Err(error) => {
                let _ = update_mutation_intent(
                    pool,
                    &intent_id,
                    "AMBIGUOUS",
                    Some(&error.to_string()),
                    &now_rfc3339(),
                )
                .await;
                return Err(error);
            }
        };
        if observed.instance_state != initial_state || observed.instance_type != initial_type {
            let error = anyhow!(
                "public EC2 state after destroy reset was {}:{}, expected {}:{}",
                observed.instance_state,
                observed.instance_type,
                initial_state,
                initial_type
            );
            let _ = update_mutation_intent(
                pool,
                &intent_id,
                "AMBIGUOUS",
                Some(&error.to_string()),
                &now_rfc3339(),
            )
            .await;
            return Err(error);
        }
        let reset_id = receipt_id("reset");
        let reset_receipt = json!({
            "schema": "foxtail.release-fixture-reset-receipt/v1",
            "operation": "reset",
            "status": "RESET",
            "receipt_id": reset_id,
            "fault_receipt_id": fault_receipt_id,
            "generation": request.authority.generation,
            "manifest_digest": manifest_digest,
            "mutation_generation": mutation_generation,
            "mutation_generation_id": generation_id,
            "target_id": target_id,
            "control_id": control_id,
            "scope": fault.try_get::<String, _>("scope")?,
            "fault_kind": fault.try_get::<String, _>("fault_kind")?,
            "applied_at": fault.try_get::<String, _>("applied_at")?,
            "prior_state": format!("{}:{}", public_before.instance_state, public_before.instance_type),
            "terminal_state": format!("{}:{}", observed.instance_state, observed.instance_type),
            "reset_at": destroyed_at,
            "reset_token_consumed": true
        });
        reset_receipts.push(reset_receipt);
    }
    let external_ec2_termination = match backend.terminate_all(&target_ids).await {
        Ok(evidence) => evidence,
        Err(error) => {
            let _ = update_mutation_intent(
                pool,
                &intent_id,
                "AMBIGUOUS",
                Some(&error.to_string()),
                &now_rfc3339(),
            )
            .await;
            return Err(error);
        }
    };
    let mut tx = pool.begin().await?;
    for fault in &active_faults {
        let fault_receipt_id: String = fault.try_get("receipt_id")?;
        let reset_id = reset_receipts
            .iter()
            .find(|receipt| receipt["fault_receipt_id"].as_str() == Some(fault_receipt_id.as_str()))
            .and_then(|receipt| receipt["receipt_id"].as_str())
            .ok_or_else(|| anyhow!("destroy reset receipt id is missing"))?;
        let target_id: String = fault.try_get("target_id")?;
        let target_updated = sqlx::query(
            "UPDATE fixture_mutation_resources SET instance_state = initial_state,
                    instance_type = initial_type, external_status = 'RESTORED',
                    external_identity_verified = 1
             WHERE resource_id = ? AND mutation_generation = ? AND generation_id = ?
               AND retired_at IS NULL",
        )
        .bind(&target_id)
        .bind(mutation_generation)
        .bind(&generation_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let fault_updated = sqlx::query(
            "UPDATE fixture_faults SET status = 'RESET', reset_at = ?, reset_receipt_id = ?
             WHERE receipt_id = ? AND mutation_generation = ? AND generation_id = ? AND status = 'ACTIVE'",
        )
        .bind(&destroyed_at)
        .bind(reset_id)
        .bind(&fault_receipt_id)
        .bind(mutation_generation)
        .bind(&generation_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if target_updated != 1 || fault_updated != 1 {
            let error =
                anyhow!("destroy lost an active fault or target row during reset finalization");
            let _ = update_mutation_intent(
                pool,
                &intent_id,
                "AMBIGUOUS",
                Some(&error.to_string()),
                &now_rfc3339(),
            )
            .await;
            return Err(error);
        }
        let reset_receipt = reset_receipts
            .iter()
            .find(|receipt| receipt["receipt_id"].as_str() == Some(reset_id))
            .ok_or_else(|| anyhow!("destroy reset receipt is missing"))?;
        let reset_bytes = canonical_receipt(reset_receipt.clone())?;
        persist_receipt(
            &mut tx,
            "reset",
            reset_id,
            ReceiptContext {
                mutation_generation: Some(mutation_generation),
                generation_id: Some(&generation_id),
                manifest_digest: Some(&manifest_digest),
            },
            &reset_bytes,
            &destroyed_at,
        )
        .await?;
    }
    for id in &target_ids {
        sqlx::query("DELETE FROM metrics WHERE resource_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM cost_records WHERE resource_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM resources WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "UPDATE fixture_mutation_resources SET retired_at = ?, external_status = 'DESTROYED',
                external_identity_verified = 1
         WHERE mutation_generation = ? AND generation_id = ? AND retired_at IS NULL",
    )
    .bind(&destroyed_at)
    .bind(mutation_generation)
    .bind(&generation_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let remaining_inventory_count: i64 = if target_ids.is_empty() {
        0
    } else {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM resources WHERE id IN (SELECT resource_id FROM fixture_mutation_resources WHERE mutation_generation = ?)",
        )
        .bind(mutation_generation)
        .fetch_one(&mut *tx)
        .await?
    };
    if remaining_inventory_count != 0 {
        bail!("destroy could not prove all mutation identities absent from public inventory")
    }
    let absent_count = target_ids.len();
    sqlx::query(
        "UPDATE fixture_mutation_generations SET state = 'DESTROYED', external_status = 'DESTROYED',
                public_absence = ?, destroyed_at = ?
         WHERE mutation_generation = ? AND generation_id = ? AND state = 'ACTIVE'",
    )
    .bind(serde_json::to_string(&json!({
        "checked_at": destroyed_at,
        "resource_ids": &target_ids,
        "all_absent": true,
        "absent_count": target_ids.len()
    }))?)
    .bind(&destroyed_at)
    .bind(mutation_generation)
    .bind(&generation_id)
    .execute(&mut *tx)
    .await?;
    let receipt = json!({
        "schema": "foxtail.release-fixture-destroy-receipt/v1",
        "operation": "destroy",
        "status": "DESTROYED",
        "receipt_id": destroy_receipt_id,
        "generation": request.authority.generation,
        "manifest_digest": manifest_digest,
        "mutation_generation": mutation_generation,
        "mutation_generation_id": generation_id,
        "faults_reset": reset_receipts,
        "targets_destroyed": target_records,
        "external_ec2_termination": external_ec2_termination,
        "public_inventory_absence": {
            "checked": true,
            "all_absent": true,
            "absent_count": absent_count,
            "resource_ids": &target_ids
        },
        "destroyed_at": destroyed_at
    });
    let receipt_bytes = canonical_receipt(receipt)?;
    persist_receipt(
        &mut tx,
        "destroy",
        &destroy_receipt_id,
        ReceiptContext {
            mutation_generation: Some(mutation_generation),
            generation_id: Some(&generation_id),
            manifest_digest: Some(&manifest_digest),
        },
        &receipt_bytes,
        &destroyed_at,
    )
    .await?;
    sqlx::query(
        "DELETE FROM fixture_realizations
         WHERE singleton_id = 1 AND mutation_generation = ? AND mutation_generation_id = ?",
    )
    .bind(mutation_generation)
    .bind(&generation_id)
    .execute(&mut *tx)
    .await?;
        tx.commit().await?;
        Ok::<Vec<u8>, anyhow::Error>(receipt_bytes)
    }
    .await;
    match operation_result {
        Ok(receipt_bytes) => {
            match update_mutation_intent(pool, &intent_id, "SUCCEEDED", None, &now_rfc3339()).await
            {
                Ok(()) => Ok(receipt_bytes),
                Err(error) => {
                    finalize_intent_ambiguous(pool, &intent_id, &error).await;
                    Err(error)
                }
            }
        }
        Err(error) => {
            finalize_intent_ambiguous(pool, &intent_id, &error).await;
            Err(error)
        }
    }
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
struct ResourceIdentity {
    id: String,
    region: String,
    scenario: String,
    instance_state: String,
    instance_type: String,
    availability_zone: Option<String>,
    tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct MetricObservation {
    namespace: String,
    metric_name: String,
    seconds_from_now: i64,
    value: f64,
}

#[derive(Debug, Clone)]
struct CostObservation {
    seconds_from_now: i64,
    amount: f64,
}

#[derive(Debug, Clone)]
struct EstateResource {
    id: String,
    region: String,
    scenario: String,
    instance_state: String,
    instance_type: String,
    availability_zone: Option<String>,
    tags: BTreeMap<String, String>,
    avg_cpu: Option<f64>,
    metric_count: i64,
    cost_record_count: i64,
    metrics: Vec<MetricObservation>,
    costs: Vec<CostObservation>,
}

struct ManifestContext<'a> {
    definition_digest: &'a str,
    assigned: &'a BTreeMap<String, EstateResource>,
    complete_resources: &'a BTreeMap<String, EstateResource>,
    region: &'a str,
    account_id: &'a str,
    endpoint_url: &'a str,
    localstack_version: &'a str,
    source_revision: &'a str,
    anchor: DateTime<Utc>,
    generation: i64,
    read_only_fingerprint: &'a str,
    complete_fingerprint: &'a str,
    mutation_generation: Option<i64>,
    mutation_generation_id: Option<&'a str>,
    mutation_targets: &'a [Value],
}

fn assign_realized_resources(resources: &[ResourceIdentity]) -> BTreeMap<String, ResourceIdentity> {
    MATERIALIZATION_PROFILES
        .iter()
        .zip(resources.iter().take(MATERIALIZATION_PROFILES.len()))
        .map(|(profile, resource)| (profile.control_id.to_string(), resource.clone()))
        .collect()
}

fn history_offsets() -> BTreeSet<i64> {
    (0..HISTORY_DAYS)
        .map(|day| -(day * DAY_SECONDS + HOUR_SECONDS))
        .collect()
}

fn missing_history_offsets(resource: &EstateResource) -> Vec<i64> {
    let observed = resource
        .metrics
        .iter()
        .filter(|metric| metric.namespace == "AWS/EC2" && metric.metric_name == "CPUUtilization")
        .map(|metric| metric.seconds_from_now)
        .collect::<BTreeSet<_>>();
    history_offsets().difference(&observed).copied().collect()
}

async fn materialize_control_evidence(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    control_id: &str,
    resource_id: &str,
) -> Result<()> {
    let profile = materialization_profile(control_id)?;
    sqlx::query("DELETE FROM metrics WHERE resource_id = ?")
        .bind(resource_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM cost_records WHERE resource_id = ?")
        .bind(resource_id)
        .execute(&mut **tx)
        .await?;

    for day in 0..HISTORY_DAYS {
        let offset = -(day * DAY_SECONDS + HOUR_SECONDS);
        if profile.missing_cpu_day != Some(day) {
            insert_metric(tx, resource_id, "CPUUtilization", offset, profile.cpu_value).await?;
        }
        insert_metric(
            tx,
            resource_id,
            "NetworkIn",
            offset,
            NETWORK_IN_BASE + (day as f64 * NETWORK_PER_DAY_INCREMENT),
        )
        .await?;
        insert_metric(
            tx,
            resource_id,
            "NetworkOut",
            offset,
            NETWORK_OUT_BASE + (day as f64 * NETWORK_PER_DAY_INCREMENT),
        )
        .await?;
        sqlx::query(
            "INSERT INTO cost_records (resource_id, seconds_from_now, amount)
             VALUES (?, ?, ?)",
        )
        .bind(resource_id)
        .bind(offset)
        .bind(profile.cost_amount)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn insert_metric(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    resource_id: &str,
    metric_name: &str,
    seconds_from_now: i64,
    value: f64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO metrics (resource_id, namespace, metric_name, seconds_from_now, value)
         VALUES (?, 'AWS/EC2', ?, ?, ?)",
    )
    .bind(resource_id)
    .bind(metric_name)
    .bind(seconds_from_now)
    .bind(value)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn load_estate_resources(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    identities: &[ResourceIdentity],
) -> Result<Vec<EstateResource>> {
    let mut resources = Vec::with_capacity(identities.len());
    for identity in identities {
        let metric_rows = sqlx::query(
            "SELECT namespace, metric_name, seconds_from_now, value
             FROM metrics
             WHERE resource_id = ?
             ORDER BY namespace, metric_name, seconds_from_now",
        )
        .bind(&identity.id)
        .fetch_all(&mut **tx)
        .await?;
        let metrics = metric_rows
            .into_iter()
            .map(|row| {
                Ok(MetricObservation {
                    namespace: row.try_get("namespace")?,
                    metric_name: row.try_get("metric_name")?,
                    seconds_from_now: row.try_get("seconds_from_now")?,
                    value: row.try_get("value")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let cost_rows = sqlx::query(
            "SELECT seconds_from_now, amount
             FROM cost_records
             WHERE resource_id = ?
             ORDER BY seconds_from_now",
        )
        .bind(&identity.id)
        .fetch_all(&mut **tx)
        .await?;
        let costs = cost_rows
            .into_iter()
            .map(|row| {
                Ok(CostObservation {
                    seconds_from_now: row.try_get("seconds_from_now")?,
                    amount: row.try_get("amount")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let cpu_values = metrics
            .iter()
            .filter(|metric| {
                metric.namespace == "AWS/EC2" && metric.metric_name == "CPUUtilization"
            })
            .map(|metric| metric.value)
            .collect::<Vec<_>>();
        let avg_cpu = if cpu_values.is_empty() {
            None
        } else {
            Some(cpu_values.iter().sum::<f64>() / cpu_values.len() as f64)
        };
        resources.push(EstateResource {
            id: identity.id.clone(),
            region: identity.region.clone(),
            scenario: identity.scenario.clone(),
            instance_state: identity.instance_state.clone(),
            instance_type: identity.instance_type.clone(),
            availability_zone: identity.availability_zone.clone(),
            tags: identity.tags.clone(),
            avg_cpu,
            metric_count: metrics.len() as i64,
            cost_record_count: costs.len() as i64,
            metrics,
            costs,
        });
    }
    Ok(resources)
}

fn validate_realized_controls(assigned: &BTreeMap<String, EstateResource>) -> Result<()> {
    let expected_offsets = history_offsets();
    for (control_id, resource) in assigned {
        let profile = materialization_profile(control_id)?;
        let cpu_offsets = resource
            .metrics
            .iter()
            .filter(|metric| {
                metric.namespace == "AWS/EC2" && metric.metric_name == "CPUUtilization"
            })
            .map(|metric| metric.seconds_from_now)
            .collect::<BTreeSet<_>>();
        let cost_offsets = resource
            .costs
            .iter()
            .map(|cost| cost.seconds_from_now)
            .collect::<BTreeSet<_>>();
        if cost_offsets != expected_offsets
            || resource.costs.iter().any(|cost| {
                !cost.amount.is_finite() || cost.amount <= 0.0 || cost.amount != profile.cost_amount
            })
        {
            bail!("{control_id} does not have complete positive cost evidence")
        }
        let average_cpu = resource
            .avg_cpu
            .ok_or_else(|| anyhow!("{control_id} has no CPUUtilization evidence"))?;
        if !average_cpu.is_finite() {
            bail!("{control_id} has a non-finite CPUUtilization average")
        }
        if resource.metrics.iter().any(|metric| {
            metric.namespace == "AWS/EC2"
                && metric.metric_name == "CPUUtilization"
                && metric.value != profile.cpu_value
        }) {
            bail!("{control_id} has a CPUUtilization value outside its published profile")
        }
        let expected_missing_offsets = profile
            .missing_cpu_day
            .map(|day| -(day * DAY_SECONDS + HOUR_SECONDS))
            .into_iter()
            .collect::<Vec<_>>();
        match control_id.as_str() {
            "ec2-idle-positive-001" | "ec2-resize-positive-001" => {
                if average_cpu >= LOW_CPU_MAX_EXCLUSIVE || cpu_offsets != expected_offsets {
                    bail!("{control_id} is not a complete low-utilization control")
                }
            }
            "ec2-idle-negative-001" => {
                if average_cpu <= BUSY_CPU_MIN_EXCLUSIVE || cpu_offsets != expected_offsets {
                    bail!("{control_id} is not a complete busy-utilization control")
                }
            }
            "ec2-idle-degraded-001" => {
                if average_cpu >= LOW_CPU_MAX_EXCLUSIVE
                    || cpu_offsets.len() != (HISTORY_DAYS - 1) as usize
                    || missing_history_offsets(resource) != expected_missing_offsets
                {
                    bail!("{control_id} does not have exactly one scoped missing CPU history day")
                }
            }
            "ec2-resize-negative-001" => {
                if !(OPTIMIZED_CPU_MIN_INCLUSIVE..=OPTIMIZED_CPU_MAX_INCLUSIVE)
                    .contains(&average_cpu)
                    || cpu_offsets != expected_offsets
                {
                    bail!("{control_id} is not a complete optimized resize control")
                }
            }
            _ => bail!("unexpected realized control '{control_id}'"),
        }
    }
    Ok(())
}

fn resource_arn(region: &str, account_id: &str, resource_id: &str) -> String {
    format!("arn:aws:ec2:{region}:{account_id}:instance/{resource_id}")
}

fn estate_fingerprint(
    resources: &BTreeMap<String, EstateResource>,
    region: &str,
    account_id: &str,
) -> Result<String> {
    let rows = resources
        .iter()
        .map(|(control_id, resource)| {
            let (role, scenario) = role_and_intent(control_id);
            json!({
                "control_id": control_id,
                "resource_id": resource.id,
                "resource_type": "ec2",
                "region": resource.region,
                "scenario": resource.scenario,
                "instance_state": resource.instance_state,
                "instance_type": resource.instance_type,
                "availability_zone": resource
                    .availability_zone
                    .clone()
                    .unwrap_or_else(|| format!("{}a", region)),
                "tags": observation_tags(resource, control_id, role, scenario),
                "metrics": resource.metrics.iter().map(|metric| json!({
                    "namespace": metric.namespace,
                    "metric_name": metric.metric_name,
                    "seconds_from_now": metric.seconds_from_now,
                    "value": metric.value
                })).collect::<Vec<_>>(),
                "cost_records": resource.costs.iter().map(|cost| json!({
                    "seconds_from_now": cost.seconds_from_now,
                    "amount": cost.amount
                })).collect::<Vec<_>>()
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
        mutation_generation,
        mutation_generation_id,
        mutation_targets,
    } = context;
    let resource_entries = assigned
        .iter()
        .map(|(control_id, resource)| {
            let (role, scenario_intent) = role_and_intent(control_id);
            let tags = observation_tags(resource, control_id, role, scenario_intent);
            json!({
                "control_id": control_id,
                "role": role,
                "resource_id": resource.id,
                "resource_type": "ec2",
                "aws_identity": resource_arn(region, account_id, &resource.id),
                "scenario": scenario_intent,
                "evidence": evidence_declaration(control_id, resource),
                "observed": {
                    "instance_state": resource.instance_state,
                    "instance_type": resource.instance_type,
                    "availability_zone": resource
                        .availability_zone
                        .clone()
                        .unwrap_or_else(|| format!("{}a", region)),
                    "tags": tags,
                    "metric_count": resource.metric_count,
                    "cost_record_count": resource.cost_record_count,
                    "average_cpu": resource.avg_cpu,
                    "metric_names": resource.metrics.iter().map(|metric| metric.metric_name.clone()).collect::<BTreeSet<_>>(),
                    "cpu_offsets": resource.metrics.iter()
                        .filter(|metric| metric.metric_name == "CPUUtilization")
                        .map(|metric| metric.seconds_from_now)
                        .collect::<BTreeSet<_>>(),
                    "cost_offsets": resource.costs.iter()
                        .map(|cost| cost.seconds_from_now)
                        .collect::<BTreeSet<_>>()
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
                    "resourcegroupstaggingapi.get-resources",
                    "cloudwatch.list-metrics",
                    "cloudwatch.get-metric-statistics",
                    "cost-explorer.get-cost-and-usage",
                    "compute-optimizer.get-ec2-instance-recommendations"
                ],
                "clock": {"anchor": anchor.to_rfc3339_opts(SecondsFormat::Secs, true), "required_history_days": HISTORY_DAYS},
                "metric": {"namespace": "AWS/EC2", "metric_name": "CPUUtilization"},
                "cost": {"metric": "UnblendedCost"}
            })
        })
        .collect::<Vec<_>>();

    let mut control_catalogue = resource_entries.clone();
    for (index, control_id) in MUTATION_CONTROL_IDS.into_iter().enumerate() {
        let (role, scenario_intent) = role_and_intent(control_id);
        let target = mutation_targets
            .iter()
            .find(|target| target.get("control_id").and_then(Value::as_str) == Some(control_id));
        control_catalogue.push(json!({
            "control_id": control_id,
            "role": role,
            "service": "ec2",
            "scenario": scenario_intent,
            "realization_status": if target.is_some() { "realized" } else { "declared-only" },
            "target_kind": MUTATION_TARGET_KINDS[index],
            "realization": target.cloned().unwrap_or_else(|| json!({
                "lifecycle": "qualification-only",
                "mutation_controls_declared": true
            }))
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
            "required_history_days": HISTORY_DAYS,
            "reusable_until": reusable_until.to_rfc3339_opts(SecondsFormat::Secs, true)
        },
        "generation": generation,
        "mutation_generation": mutation_generation,
        "mutation_generation_id": mutation_generation_id,
        "resources": resource_entries,
        "mutation_resources": mutation_targets,
        "evidence_declarations": evidence_declarations,
        "control_catalogue": control_catalogue,
        "fault_profiles": []
    }))
}

pub(crate) fn role_and_intent(control_id: &str) -> (&'static str, &'static str) {
    match control_id {
        "ec2-idle-positive-001" => ("positive", "ec2.idle.complete-history"),
        "ec2-idle-negative-001" => ("negative", "ec2.busy.complete-history"),
        "ec2-idle-degraded-001" => ("degraded", "ec2.idle.scoped-missing-day"),
        "ec2-resize-positive-001" => ("positive", "ec2.resize.fresh-compatible-recommendation"),
        "ec2-resize-negative-001" => ("negative", "ec2.resize.no-compatible-recommendation"),
        "ec2-mutation-stop-001" => ("mutation", "ec2.mutation.disposable-stop"),
        "ec2-mutation-resize-001" => ("mutation", "ec2.mutation.disposable-resize"),
        "ec2-mutation-stop-recovery-001" => ("mutation", "ec2.mutation.disposable-stop-recovery"),
        "ec2-mutation-resize-restoration-001" => {
            ("mutation", "ec2.mutation.disposable-resize-restoration")
        }
        _ => ("degraded", "ec2.unknown"),
    }
}

fn evidence_declaration(control_id: &str, resource: &EstateResource) -> Value {
    let missing_cpu_offsets = missing_history_offsets(resource);
    let mut evidence = json!({
        "cloudwatch_complete_days": HISTORY_DAYS - missing_cpu_offsets.len() as i64,
        "cloudwatch_expected_days": HISTORY_DAYS,
        "cloudwatch_missing_offsets": missing_cpu_offsets,
        "cost_complete_days": resource.costs.iter().map(|cost| cost.seconds_from_now).collect::<BTreeSet<_>>().len(),
        "cost_expected_days": HISTORY_DAYS,
        "topology": "independently-observable",
        "observed_metric_count": resource.metric_count,
        "observed_cost_record_count": resource.cost_record_count
    });
    if control_id == "ec2-idle-degraded-001" {
        evidence["degradation"] = json!("scoped-missing-day");
    }
    if control_id.starts_with("ec2-resize") {
        evidence["recommendation_bound_to_current_type"] = json!(true);
        evidence["recommendation_observed_within_days"] = json!(HISTORY_DAYS);
    }
    evidence
}

fn observation_tags(
    resource: &EstateResource,
    control_id: &str,
    role: &str,
    scenario: &str,
) -> BTreeMap<String, String> {
    let mut tags = resource.tags.clone();
    // Fixture-owned tags are scope assertions, not hints. Replace any source
    // tag with the canonical value so a stale LocalStack tag cannot contradict
    // the manifest control identity that Foxtail publishes.
    tags.insert("Name".to_string(), resource.id.clone());
    tags.insert("FoxtailFixture".to_string(), FIXTURE_VERSION.to_string());
    tags.insert("FoxtailControl".to_string(), control_id.to_string());
    tags.insert("FoxtailRole".to_string(), role.to_string());
    tags.insert("FoxtailScenario".to_string(), scenario.to_string());
    tags
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

fn mutation_identity_values(manifest: &Value) -> Result<Vec<Value>> {
    let Some(raw_resources) = manifest.get("mutation_resources") else {
        // Manifests written before disposable mutation generations were added
        // remain readable as read-only fixture state.
        return Ok(Vec::new());
    };
    let resources = raw_resources
        .as_array()
        .ok_or_else(|| anyhow!("manifest mutation_resources must be an array"))?;
    Ok(resources
        .iter()
        .map(|resource| {
            json!({
                "control_id": resource.get("control_id").cloned().unwrap_or(Value::Null),
                "target_kind": resource.get("target_kind").cloned().unwrap_or(Value::Null),
                "resource_id": resource.get("resource_id").cloned().unwrap_or(Value::Null),
                "aws_identity": resource.get("aws_identity").cloned().unwrap_or(Value::Null)
            })
        })
        .collect())
}

fn canonical_status_bytes(
    status: &str,
    definition_digest: &str,
    manifest_digest: Option<&str>,
    generation: Option<i64>,
    identities: &[Value],
    manifest: Option<&Value>,
    active_faults: &[Value],
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
        ,"active_faults": active_faults
    });
    if let Some(manifest) = manifest {
        value["clock"] = manifest.get("clock").cloned().unwrap_or(Value::Null);
        value["environment"] = manifest.get("environment").cloned().unwrap_or(Value::Null);
        value["mutation_generation"] = manifest
            .get("mutation_generation")
            .cloned()
            .unwrap_or(Value::Null);
        value["mutation_generation_id"] = manifest
            .get("mutation_generation_id")
            .cloned()
            .unwrap_or(Value::Null);
        value["mutation_identities"] = Value::Array(manifest
            .get("mutation_resources")
            .and_then(Value::as_array)
            .map(|resources| {
                resources
                    .iter()
                    .map(|resource| {
                        json!({
                            "control_id": resource.get("control_id").cloned().unwrap_or(Value::Null),
                            "target_kind": resource.get("target_kind").cloned().unwrap_or(Value::Null),
                            "resource_id": resource.get("resource_id").cloned().unwrap_or(Value::Null),
                            "aws_identity": resource.get("aws_identity").cloned().unwrap_or(Value::Null)
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default());
    }
    canonical_bytes(&value)
}

fn canonical_identities_bytes(
    status: &str,
    manifest_digest: Option<&str>,
    identities: &[Value],
    mutation_identities: &[Value],
) -> Result<Vec<u8>> {
    canonical_bytes(&json!({
        "schema": "foxtail.release-fixture-identities/v1",
        "fixture": FIXTURE_VERSION,
        "status": status,
        "manifest_digest": manifest_digest,
        "identities": identities,
        "mutation_identities": mutation_identities,
        "mutation_resource_ids": mutation_identities.iter().filter_map(|value| value.get("resource_id").and_then(Value::as_str)).collect::<Vec<_>>(),
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

/// Execute one native fixture CLI subcommand and return the exact canonical
/// document that the binary prints. Keeping this dispatch next to the
/// lifecycle boundary lets the HTTP handlers and CLI integration tests prove
/// the same operation contract without spawning a second process or bypassing
/// the public mutation adapter.
pub async fn execute_fixture_cli_command(
    pool: &SqlitePool,
    command: crate::cli::FixtureCommands,
) -> Result<Vec<u8>> {
    use crate::cli::FixtureCommands;

    let bytes = match command {
        FixtureCommands::Definition { version } => {
            validate_version(Some(&version))?;
            canonical_definition()?.0
        }
        FixtureCommands::Realize {
            version,
            clock_anchor,
            account_id,
            region,
            endpoint_url,
            localstack_version,
        } => realization_response(
            &realize(
                pool,
                RealizeRequest {
                    version: Some(version),
                    clock_anchor,
                    account_id,
                    region,
                    endpoint_url,
                    localstack_version,
                    force_new: false,
                    reuse_intent_id: None,
                    defer_intent_finalization: false,
                    allowed_intent_ids: Vec::new(),
                },
            )
            .await?,
        )?,
        FixtureCommands::Status => read_state(pool).await?.status_bytes,
        FixtureCommands::Manifest => read_state(pool)
            .await?
            .manifest_bytes
            .ok_or_else(|| anyhow!("fixture has not been realized"))?,
        FixtureCommands::Identities => read_state(pool).await?.identities_bytes,
        FixtureCommands::MutationStatus => mutation_status(pool).await?,
        FixtureCommands::Fault {
            authority,
            control_id,
            target_id,
            scope,
            fault_kind,
            application_time,
        } => {
            apply_fault(
                pool,
                FaultRequest {
                    authority: mutation_authority_from_cli(authority),
                    control_id,
                    target_id,
                    scope,
                    fault_kind,
                    application_time,
                },
            )
            .await?
        }
        FixtureCommands::Reset {
            authority,
            receipt_id,
            reset_token,
        } => {
            reset_fault(
                pool,
                ResetRequest {
                    authority: mutation_authority_from_cli(authority),
                    receipt_id,
                    reset_token,
                },
            )
            .await?
        }
        FixtureCommands::Recreate {
            authority,
            clock_anchor,
        } => {
            recreate(
                pool,
                RecreateRequest {
                    authority: mutation_authority_from_cli(authority),
                    clock_anchor,
                },
            )
            .await?
        }
        FixtureCommands::Destroy { authority } => {
            destroy(
                pool,
                DestroyRequest {
                    authority: mutation_authority_from_cli(authority),
                },
            )
            .await?
        }
    };
    Ok(bytes)
}

fn mutation_authority_from_cli(
    args: crate::cli::FixtureMutationAuthorityArgs,
) -> MutationAuthority {
    MutationAuthority {
        version: Some(args.version),
        generation: Some(args.generation),
        manifest_digest: Some(args.manifest_digest),
        mutation_generation: Some(args.mutation_generation),
        mutation_generation_id: Some(args.mutation_generation_id),
    }
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
            (
                "/generation_rules/assignment/selection",
                json!("last five EC2 resources after ascending id sort"),
            ),
            (
                "/generation_rules/evidence_profiles/0/cpu_value",
                json!(6.0),
            ),
            (
                "/generation_rules/evidence_profiles/2/missing_cpu_day",
                json!(7),
            ),
            ("/generation_rules/history/offset_seconds/0", json!(-7200)),
        ] {
            let mut changed = definition.clone();
            set_pointer(&mut changed, path, replacement);
            assert_ne!(canonical_digest(&changed).unwrap(), baseline, "{path}");
        }
    }

    #[test]
    fn definition_serializes_implementation_owned_materialization_profiles() {
        let definition = definition_with_digest().unwrap();
        let profiles = definition["generation_rules"]["evidence_profiles"]
            .as_array()
            .unwrap();
        assert_eq!(profiles.len(), MATERIALIZATION_PROFILES.len());
        assert_eq!(
            definition["generation_rules"]["assignment"]["control_order"],
            json!(
                MATERIALIZATION_PROFILES
                    .iter()
                    .map(|profile| profile.control_id)
                    .collect::<Vec<_>>()
            )
        );
        assert_eq!(
            definition["generation_rules"]["history"]["offset_seconds"],
            json!(history_offsets().into_iter().collect::<Vec<_>>())
        );
        for (serialized, profile) in profiles.iter().zip(MATERIALIZATION_PROFILES) {
            assert_eq!(serialized["control_id"], profile.control_id);
            assert_eq!(serialized["cpu_value"], profile.cpu_value);
            assert_eq!(serialized["cost_amount"], profile.cost_amount);
            assert_eq!(
                serialized["missing_cpu_day"],
                json!(profile.missing_cpu_day)
            );
        }
    }

    #[test]
    fn account_scope_defaults_to_and_accepts_authoritative_public_identity() {
        assert_eq!(
            resolve_account_id(None).unwrap(),
            authoritative_account_id()
        );
        assert_eq!(
            resolve_account_id(Some(authoritative_account_id())).unwrap(),
            authoritative_account_id()
        );
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
    fn validation_rejects_forbidden_policy_fields_at_any_depth() {
        let mut definition = definition_with_digest().unwrap();
        definition["controls"][0]["evidence"]["expected_finding"] = json!("over_provisioned");
        let error = validate_document(&definition, "digest")
            .unwrap_err()
            .to_string();
        assert!(error.contains("forbidden policy field"));
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
        assert_eq!(
            value["generator"]["source_revision"],
            "dbe899e5df8a56c434768a71643e55b9e1315582"
        );
        assert_eq!(
            value["environment"]["estate_fingerprint"],
            canonical_digest(&json!({
                "mutation_generation": value["mutation_generation"],
                "mutation_generation_id": value["mutation_generation_id"],
                "read_only_estate_fingerprint": value["environment"]
                    ["read_only_estate_fingerprint"],
                "mutation_targets": value["mutation_resources"]
            }))
            .unwrap()
        );
        assert_eq!(value["resources"].as_array().unwrap().len(), 5);
    }

    #[tokio::test]
    async fn generated_manifest_matches_checked_in_canonical_golden() {
        let path = std::env::temp_dir().join(format!(
            "foxtail-fixture-golden-{}.db",
            uuid::Uuid::new_v4()
        ));
        let pool = crate::db::init(&format!("sqlite:{}", path.display()))
            .await
            .unwrap();
        for index in 0..5 {
            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario, tags)
                 VALUES (?, 'ec2', 'us-east-1', 'Baseline', '{}')",
            )
            .bind(format!("i-empty-fixture-{index}"))
            .execute(&pool)
            .await
            .unwrap();
        }
        let snapshot = realize(
            &pool,
            RealizeRequest {
                clock_anchor: Some("2026-08-05T00:00:00Z".to_string()),
                account_id: Some(DEFAULT_ACCOUNT_ID.to_string()),
                region: Some(DEFAULT_REGION.to_string()),
                endpoint_url: Some(DEFAULT_LOCALSTACK_ENDPOINT.to_string()),
                localstack_version: Some("unknown".to_string()),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap();
        let golden = include_bytes!("../tests/fixtures/release-qualification-v1.manifest.json");
        let golden = golden.strip_suffix(b"\n").unwrap_or(golden);
        let expected: Value = serde_json::from_slice(golden).unwrap();
        let mut manifest: Value = serde_json::from_slice(&snapshot.manifest_bytes).unwrap();
        manifest["generator"]["source_revision"] = expected["generator"]["source_revision"].clone();
        manifest["digest"] = json!(canonical_digest(&manifest).unwrap());
        assert_eq!(canonical_bytes(&manifest).unwrap(), golden);
        assert!(validate_policy_fields(&manifest, "$").is_ok());
        pool.close().await;
    }

    #[tokio::test]
    async fn mismatched_account_scope_fails_before_materialization_or_commit() {
        let path = std::env::temp_dir().join(format!(
            "foxtail-fixture-account-mismatch-{}.db",
            uuid::Uuid::new_v4()
        ));
        let pool = crate::db::init(&format!("sqlite:{}", path.display()))
            .await
            .unwrap();
        for index in 0..5 {
            sqlx::query(
                "INSERT INTO resources (id, resource_type, region, scenario, tags)
                 VALUES (?, 'ec2', 'us-east-1', 'Baseline', '{}')",
            )
            .bind(format!("i-account-mismatch-{index}"))
            .execute(&pool)
            .await
            .unwrap();
        }

        let error = realize(
            &pool,
            RealizeRequest {
                account_id: Some("999999999999".to_string()),
                ..RealizeRequest::default()
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("does not match public AWS account"));

        let metric_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM metrics")
            .fetch_one(&pool)
            .await
            .unwrap();
        let cost_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_records")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(metric_count, 0);
        assert_eq!(cost_count, 0);
        assert_eq!(read_state(&pool).await.unwrap().status, "ABSENT");
        pool.close().await;
    }

    #[tokio::test]
    async fn mutation_lifecycle_uses_four_disposable_targets_and_proves_recreation_and_destroy() {
        with_isolated_qualification(async {
            let path = std::env::temp_dir().join(format!(
                "foxtail-mutation-lifecycle-{}.db",
                uuid::Uuid::new_v4()
            ));
            let pool = crate::db::init(&format!("sqlite:{}", path.display()))
                .await
                .unwrap();
            for index in 0..5 {
                sqlx::query(
                    "INSERT INTO resources (id, resource_type, region, scenario, tags)
                 VALUES (?, 'ec2', 'us-east-1', 'Baseline', '{}')",
                )
                .bind(format!("i-mutation-source-{index}"))
                .execute(&pool)
                .await
                .unwrap();
            }
            let first = realize(
                &pool,
                RealizeRequest {
                    clock_anchor: Some("2026-08-05T00:00:00Z".to_string()),
                    endpoint_url: Some(format!("mock://{}", uuid::Uuid::new_v4())),
                    ..RealizeRequest::default()
                },
            )
            .await
            .unwrap();
            let first_manifest: Value = serde_json::from_slice(&first.manifest_bytes).unwrap();
            let first_targets = first_manifest["mutation_resources"]
                .as_array()
                .unwrap()
                .clone();
            assert_eq!(first_targets.len(), 4);
            assert_eq!(
                first_targets
                    .iter()
                    .filter_map(|target| target["resource_id"].as_str())
                    .collect::<BTreeSet<_>>()
                    .len(),
                4
            );
            assert!(first_targets.iter().all(|target| {
                !first_manifest["resources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|read_only| read_only["resource_id"] == target["resource_id"])
            }));

            let authority = MutationAuthority {
                version: Some(FIXTURE_VERSION.to_string()),
                generation: Some(first.generation),
                manifest_digest: Some(first.manifest_digest.clone()),
                mutation_generation: Some(first_manifest["mutation_generation"].as_i64().unwrap()),
                mutation_generation_id: Some(
                    first_manifest["mutation_generation_id"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                ),
            };
            let stop = first_targets
                .iter()
                .find(|target| target["target_kind"] == "stop")
                .unwrap();
            let fault: Value = serde_json::from_slice(
                &apply_fault(
                    &pool,
                    FaultRequest {
                        authority: authority.clone(),
                        control_id: stop["control_id"].as_str().unwrap().to_string(),
                        target_id: stop["resource_id"].as_str().unwrap().to_string(),
                        scope: "target".to_string(),
                        fault_kind: "stop".to_string(),
                        application_time: Some("2026-08-05T00:00:00Z".to_string()),
                    },
                )
                .await
                .unwrap(),
            )
            .unwrap();
            assert_eq!(fault["manifest_digest"], first.manifest_digest);
            assert_eq!(fault["reset_token_use"], "one-use");
            assert!(
                apply_fault(
                    &pool,
                    FaultRequest {
                        authority: authority.clone(),
                        control_id: stop["control_id"].as_str().unwrap().to_string(),
                        target_id: stop["resource_id"].as_str().unwrap().to_string(),
                        scope: "target".to_string(),
                        fault_kind: "stop".to_string(),
                        application_time: None,
                    },
                )
                .await
                .is_err()
            );
            reset_fault(
                &pool,
                ResetRequest {
                    authority: authority.clone(),
                    receipt_id: fault["receipt_id"].as_str().unwrap().to_string(),
                    reset_token: fault["reset_token"].as_str().unwrap().to_string(),
                },
            )
            .await
            .unwrap();
            assert!(
                reset_fault(
                    &pool,
                    ResetRequest {
                        authority: authority.clone(),
                        receipt_id: fault["receipt_id"].as_str().unwrap().to_string(),
                        reset_token: fault["reset_token"].as_str().unwrap().to_string(),
                    },
                )
                .await
                .is_err()
            );

            let recreate_receipt: Value = serde_json::from_slice(
                &recreate(
                    &pool,
                    RecreateRequest {
                        authority: authority.clone(),
                        clock_anchor: Some("2026-08-05T00:00:00Z".to_string()),
                    },
                )
                .await
                .unwrap(),
            )
            .unwrap();
            assert_eq!(recreate_receipt["status"], "RECREATED");
            let second = read_state(&pool).await.unwrap();
            let second_manifest: Value =
                serde_json::from_slice(second.manifest_bytes.as_ref().unwrap()).unwrap();
            assert_ne!(
                first_manifest["mutation_generation_id"],
                second_manifest["mutation_generation_id"]
            );
            assert_ne!(
                first_manifest["mutation_resources"],
                second_manifest["mutation_resources"]
            );
            assert_ne!(
                first.manifest_digest,
                second.manifest_digest.as_deref().unwrap()
            );
            assert_eq!(
                first_manifest["environment"]["read_only_estate_fingerprint"],
                second_manifest["environment"]["read_only_estate_fingerprint"]
            );

            let second_authority = MutationAuthority {
                version: Some(FIXTURE_VERSION.to_string()),
                generation: second.generation,
                manifest_digest: second.manifest_digest.clone(),
                mutation_generation: Some(second_manifest["mutation_generation"].as_i64().unwrap()),
                mutation_generation_id: Some(
                    second_manifest["mutation_generation_id"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                ),
            };
            let second_stop = second_manifest["mutation_resources"]
                .as_array()
                .unwrap()
                .iter()
                .find(|target| target["target_kind"] == "stop")
                .unwrap();
            apply_fault(
                &pool,
                FaultRequest {
                    authority: second_authority.clone(),
                    control_id: second_stop["control_id"].as_str().unwrap().to_string(),
                    target_id: second_stop["resource_id"].as_str().unwrap().to_string(),
                    scope: "target".to_string(),
                    fault_kind: "stop".to_string(),
                    application_time: Some("2026-08-05T00:00:00Z".to_string()),
                },
            )
            .await
            .unwrap();
            let destroy_receipt: Value = serde_json::from_slice(
                &destroy(
                    &pool,
                    DestroyRequest {
                        authority: second_authority,
                    },
                )
                .await
                .unwrap(),
            )
            .unwrap();
            assert_eq!(destroy_receipt["status"], "DESTROYED");
            assert_eq!(
                destroy_receipt["public_inventory_absence"]["all_absent"],
                true
            );
            assert_eq!(destroy_receipt["faults_reset"][0]["status"], "RESET");
            let reset_receipts: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM fixture_operation_receipts WHERE operation = 'reset'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(reset_receipts, 2);
            let mutation_rows: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM resources WHERE id LIKE 'i-foxtail-mutation-%'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(mutation_rows, 0);
            assert_eq!(read_state(&pool).await.unwrap().status, "ABSENT");
            let next = realize(
                &pool,
                RealizeRequest {
                    clock_anchor: Some("2026-08-05T00:00:00Z".to_string()),
                    endpoint_url: Some(format!("mock://{}", uuid::Uuid::new_v4())),
                    ..RealizeRequest::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(next.generation, 3);
            assert_ne!(next.manifest_digest, first.manifest_digest);
            pool.close().await;
        })
        .await;
    }

    #[test]
    fn mutation_requests_reject_unknown_fields() {
        let authority = json!({
            "version": FIXTURE_VERSION,
            "generation": 1,
            "manifest_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "mutation_generation": 1,
            "mutation_generation_id": "mg-0001"
        });
        let mut fault = authority.clone();
        fault["control_id"] = json!("ec2-mutation-stop-001");
        fault["target_id"] = json!("i-foxtail-mutation-g0001-stop");
        fault["scope"] = json!("target");
        fault["fault_kind"] = json!("stop");
        fault["unknown"] = json!(true);
        assert!(parse_fault_request(&serde_json::to_vec(&fault).unwrap()).is_err());

        let mut recreate = authority;
        recreate["unknown"] = json!(true);
        assert!(parse_recreate_request(&serde_json::to_vec(&recreate).unwrap()).is_err());
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
