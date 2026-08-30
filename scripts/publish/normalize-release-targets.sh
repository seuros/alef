#!/usr/bin/env bash
set -euo pipefail

#   RAW_TARGETS - the raw `targets` workflow input before any normalization; may be unset,
#                 empty, or whitespace-only.
#
# Prints the effective targets value to stdout: RAW_TARGETS trimmed of leading/trailing
# whitespace, or "crates,cli" when that trim reduces to the empty string.
#
# ~keep A whitespace-only `targets` input (e.g. a stray space left in a manual dispatch form)
# is a non-empty string, so it is truthy and wins the
# `inputs.targets || client_payload.targets || 'crates,cli'` fallback chain in publish.yaml
# even though it carries no real target. `alef release-metadata` then trims that string on its
# own side and treats the resulting empty string as "release everything" (see `parse_targets`
# in src/cli/commands/release_metadata.rs, which intentionally maps a truly empty `--targets`
# to "all" for other callers), silently enabling every opt-in target including Homebrew and
# Scoop on what was meant to be a routine crates+cli release. Trimming here, before that
# fallback chain runs, closes the gap without changing Alef's own empty-means-all contract,
# which other callers still rely on.

raw="${RAW_TARGETS-}"
trimmed="$(printf '%s' "$raw" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"

if [[ -z "$trimmed" ]]; then
  echo "crates,cli"
else
  echo "$trimmed"
fi
