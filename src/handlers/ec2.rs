use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use crate::fixture;

/// The subset of the EC2 Query protocol needed by the read-only fixture
/// observation surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub action: String,
    pub instance_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribeInstanceAttributeQuery {
    pub instance_id: String,
    pub attribute: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribeInstanceTypesQuery {
    pub instance_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ec2Query {
    Instances(Query),
    InstanceAttribute(DescribeInstanceAttributeQuery),
    InstanceTypes(DescribeInstanceTypesQuery),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceTypeObservation {
    pub instance_type: String,
    pub supported_root_device_types: Vec<String>,
    pub supported_virtualization_types: Vec<String>,
    pub supported_architectures: Vec<String>,
    pub ena_support: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub resource_id: String,
    pub instance_state: String,
    pub instance_type: String,
    pub disable_api_termination: bool,
    pub availability_zone: String,
    pub tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestObservations {
    pub account_id: String,
    pub anchor: String,
    pub observations: Vec<Observation>,
    pub instance_type_catalogue: Vec<InstanceTypeObservation>,
}

#[allow(dead_code)]
pub fn parse_query_from_form(body: &[u8]) -> std::result::Result<Query, String> {
    let pairs: Vec<(String, String)> = serde_urlencoded::from_bytes(body)
        .map_err(|error| format!("Failed to parse EC2 Query body: {error}"))?;
    let mut action = None;
    let mut instance_ids = BTreeMap::new();
    for (key, value) in pairs {
        match key.as_str() {
            "Action" => set_unique(&mut action, value, "Action")?,
            key if key.starts_with("InstanceId.") => {
                insert_member(&mut instance_ids, key, value, "InstanceId")?
            }
            // Keep the legacy parser available for DescribeInstances-focused
            // callers; the dispatcher uses parse_ec2_query_from_form for all
            // supported observation operations and therefore rejects these
            // members for the wrong action.
            _ => {}
        }
    }
    Ok(Query {
        action: action.ok_or_else(|| "Missing required field 'Action'.".to_string())?,
        instance_ids: contiguous_members(instance_ids, "InstanceId")?,
    })
}

pub fn parse_ec2_query_from_form(body: &[u8]) -> std::result::Result<Ec2Query, String> {
    let pairs: Vec<(String, String)> = serde_urlencoded::from_bytes(body)
        .map_err(|error| format!("Failed to parse EC2 Query body: {error}"))?;
    let mut action = None;
    let mut instance_id = None;
    let mut attribute = None;
    let mut instance_ids = BTreeMap::new();
    let mut instance_types = BTreeMap::new();
    let mut next_token = None;
    let mut max_results = None;
    for (key, value) in pairs {
        match key.as_str() {
            "Action" => set_unique(&mut action, value, "Action")?,
            "Version" => {}
            "InstanceId" => set_unique(&mut instance_id, value, "InstanceId")?,
            "Attribute" => set_unique(&mut attribute, value, "Attribute")?,
            "NextToken" => set_unique(&mut next_token, value, "NextToken")?,
            "MaxResults" => set_unique(&mut max_results, value, "MaxResults")?,
            key if key.starts_with("InstanceId.") => {
                insert_member(&mut instance_ids, key, value, "InstanceId")?
            }
            key if key.starts_with("InstanceType.") || key.starts_with("InstanceTypes.") => {
                insert_member_any_prefix(&mut instance_types, key, value, "InstanceType")?
            }
            _ => return Err(format!("Unsupported EC2 Query member '{key}'.")),
        }
    }

    let action = action.ok_or_else(|| "Missing required field 'Action'.".to_string())?;
    match action.as_str() {
        "DescribeInstances" => {
            if instance_id.is_some()
                || attribute.is_some()
                || !instance_types.is_empty()
                || next_token.is_some()
                || max_results.is_some()
            {
                return Err("DescribeInstances received an unsupported member.".to_string());
            }
            let instance_ids = contiguous_members(instance_ids, "InstanceId")?;
            Ok(Ec2Query::Instances(Query {
                action,
                instance_ids,
            }))
        }
        "DescribeInstanceAttribute" => {
            if !instance_ids.is_empty()
                || !instance_types.is_empty()
                || next_token.is_some()
                || max_results.is_some()
            {
                return Err("DescribeInstanceAttribute received an unsupported member.".to_string());
            }
            let instance_id = required_scalar(instance_id, "InstanceId")?;
            let attribute = required_scalar(attribute, "Attribute")?;
            if attribute != "disableApiTermination" {
                return Err(format!("The attribute '{attribute}' is not supported."));
            }
            Ok(Ec2Query::InstanceAttribute(
                DescribeInstanceAttributeQuery {
                    instance_id,
                    attribute,
                },
            ))
        }
        "DescribeInstanceTypes" => {
            if instance_id.is_some()
                || attribute.is_some()
                || next_token.is_some()
                || max_results.is_some()
            {
                return Err("DescribeInstanceTypes received an unsupported member.".to_string());
            }
            let instance_types = contiguous_members(instance_types, "InstanceType")?;
            if instance_types.is_empty() {
                return Err(
                    "DescribeInstanceTypes requires at least one InstanceType member.".to_string(),
                );
            }
            if instance_types.iter().collect::<BTreeSet<_>>().len() != instance_types.len() {
                return Err("DescribeInstanceTypes received duplicate instance types.".to_string());
            }
            Ok(Ec2Query::InstanceTypes(DescribeInstanceTypesQuery {
                instance_types,
            }))
        }
        _ => Err(format!("The action '{}' is not supported", action)),
    }
}

fn set_unique<T>(slot: &mut Option<T>, value: T, name: &str) -> std::result::Result<(), String> {
    if slot.is_some() {
        return Err(format!("Duplicate {name} member."));
    }
    *slot = Some(value);
    Ok(())
}

fn insert_member(
    members: &mut BTreeMap<usize, String>,
    key: &str,
    value: String,
    name: &str,
) -> std::result::Result<(), String> {
    let index = key
        .strip_prefix(&format!("{name}."))
        .filter(|index| !index.is_empty())
        .and_then(|index| index.parse::<usize>().ok())
        .ok_or_else(|| format!("Invalid {name} member '{key}'."))?;
    if index == 0 {
        return Err(format!("Invalid {name} member '{key}'."));
    }
    if members.insert(index, value).is_some() {
        return Err(format!("Duplicate {name} member '{key}'."));
    }
    Ok(())
}

fn insert_member_any_prefix(
    members: &mut BTreeMap<usize, String>,
    key: &str,
    value: String,
    name: &str,
) -> std::result::Result<(), String> {
    let suffix = key
        .strip_prefix("InstanceType.")
        .or_else(|| key.strip_prefix("InstanceTypes."))
        .filter(|index| !index.is_empty())
        .and_then(|index| index.parse::<usize>().ok())
        .ok_or_else(|| format!("Invalid {name} member '{key}'."))?;
    if suffix == 0 {
        return Err(format!("Invalid {name} member '{key}'."));
    }
    if members.insert(suffix, value).is_some() {
        return Err(format!("Duplicate {name} member '{key}'."));
    }
    Ok(())
}

fn contiguous_members(
    members: BTreeMap<usize, String>,
    name: &str,
) -> std::result::Result<Vec<String>, String> {
    let expected = (1..=members.len()).collect::<Vec<_>>();
    let actual = members.keys().copied().collect::<Vec<_>>();
    if actual != expected {
        return Err(format!(
            "{name} members must use contiguous indexes starting at 1."
        ));
    }
    Ok(members.into_values().collect())
}

fn required_scalar(value: Option<String>, name: &str) -> std::result::Result<String, String> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("Missing required field '{name}'."))
}

pub fn action_from_form(body: &[u8]) -> Option<String> {
    serde_urlencoded::from_bytes::<Vec<(String, String)>>(body)
        .ok()
        .and_then(|pairs| {
            pairs
                .into_iter()
                .find_map(|(key, value)| (key == "Action").then_some(value))
        })
}

#[allow(dead_code)]
pub fn validate_describe_instances(query: &Query) -> std::result::Result<(), String> {
    if query.action == "DescribeInstances" {
        Ok(())
    } else {
        Err(format!("The action '{}' is not supported", query.action))
    }
}

pub fn observations_from_manifest(manifest: &Value) -> Result<ManifestObservations> {
    let environment = manifest
        .get("environment")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("fixture manifest environment is missing"))?;
    let account_id = environment
        .get("account_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("fixture manifest account_id is missing"))?
        .to_string();
    let region = environment
        .get("region")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("fixture manifest region is missing"))?;
    let anchor = manifest
        .pointer("/clock/anchor")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("fixture manifest clock anchor is missing"))?
        .to_string();
    let resources = manifest
        .get("resources")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("fixture manifest resources are missing"))?;
    let instance_type_catalogue = instance_type_catalogue_from_manifest(manifest)?;
    let catalogue_types = instance_type_catalogue
        .iter()
        .map(|item| item.instance_type.as_str())
        .collect::<BTreeSet<_>>();
    if resources.len() != fixture::REALIZED_CONTROL_IDS.len() {
        bail!(
            "fixture manifest must expose exactly {} read-only EC2 resources; found {}",
            fixture::REALIZED_CONTROL_IDS.len(),
            resources.len()
        );
    }
    let mutation_ids = manifest
        .get("mutation_resources")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("fixture manifest mutation_resources are missing or malformed"))?
        .iter()
        .filter_map(|resource| resource.get("resource_id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();

    let mut observations = Vec::with_capacity(resources.len());
    let mut seen_ids = BTreeSet::new();
    let mut seen_controls = BTreeSet::new();
    for resource in resources {
        if resource.get("resource_type").and_then(Value::as_str) != Some("ec2") {
            bail!("fixture manifest contains a non-EC2 read-only resource");
        }
        let control_id = resource
            .get("control_id")
            .and_then(Value::as_str)
            .filter(|control_id| fixture::REALIZED_CONTROL_IDS.contains(control_id))
            .ok_or_else(|| anyhow!("fixture manifest contains an unknown read-only control"))?;
        if !seen_controls.insert(control_id.to_string()) {
            bail!("fixture manifest contains duplicate read-only control '{control_id}'");
        }
        let (expected_role, expected_scenario) = fixture::role_and_intent(control_id);
        if resource.get("role").and_then(Value::as_str) != Some(expected_role)
            || resource.get("scenario").and_then(Value::as_str) != Some(expected_scenario)
        {
            bail!("fixture manifest control '{control_id}' has contradictory role or scenario");
        }
        let resource_id = resource
            .get("resource_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| anyhow!("fixture manifest resource_id is missing"))?
            .to_string();
        if !seen_ids.insert(resource_id.clone()) {
            bail!("fixture manifest contains duplicate EC2 resource_id '{resource_id}'");
        }
        if mutation_ids.contains(resource_id.as_str()) {
            bail!("fixture manifest exposes a mutation target on the read-only surface");
        }
        let expected_arn = format!("arn:aws:ec2:{region}:{account_id}:instance/{resource_id}");
        if resource.get("aws_identity").and_then(Value::as_str) != Some(expected_arn.as_str()) {
            bail!("fixture manifest EC2 identity for '{resource_id}' contradicts its scope");
        }
        let observed = resource
            .get("observed")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("fixture manifest observed metadata is missing"))?;
        let instance_state = observed
            .get("instance_state")
            .and_then(Value::as_str)
            .filter(|state| !state.is_empty())
            .ok_or_else(|| anyhow!("fixture manifest instance_state is missing"))?
            .to_string();
        if !matches!(
            instance_state.as_str(),
            "pending" | "running" | "stopped" | "shutting-down" | "terminated" | "stopping"
        ) {
            bail!("fixture manifest control '{control_id}' has an invalid instance state");
        }
        let instance_type = observed
            .get("instance_type")
            .and_then(Value::as_str)
            .filter(|instance_type| !instance_type.is_empty())
            .ok_or_else(|| anyhow!("fixture manifest instance_type is missing"))?
            .to_string();
        if !catalogue_types.contains(instance_type.as_str()) {
            bail!(
                "fixture manifest control '{control_id}' uses an instance type outside the exact catalogue"
            );
        }
        let disable_api_termination = observed
            .get("disable_api_termination")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                anyhow!("fixture manifest disable_api_termination is missing or malformed")
            })?;
        let availability_zone = observed
            .get("availability_zone")
            .and_then(Value::as_str)
            .filter(|zone| !zone.is_empty())
            .ok_or_else(|| anyhow!("fixture manifest availability_zone is missing"))?
            .to_string();
        if !availability_zone.starts_with(region) {
            bail!("fixture manifest control '{control_id}' has an out-of-scope availability zone");
        }
        let tags = parse_tags(observed.get("tags"))?;
        let expected_tags = [
            ("Name", resource_id.as_str()),
            ("FoxtailFixture", fixture::FIXTURE_VERSION),
            ("FoxtailControl", control_id),
            ("FoxtailRole", expected_role),
            ("FoxtailScenario", expected_scenario),
        ];
        if expected_tags
            .iter()
            .any(|(key, expected)| tags.get(*key).map(String::as_str) != Some(*expected))
        {
            bail!("fixture manifest control '{control_id}' has contradictory fixture tags");
        }
        if let Some(control_catalogue) = manifest.get("control_catalogue").and_then(Value::as_array)
        {
            let catalogue_entry = control_catalogue
                .iter()
                .find(|entry| entry.get("control_id").and_then(Value::as_str) == Some(control_id))
                .ok_or_else(|| {
                    anyhow!(
                        "fixture manifest control catalogue is missing read-only control '{control_id}'"
                    )
                })?;
            let catalogue_observed = catalogue_entry
                .get("observed")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    anyhow!(
                        "fixture manifest control catalogue observation for '{control_id}' is missing"
                    )
                })?;
            for field in [
                "instance_state",
                "instance_type",
                "disable_api_termination",
                "availability_zone",
                "tags",
            ] {
                if catalogue_observed.get(field) != observed.get(field) {
                    bail!(
                        "fixture manifest control '{control_id}' has contradictory duplicated observation field '{field}'"
                    );
                }
            }
        }
        observations.push(Observation {
            resource_id,
            instance_state,
            instance_type,
            disable_api_termination,
            availability_zone,
            tags,
        });
    }
    let expected_controls = fixture::REALIZED_CONTROL_IDS
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    if seen_controls != expected_controls {
        bail!("fixture manifest read-only controls are missing or extra");
    }
    Ok(ManifestObservations {
        account_id,
        anchor,
        observations,
        instance_type_catalogue,
    })
}

fn instance_type_catalogue_from_manifest(manifest: &Value) -> Result<Vec<InstanceTypeObservation>> {
    let values = manifest
        .get("ec2_instance_type_catalogue")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("fixture manifest EC2 instance type catalogue is missing"))?;
    if values.len() != fixture::EC2_INSTANCE_TYPE_CATALOGUE.len() {
        bail!(
            "fixture manifest EC2 instance type catalogue must contain exactly {} records",
            fixture::EC2_INSTANCE_TYPE_CATALOGUE.len()
        );
    }
    let mut catalogue = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let object = value.as_object().ok_or_else(|| {
            anyhow!("fixture manifest EC2 instance type catalogue record is malformed")
        })?;
        let instance_type = object
            .get("instance_type")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("fixture manifest EC2 instance type is missing"))?
            .to_string();
        if !seen.insert(instance_type.clone()) {
            bail!("fixture manifest EC2 instance type catalogue contains a duplicate");
        }
        let supported_root_device_types = string_list(object, "supported_root_device_types")?;
        let supported_virtualization_types = string_list(object, "supported_virtualization_types")?;
        let supported_architectures = string_list(object, "supported_architectures")?;
        let ena_support = object
            .get("ena_support")
            .and_then(Value::as_str)
            .filter(|value| matches!(*value, "required" | "supported" | "unsupported"))
            .ok_or_else(|| anyhow!("fixture manifest EC2 EnaSupport is missing or malformed"))?
            .to_string();
        catalogue.push(InstanceTypeObservation {
            instance_type,
            supported_root_device_types,
            supported_virtualization_types,
            supported_architectures,
            ena_support,
        });
    }
    let expected = fixture::EC2_INSTANCE_TYPE_CATALOGUE
        .iter()
        .map(|value| (*value).to_string())
        .collect::<BTreeSet<_>>();
    if seen != expected {
        bail!(
            "fixture manifest EC2 instance type catalogue is missing or contains unsupported types"
        );
    }
    let expected_facts = fixture::ec2_instance_type_catalogue_value();
    if expected_facts.as_array().map(Vec::as_slice) != Some(values.as_slice()) {
        bail!("fixture manifest EC2 instance type catalogue facts are inconsistent");
    }
    Ok(catalogue)
}

fn string_list(object: &serde_json::Map<String, Value>, key: &str) -> Result<Vec<String>> {
    let values = object.get(key).and_then(Value::as_array).ok_or_else(|| {
        anyhow!("fixture manifest EC2 catalogue field '{key}' is missing or malformed")
    })?;
    if values.is_empty() {
        bail!("fixture manifest EC2 catalogue field '{key}' must not be empty");
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .ok_or_else(|| {
                    anyhow!(
                        "fixture manifest EC2 catalogue field '{key}' contains a malformed value"
                    )
                })
        })
        .collect()
}

fn parse_tags(value: Option<&Value>) -> Result<BTreeMap<String, String>> {
    let object = value
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("fixture EC2 observation tags are missing or malformed"))?;
    object
        .iter()
        .map(|(key, value)| {
            let value = value
                .as_str()
                .ok_or_else(|| anyhow!("fixture EC2 tag '{key}' is not a string"))?;
            Ok((key.clone(), value.to_string()))
        })
        .collect()
}

pub fn describe_instances_xml(
    account_id: &str,
    anchor: &str,
    observations: &[Observation],
) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DescribeInstancesResponse xmlns=\"http://ec2.amazonaws.com/doc/2016-11-15/\"><requestId>foxtail-fixture</requestId><reservationSet><item><reservationId>r-foxtail-fixture</reservationId><ownerId>",
    );
    xml.push_str(&xml_escape(account_id));
    xml.push_str("</ownerId><groupSet/><instancesSet>");
    for observation in observations {
        xml.push_str("<item><instanceId>");
        xml.push_str(&xml_escape(&observation.resource_id));
        xml.push_str("</instanceId><imageId>ami-foxtail-fixture</imageId><instanceState><code>");
        xml.push_str(state_code(&observation.instance_state));
        xml.push_str("</code><name>");
        xml.push_str(&xml_escape(&observation.instance_state));
        xml.push_str("</name></instanceState><instanceType>");
        xml.push_str(&xml_escape(&observation.instance_type));
        xml.push_str("</instanceType><launchTime>");
        xml.push_str(&xml_escape(anchor));
        xml.push_str("</launchTime><placement><availabilityZone>");
        xml.push_str(&xml_escape(&observation.availability_zone));
        xml.push_str("</availabilityZone><tenancy>default</tenancy></placement><monitoring><state>disabled</state></monitoring><tagSet>");
        for (key, value) in &observation.tags {
            xml.push_str("<item><key>");
            xml.push_str(&xml_escape(key));
            xml.push_str("</key><value>");
            xml.push_str(&xml_escape(value));
            xml.push_str("</value></item>");
        }
        xml.push_str("</tagSet></item>");
    }
    xml.push_str("</instancesSet></item></reservationSet></DescribeInstancesResponse>");
    xml
}

pub fn describe_instance_attribute_xml(
    observation: &Observation,
    query: &DescribeInstanceAttributeQuery,
) -> String {
    let value = if observation.disable_api_termination {
        "true"
    } else {
        "false"
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DescribeInstanceAttributeResponse xmlns=\"http://ec2.amazonaws.com/doc/2016-11-15/\"><requestId>foxtail-fixture</requestId><instanceId>{}</instanceId><disableApiTermination><value>{value}</value></disableApiTermination></DescribeInstanceAttributeResponse>",
        xml_escape(&query.instance_id)
    )
}

pub fn describe_instance_types_xml(types: &[InstanceTypeObservation]) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DescribeInstanceTypesResponse xmlns=\"http://ec2.amazonaws.com/doc/2016-11-15/\"><requestId>foxtail-fixture</requestId><instanceTypeSet>",
    );
    for instance_type in types {
        xml.push_str("<item><instanceType>");
        xml.push_str(&xml_escape(&instance_type.instance_type));
        xml.push_str("</instanceType><supportedRootDeviceTypes>");
        for value in &instance_type.supported_root_device_types {
            xml.push_str("<item>");
            xml.push_str(&xml_escape(value));
            xml.push_str("</item>");
        }
        xml.push_str("</supportedRootDeviceTypes><supportedVirtualizationTypes>");
        for value in &instance_type.supported_virtualization_types {
            xml.push_str("<item>");
            xml.push_str(&xml_escape(value));
            xml.push_str("</item>");
        }
        xml.push_str("</supportedVirtualizationTypes><processorInfo><supportedArchitectures>");
        for value in &instance_type.supported_architectures {
            xml.push_str("<item>");
            xml.push_str(&xml_escape(value));
            xml.push_str("</item>");
        }
        xml.push_str("</supportedArchitectures></processorInfo><networkInfo><enaSupport>");
        xml.push_str(&xml_escape(&instance_type.ena_support));
        xml.push_str("</enaSupport></networkInfo></item>");
    }
    xml.push_str("</instanceTypeSet></DescribeInstanceTypesResponse>");
    xml
}

pub fn state_code(state: &str) -> &'static str {
    match state {
        "pending" => "0",
        "running" => "16",
        "shutting-down" => "32",
        "terminated" => "48",
        "stopping" => "64",
        "stopped" => "80",
        _ => "0",
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::{
        DescribeInstanceTypesQuery, Ec2Query, ManifestObservations, Observation, Query,
        action_from_form, describe_instance_attribute_xml, describe_instance_types_xml,
        describe_instances_xml, observations_from_manifest, parse_ec2_query_from_form,
        parse_query_from_form, state_code, validate_describe_instances,
    };
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    fn manifest() -> Value {
        let controls = [
            (
                "ec2-idle-positive-001",
                "positive",
                "ec2.idle.complete-history",
            ),
            (
                "ec2-idle-negative-001",
                "negative",
                "ec2.busy.complete-history",
            ),
            (
                "ec2-idle-degraded-001",
                "degraded",
                "ec2.idle.scoped-missing-day",
            ),
            (
                "ec2-resize-positive-001",
                "positive",
                "ec2.resize.fresh-compatible-recommendation",
            ),
            (
                "ec2-resize-negative-001",
                "negative",
                "ec2.resize.no-compatible-recommendation",
            ),
        ];
        let resources = controls
            .into_iter()
            .enumerate()
            .map(|(index, (control_id, role, scenario))| {
                let resource_id = format!("i-handler-{index}");
                json!({
                    "control_id": control_id,
                    "role": role,
                    "resource_id": resource_id,
                    "resource_type": "ec2",
                    "aws_identity": format!("arn:aws:ec2:us-east-1:123456789012:instance/{resource_id}"),
                    "scenario": scenario,
                    "observed": {
                        "instance_state": "running",
                        "instance_type": "m6i.large",
                        "disable_api_termination": false,
                        "availability_zone": "us-east-1a",
                        "tags": {
                            "Name": resource_id,
                            "FoxtailFixture": "release-qualification-v1",
                            "FoxtailControl": control_id,
                            "FoxtailRole": role,
                            "FoxtailScenario": scenario
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        json!({
            "environment": {"account_id": "123456789012", "region": "us-east-1"},
            "clock": {"anchor": "2026-08-05T00:00:00Z"},
            "ec2_instance_type_catalogue": [
                {"instance_type":"m6i.large","supported_root_device_types":["ebs"],"supported_virtualization_types":["hvm"],"supported_architectures":["x86_64"],"ena_support":"required"},
                {"instance_type":"t3.medium","supported_root_device_types":["ebs"],"supported_virtualization_types":["hvm"],"supported_architectures":["x86_64"],"ena_support":"required"},
                {"instance_type":"m6i.xlarge","supported_root_device_types":["ebs"],"supported_virtualization_types":["hvm"],"supported_architectures":["x86_64"],"ena_support":"required"}
            ],
            "resources": resources,
            "mutation_resources": []
        })
    }

    #[test]
    fn parser_rejects_malformed_and_zero_member_indexes() {
        assert!(parse_query_from_form(b"%zz").is_err());
        assert!(parse_query_from_form(b"Action=DescribeInstances&InstanceId.0=i-1").is_err());
        assert!(
            parse_query_from_form(b"Action=DescribeInstances&InstanceId.1=i-1&InstanceId.1=i-2")
                .is_err()
        );
    }

    #[test]
    fn parser_requires_action_and_rejects_unsupported_action() {
        assert!(parse_query_from_form(b"Version=2016-11-15").is_err());
        let query = parse_query_from_form(b"Action=DescribeImages").unwrap();
        assert_eq!(query.action, "DescribeImages");
        assert!(validate_describe_instances(&query).is_err());
        assert_eq!(
            action_from_form(b"Action=DescribeImages"),
            Some("DescribeImages".to_string())
        );
    }

    #[test]
    fn manifest_validation_rejects_duplicate_unknown_missing_and_contradictory_controls() {
        let base = manifest();
        let mut duplicate = base.clone();
        duplicate["resources"][1]["control_id"] = duplicate["resources"][0]["control_id"].clone();
        assert!(observations_from_manifest(&duplicate).is_err());

        let mut unknown = base.clone();
        unknown["resources"][0]["control_id"] = json!("ec2-unknown");
        assert!(observations_from_manifest(&unknown).is_err());

        let mut missing = base.clone();
        missing["resources"].as_array_mut().unwrap().pop();
        assert!(observations_from_manifest(&missing).is_err());

        let mut scope = base.clone();
        scope["resources"][0]["aws_identity"] =
            json!("arn:aws:ec2:us-west-2:123456789012:instance/i-handler-0");
        assert!(observations_from_manifest(&scope).is_err());

        let mut tags = base;
        tags["resources"][0]["observed"]["tags"]["FoxtailControl"] = json!("wrong-control");
        assert!(observations_from_manifest(&tags).is_err());
    }

    #[test]
    fn manifest_validation_rejects_invalid_state_and_mutation_overlap() {
        let mut invalid_state = manifest();
        invalid_state["resources"][0]["observed"]["instance_state"] = json!("unknown");
        assert!(observations_from_manifest(&invalid_state).is_err());

        let mut overlap = manifest();
        overlap["mutation_resources"] = json!([{"resource_id": "i-handler-0"}]);
        assert!(observations_from_manifest(&overlap).is_err());
    }

    #[test]
    fn manifest_validation_requires_fixture_owned_attribute_and_type_facts() {
        let base = manifest();
        let mut missing_attribute = base.clone();
        missing_attribute["resources"][0]["observed"]
            .as_object_mut()
            .unwrap()
            .remove("disable_api_termination");
        assert!(observations_from_manifest(&missing_attribute).is_err());

        let mut malformed_attribute = base.clone();
        malformed_attribute["resources"][0]["observed"]["disable_api_termination"] = json!("false");
        assert!(observations_from_manifest(&malformed_attribute).is_err());

        let mut missing_catalogue = base.clone();
        missing_catalogue["ec2_instance_type_catalogue"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(observations_from_manifest(&missing_catalogue).is_err());

        let mut contradictory_catalogue = base;
        contradictory_catalogue["ec2_instance_type_catalogue"][0]["instance_type"] =
            json!("m5.large");
        assert!(observations_from_manifest(&contradictory_catalogue).is_err());

        let mut contradictory_facts = manifest();
        contradictory_facts["ec2_instance_type_catalogue"][0]["ena_support"] = json!("supported");
        assert!(observations_from_manifest(&contradictory_facts).is_err());

        let mut duplicated_observation: Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/release-qualification-v1.manifest.json"
        ))
        .unwrap();
        duplicated_observation["control_catalogue"][0]["observed"]["disable_api_termination"] =
            json!(true);
        assert!(observations_from_manifest(&duplicated_observation).is_err());
    }

    #[test]
    fn serializer_escapes_xml_and_maps_ec2_state_codes() {
        assert_eq!(state_code("running"), "16");
        assert_eq!(state_code("stopped"), "80");
        assert_eq!(state_code("unknown"), "0");
        let observation = Observation {
            resource_id: "i-<escaped>".to_string(),
            instance_state: "running".to_string(),
            instance_type: "m6i&large".to_string(),
            disable_api_termination: false,
            availability_zone: "us-east-1a".to_string(),
            tags: BTreeMap::from([("k<>&\"'".to_string(), "v<>&\"'".to_string())]),
        };
        let xml = describe_instances_xml("123456789012", "2026-08-05T00:00:00Z", &[observation]);
        assert!(xml.contains("i-&lt;escaped&gt;"));
        assert!(xml.contains("m6i&amp;large"));
        assert!(xml.contains("k&lt;&gt;&amp;&quot;&apos;"));
        assert!(xml.contains("<code>16</code>"));
    }

    #[test]
    fn parser_preserves_ordered_instance_members() {
        let query =
            parse_query_from_form(b"Action=DescribeInstances&InstanceId.2=i-2&InstanceId.1=i-1")
                .unwrap();
        assert_eq!(
            query,
            Query {
                action: "DescribeInstances".to_string(),
                instance_ids: vec!["i-1".to_string(), "i-2".to_string()]
            }
        );
    }

    #[test]
    fn observations_shape_is_stable() {
        let shaped = observations_from_manifest(&manifest()).unwrap();
        assert_eq!(shaped.account_id, "123456789012");
        assert_eq!(shaped.anchor, "2026-08-05T00:00:00Z");
        assert_eq!(shaped.observations.len(), 5);
        assert_eq!(shaped.instance_type_catalogue.len(), 3);
        let _: ManifestObservations = shaped;
    }

    #[test]
    fn parser_supports_exact_attribute_and_type_forms_and_rejects_duplicates() {
        let attribute = parse_ec2_query_from_form(
            b"Action=DescribeInstanceAttribute&Version=2016-11-15&InstanceId=i-1&Attribute=disableApiTermination",
        )
        .unwrap();
        assert!(matches!(attribute, Ec2Query::InstanceAttribute(_)));

        let types = parse_ec2_query_from_form(
            b"Action=DescribeInstanceTypes&InstanceType.2=m6i.xlarge&InstanceType.1=m6i.large",
        )
        .unwrap();
        assert_eq!(
            types,
            Ec2Query::InstanceTypes(DescribeInstanceTypesQuery {
                instance_types: vec!["m6i.large".to_string(), "m6i.xlarge".to_string()]
            })
        );
        for body in [
            b"Action=DescribeInstanceAttribute&Action=DescribeInstances&InstanceId=i-1&Attribute=disableApiTermination" as &[u8],
            b"Action=DescribeInstanceAttribute&InstanceId=i-1&InstanceId=i-2&Attribute=disableApiTermination",
            b"Action=DescribeInstanceTypes&InstanceType.1=m6i.large&InstanceType.1=t3.medium",
            b"Action=DescribeInstanceTypes&InstanceType.2=m6i.large",
            b"Action=DescribeInstanceTypes&InstanceType.1=m6i.large&InstanceType.2=m6i.large",
            b"Action=DescribeInstanceTypes&InstanceType.1=m6i.large&NextToken=opaque",
        ] {
            assert!(parse_ec2_query_from_form(body).is_err());
        }
    }

    #[test]
    fn serializers_emit_aws_cli_attribute_and_type_shapes() {
        let observation = Observation {
            resource_id: "i-1".to_string(),
            instance_state: "running".to_string(),
            instance_type: "m6i.large".to_string(),
            disable_api_termination: true,
            availability_zone: "us-east-1a".to_string(),
            tags: BTreeMap::new(),
        };
        let query = super::DescribeInstanceAttributeQuery {
            instance_id: "i-1".to_string(),
            attribute: "disableApiTermination".to_string(),
        };
        let attribute_xml = describe_instance_attribute_xml(&observation, &query);
        assert!(attribute_xml.contains("<instanceId>i-1</instanceId>"));
        assert!(attribute_xml.contains("<disableApiTermination><value>true</value>"));
        let type_xml = describe_instance_types_xml(&[super::InstanceTypeObservation {
            instance_type: "m6i.large".to_string(),
            supported_root_device_types: vec!["ebs".to_string()],
            supported_virtualization_types: vec!["hvm".to_string()],
            supported_architectures: vec!["x86_64".to_string()],
            ena_support: "required".to_string(),
        }]);
        for field in [
            "<instanceType>m6i.large</instanceType>",
            "<supportedRootDeviceTypes><item>ebs</item>",
            "<supportedVirtualizationTypes><item>hvm</item>",
            "<processorInfo><supportedArchitectures><item>x86_64</item>",
            "<networkInfo><enaSupport>required</enaSupport>",
        ] {
            assert!(type_xml.contains(field), "missing {field}");
        }
    }
}
