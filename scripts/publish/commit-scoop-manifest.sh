#!/usr/bin/env bash
set -euo pipefail

#   BUCKET_DIR    - path to the scoop-bucket checkout (git operations run against this tree)
#   MANIFEST_PATH - path to the rendered manifest, relative to BUCKET_DIR (e.g. bucket/alef.json)
#   VERSION       - version string used in the commit message
#
# Stages MANIFEST_PATH and commits+pushes only if the staged content actually differs from
# HEAD. Prints exactly "committed" or "skipped" as the last line of stdout.

bucket_dir="${BUCKET_DIR:?BUCKET_DIR is required (path to the scoop-bucket checkout)}"
manifest_path="${MANIFEST_PATH:?MANIFEST_PATH is required (manifest path relative to BUCKET_DIR)}"
version="${VERSION:?VERSION is required}"

cd "$bucket_dir"

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"

# ~keep Stage before diffing, not `git diff --quiet` against the working tree: on the very
# first publish the manifest file doesn't exist in scoop-bucket's history yet, so it is
# untracked, and plain `git diff` never reports untracked paths -- it exits 0 ("no
# differences") and this would silently skip the one publish where the file is brand new.
# `git add` first, then `git diff --cached --quiet` against the index, which does see a
# newly-staged file as a real change. See tests/publish_scoop_manifest_commit_test.rs, which
# reproduces both forms against a real git repo and proves only the staged form catches it.
git add -- "$manifest_path"
if git diff --cached --quiet -- "$manifest_path"; then
  echo "No manifest changes; skipping commit." >&2
  echo "skipped"
  exit 0
fi

git commit -m "alef ${version}"
git push origin HEAD
echo "committed"
