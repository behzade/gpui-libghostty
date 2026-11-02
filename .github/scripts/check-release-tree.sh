#!/usr/bin/env bash
set -euo pipefail

# Cargo considers our included, Git-ignored upstream tree dirty. Before using
# --allow-dirty for that generated input, require all tracked and unignored
# files to match the release checkout.
if [[ -n "$(git status --porcelain --untracked-files=all)" ]]; then
  echo 'Refusing to package or publish changes outside the generated source tree.' >&2
  git status --short >&2
  exit 1
fi
