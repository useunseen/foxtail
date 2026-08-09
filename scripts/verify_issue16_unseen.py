#!/usr/bin/env python3
"""Verify Foxtail issue #16 against the exact committed Unseen assessor.

The default boundary proof is deliberately read-only and dependency-light on
Foxtail: it reads the checked-in canonical definition/manifest, archives the
requested Unseen commit, verifies that the imported modules came from that
archive, and sends a public-response-shaped EC2 evidence payload through the
registered production adapter and assessor.  ``--live`` is a separate mode
that collects the five controls from an already-realized Foxtail HTTP service
through Unseen's ordinary AWS-compatible observation port.
The live mode leaves the committed manifest untouched and reports any
downstream fingerprint-refresh blocker explicitly.

This verifier never imports a sibling Unseen working tree and never implements
an evidence threshold.  The CPU values below are fixture evidence and the
exact-boundary assertion is delegated to the archived registered assessor.
"""

from __future__ import annotations

import argparse
import copy
import datetime as dt
from dataclasses import dataclass
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
from typing import Any, Callable, Mapping
from urllib.request import Request, urlopen


UNSEEN_REVISION = "f4c5e7802def856fb4d4ec6996cbd616ea16bd95"
ROOT = Path(__file__).resolve().parents[1]
DEFAULT_UNSEEN_REPO = Path("/Users/murphy/workspace/iacai0/unseen-agent")
DEFINITION_PATH = ROOT / "tests/fixtures/release-qualification-v1.definition.json"
MANIFEST_PATH = ROOT / "tests/fixtures/release-qualification-v1.manifest.json"


@dataclass
class ExactUnseenArchive:
    """Own exact archived imports and their temporary extraction lifetime."""

    observation_port: Any
    derive_ec2_evidence_input: Callable[[Mapping[str, Any]], Mapping[str, Any]]
    registrations: Any
    derive_oracle_receipt: Callable[..., Any]
    archive_root: Path
    _temporary_directory: tempfile.TemporaryDirectory[str]

    def close(self) -> None:
        self._temporary_directory.cleanup()


def _load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise SystemExit(f"expected an object in {path}")
    return value


def _run(*command: str) -> str:
    process = subprocess.run(command, check=False, capture_output=True, text=True)
    if process.returncode:
        detail = process.stderr.strip() or process.stdout.strip()
        raise RuntimeError(f"command failed ({' '.join(command)}): {detail}")
    return process.stdout.strip()


def _archive_exact_unseen(repo: Path, destination: Path) -> None:
    resolved = _run("git", "-C", str(repo), "rev-parse", "--verify", f"{UNSEEN_REVISION}^{{commit}}")
    if resolved != UNSEEN_REVISION:
        raise RuntimeError(
            f"Unseen repository resolved {resolved}, expected committed revision {UNSEEN_REVISION}"
        )
    archive_path = destination / "unseen.tar"
    with archive_path.open("wb") as stream:
        process = subprocess.run(
            ["git", "-C", str(repo), "archive", "--format=tar", UNSEEN_REVISION],
            check=False,
            stdout=stream,
            stderr=subprocess.PIPE,
        )
    if process.returncode:
        detail = process.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"unable to archive committed Unseen source: {detail}")
    with tarfile.open(archive_path) as archive:
        archive.extractall(destination / "unseen", filter="data")
    (destination / "unseen-commit.txt").write_text(f"{UNSEEN_REVISION}\n", encoding="utf-8")


def _import_exact_unseen(repo: Path) -> ExactUnseenArchive:
    temp = tempfile.TemporaryDirectory(prefix="foxtail-issue16-unseen-")
    archive_root = Path(temp.name)
    _archive_exact_unseen(repo, archive_root)
    extracted = archive_root / "unseen"
    sys.path.insert(0, str(extracted))
    try:
        from unseen.evals.readiness import AwsCompatibleObservationPort
        from unseen.extensions.scan.ec2_evidence import derive_ec2_evidence_input
        from unseen.extensions.scan.evidence_policy_registry import (
            EVIDENCE_POLICY_REGISTRATIONS,
        )
        from unseen.extensions.scan.oracle_derivation import derive_oracle_receipt
    except Exception:
        temp.cleanup()
        raise

    imported_modules = {
        "readiness": sys.modules[AwsCompatibleObservationPort.__module__],
        "ec2_evidence": sys.modules[derive_ec2_evidence_input.__module__],
        "oracle_derivation": sys.modules[derive_oracle_receipt.__module__],
    }
    for label, module in imported_modules.items():
        module_path = Path(str(getattr(module, "__file__", ""))).resolve()
        try:
            module_path.relative_to(extracted.resolve())
        except ValueError as error:
            temp.cleanup()
            raise RuntimeError(
                f"{label} imported outside exact Unseen archive: {module_path}"
            ) from error

    # Keep the temporary extraction alive for the caller; the dataclass owns
    # cleanup after the boundary or live proof completes.
    return ExactUnseenArchive(
        observation_port=AwsCompatibleObservationPort,
        derive_ec2_evidence_input=derive_ec2_evidence_input,
        registrations=EVIDENCE_POLICY_REGISTRATIONS,
        derive_oracle_receipt=derive_oracle_receipt,
        archive_root=extracted,
        _temporary_directory=temp,
    )


def _timestamp(value: Any, reason: str) -> dt.datetime:
    raw = str(value or "").strip()
    try:
        parsed = dt.datetime.fromisoformat(raw.replace("Z", "+00:00"))
    except ValueError:
        raise RuntimeError(reason) from None
    if parsed.tzinfo is None:
        raise RuntimeError(f"{reason}: timestamp must include a timezone")
    return parsed.astimezone(dt.timezone.utc)


def _canonical_positive_facts(
    definition: Mapping[str, Any], manifest: Mapping[str, Any]
) -> dict[str, Any]:
    """Read every boundary input from Foxtail's canonical fixture evidence."""

    profiles = definition.get("generation_rules", {}).get("evidence_profiles", [])
    profile = next(
        (item for item in profiles if item.get("control_id") == "ec2-idle-positive-001"),
        None,
    )
    if not isinstance(profile, Mapping):
        raise RuntimeError("canonical definition is missing the positive evidence profile")
    resource = next(
        (item for item in manifest.get("resources", []) if item.get("control_id") == "ec2-idle-positive-001"),
        None,
    )
    if not isinstance(resource, Mapping):
        raise RuntimeError("canonical manifest is missing the positive realized resource")
    observed = resource.get("observed")
    if not isinstance(observed, Mapping):
        raise RuntimeError("canonical manifest positive resource has no observed facts")
    if profile.get("cpu_value") != 4.0 or observed.get("average_cpu") != 4.0:
        raise RuntimeError(
            "canonical positive realization is not the expected fixture-owned 4.0 peak"
        )
    tags = observed.get("tags")
    if not isinstance(tags, Mapping):
        raise RuntimeError("canonical positive realization has no public classification tags")
    required_classification = ("Owner", "Criticality", "Environment")
    if any(not str(tags.get(key, "")).strip() for key in required_classification):
        raise RuntimeError(
            "canonical positive realization is missing required public classification tags"
        )
    environment = manifest.get("environment")
    if not isinstance(environment, Mapping):
        raise RuntimeError("canonical manifest environment is missing")
    rules = definition.get("generation_rules")
    if not isinstance(rules, Mapping):
        raise RuntimeError("canonical definition generation rules are missing")
    history = rules.get("history")
    network_profile = rules.get("network_profile")
    if not isinstance(history, Mapping) or not isinstance(network_profile, Mapping):
        raise RuntimeError("canonical definition history/network rules are missing")
    offsets = history.get("offset_seconds")
    if not isinstance(offsets, list) or not offsets or any(type(value) is not int for value in offsets):
        raise RuntimeError("canonical definition history offsets are missing or malformed")
    offsets = [int(value) for value in offsets]
    expected_offsets = sorted(offsets)
    for field in ("cpu_offsets", "cost_offsets"):
        observed_offsets = observed.get(field)
        if not isinstance(observed_offsets, list) or sorted(observed_offsets) != expected_offsets:
            raise RuntimeError(f"canonical positive realization has inconsistent {field}")
    if observed.get("metric_count") != len(offsets) * 3 or observed.get("cost_record_count") != len(offsets):
        raise RuntimeError("canonical positive realization has inconsistent evidence counts")
    anchor = _timestamp(manifest.get("clock", {}).get("anchor"), "canonical clock anchor missing")
    network_in_base = network_profile.get("network_in_base")
    network_out_base = network_profile.get("network_out_base")
    network_increment = network_profile.get("per_day_increment")
    if any(
        type(value) not in {int, float} or not math.isfinite(float(value))
        for value in (network_in_base, network_out_base, network_increment)
    ):
        raise RuntimeError("canonical network profile is missing numeric formulas")
    if float(network_in_base) == float(network_out_base):
        raise RuntimeError("canonical network profile must keep NetworkIn and NetworkOut bases distinct")
    catalogue = next(
        (item for item in manifest.get("ec2_instance_type_catalogue", []) if item.get("instance_type") == "m6i.large"),
        None,
    )
    if not isinstance(catalogue, Mapping):
        raise RuntimeError("canonical manifest catalogue is missing m6i.large")
    return {
        "account_id": str(environment["account_id"]),
        "region": str(environment["region"]),
        "resource_id": str(resource["resource_id"]),
        "observed_at": anchor,
        "instance_state": str(observed["instance_state"]),
        "instance_type": str(observed["instance_type"]),
        "tags": dict(tags),
        "root_device_type": str(catalogue["supported_root_device_types"][0]),
        "architecture": str(catalogue["supported_architectures"][0]),
        "virtualization_type": str(catalogue["supported_virtualization_types"][0]),
        "ena_support": str(catalogue["ena_support"]),
        "disable_api_termination": bool(observed["disable_api_termination"]),
        "cost_amount": float(profile["cost_amount"]),
        "cpu_value": float(profile["cpu_value"]),
        "offsets": offsets,
        "network_in_base": float(network_in_base),
        "network_out_base": float(network_out_base),
        "network_increment": float(network_increment),
    }


def _metric_value(facts: Mapping[str, Any], metric_name: str, offset: int, cpu_value: float) -> float:
    if metric_name == "CPUUtilization":
        return cpu_value
    if metric_name == "NetworkIn":
        base_key = "network_in_base"
    elif metric_name == "NetworkOut":
        base_key = "network_out_base"
    else:
        raise RuntimeError(f"unsupported canonical metric {metric_name!r}")
    day = (abs(offset) - 3_600) // 86_400
    return float(facts[base_key]) + float(day) * float(facts["network_increment"])


def _public_payload(facts: Mapping[str, Any]) -> dict[str, Any]:
    observed_at = facts["observed_at"].isoformat().replace("+00:00", "Z")
    offsets = [int(offset) for offset in facts["offsets"]]
    cpu_value = float(facts["cpu_value"])
    metric_rows = [
        {
            "metric_name": metric_name,
            "seconds_from_now": offset,
            "value": _metric_value(facts, metric_name, offset, cpu_value),
        }
        for offset in offsets
        for metric_name in ("CPUUtilization", "NetworkIn", "NetworkOut")
    ]
    datapoints: dict[str, list[dict[str, Any]]] = {}
    for metric_name in ("CPUUtilization", "NetworkIn", "NetworkOut"):
        statistic = "Maximum" if metric_name == "CPUUtilization" else "Sum"
        datapoints[metric_name] = [
            {
                "Timestamp": (facts["observed_at"] + dt.timedelta(seconds=offset))
                .isoformat()
                .replace("+00:00", "Z"),
                statistic: _metric_value(facts, metric_name, offset, cpu_value),
            }
            for offset in offsets
        ]
    cost_rows = [
        {
            "TimePeriod": {
                "Start": (facts["observed_at"] + dt.timedelta(seconds=offset))
                .date()
                .isoformat()
            },
            "Groups": [
                {
                    "Keys": [facts["resource_id"]],
                    "Metrics": {"UnblendedCost": {"Amount": str(facts["cost_amount"])}},
                }
            ],
        }
        for offset in offsets
    ]
    return {
        "account_id": facts["account_id"],
        "region": facts["region"],
        "resource_id": facts["resource_id"],
        "observed_at": observed_at,
        "public_row": {
            "instance_state": facts["instance_state"],
            "instance_type": facts["instance_type"],
            "tags": facts["tags"],
            "configuration": {
                "RootDeviceType": facts["root_device_type"],
                "InstanceLifecycle": "on-demand",
                "IamInstanceProfile": None,
                "Architecture": facts["architecture"],
                "VirtualizationType": facts["virtualization_type"],
                "NetworkInterfaces": [],
                "BlockDeviceMappings": [],
                "EnaSupport": facts["ena_support"],
            },
            "metrics": metric_rows,
        },
        "raw_evidence": {
            "inventory": {
                "resource_id": facts["resource_id"],
                "account_id": facts["account_id"],
                "region": facts["region"],
                "observed_at": observed_at,
                "pagination_complete": True,
                "page_count": 1,
                "provenance": "aws.ec2.describe-instances",
            },
            "instance_attributes": {
                "resource_id": facts["resource_id"],
                "account_id": facts["account_id"],
                "region": facts["region"],
                "observed_at": observed_at,
                "pagination_complete": True,
                "page_count": 1,
                "provenance": "aws.ec2.describe-instance-attribute",
                "response": {
                    "DisableApiTermination": {"Value": facts["disable_api_termination"]}
                },
            },
            "cloudwatch": {
                "resource_id": facts["resource_id"],
                "account_id": facts["account_id"],
                "region": facts["region"],
                "observed_at": observed_at,
                "pagination_complete": True,
                "page_count": 1,
                "provenance": "aws.cloudwatch.get-metric-statistics",
                "response": {
                    name: {"metric_statistics": {"Datapoints": points}}
                    for name, points in datapoints.items()
                },
            },
            "cost_explorer": {
                "resource_id": facts["resource_id"],
                "account_id": facts["account_id"],
                "region": facts["region"],
                "observed_at": observed_at,
                "pagination_complete": True,
                "page_count": 1,
                "provenance": "aws.ce.get-cost-and-usage",
                "response": {"ResultsByTime": cost_rows},
            },
        },
    }


def _replace_cpu_peak_boundary(payload: Mapping[str, Any], peak_cpu: float) -> dict[str, Any]:
    """Copy one public payload and replace CPU in both retained metric paths."""

    boundary_payload = copy.deepcopy(payload)
    raw_points = boundary_payload["raw_evidence"]["cloudwatch"]["response"]["CPUUtilization"][
        "metric_statistics"
    ]["Datapoints"]
    compact_rows = boundary_payload["public_row"]["metrics"]
    for point in raw_points:
        point["Maximum"] = peak_cpu
    for row in compact_rows:
        if row["metric_name"] == "CPUUtilization":
            row["value"] = peak_cpu
    return boundary_payload


def _registration(registrations: Any, identity: str) -> Any:
    matches = [item for item in registrations if getattr(item, "identity", "") == identity]
    if len(matches) != 1:
        raise RuntimeError(f"expected exactly one registered assessor {identity}, found {len(matches)}")
    return matches[0]


def run_boundary(archive: ExactUnseenArchive) -> dict[str, Any]:
    definition = _load_json(DEFINITION_PATH)
    manifest = _load_json(MANIFEST_PATH)
    facts = _canonical_positive_facts(definition, manifest)
    registration = _registration(archive.registrations, "assess_ec2_evidence:v1")
    if registration.finding_type != "idle_instance":
        raise RuntimeError(
            f"registered assessor selected the unexpected finding type {registration.finding_type!r}"
        )

    payload = _public_payload(facts)
    normalized = archive.derive_ec2_evidence_input(payload)
    positive = registration.assessor(normalized)
    if positive.outcome != "optimization_finding" or positive.finding is None:
        raise RuntimeError(f"4.0 canonical public input did not produce one finding: {positive}")
    if positive.finding.finding_type != "idle_instance":
        raise RuntimeError("4.0 finding did not carry the registered idle_instance type")

    boundary_payload = _replace_cpu_peak_boundary(payload, 5.0)
    boundary = registration.assessor(archive.derive_ec2_evidence_input(boundary_payload))
    if boundary.outcome != "inventory_observation" or boundary.finding is not None:
        raise RuntimeError(f"exact 5.0 boundary unexpectedly produced a finding: {boundary}")

    return {
        "source_revision": UNSEEN_REVISION,
        "archive_root": str(archive.archive_root),
        "assessor_identity": registration.identity,
        "finding_type": registration.finding_type,
        "positive_peak": 4.0,
        "positive_outcome": positive.outcome,
        "positive_finding_type": positive.finding.finding_type,
        "boundary_peak": 5.0,
        "boundary_outcome": boundary.outcome,
        "boundary_finding": boundary.finding is not None,
    }


def _get_json(url: str) -> dict[str, Any]:
    request = Request(url, headers={"accept": "application/json"})
    with urlopen(request, timeout=15) as response:  # noqa: S310 - caller supplies a local endpoint
        value = json.loads(response.read().decode("utf-8"))
    if not isinstance(value, dict):
        raise RuntimeError(f"endpoint returned a non-object JSON document: {url}")
    return value


def run_live(archive: ExactUnseenArchive, observation_endpoint: str, localstack_endpoint: str) -> dict[str, Any]:
    observation_endpoint = observation_endpoint.rstrip("/")
    localstack_endpoint = localstack_endpoint.rstrip("/")
    definition = _get_json(f"{observation_endpoint}/_mock/fixture/definition?version=release-qualification-v1")
    manifest = _get_json(f"{observation_endpoint}/_mock/fixture/manifest")
    fixture_status = _get_json(f"{observation_endpoint}/_mock/fixture/status")
    mutation_status = _get_json(f"{observation_endpoint}/_mock/fixture/mutation/status")
    environment = manifest.get("environment")
    if not isinstance(environment, Mapping):
        raise RuntimeError("live manifest environment is missing")
    port = archive.observation_port(
        region=str(environment["region"]),
        manifest=manifest,
        definition_digest=str(definition.get("digest", "")),
        manifest_digest=str(manifest.get("digest", "")),
        fixture_status=fixture_status,
        mutation_status=mutation_status,
        observation_endpoint_url=observation_endpoint,
        localstack_endpoint_url=localstack_endpoint,
    )
    observations = port.observe()
    observed_at = dt.datetime.fromisoformat(
        str(observations["observed_at"]).replace("Z", "+00:00")
    )
    receipt = archive.derive_oracle_receipt(
        definition=definition,
        manifest=manifest,
        observations=observations,
        now=observed_at + dt.timedelta(seconds=1),
    )
    outcomes = {item.control_id: item for item in receipt.outcomes}
    expected = {
        "ec2-idle-positive-001": "finding",
        "ec2-idle-negative-001": "no_finding",
        "ec2-idle-degraded-001": "blocked",
        "ec2-resize-positive-001": "finding",
        "ec2-resize-negative-001": "no_finding",
    }
    if {control_id: item.outcome for control_id, item in outcomes.items()} != expected:
        raise RuntimeError(
            "live outcome matrix mismatch: "
            + json.dumps({control_id: item.outcome for control_id, item in outcomes.items()}, sort_keys=True)
        )
    if "assess_ec2_evidence:v1" not in receipt.registration_identities:
        raise RuntimeError("live receipt omitted the registered idle assessor identity")
    if "assess_ec2_resize_evidence:v1" not in receipt.registration_identities:
        raise RuntimeError("live receipt omitted the registered resize assessor identity")
    expected_consumer_blockers = {
        "read_only_estate_fingerprint_mismatch",
        "estate_fingerprint_mismatch",
    }
    if not expected_consumer_blockers.issubset(set(receipt.reasons)):
        raise RuntimeError(
            "live receipt did not report the expected unmodified-manifest "
            f"fingerprint refresh blockers: {receipt.reasons}"
        )
    allowed_global_reasons = expected_consumer_blockers | {
        "incomplete_window:ec2-idle-degraded-001",
        "oracle_outcome_blocked",
    }
    unexpected_global_reasons = set(receipt.reasons) - allowed_global_reasons
    if unexpected_global_reasons:
        raise RuntimeError(
            "live receipt reported unexpected global blockers: "
            + ", ".join(sorted(unexpected_global_reasons))
        )
    forbidden_fragments = (
        "unsupported_capability",
        "stale",
        "identity_mismatch",
        "resource_fingerprint_mismatch",
        "missing_attribute",
        "compatibility",
        "fixture_intent_contradiction",
    )
    blockers: dict[str, list[str]] = {}
    for control_id, item in outcomes.items():
        reasons = [*item.blockers]
        if item.outcome == "blocked":
            reasons.extend(reason for reason in receipt.reasons if control_id in reason)
        blockers[control_id] = reasons
        if control_id != "ec2-idle-degraded-001" and any(
            fragment in reason for reason in reasons for fragment in forbidden_fragments
        ):
            raise RuntimeError(f"complete control {control_id} has an unexpected blocker: {reasons}")
    degraded_reasons = blockers["ec2-idle-degraded-001"]
    if not any("incomplete_window" in reason for reason in degraded_reasons):
        raise RuntimeError("degraded control was not blocked specifically for its incomplete window")
    return {
        "source_revision": UNSEEN_REVISION,
        "assessor_identities": sorted(receipt.registration_identities),
        "ready": receipt.ready,
        "decision": receipt.decision,
        "captured_manifest_digest": manifest.get("digest"),
        "receipt_contract": "blocked_by_unmodified_manifest_fingerprint_refresh",
        "expected_consumer_blockers": sorted(expected_consumer_blockers),
        "reasons": list(receipt.reasons),
        "outcomes": {control_id: item.outcome for control_id, item in outcomes.items()},
        "blockers": blockers,
        "receipt_digest": receipt.receipt_digest,
    }


def run_self_tests() -> dict[str, Any]:
    """Exercise canonical-input and boundary helpers without importing Unseen."""

    definition = _load_json(DEFINITION_PATH)
    manifest = _load_json(MANIFEST_PATH)
    facts = _canonical_positive_facts(definition, manifest)
    payload = _public_payload(facts)
    if payload["public_row"]["tags"] != facts["tags"]:
        raise RuntimeError("self-test payload did not consume canonical classification tags")
    if [row["seconds_from_now"] for row in payload["public_row"]["metrics"]][::3] != facts["offsets"]:
        raise RuntimeError("self-test payload did not consume canonical history offsets")
    for metric_name, base_key in (
        ("NetworkIn", "network_in_base"),
        ("NetworkOut", "network_out_base"),
    ):
        expected_network = max(
            _metric_value(facts, metric_name, offset, facts["cpu_value"])
            for offset in facts["offsets"]
        )
        observed_network = max(
            row["value"]
            for row in payload["public_row"]["metrics"]
            if row["metric_name"] == metric_name
        )
        if observed_network != expected_network:
            raise RuntimeError(
                f"self-test payload did not consume canonical {metric_name} formula"
            )
        if facts[base_key] not in {facts["network_in_base"], facts["network_out_base"]}:
            raise RuntimeError(f"self-test omitted canonical {base_key}")
    if facts["network_in_base"] == facts["network_out_base"]:
        raise RuntimeError("self-test canonical network bases unexpectedly collapsed")
    missing_network_out = copy.deepcopy(definition)
    del missing_network_out["generation_rules"]["network_profile"]["network_out_base"]
    try:
        _canonical_positive_facts(missing_network_out, manifest)
    except RuntimeError as error:
        if "network profile" not in str(error):
            raise RuntimeError(f"self-test missing network_out_base failed for the wrong reason: {error}") from error
    else:
        raise RuntimeError("self-test accepted a canonical definition without network_out_base")
    drifting_network_out = copy.deepcopy(definition)
    drifting_network_out["generation_rules"]["network_profile"]["network_out_base"] = facts[
        "network_in_base"
    ]
    try:
        _canonical_positive_facts(drifting_network_out, manifest)
    except RuntimeError as error:
        if "distinct" not in str(error):
            raise RuntimeError(f"self-test network base drift failed for the wrong reason: {error}") from error
    else:
        raise RuntimeError("self-test accepted collapsed NetworkIn/NetworkOut bases")
    missing_tags = copy.deepcopy(manifest)
    positive = next(
        item
        for item in missing_tags["resources"]
        if item.get("control_id") == "ec2-idle-positive-001"
    )
    del positive["observed"]["tags"]["Owner"]
    try:
        _canonical_positive_facts(definition, missing_tags)
    except RuntimeError as error:
        if "classification tags" not in str(error):
            raise RuntimeError(f"self-test missing-tag failure had wrong reason: {error}") from error
    else:
        raise RuntimeError("self-test accepted a canonical manifest with a missing Owner tag")
    boundary = _replace_cpu_peak_boundary(payload, 5.0)
    boundary_values = {
        point["Maximum"]
        for point in boundary["raw_evidence"]["cloudwatch"]["response"]["CPUUtilization"][
            "metric_statistics"
        ]["Datapoints"]
    }
    compact_values = {
        row["value"]
        for row in boundary["public_row"]["metrics"]
        if row["metric_name"] == "CPUUtilization"
    }
    if boundary_values != {5.0} or compact_values != {5.0}:
        raise RuntimeError("self-test boundary helper did not update both CPU evidence paths")
    return {
        "canonical_tags": sorted(facts["tags"]),
        "canonical_offsets": len(facts["offsets"]),
        "network_in_base": facts["network_in_base"],
        "network_out_base": facts["network_out_base"],
        "boundary_peak": 5.0,
        "self_test": "passed",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--unseen-repo",
        type=Path,
        default=Path(os.environ.get("UNSEEN_REPO", DEFAULT_UNSEEN_REPO)),
        help="read-only sibling repository containing the committed Unseen object",
    )
    parser.add_argument(
        "--live",
        action="store_true",
        help="collect the live five-control matrix from an already-realized service",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="test canonical evidence and boundary helpers without importing Unseen",
    )
    parser.add_argument("--observation-endpoint", default="http://127.0.0.1:8080")
    parser.add_argument("--localstack-endpoint", default="http://127.0.0.1:4566")
    args = parser.parse_args()

    if args.self_test:
        print(json.dumps(run_self_tests(), indent=2, sort_keys=True))
        return 0
    if not args.unseen_repo.is_dir():
        raise SystemExit(f"Unseen repository does not exist: {args.unseen_repo}")
    archive = _import_exact_unseen(args.unseen_repo)
    try:
        result = (
            run_live(archive, args.observation_endpoint, args.localstack_endpoint)
            if args.live
            else run_boundary(archive)
        )
        print(
            json.dumps(
                {
                    "archive_root": str(archive.archive_root),
                    "revision": UNSEEN_REVISION,
                    **result,
                },
                indent=2,
                sort_keys=True,
            )
        )
    finally:
        archive.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
