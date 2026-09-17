#!/usr/bin/env bash
set -euo pipefail
helper="$(dirname "${BASH_SOURCE[0]}")/opencode_adapter.py"
# Bind the helper to this launcher, whose hash is recorded in match provenance.
expected="9d2ea586c7b02a8ae19d0c10e94d3ec57a288bcd534b092b6c62852b86b6dc9a"
actual="$(sha256sum "$helper" | cut -d' ' -f1)"
[[ "$actual" == "$expected" ]] || { echo 'OpenCode helper checksum mismatch' >&2; exit 2; }
catalog="$(dirname "$helper")/../suites/model-pool.json"
catalog_hash="$(sha256sum "$catalog" | cut -d' ' -f1)"
[[ "$catalog_hash" == "431f2d8051c67613918f4a18c4bdcb970f3df1c8e422a746ea109f07e10cdbad" ]] || { echo 'OpenCode model catalog checksum mismatch' >&2; exit 2; }
exec python3 "$helper" "$@"
