from __future__ import annotations

import argparse
import json
import re
import sys
from collections.abc import Iterable, Mapping
from typing import Any


REDACTED = "<redacted>"
SENSITIVE_KEY = re.compile(
    r"(?:^|[_-])(api[_-]?key|token|secret|password|passwd|authorization|cookie|credential|private[_-]?key)(?:$|[_-])",
    re.IGNORECASE,
)


def sensitive_values(mapping: Mapping[str, Any]) -> list[str]:
    values: list[str] = []
    for key, value in mapping.items():
        if SENSITIVE_KEY.search(str(key)) and isinstance(value, str) and len(value) >= 4:
            values.append(value)
        elif isinstance(value, Mapping):
            values.extend(sensitive_values(value))
    return values


def redact_text(text: str, secrets: Iterable[str] = ()) -> str:
    result = text
    for secret in sorted({item for item in secrets if len(item) >= 4}, key=len, reverse=True):
        result = result.replace(secret, REDACTED)
    result = re.sub(r"(?i)(authorization\s*:\s*bearer\s+)[^\s]+", rf"\1{REDACTED}", result)
    result = re.sub(r"(?i)(bearer\s+)[A-Za-z0-9._~+/=-]+", rf"\1{REDACTED}", result)
    result = re.sub(r"(https?://)[^/@\s]+:[^/@\s]+@", rf"\1{REDACTED}@", result)
    result = re.sub(
        r"(?i)([?&](?:api[_-]?key|token|secret|password)=)[^&#\s]+",
        rf"\1{REDACTED}",
        result,
    )
    result = re.sub(
        r"(?i)\b(api[_-]?key|token|secret|password|passwd)\s*([=:])\s*[^\s,;]+",
        lambda match: f"{match.group(1)}{match.group(2)}{REDACTED}",
        result,
    )
    return result


def redact_mapping(value: Any, secrets: Iterable[str] = ()) -> Any:
    known = list(secrets)
    if isinstance(value, Mapping):
        known.extend(sensitive_values(value))
        return {
            key: REDACTED if SENSITIVE_KEY.search(str(key)) else redact_mapping(item, known)
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [redact_mapping(item, known) for item in value]
    if isinstance(value, str):
        return redact_text(value, known)
    return value


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Redact secrets from text or JSON on stdin.")
    parser.add_argument("--json", action="store_true", help="Parse stdin as JSON and preserve JSON structure")
    args = parser.parse_args(argv)
    raw = sys.stdin.read()
    if args.json:
        print(json.dumps(redact_mapping(json.loads(raw)), indent=2, sort_keys=True))
    else:
        sys.stdout.write(redact_text(raw))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
