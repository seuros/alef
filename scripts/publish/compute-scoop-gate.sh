#!/usr/bin/env bash
set -euo pipefail

#   RELEASE_TARGETS - the `release_targets` output from prepare-release-metadata: a
#                      comma-separated target list, "all", or "none".
#
# Prints exactly "true" or "false" to stdout.
#
# ~keep Kept as a standalone, unit-testable script instead of inline workflow bash because
# `prepare-release-metadata@v1` does not declare a `release_scoop` output (unlike
# `release_homebrew`) -- the Scoop gate must be derived here, from the target list that IS
# declared, rather than depending on an Actions-repo change this branch does not control.
#
# ~keep Every comparison trims its operand defensively: `release_targets` is built without
# stray whitespace under normal operation, but a leading/trailing space on either the whole
# string or one comma-separated element must not silently flip the gate the wrong way in
# either direction (untrimmed " scoop" previously read as absent, the mirror image of the
# untrimmed-whole-string bug fixed in normalize-release-targets.sh).

trim() {
  printf '%s' "$1" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//'
}

targets="$(trim "${RELEASE_TARGETS:?RELEASE_TARGETS is required}")"

if [[ "$targets" == "all" ]]; then
  echo "true"
  exit 0
fi

IFS=',' read -ra target_list <<<"$targets"
for target in "${target_list[@]}"; do
  if [[ "$(trim "$target")" == "scoop" ]]; then
    echo "true"
    exit 0
  fi
done

echo "false"
