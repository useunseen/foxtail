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
pub struct Observation {
    pub resource_id: String,
    pub instance_state: String,
    pub instance_type: String,
    pub availability_zone: String,
    pub tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestObservations {
    pub account_id: String,
    pub anchor: String,
    pub observations: Vec<Observation>,
}

pub fn parse_query_from_form(body: &[u8]) -> std::result::Result<Query, String> {
    let pairs: Vec<(String, String)> = serde_urlencoded::from_bytes(body)
        .map_err(|error| format!("Failed to parse EC2 Query body: {error}"))?;
    let mut action = None;
    let mut instance_ids = BTreeMap::new();
    for (key, value) in pairs {
        match key.as_str() {
            "Action" => action = Some(value),
            key if key.starts_with("InstanceId.") => {
                let index = key
                    .strip_prefix("InstanceId.")
                    .filter(|index| !index.is_empty())
                    .and_then(|index| index.parse::<usize>().ok())
                    .ok_or_else(|| format!("Invalid InstanceId member '{}'.", key))?;
                if index == 0 {
                    return Err(format!("Invalid InstanceId member '{}'.", key));
                }
                if instance_ids.insert(index, value).is_some() {
                    return Err(format!("Duplicate InstanceId member '{}'.", key));
                }
            }
            // AWS Query requests carry Version and may include filters. The
            // fixture surface only needs the identity selector; unsupported
            // members are intentionally ignored for CLI compatibility.
            _ => {}
        }
    }
    Ok(Query {
        action: action.ok_or_else(|| "Missing required field 'Action'.".to_string())?,
        instance_ids: instance_ids.into_values().collect(),
    })
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
        observations.push(Observation {
            resource_id,
            instance_state,
            instance_type,
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
    })
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
        ManifestObservations, Observation, Query, action_from_form, describe_instances_xml,
        observations_from_manifest, parse_query_from_form, state_code, validate_describe_instances,
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
    fn serializer_escapes_xml_and_maps_ec2_state_codes() {
        assert_eq!(state_code("running"), "16");
        assert_eq!(state_code("stopped"), "80");
        assert_eq!(state_code("unknown"), "0");
        let observation = Observation {
            resource_id: "i-<escaped>".to_string(),
            instance_state: "running".to_string(),
            instance_type: "m6i&large".to_string(),
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
        let _: ManifestObservations = shaped;
    }
}
