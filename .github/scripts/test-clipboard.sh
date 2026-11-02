#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
test_dir=$(mktemp -d "${TMPDIR:-/tmp}/gpui-clipboard.XXXXXX")
trap 'rm -rf -- "$test_dir"' EXIT
ghostty_source=${GHOSTTY_SOURCE_DIR:-$(cargo run --quiet --locked --manifest-path "$repo_root/Cargo.toml" -p xtask -- check-ghostty)}

"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -fsanitize=address,undefined \
  -I "$ghostty_source/include" \
  "$repo_root/crates/gpui-ghostty/tests/clipboard.c" -o "$test_dir/clipboard"
"$test_dir/clipboard"
