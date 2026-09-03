from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path
from typing import Any


PACKAGE_ROOT = Path(__file__).resolve().parent
SCHEMA_ROOT = PACKAGE_ROOT / "schema"


class ManifestError(ValueError):
    """A JSON manifest does not satisfy its locked schema."""


class BaselineLockError(RuntimeError):
    """A locked evaluator input is missing, malformed, or changed."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _json_type_matches(value: Any, expected: str) -> bool:
    return {
        "null": value is None,
        "object": isinstance(value, dict),
        "array": isinstance(value, list),
        "string": isinstance(value, str),
        "integer": isinstance(value, int) and not isinstance(value, bool),
        "number": isinstance(value, (int, float)) and not isinstance(value, bool),
        "boolean": isinstance(value, bool),
    }.get(expected, False)


def _resolve_ref(schema_root: dict[str, Any], ref: str) -> dict[str, Any]:
    if not ref.startswith("#/"):
        raise ManifestError(f"unsupported schema reference: {ref}")
    current: Any = schema_root
    for part in ref[2:].split("/"):
        current = current[part.replace("~1", "/").replace("~0", "~")]
    if not isinstance(current, dict):
        raise ManifestError(f"schema reference is not an object: {ref}")
    return current


def _validate(value: Any, schema: dict[str, Any], root: dict[str, Any], path: str) -> None:
    if "$ref" in schema:
        _validate(value, _resolve_ref(root, schema["$ref"]), root, path)
        return

    expected = schema.get("type")
    if expected is not None:
        expected_types = [expected] if isinstance(expected, str) else expected
        if not any(_json_type_matches(value, item) for item in expected_types):
            raise ManifestError(f"{path}: expected type {expected_types}, got {type(value).__name__}")

    if "enum" in schema and value not in schema["enum"]:
        raise ManifestError(f"{path}: value {value!r} is not in enum {schema['enum']!r}")

    if isinstance(value, str):
        if len(value) < schema.get("minLength", 0):
            raise ManifestError(f"{path}: string is shorter than minLength")
        pattern = schema.get("pattern")
        if pattern and re.search(pattern, value) is None:
            raise ManifestError(f"{path}: value does not match pattern {pattern!r}")

    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if "minimum" in schema and value < schema["minimum"]:
            raise ManifestError(f"{path}: value is below minimum {schema['minimum']}")

    if isinstance(value, list):
        if schema.get("uniqueItems"):
            serialized = [json.dumps(item, sort_keys=True) for item in value]
            if len(serialized) != len(set(serialized)):
                raise ManifestError(f"{path}: array items must be unique")
        item_schema = schema.get("items")
        if item_schema:
            for index, item in enumerate(value):
                _validate(item, item_schema, root, f"{path}[{index}]")

    if isinstance(value, dict):
        properties = schema.get("properties", {})
        for required in schema.get("required", []):
            if required not in value:
                raise ManifestError(f"{path}.{required}: required property is missing")
        if schema.get("additionalProperties") is False:
            unknown = sorted(set(value) - set(properties))
            if unknown:
                raise ManifestError(f"{path}: additional property {unknown[0]!r} is not allowed")
        for key, item in value.items():
            if key in properties:
                _validate(item, properties[key], root, f"{path}.{key}")


def validate_manifest(data: Any, schema_name: str) -> None:
    schema_path = SCHEMA_ROOT / f"{schema_name}.schema.json"
    try:
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ManifestError(f"cannot load schema {schema_path}: {error}") from error
    _validate(data, schema, schema, "$")


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise ManifestError(f"cannot read manifest {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise ManifestError(f"invalid JSON in {path}: {error}") from error


def load_task_manifest(path: Path | str) -> dict[str, Any]:
    manifest_path = Path(path).resolve()
    data = load_json(manifest_path)
    validate_manifest(data, "task")
    return data


def validate_baseline_lock(
    repo_root: Path | str,
    lock_path: Path | str | None = None,
) -> list[Path]:
    root = Path(repo_root).resolve()
    lock = Path(lock_path).resolve() if lock_path else root / "competitive-eval" / "baselines" / "baseline.lock.json"
    try:
        data = json.loads(lock.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BaselineLockError(f"cannot read baseline lock {lock}: {error}") from error
    if data.get("schema_version") != 1 or not isinstance(data.get("locked_files"), list):
        raise BaselineLockError("unsupported or malformed baseline lock")
    checked: list[Path] = []
    seen: set[str] = set()
    for index, item in enumerate(data["locked_files"]):
        if not isinstance(item, dict) or not isinstance(item.get("path"), str):
            raise BaselineLockError(f"invalid locked_files entry {index}")
        relative = Path(item["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise BaselineLockError(f"locked path escapes repository: {relative}")
        if item["path"] in seen:
            raise BaselineLockError(f"duplicate locked path: {relative}")
        seen.add(item["path"])
        target = (root / relative).resolve()
        try:
            target.relative_to(root)
        except ValueError as error:
            raise BaselineLockError(f"locked path escapes repository: {relative}") from error
        if not target.is_file():
            raise BaselineLockError(f"locked file is missing: {relative}")
        actual = sha256_file(target)
        expected = item.get("sha256")
        if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
            raise BaselineLockError(f"invalid locked hash for {relative}")
        if actual != expected:
            raise BaselineLockError(f"hash mismatch for locked file {relative}: expected {expected}, got {actual}")
        checked.append(target)
    return checked
