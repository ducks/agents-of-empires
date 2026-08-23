#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
aoe="${AOE_BIN:-${repo_root}/target/release/agents-of-empires}"
"$aoe" arena validate "${repo_root}/arenas/blueprint-build"
test -s "${repo_root}/arenas/blueprint-build/blueprint.png"
