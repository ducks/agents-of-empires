#!/usr/bin/env bash
set -euo pipefail
helper="$(dirname "${BASH_SOURCE[0]}")/term_llm_adapter.py"
# Bind the implementation and its pinned release to recorded adapter provenance.
expected="2efc2b492fac8ddb785e97f137f214053b53dc473bc21a708a7b2a722157ad7e"
actual="$(sha256sum "$helper" | cut -d' ' -f1)"
[[ "$actual" == "$expected" ]] || { echo 'term-llm helper checksum mismatch' >&2; exit 2; }
exec python3 "$helper" "$@"
