#!/usr/bin/env bash
set -euo pipefail

# A Linux-only shim test: no Zig build, graphics driver, or display is required.
# Pass --require-theme-ready to fail on the known stale-frame limitation.
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
test_dir=$(mktemp -d "${TMPDIR:-/tmp}/gpui-frame-readiness.XXXXXX")
trap 'rm -rf -- "$test_dir"' EXIT
ghostty_source=${GHOSTTY_SOURCE_DIR:-$(cargo run --quiet --locked --manifest-path "$repo_root/Cargo.toml" -p xtask -- check-ghostty)}

"${CC:-cc}" \
  -std=c11 -D_POSIX_C_SOURCE=200809L -Wall -Wextra -Werror \
  -ffunction-sections -fdata-sections \
  -I "$ghostty_source/include" \
  "$repo_root/crates/gpui-ghostty/tests/frame_readiness.c" \
  -Wl,--gc-sections -o "$test_dir/frame-readiness"
"$test_dir/frame-readiness" "$@"
