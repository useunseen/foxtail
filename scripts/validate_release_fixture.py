#!/usr/bin/env python3
"""Validate release-fixture documents with JSON Schema Draft 2020-12.

The smoke test calls this script against both checked-in goldens and the
documents returned by the running service.  Install the pinned development
dependency with ``python3 -m pip install -r scripts/requirements.txt`` when
the host does not already provide it.
"""

from __future__ import annotations

import argparse
import copy
import importlib.metadata
import json
import sys
from pathlib import Path
from typing import Any, Iterable

try:
    import jsonschema
except ImportError as error:  # pragma: no cover - exercised by the CLI
    print(
        "jsonschema is required; install the pinned dependency with "
        "python3 -m pip install -r scripts/requirements.txt",
        file=sys.stderr,
    )
    raise SystemExit(2) from error


REQUIRED_JSONSCHEMA_VERSION = "4.26.0"
FORBIDDEN_KEYS = frozenset(
    {
        "Required",
        "Stretch",
        "required",
        "stretch",
        "expected_finding",
        "expected_findings",
        "authoritative_expected_finding",
        "authoritative_finding",
        "finding",
    }
)


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"failed to read JSON document {path}: {error}") from error


def iter_forbidden_paths(value: Any, path: str = "$") -> Iterable[str]:
    if isinstance(value, dict):
        for key, child in value.items():
            child_path = f"{path}.{key}"
            if key in FORBIDDEN_KEYS:
                yield child_path
            yield from iter_forbidden_paths(child, child_path)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from iter_forbidden_paths(child, f"{path}[{index}]")


def validate(schema_path: Path, document_path: Path) -> Any:
    schema = load_json(schema_path)
    document = load_json(document_path)
    try:
        jsonschema.Draft202012Validator.check_schema(schema)
    except jsonschema.exceptions.SchemaError as error:
        raise SystemExit(f"invalid Draft 2020-12 schema {schema_path}: {error}") from error

    validator = jsonschema.Draft202012Validator(schema, format_checker=jsonschema.FormatChecker())
    errors = sorted(validator.iter_errors(document), key=lambda error: list(error.path))
    if errors:
        details = "; ".join(
            f"{'.'.join(str(part) for part in error.path) or '$'}: {error.message}"
            for error in errors[:5]
        )
        raise SystemExit(f"{document_path} failed Draft 2020-12 validation: {details}")

    forbidden = list(iter_forbidden_paths(document))
    if forbidden:
        raise SystemExit(
            f"{document_path} contains forbidden policy fields: {', '.join(forbidden)}"
        )
    return document


def expect_forbidden_rejection(schema_path: Path, document: Any, label: str, path: tuple[Any, ...], key: str) -> None:
    mutated = copy.deepcopy(document)
    target = mutated
    for part in path:
        target = target[part]
    target[key] = True
    try:
        # The recursive policy check is intentionally independent of the schema
        # so a permissive additionalProperties branch cannot admit policy text.
        validate_value(schema_path, mutated)
    except ValueError:
        return
    raise SystemExit(f"negative policy check unexpectedly accepted {label}.{key}")


def validate_value(schema_path: Path, document: Any) -> None:
    schema = load_json(schema_path)
    validator = jsonschema.Draft202012Validator(schema, format_checker=jsonschema.FormatChecker())
    errors = list(validator.iter_errors(document))
    forbidden = list(iter_forbidden_paths(document))
    if errors or forbidden:
        raise ValueError("forbidden policy field rejected")


def expect_schema_rejection(
    schema_path: Path,
    document: Any,
    label: str,
    mutate: Any,
) -> None:
    mutated = copy.deepcopy(document)
    mutate(mutated)
    try:
        validate_value(schema_path, mutated)
    except ValueError:
        return
    raise SystemExit(f"negative schema check unexpectedly accepted {label}")


def run_negative_checks(
    definition_schema: Path,
    definition: Any,
    manifest_schema: Path,
    manifest: Any,
) -> None:
    expect_forbidden_rejection(
        definition_schema,
        definition,
        "definition.controls[0].evidence",
        ("controls", 0, "evidence"),
        "expected_finding",
    )
    expect_forbidden_rejection(
        manifest_schema,
        manifest,
        "manifest.resources[0].observed",
        ("resources", 0, "observed"),
        "Required",
    )
    expect_forbidden_rejection(
        manifest_schema,
        manifest,
        "manifest.control_catalogue[0]",
        ("control_catalogue", 0),
        "Stretch",
    )
    for field in ("instance_state", "instance_type", "availability_zone", "tags"):
        expect_schema_rejection(
            manifest_schema,
            manifest,
            f"manifest.resources[0].observed missing {field}",
            lambda value, field=field: value["resources"][0]["observed"].pop(field, None),
        )

    expect_schema_rejection(
        manifest_schema,
        manifest,
        "manifest.resources extra sixth read-only resource",
        lambda value: value["resources"].append(copy.deepcopy(value["resources"][0])),
    )


def parse_args() -> argparse.Namespace:
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--definition", type=Path)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument(
        "--definition-golden",
        type=Path,
        default=root / "tests/fixtures/release-qualification-v1.definition.json",
    )
    parser.add_argument(
        "--manifest-golden",
        type=Path,
        default=root / "tests/fixtures/release-qualification-v1.manifest.json",
    )
    parser.add_argument(
        "--definition-schema",
        type=Path,
        default=root / "schemas/release-fixture-definition-v1.schema.json",
    )
    parser.add_argument(
        "--manifest-schema",
        type=Path,
        default=root / "schemas/release-fixture-manifest-v1.schema.json",
    )
    parser.add_argument(
        "--mutation-status",
        type=Path,
        help="validate one emitted release-fixture mutation status document",
    )
    parser.add_argument(
        "--mutation-status-schema",
        type=Path,
        default=root / "schemas/release-fixture-mutation-status-v1.schema.json",
    )
    parser.add_argument(
        "--receipt",
        action="append",
        type=Path,
        default=[],
        help="validate one emitted operation receipt (repeatable)",
    )
    parser.add_argument(
        "--receipt-schema",
        type=Path,
        default=root / "schemas/release-fixture-receipt-v1.schema.json",
    )
    parser.add_argument("--negative", action="store_true", help="run forbidden-field rejection checks")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    installed_version = importlib.metadata.version("jsonschema")
    if installed_version != REQUIRED_JSONSCHEMA_VERSION:
        print(
            f"jsonschema {REQUIRED_JSONSCHEMA_VERSION} is required; found {installed_version}. "
            "Install with python3 -m pip install -r scripts/requirements.txt",
            file=sys.stderr,
        )
        return 2

    definition = manifest = None
    if not args.mutation_status and not args.receipt:
        definition = validate(args.definition_schema, args.definition or args.definition_golden)
        manifest = validate(args.manifest_schema, args.manifest or args.manifest_golden)
    elif args.definition or args.manifest:
        definition = validate(args.definition_schema, args.definition or args.definition_golden)
        manifest = validate(args.manifest_schema, args.manifest or args.manifest_golden)
    if args.mutation_status:
        validate(args.mutation_status_schema, args.mutation_status)
    for receipt_path in args.receipt:
        validate(args.receipt_schema, receipt_path)
    if args.negative and definition is not None and manifest is not None:
        run_negative_checks(args.definition_schema, definition, args.manifest_schema, manifest)
    print(
        "validated Draft 2020-12 release-fixture documents "
        f"with jsonschema {installed_version}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
