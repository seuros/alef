#!/usr/bin/env python3
"""Reject backend-local generic casing and serde helper definitions."""

from __future__ import annotations

import re
import sys
from pathlib import Path

MESSAGE = (
    "Backend-local generic casing/serde helpers are not allowed. Use "
    "`src/codegen/naming.rs`, or a context-specific backend wrapper that delegates to it."
)

BANNED_FUNCTIONS = {
    "apply_rename_all",
    "apply_serde_rename",
    "wire_variant_value",
    "variant_discriminator",
    "serde_variant_name",
    "unit_enum_raw_value",
    "variant_serde_name",
    "to_snake_case",
    "to_camel_case",
    "to_pascal_case",
    "pascal_case",
    "pascal_to_snake",
}

# (posix path, function name) pairs that are exempt from BANNED_FUNCTIONS, each with an
# inline reason. Entries here must be either the canonical definition in
# `src/codegen/naming.rs` itself, or a context-specific wrapper that delegates to it — never
# an independent reimplementation. Do not add an entry to silence a real duplicate; consolidate
# the duplicate instead.
ALLOWLIST: dict[tuple[str, str], str] = {
    ("src/codegen/naming/wire.rs", "wire_variant_value"): (
        "canonical definition; codegen::naming IS the canonical implementation"
    ),
    ("src/codegen/naming/case.rs", "pascal_to_snake"): (
        "canonical definition; codegen::naming IS the canonical implementation"
    ),
    ("src/backends/java/gen_bindings/helpers.rs", "java_apply_rename_all"): (
        "thin wrapper delegating to naming::apply_serde_rename_all; kept because "
        "src/backends/java/gen_bindings/types/enums.rs still calls it directly"
    ),
}

# Longest-name-first so a prefixed banned name (e.g. `pascal_to_snake_case` containing
# `to_snake_case`) and its shorter sibling never race for which alternative matches first.
_BANNED_ALTERNATION = "|".join(re.escape(name) for name in sorted(BANNED_FUNCTIONS, key=len, reverse=True))

# `\w*_?` catches a prefixed variant of a banned name (e.g. `java_apply_rename_all`), not
# just an exact match, so a language prefix can no longer be used to dodge the ban.
FUNCTION_PATTERN = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+"
    rf"(\w*_?(?:{_BANNED_ALTERNATION}))\b"
)


def read_text(path: Path) -> str | None:
    try:
        data = path.read_bytes()
    except OSError:
        return None
    if b"\x00" in data:
        return None
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return None


def violations_for_file(path: Path) -> list[str]:
    normalized = path.as_posix()
    if not normalized.startswith("src/") or path.suffix != ".rs":
        return []

    content = read_text(path)
    if content is None:
        return []

    violations: list[str] = []
    for line_number, line in enumerate(content.splitlines(), start=1):
        match = FUNCTION_PATTERN.search(line)
        if match:
            function_name = match.group(1)
            if (normalized, function_name) in ALLOWLIST:
                continue
            violations.append(f"{path}:{line_number}: backend-local helper `{function_name}`")
    return violations


def main(argv: list[str] | None = None) -> int:
    paths = [Path(raw) for raw in (argv if argv is not None else sys.argv[1:])]
    if not paths:
        paths = list(Path("src").rglob("*.rs"))

    violations: list[str] = []
    for path in paths:
        if path.is_file():
            violations.extend(violations_for_file(path))

    if violations:
        for violation in violations:
            print(violation, file=sys.stderr)
        print(f"\n{MESSAGE}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
