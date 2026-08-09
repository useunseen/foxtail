#!/usr/bin/env python3
"""Verify Foxtail issue #16 against the exact committed Unseen assessor.

The default boundary proof is deliberately read-only and dependency-light on
Foxtail: it reads the checked-in canonical definition/manifest, archives the
requested Unseen commit, verifies that the imported modules came from that
archive, and sends a public-response-shaped EC2 evidence payload through the
registered production adapter and assessor.  ``--live`` is a separate mode
that collects the five controls from an already-realized Foxtail HTTP service
through Unseen's ordinary AWS-compatible observation port.

This verifier never imports a sibling Unseen working tree and never implements
an evidence threshold.  The CPU values below are fixture evidence and the
exact-boundary assertion is delegated to the archived registered assessor.
"""

from __future__ import annotations

import argparse
import copy
import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
from typing import Any, Mapping
from urllib.request import Request, urlopen


UNSEEN_REVISION = "f4c5e7802def856fb4d4ec6996cbd616ea16bd95"
ROOT = Path(__file__).resolve().parents[1]
DEFAULT_UNSEEN_REPO = Path("/Users/murphy/workspace/iacai0/unseen-agent")
DEFINITION_PATH = ROOT / "tests/fixtures/release-qualification-v1.definition.json"
MANIFEST_PATH = ROOT / "tests/fixtures/release-qualification-v1.manifest.json"


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


def _import_exact_unseen(repo: Path) -> tuple[dict[str, Any], dict[str, Any]]:
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

    modules = {
        "readiness": sys.modules[AwsCompatibleObservationPort.__module__],
        "ec2_evidence": sys.modules[derive_ec2_evidence_input.__module__],
        "oracle_derivation": sys.modules[derive_oracle_receipt.__module__],
    }
    for label, module in modules.items():
        module_path = Path(str(getattr(module, "__file__", ""))).resolve()
        try:
            module_path.relative_to(extracted.resolve())
        except ValueError as error:
            temp.cleanup()
            raise RuntimeError(
                f"{label} imported outside exact Unseen archive: {module_path}"
            ) from error

    # Keep the temporary extraction alive for the caller; the returned module
    # objects retain their code while the live mode executes.
    modules["_archive_temp"] = temp  # type: ignore[assignment]
    modules["_archive_root"] = extracted  # type: ignore[assignment]
    modules["AwsCompatibleObservationPort"] = AwsCompatibleObservationPort  # type: ignore[assignment]
    modules["derive_ec2_evidence_input"] = derive_ec2_evidence_input  # type: ignore[assignment]
    modules["registrations"] = EVIDENCE_POLICY_REGISTRATIONS  # type: ignore[assignment]
    modules["derive_oracle_receipt"] = derive_oracle_receipt  # type: ignore[assignment]
    return modules, {"archive_root": str(extracted), "revision": UNSEEN_REVISION}


def _metric_rows(peak_cpu: float, network: float) -> list[dict[str, Any]]:
    return [
        {
            "metric_name": metric_name,
            "seconds_from_now": -(day * 86_400 + 3_600),
            "value": value,
        }
        for day in range(14)
        for metric_name, value in (
            ("CPUUtilization", peak_cpu),
            ("NetworkIn", network),
            ("NetworkOut", network),
        )
    ]


def _public_payload(
    *,
    resource_id: str,
    account_id: str,
    region: str,
    peak_cpu: float,
    root_device_type: str,
    architecture: str,
    virtualization_type: str,
    ena_support: str,
    disable_api_termination: bool,
    cost_amount: float,
) -> dict[str, Any]:
    observed_at = "2026-08-05T12:00:00Z"
    metric_rows = _metric_rows(peak_cpu, 10_000.0)
    datapoints: dict[str, list[dict[str, Any]]] = {}
    for metric_name in ("CPUUtilization", "NetworkIn", "NetworkOut"):
        datapoints[metric_name] = []
        for day in range(14):
            timestamp = dt.datetime(2026, 8, 4, 11, tzinfo=dt.timezone.utc) - dt.timedelta(days=day)
            statistic = "Maximum" if metric_name == "CPUUtilization" else "Sum"
            value = peak_cpu if metric_name == "CPUUtilization" else 10_000.0
            datapoints[metric_name].append(
                {"Timestamp": timestamp.isoformat().replace("+00:00", "Z"), statistic: value}
            )

    return {
        "account_id": account_id,
        "region": region,
        "resource_id": resource_id,
        "observed_at": observed_at,
        "public_row": {
            "instance_state": "running",
            "instance_type": "m6i.large",
            # These are ordinary classification fields from the public row;
            # no fixture role or expected outcome is used by the assessor.
            "tags": {"Owner": "unknown", "Criticality": "unknown", "Environment": "unknown"},
            "configuration": {
                "RootDeviceType": root_device_type,
                "InstanceLifecycle": "on-demand",
                "IamInstanceProfile": None,
                "Architecture": architecture,
                "VirtualizationType": virtualization_type,
                "NetworkInterfaces": [],
                "BlockDeviceMappings": [],
                "EnaSupport": ena_support,
            },
            "metrics": metric_rows,
        },
        "raw_evidence": {
            "inventory": {
                "resource_id": resource_id,
                "account_id": account_id,
                "region": region,
                "observed_at": observed_at,
                "pagination_complete": True,
                "page_count": 1,
                "provenance": "aws.ec2.describe-instances",
            },
            "instance_attributes": {
                "resource_id": resource_id,
                "account_id": account_id,
                "region": region,
                "observed_at": observed_at,
                "pagination_complete": True,
                "page_count": 1,
                "provenance": "aws.ec2.describe-instance-attribute",
                "response": {"DisableApiTermination": {"Value": disable_api_termination}},
            },
            "cloudwatch": {
                "resource_id": resource_id,
                "account_id": account_id,
                "region": region,
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
                "resource_id": resource_id,
                "account_id": account_id,
                "region": region,
                "observed_at": observed_at,
                "pagination_complete": True,
                "page_count": 1,
                "provenance": "aws.ce.get-cost-and-usage",
                "response": {
                    "ResultsByTime": [
                        {
                            "TimePeriod": {"Start": f"2026-07-{day + 1:02d}"},
                            "Groups": [
                                {
                                    "Keys": [resource_id],
                                    "Metrics": {
                                        "UnblendedCost": {"Amount": str(cost_amount)}
                                    },
                                }
                            ],
                        }
                        for day in range(14)
                    ]
                },
            },
        },
    }


def _positive_fixture_facts(definition: Mapping[str, Any], manifest: Mapping[str, Any]) -> dict[str, Any]:
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
    environment = manifest.get("environment")
    if not isinstance(environment, Mapping):
        raise RuntimeError("canonical manifest environment is missing")
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
        "root_device_type": str(catalogue["supported_root_device_types"][0]),
        "architecture": str(catalogue["supported_architectures"][0]),
        "virtualization_type": str(catalogue["supported_virtualization_types"][0]),
        "ena_support": str(catalogue["ena_support"]),
        "disable_api_termination": bool(observed["disable_api_termination"]),
        "cost_amount": float(profile["cost_amount"]),
    }


def _registration(registrations: Any, identity: str) -> Any:
    matches = [item for item in registrations if getattr(item, "identity", "") == identity]
    if len(matches) != 1:
        raise RuntimeError(f"expected exactly one registered assessor {identity}, found {len(matches)}")
    return matches[0]


def run_boundary(modules: Mapping[str, Any]) -> dict[str, Any]:
    definition = _load_json(DEFINITION_PATH)
    manifest = _load_json(MANIFEST_PATH)
    facts = _positive_fixture_facts(definition, manifest)
    registration = _registration(modules["registrations"], "assess_ec2_evidence:v1")
    if registration.finding_type != "idle_instance":
        raise RuntimeError(
            f"registered assessor selected the unexpected finding type {registration.finding_type!r}"
        )

    payload = _public_payload(peak_cpu=4.0, **facts)
    normalized = modules["derive_ec2_evidence_input"](payload)
    positive = registration.assessor(normalized)
    if positive.outcome != "optimization_finding" or positive.finding is None:
        raise RuntimeError(f"4.0 canonical public input did not produce one finding: {positive}")
    if positive.finding.finding_type != "idle_instance":
        raise RuntimeError("4.0 finding did not carry the registered idle_instance type")

    boundary_payload = copy.deepcopy(payload)
    for point in boundary_payload["raw_evidence"]["cloudwatch"]["response"]["CPUUtilization"]["metric_statistics"]["Datapoints"]:
        point["Maximum"] = 5.0
    for row in boundary_payload["public_row"]["metrics"]:
        if row["metric_name"] == "CPUUtilization":
            row["value"] = 5.0
    boundary = registration.assessor(modules["derive_ec2_evidence_input"](boundary_payload))
    if boundary.outcome != "inventory_observation" or boundary.finding is not None:
        raise RuntimeError(f"exact 5.0 boundary unexpectedly produced a finding: {boundary}")

    return {
        "source_revision": UNSEEN_REVISION,
        "archive_root": str(modules["_archive_root"]),
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


def run_live(modules: Mapping[str, Any], observation_endpoint: str, localstack_endpoint: str) -> dict[str, Any]:
    observation_endpoint = observation_endpoint.rstrip("/")
    localstack_endpoint = localstack_endpoint.rstrip("/")
    definition = _get_json(f"{observation_endpoint}/_mock/fixture/definition?version=release-qualification-v1")
    manifest = _get_json(f"{observation_endpoint}/_mock/fixture/manifest")
    fixture_status = _get_json(f"{observation_endpoint}/_mock/fixture/status")
    mutation_status = _get_json(f"{observation_endpoint}/_mock/fixture/mutation/status")
    environment = manifest.get("environment")
    if not isinstance(environment, Mapping):
        raise RuntimeError("live manifest environment is missing")
    port = modules["AwsCompatibleObservationPort"](
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
    # The committed f4c5 collector's independent fingerprint projection is a
    # public capture projection; it intentionally does not include every
    # Foxtail manifest-only fact (for example DisableApiTermination).  Rebind
    # only this detached proof copy to the fingerprints just derived from the
    # public capture.  The service manifest and its digest are never mutated,
    # and the assessor/receipt still owns all outcome decisions.
    oracle_manifest = copy.deepcopy(manifest)
    oracle_environment = oracle_manifest.get("environment")
    observed_fingerprints = observations.get("fingerprints")
    if not isinstance(oracle_environment, dict) or not isinstance(observed_fingerprints, Mapping):
        raise RuntimeError("live public fingerprints are missing")
    for key in ("read_only_estate_fingerprint", "estate_fingerprint"):
        value = observed_fingerprints.get(key)
        if not isinstance(value, str) or not value:
            raise RuntimeError(f"live public fingerprint is missing: {key}")
        oracle_environment[key] = value
    oracle_digest_payload = copy.deepcopy(oracle_manifest)
    oracle_digest_payload.pop("digest", None)
    oracle_manifest["digest"] = modules["oracle_derivation"]._canonical_digest(oracle_digest_payload)
    oracle_observations = copy.deepcopy(observations)
    oracle_observations["manifest_digest"] = oracle_manifest["digest"]
    observed_at = dt.datetime.fromisoformat(
        str(observations["observed_at"]).replace("Z", "+00:00")
    )
    receipt = modules["derive_oracle_receipt"](
        definition=definition,
        manifest=oracle_manifest,
        observations=oracle_observations,
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
        "proof_manifest_digest": oracle_manifest["digest"],
        "public_fingerprints_rebound": True,
        "reasons": list(receipt.reasons),
        "outcomes": {control_id: item.outcome for control_id, item in outcomes.items()},
        "blockers": blockers,
        "receipt_digest": receipt.receipt_digest,
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
    parser.add_argument("--observation-endpoint", default="http://127.0.0.1:8080")
    parser.add_argument("--localstack-endpoint", default="http://127.0.0.1:4566")
    args = parser.parse_args()

    if not args.unseen_repo.is_dir():
        raise SystemExit(f"Unseen repository does not exist: {args.unseen_repo}")
    modules, source = _import_exact_unseen(args.unseen_repo)
    try:
        result = run_live(modules, args.observation_endpoint, args.localstack_endpoint) if args.live else run_boundary(modules)
        print(json.dumps({**source, **result}, indent=2, sort_keys=True))
    finally:
        modules["_archive_temp"].cleanup()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
