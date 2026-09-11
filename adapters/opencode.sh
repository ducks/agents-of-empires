#!/usr/bin/env bash
set -euo pipefail
helper="$(dirname "${BASH_SOURCE[0]}")/opencode_adapter.py"
# Bind the helper to this launcher, whose hash is recorded in match provenance.
expected="772dedf93dbef525621eb22d7013438565dd97778ec930b2b54328721c16efef"
actual="$(sha256sum "$helper" | cut -d' ' -f1)"
[[ "$actual" == "$expected" ]] || { echo 'OpenCode helper checksum mismatch' >&2; exit 2; }
exec python3 "$helper"
