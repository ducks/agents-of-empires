#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
binary="${AOE_BIN:-agents-of-empires}"
"$binary" arena validate "$root"
