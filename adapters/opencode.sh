#!/usr/bin/env bash
set -euo pipefail
helper="$(dirname "${BASH_SOURCE[0]}")/opencode_adapter.py"
# Bind the helper to this launcher, whose hash is recorded in match provenance.
expected="bdd268b03dfd6dc92820831d5064ad1e5118921ce844ff929e236da682669c38"
actual="$(sha256sum "$helper" | cut -d' ' -f1)"
[[ "$actual" == "$expected" ]] || { echo 'OpenCode helper checksum mismatch' >&2; exit 2; }
catalog="$(dirname "$helper")/opencode-go-models.json"
catalog_hash="$(sha256sum "$catalog" | cut -d' ' -f1)"
[[ "$catalog_hash" == "546dee7d5322f92a562a852b5cdd3f0462ada54cc4555ccf8dfb8a3cb0b0617a" ]] || { echo 'OpenCode model catalog checksum mismatch' >&2; exit 2; }
exec python3 "$helper" "$@"
