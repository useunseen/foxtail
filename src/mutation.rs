//! Canonical disposable mutation catalogue and the live EC2 adapter.
//!
//! The fixture database records the contract and lifecycle ledger, but it is
//! never the authority for the live instance state. This module owns the
//! scenario metadata once and keeps all EC2 calls behind one reviewable
//! boundary.

use anyhow::{Context, Result, anyhow, bail};
use aws_config::BehaviorVersion;
use aws_sdk_ec2::Client;
use aws_sdk_ec2::config::{Credentials, Region};
use aws_sdk_ec2::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_ec2::operation::describe_instances::DescribeInstancesError;
use aws_sdk_ec2::types::{
    AttributeValue, Filter, Instance, InstanceStateName, InstanceType, ResourceType, Tag,
    TagSpecification,
};
use aws_smithy_http_client::Builder as HttpClientBuilder;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration as StdDuration;
use tokio::time::sleep;

pub const MUTATION_AMI_ENV: &str = "FOXTAIL_MUTATION_AMI_ID";
pub const MUTATION_SUBNET_ENV: &str = "FOXTAIL_MUTATION_SUBNET_ID";
pub const MUTATION_SECURITY_GROUP_ENV: &str = "FOXTAIL_MUTATION_SECURITY_GROUP_ID";
pub const DEFAULT_MUTATION_AMI_ID: &str = "ami-00000000";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetKind {
    Stop,
    Resize,
    StopRecovery,
    ResizeRestoration,
}

impl TargetKind {
    pub const ALL: [Self; 4] = [
        Self::Stop,
        Self::Resize,
        Self::StopRecovery,
        Self::ResizeRestoration,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Resize => "resize",
            Self::StopRecovery => "stop-recovery",
            Self::ResizeRestoration => "resize-restoration",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SetupFaultKind {
    Stop,
    Resize,
}

impl SetupFaultKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Resize => "resize",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MutationScenario {
    pub control_id: &'static str,
    pub target_kind: TargetKind,
    pub setup_fault_kind: SetupFaultKind,
    pub initial_state: &'static str,
    pub initial_type: &'static str,
    pub terminal_state: &'static str,
    pub terminal_type: &'static str,
    pub restored_state: &'static str,
    pub restored_type: &'static str,
}

pub const CATALOGUE: [MutationScenario; 4] = [
    MutationScenario {
        control_id: "ec2-mutation-stop-001",
        target_kind: TargetKind::Stop,
        setup_fault_kind: SetupFaultKind::Stop,
        initial_state: "running",
        initial_type: "m6i.large",
        terminal_state: "stopped",
        terminal_type: "m6i.large",
        restored_state: "running",
        restored_type: "m6i.large",
    },
    MutationScenario {
        control_id: "ec2-mutation-resize-001",
        target_kind: TargetKind::Resize,
        setup_fault_kind: SetupFaultKind::Resize,
        initial_state: "stopped",
        initial_type: "m6i.large",
        terminal_state: "stopped",
        terminal_type: "m6i.medium",
        restored_state: "stopped",
        restored_type: "m6i.large",
    },
    MutationScenario {
        control_id: "ec2-mutation-stop-recovery-001",
        target_kind: TargetKind::StopRecovery,
        setup_fault_kind: SetupFaultKind::Stop,
        initial_state: "running",
        initial_type: "m6i.large",
        terminal_state: "stopped",
        terminal_type: "m6i.large",
        restored_state: "running",
        restored_type: "m6i.large",
    },
    MutationScenario {
        control_id: "ec2-mutation-resize-restoration-001",
        target_kind: TargetKind::ResizeRestoration,
        setup_fault_kind: SetupFaultKind::Resize,
        initial_state: "stopped",
        initial_type: "m6i.medium",
        terminal_state: "stopped",
        terminal_type: "m6i.large",
        restored_state: "stopped",
        restored_type: "m6i.medium",
    },
];

pub fn scenario_for_control(control_id: &str) -> Result<&'static MutationScenario> {
    CATALOGUE
        .iter()
        .find(|scenario| scenario.control_id == control_id)
        .ok_or_else(|| anyhow!("unknown mutation control '{control_id}'"))
}

pub fn scenario_for_target_kind(target_kind: &str) -> Result<&'static MutationScenario> {
    CATALOGUE
        .iter()
        .find(|scenario| scenario.target_kind.as_str() == target_kind)
        .ok_or_else(|| anyhow!("unknown mutation target kind '{target_kind}'"))
}

pub fn resource_id_hint(generation: i64, target_kind: TargetKind) -> String {
    format!(
        "i-foxtail-mutation-g{generation:04}-{}",
        target_kind.as_str()
    )
}

pub fn generation_id(generation: i64) -> String {
    format!("mg-{generation:04}")
}

fn resource_id_hint_from_generation_id(generation_id: &str, target_kind: TargetKind) -> String {
    let generation = generation_id
        .strip_prefix("mg-")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_default();
    resource_id_hint(generation, target_kind)
}

fn client_token(generation_id: &str, scenario: &MutationScenario) -> String {
    format!("foxtail-{generation_id}-{}", scenario.control_id)
}

fn observed_instance(instance: &Instance) -> Option<ObservedInstance> {
    let resource_id = instance.instance_id()?.to_string();
    Some(ObservedInstance {
        resource_id,
        instance_state: instance
            .state()
            .and_then(|state| state.name())
            .map(InstanceStateName::as_str)
            .unwrap_or("unknown")
            .to_string(),
        instance_type: instance
            .instance_type()
            .map(InstanceType::as_str)
            .unwrap_or_default()
            .to_string(),
        availability_zone: instance
            .placement()
            .and_then(|placement| placement.availability_zone())
            .map(str::to_string),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservedInstance {
    pub resource_id: String,
    pub instance_state: String,
    pub instance_type: String,
    pub availability_zone: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalTerminationState {
    Terminated,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalTerminationEvidence {
    pub resource_id: String,
    pub state: ExternalTerminationState,
}

#[derive(Debug)]
pub struct ProvisionFailure {
    pub returned_ids: Vec<String>,
    pub cause: String,
}

impl std::fmt::Display for ProvisionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "mutation cleanup ambiguous; returned_ids={:?}; {}",
            self.returned_ids, self.cause
        )
    }
}

impl std::error::Error for ProvisionFailure {}

#[derive(Default)]
struct MockState {
    instances: BTreeMap<String, ObservedInstance>,
    describe_error: bool,
    fail_dispatch: bool,
    fail_setup_at: Option<usize>,
    fail_cleanup: bool,
    launch_count: usize,
    fail_setup_target: Option<String>,
    lost_response_at: Option<usize>,
    empty_response_at: Option<usize>,
    cleanup_delay_ms: Option<u64>,
}

enum BackendKind {
    Aws(Client),
    Mock(Arc<Mutex<MockState>>),
}

static MOCK_STATES: OnceLock<Mutex<BTreeMap<String, Arc<Mutex<MockState>>>>> = OnceLock::new();

#[derive(Clone)]
pub struct Ec2MutationBackend {
    backend: Arc<BackendKind>,
    region: String,
    account_id: String,
}

impl Ec2MutationBackend {
    pub async fn connect(endpoint_url: &str, region: &str, account_id: &str) -> Result<Self> {
        if let Some(mock_key) = endpoint_url.strip_prefix("mock://") {
            let states = MOCK_STATES.get_or_init(|| Mutex::new(BTreeMap::new()));
            let state = states
                .lock()
                .map_err(|_| anyhow!("mock mutation state lock poisoned"))?
                .entry(mock_key.to_string())
                .or_insert_with(|| {
                    let fail_setup_at = mock_key
                        .strip_prefix("fail-setup-")
                        .or_else(|| mock_key.strip_prefix("fail-cleanup-setup-"))
                        .and_then(|value| value.parse::<usize>().ok());
                    let fail_dispatch = mock_key == "pre-dispatch";
                    let describe_error = mock_key == "describe-error";
                    let lost_response_at = mock_key
                        .strip_prefix("lost-response-")
                        .and_then(|value| value.parse::<usize>().ok());
                    let empty_response_at = mock_key
                        .strip_prefix("empty-id-")
                        .and_then(|value| value.parse::<usize>().ok());
                    let cleanup_delay_ms = mock_key
                        .strip_prefix("slow-cleanup-")
                        .and_then(|value| value.parse::<u64>().ok());
                    Arc::new(Mutex::new(MockState {
                        describe_error,
                        fail_dispatch,
                        fail_setup_at,
                        fail_cleanup: mock_key == "fail-cleanup"
                            || mock_key.starts_with("fail-cleanup-setup-"),
                        lost_response_at,
                        empty_response_at,
                        cleanup_delay_ms,
                        ..MockState::default()
                    }))
                })
                .clone();
            return Ok(Self {
                backend: Arc::new(BackendKind::Mock(state)),
                region: region.to_string(),
                account_id: account_id.to_string(),
            });
        }
        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(region.to_string()))
            .endpoint_url(endpoint_url)
            .credentials_provider(Credentials::new("test", "test", None, None, "static"));
        if endpoint_url.starts_with("http://") {
            loader = loader.http_client(HttpClientBuilder::new().build_http());
        }
        let config = loader.load().await;
        Ok(Self {
            backend: Arc::new(BackendKind::Aws(Client::new(&config))),
            region: region.to_string(),
            account_id: account_id.to_string(),
        })
    }

    pub fn resource_arn(&self, resource_id: &str) -> String {
        format!(
            "arn:aws:ec2:{}:{}:instance/{resource_id}",
            self.region, self.account_id
        )
    }

    fn aws_client(&self) -> &Client {
        match self.backend.as_ref() {
            BackendKind::Aws(client) => client,
            BackendKind::Mock(_) => {
                unreachable!("mock backend must be handled before AWS dispatch")
            }
        }
    }

    pub async fn provision_generation(
        &self,
        generation: i64,
        generation_id: &str,
    ) -> Result<Vec<(MutationScenario, ObservedInstance)>> {
        let mut provisioned = Vec::with_capacity(CATALOGUE.len());
        for scenario in CATALOGUE {
            let resource_id = match self
                .launch_target(generation, generation_id, &scenario)
                .await
            {
                Ok(resource_id) => resource_id,
                Err(error) => {
                    let mut ids = provisioned
                        .iter()
                        .map(|(_, observed): &(MutationScenario, ObservedInstance)| {
                            observed.resource_id.clone()
                        })
                        .collect::<Vec<_>>();
                    let ambiguous_launch = error
                        .downcast_ref::<ProvisionFailure>()
                        .map(|failure| failure.returned_ids.clone());
                    if let Some(returned_ids) = &ambiguous_launch {
                        ids.extend(returned_ids.iter().cloned());
                    }
                    let cleanup = self.cleanup_after_provision_failure(&ids, error).await;
                    if ambiguous_launch.is_some() {
                        let cleanup_error = cleanup.unwrap_or_else(|error| error);
                        return Err(anyhow::Error::new(ProvisionFailure {
                            returned_ids: ids,
                            cause: cleanup_error.to_string(),
                        }));
                    }
                    return Err(cleanup.unwrap_or_else(|cleanup_error| cleanup_error));
                }
            };
            match self.prepare_target(&resource_id, &scenario).await {
                Ok(observed) => provisioned.push((scenario, observed)),
                Err(error) => {
                    let mut ids = provisioned
                        .iter()
                        .map(|(_, observed): &(MutationScenario, ObservedInstance)| {
                            observed.resource_id.clone()
                        })
                        .collect::<Vec<_>>();
                    ids.push(resource_id);
                    return Err(self
                        .cleanup_after_provision_failure(&ids, error)
                        .await
                        .unwrap_or_else(|cleanup_error| cleanup_error));
                }
            }
        }
        Ok(provisioned)
    }

    async fn launch_target(
        &self,
        generation: i64,
        generation_id: &str,
        scenario: &MutationScenario,
    ) -> Result<String> {
        if let BackendKind::Mock(state) = self.backend.as_ref() {
            let resource_id = resource_id_hint(generation, scenario.target_kind);
            let observed = ObservedInstance {
                resource_id: resource_id.clone(),
                instance_state: scenario.initial_state.to_string(),
                instance_type: scenario.initial_type.to_string(),
                availability_zone: Some("mock-az".to_string()),
            };
            let response_lost = {
                let mut state = state
                    .lock()
                    .map_err(|_| anyhow!("mock mutation state lock poisoned"))?;
                if state.instances.contains_key(&resource_id) {
                    return Ok(resource_id);
                }
                if state.fail_dispatch {
                    bail!("injected EC2 RunInstances failure before dispatch");
                }
                state
                    .instances
                    .insert(resource_id.clone(), observed.clone());
                state.launch_count += 1;
                if state.fail_setup_at == Some(state.launch_count) {
                    state.fail_setup_target = Some(resource_id.clone());
                }
                state.lost_response_at == Some(state.launch_count)
                    || state.empty_response_at == Some(state.launch_count)
            };
            if response_lost {
                return self
                    .reconcile_launched_target(
                        generation_id,
                        scenario,
                        anyhow!("mock RunInstances response did not identify the applied target"),
                        false,
                    )
                    .await;
            }
            return Ok(resource_id);
        }
        let image_id =
            std::env::var(MUTATION_AMI_ENV).unwrap_or_else(|_| DEFAULT_MUTATION_AMI_ID.to_string());
        let tags = [
            ("Name", resource_id_hint(generation, scenario.target_kind)),
            ("FoxtailFixture", "release-qualification-v1".to_string()),
            ("FoxtailMutationGeneration", generation.to_string()),
            ("FoxtailMutationGenerationId", generation_id.to_string()),
            ("FoxtailMutationControl", scenario.control_id.to_string()),
            (
                "FoxtailMutationTarget",
                scenario.target_kind.as_str().to_string(),
            ),
        ];
        let tag_specification = tags.into_iter().fold(
            TagSpecification::builder().resource_type(ResourceType::Instance),
            |builder, (key, value)| builder.tags(Tag::builder().key(key).value(value).build()),
        );
        let client = match self.backend.as_ref() {
            BackendKind::Aws(client) => client,
            BackendKind::Mock(_) => unreachable!(),
        };
        let mut builder = client
            .run_instances()
            .client_token(client_token(generation_id, scenario))
            .image_id(image_id)
            .instance_type(InstanceType::from(scenario.initial_type))
            .min_count(1)
            .max_count(1)
            .tag_specifications(tag_specification.build());
        if let Ok(subnet_id) = std::env::var(MUTATION_SUBNET_ENV) {
            builder = builder.subnet_id(subnet_id);
        }
        if let Ok(security_group_id) = std::env::var(MUTATION_SECURITY_GROUP_ENV) {
            builder = builder.security_group_ids(security_group_id);
        }
        let response = match builder.send().await {
            Ok(response) => response,
            Err(error) => {
                let definitely_pre_dispatch = definitely_pre_dispatch_error(&format!("{error:?}"));
                return self
                    .reconcile_launched_target(
                        generation_id,
                        scenario,
                        anyhow!("RunInstances response outcome is ambiguous: {error}"),
                        definitely_pre_dispatch,
                    )
                    .await;
            }
        };
        let instances = response
            .instances()
            .iter()
            .filter_map(observed_instance)
            .collect::<Vec<_>>();
        match instances.as_slice() {
            [observed] => Ok(observed.resource_id.clone()),
            [] => {
                self.reconcile_launched_target(
                    generation_id,
                    scenario,
                    anyhow!("RunInstances returned no instance identity"),
                    false,
                )
                .await
            }
            _ => Err(anyhow::Error::new(ProvisionFailure {
                returned_ids: instances
                    .iter()
                    .map(|observed| observed.resource_id.clone())
                    .collect(),
                cause: "RunInstances returned multiple identities for one target".to_string(),
            })),
        }
    }

    async fn reconcile_launched_target(
        &self,
        generation_id: &str,
        scenario: &MutationScenario,
        cause: anyhow::Error,
        definitely_pre_dispatch: bool,
    ) -> Result<String> {
        let matches = match self.find_tagged_instances(generation_id, scenario).await {
            Ok(matches) => matches,
            Err(error) => {
                if definitely_pre_dispatch {
                    return Err(cause.context("EC2 mutation RunInstances failed before dispatch"));
                }
                return Err(anyhow::Error::new(ProvisionFailure {
                    returned_ids: Vec::new(),
                    cause: format!("{cause}; public reconciliation failed: {error}"),
                }));
            }
        };
        match matches.as_slice() {
            [observed] => Ok(observed.resource_id.clone()),
            [] if definitely_pre_dispatch => {
                Err(cause.context("EC2 mutation RunInstances failed before dispatch"))
            }
            [] => Err(anyhow::Error::new(ProvisionFailure {
                returned_ids: Vec::new(),
                cause: format!("{cause}; no exact generation/control identity was found"),
            })),
            _ => Err(anyhow::Error::new(ProvisionFailure {
                returned_ids: matches
                    .iter()
                    .map(|observed| observed.resource_id.clone())
                    .collect(),
                cause: format!("{cause}; multiple exact generation/control identities were found"),
            })),
        }
    }

    async fn find_tagged_instances(
        &self,
        generation_id: &str,
        scenario: &MutationScenario,
    ) -> Result<Vec<ObservedInstance>> {
        if let BackendKind::Mock(state) = self.backend.as_ref() {
            let state = state
                .lock()
                .map_err(|_| anyhow!("mock mutation state lock poisoned"))?;
            let resource_id =
                resource_id_hint_from_generation_id(generation_id, scenario.target_kind);
            return Ok(state
                .instances
                .get(&resource_id)
                .cloned()
                .into_iter()
                .collect());
        }
        let response = self
            .aws_client()
            .describe_instances()
            .filters(
                Filter::builder()
                    .name("tag:FoxtailMutationGenerationId")
                    .values(generation_id)
                    .build(),
            )
            .filters(
                Filter::builder()
                    .name("tag:FoxtailMutationControl")
                    .values(scenario.control_id)
                    .build(),
            )
            .filters(
                Filter::builder()
                    .name("tag:FoxtailMutationTarget")
                    .values(scenario.target_kind.as_str())
                    .build(),
            )
            .send()
            .await
            .context("reconcile RunInstances identity through public EC2 DescribeInstances")?;
        Ok(response
            .reservations()
            .iter()
            .flat_map(|reservation| reservation.instances())
            .filter_map(observed_instance)
            .collect())
    }

    async fn prepare_target(
        &self,
        resource_id: &str,
        scenario: &MutationScenario,
    ) -> Result<ObservedInstance> {
        if let BackendKind::Mock(state) = self.backend.as_ref() {
            let mut state = state
                .lock()
                .map_err(|_| anyhow!("mock mutation state lock poisoned"))?;
            if state.fail_setup_target.as_deref() == Some(resource_id) {
                state.fail_setup_target = None;
                bail!("injected mutation setup failure for {resource_id}");
            }
            let target = state
                .instances
                .get_mut(resource_id)
                .ok_or_else(|| anyhow!("public EC2 describe returned no target '{resource_id}'"))?;
            if target.instance_state != scenario.initial_state
                || target.instance_type != scenario.initial_type
            {
                bail!(
                    "mock mutation launch state for {resource_id} was {}:{}, expected {}:{}",
                    target.instance_state,
                    target.instance_type,
                    scenario.initial_state,
                    scenario.initial_type
                );
            }
            return Ok(target.clone());
        }
        let observed = self
            .wait_for_instance(resource_id, "running", scenario.initial_type)
            .await?;
        if scenario.initial_state == "stopped" {
            self.aws_client()
                .stop_instances()
                .instance_ids(resource_id)
                .send()
                .await
                .context("stop initial stopped mutation target")?;
            self.wait_for_instance(resource_id, "stopped", scenario.initial_type)
                .await
        } else {
            Ok(observed)
        }
    }

    async fn cleanup_after_provision_failure(
        &self,
        ids: &[String],
        error: anyhow::Error,
    ) -> Result<anyhow::Error> {
        if ids.is_empty() {
            return Ok(error);
        }
        if let Err(cleanup_error) = self.terminate_all(ids).await {
            return Err(anyhow::Error::new(ProvisionFailure {
                returned_ids: ids.to_vec(),
                cause: format!("setup_error={error}; cleanup_error={cleanup_error}"),
            }));
        }
        Ok(error.context(format!(
            "returned_ids={ids:?}; external_ec2_termination_proven"
        )))
    }

    pub async fn apply_setup_fault(
        &self,
        target_id: &str,
        scenario: &MutationScenario,
        fault_kind: SetupFaultKind,
    ) -> Result<ObservedInstance> {
        if scenario.setup_fault_kind != fault_kind {
            bail!(
                "fault kind '{}' is not valid for target scenario '{}'",
                fault_kind.as_str(),
                scenario.target_kind.as_str()
            );
        }
        if let BackendKind::Mock(state) = self.backend.as_ref() {
            let mut state = state
                .lock()
                .map_err(|_| anyhow!("mock mutation state lock poisoned"))?;
            let observed = state
                .instances
                .get_mut(target_id)
                .ok_or_else(|| anyhow!("public EC2 describe returned no target '{target_id}'"))?;
            match fault_kind {
                SetupFaultKind::Stop => observed.instance_state = "stopped".to_string(),
                SetupFaultKind::Resize => {
                    observed.instance_state = "stopped".to_string();
                    observed.instance_type = scenario.terminal_type.to_string();
                }
            }
            return Ok(observed.clone());
        }
        let before = self.describe_instance(target_id).await?;
        match fault_kind {
            SetupFaultKind::Stop => {
                self.aws_client()
                    .stop_instances()
                    .instance_ids(target_id)
                    .send()
                    .await
                    .context("stop disposable mutation target")?;
                self.wait_for_instance(target_id, "stopped", scenario.terminal_type)
                    .await
            }
            SetupFaultKind::Resize => {
                if before.instance_state != "stopped" {
                    self.aws_client()
                        .stop_instances()
                        .instance_ids(target_id)
                        .send()
                        .await
                        .context("stop target before resize")?;
                    self.wait_for_instance_state(target_id, "stopped").await?;
                }
                self.aws_client()
                    .modify_instance_attribute()
                    .instance_id(target_id)
                    .instance_type(
                        AttributeValue::builder()
                            .value(scenario.terminal_type)
                            .build(),
                    )
                    .send()
                    .await
                    .context("resize disposable mutation target")?;
                self.wait_for_instance(target_id, "stopped", scenario.terminal_type)
                    .await
            }
        }
    }

    pub async fn reset_setup_fault(
        &self,
        target_id: &str,
        scenario: &MutationScenario,
    ) -> Result<ObservedInstance> {
        if let BackendKind::Mock(state) = self.backend.as_ref() {
            let mut state = state
                .lock()
                .map_err(|_| anyhow!("mock mutation state lock poisoned"))?;
            let observed = state
                .instances
                .get_mut(target_id)
                .ok_or_else(|| anyhow!("public EC2 describe returned no target '{target_id}'"))?;
            observed.instance_state = scenario.initial_state.to_string();
            observed.instance_type = scenario.initial_type.to_string();
            return Ok(observed.clone());
        }
        let before = self.describe_instance(target_id).await?;
        if before.instance_state != "stopped" {
            self.aws_client()
                .stop_instances()
                .instance_ids(target_id)
                .send()
                .await
                .context("stop target before reset")?;
            self.wait_for_instance(target_id, "stopped", &before.instance_type)
                .await?;
        }
        if before.instance_type != scenario.initial_type {
            self.aws_client()
                .modify_instance_attribute()
                .instance_id(target_id)
                .instance_type(
                    AttributeValue::builder()
                        .value(scenario.initial_type)
                        .build(),
                )
                .send()
                .await
                .context("restore mutation target instance type")?;
        }
        if scenario.initial_state == "running" {
            self.aws_client()
                .start_instances()
                .instance_ids(target_id)
                .send()
                .await
                .context("start mutation target during reset")?;
            self.wait_for_instance(target_id, "running", scenario.initial_type)
                .await
        } else {
            self.wait_for_instance(target_id, "stopped", scenario.initial_type)
                .await
        }
    }

    pub async fn describe_instance(&self, target_id: &str) -> Result<ObservedInstance> {
        if let BackendKind::Mock(state) = self.backend.as_ref() {
            let state = state
                .lock()
                .map_err(|_| anyhow!("mock mutation state lock poisoned"))?;
            if state.describe_error {
                bail!("injected EC2 DescribeInstances failure");
            }
            return state
                .instances
                .get(target_id)
                .cloned()
                .ok_or_else(|| anyhow!("public EC2 describe returned no target '{target_id}'"));
        }
        let response = self
            .aws_client()
            .describe_instances()
            .instance_ids(target_id)
            .send()
            .await
            .context("describe disposable mutation target")?;
        response
            .reservations()
            .iter()
            .flat_map(|reservation| reservation.instances())
            .find_map(|instance| {
                let id = instance.instance_id()?.to_string();
                if id != target_id {
                    return None;
                }
                let state = instance
                    .state()
                    .and_then(|state| state.name())
                    .map(InstanceStateName::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let instance_type = instance
                    .instance_type()
                    .map(InstanceType::as_str)
                    .unwrap_or_default()
                    .to_string();
                Some(ObservedInstance {
                    resource_id: id,
                    instance_state: state,
                    instance_type,
                    availability_zone: instance
                        .placement()
                        .and_then(|placement| placement.availability_zone())
                        .map(str::to_string),
                })
            })
            .ok_or_else(|| anyhow!("public EC2 describe returned no target '{target_id}'"))
    }

    pub async fn verify_destroyed(
        &self,
        target_id: &str,
    ) -> Result<Option<ExternalTerminationState>> {
        match self.describe_instance(target_id).await {
            Ok(observed) if is_terminal_instance_state(&observed.instance_state) => {
                Ok(Some(ExternalTerminationState::Terminated))
            }
            Ok(_) => Ok(None),
            Err(error) if is_not_found_error(&error) => {
                Ok(Some(ExternalTerminationState::NotFound))
            }
            Err(error) => Err(error),
        }
    }

    pub async fn terminate_all(
        &self,
        target_ids: &[String],
    ) -> Result<Vec<ExternalTerminationEvidence>> {
        if target_ids.is_empty() {
            return Ok(Vec::new());
        }
        if let BackendKind::Mock(state) = self.backend.as_ref() {
            let cleanup_delay_ms = {
                let mut state = state
                    .lock()
                    .map_err(|_| anyhow!("mock mutation state lock poisoned"))?;
                if state.fail_cleanup {
                    bail!("injected mutation cleanup failure");
                }
                for target_id in target_ids {
                    state.instances.remove(target_id);
                }
                state.cleanup_delay_ms
            };
            if let Some(delay_ms) = cleanup_delay_ms {
                sleep(StdDuration::from_millis(delay_ms)).await;
            }
            return Ok(target_ids
                .iter()
                .map(|resource_id| ExternalTerminationEvidence {
                    resource_id: resource_id.clone(),
                    state: ExternalTerminationState::NotFound,
                })
                .collect());
        }
        self.aws_client()
            .terminate_instances()
            .set_instance_ids(Some(target_ids.to_vec()))
            .send()
            .await
            .context("terminate disposable mutation targets")?;
        let deadline = Utc::now() + Duration::seconds(30);
        loop {
            let mut evidence = Vec::with_capacity(target_ids.len());
            for target_id in target_ids {
                if let Some(state) = self.verify_destroyed(target_id).await? {
                    evidence.push(ExternalTerminationEvidence {
                        resource_id: target_id.clone(),
                        state,
                    });
                }
            }
            if evidence.len() == target_ids.len() {
                return Ok(evidence);
            }
            if Utc::now() >= deadline {
                bail!(
                    "timed out waiting for mutation targets to reach terminated or not-found state"
                )
            }
            sleep(StdDuration::from_millis(200)).await;
        }
    }

    async fn wait_for_instance(
        &self,
        target_id: &str,
        expected_state: &str,
        expected_type: &str,
    ) -> Result<ObservedInstance> {
        let deadline = Utc::now() + Duration::seconds(30);
        loop {
            let observed = self.describe_instance(target_id).await?;
            if observed.instance_state == expected_state && observed.instance_type == expected_type
            {
                return Ok(observed);
            }
            if Utc::now() >= deadline {
                bail!(
                    "timed out waiting for target {target_id}: expected {expected_state}:{expected_type}, observed {}:{}",
                    observed.instance_state,
                    observed.instance_type
                )
            }
            sleep(StdDuration::from_millis(200)).await;
        }
    }

    async fn wait_for_instance_state(
        &self,
        target_id: &str,
        expected_state: &str,
    ) -> Result<ObservedInstance> {
        let deadline = Utc::now() + Duration::seconds(30);
        loop {
            let observed = self.describe_instance(target_id).await?;
            if observed.instance_state == expected_state {
                return Ok(observed);
            }
            if Utc::now() >= deadline {
                bail!(
                    "timed out waiting for target {target_id}: expected state {expected_state}, observed {}:{}",
                    observed.instance_state,
                    observed.instance_type
                )
            }
            sleep(StdDuration::from_millis(200)).await;
        }
    }
}

fn is_not_found_error(error: &anyhow::Error) -> bool {
    if error.chain().any(|cause| {
        cause
            .downcast_ref::<SdkError<DescribeInstancesError>>()
            .and_then(|sdk_error| sdk_error.as_service_error())
            .and_then(|service_error| service_error.code())
            .is_some_and(|code| {
                code.contains("NotFound") || code.contains("not found") || code == "NoSuchEntity"
            })
    }) {
        return true;
    }
    let message = error.to_string().to_ascii_lowercase();
    message.contains("no target")
        || message.contains("notfound")
        || message.contains("not found")
        || message.contains("does not exist")
}

fn is_terminal_instance_state(state: &str) -> bool {
    state == "terminated"
}

fn definitely_pre_dispatch_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("connection refused")
        || message.contains("connectionrefused")
        || message.contains("failed to resolve")
        || message.contains("name or service not known")
        || message.contains("no such host")
}

#[cfg(test)]
mod tests {
    use super::is_terminal_instance_state;

    #[test]
    fn only_terminated_is_external_destruction_proof() {
        assert!(is_terminal_instance_state("terminated"));
        for state in ["running", "stopped", "shutting-down", "unknown"] {
            assert!(!is_terminal_instance_state(state), "state={state}");
        }
    }
}
