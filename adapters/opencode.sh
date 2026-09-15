#!/usr/bin/env bash
set -euo pipefail
helper="$(dirname "${BASH_SOURCE[0]}")/opencode_adapter.py"
# Bind the helper to this launcher, whose hash is recorded in match provenance.
expected="90b551033ff2bd7c50ff9d29a8343793d2ed0b0f36c9c3e6d87d9713554c9135"
actual="$(sha256sum "$helper" | cut -d' ' -f1)"
[[ "$actual" == "$expected" ]] || { echo 'OpenCode helper checksum mismatch' >&2; exit 2; }
catalog="$(dirname "$helper")/../suites/model-pool.json"
catalog_hash="$(sha256sum "$catalog" | cut -d' ' -f1)"
[[ "$catalog_hash" == "431f2d8051c67613918f4a18c4bdcb970f3df1c8e422a746ea109f07e10cdbad" ]] || { echo 'OpenCode model catalog checksum mismatch' >&2; exit 2; }
exec python3 "$helper" "$@"
