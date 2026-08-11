#!/usr/bin/env bash
set -euo pipefail

host=${1:?usage: verify.sh HOST PORT EXPECTED_BODY}
port=${2:?usage: verify.sh HOST PORT EXPECTED_BODY}
expected=${3:?usage: verify.sh HOST PORT EXPECTED_BODY}

actual=$(curl --fail --silent --show-error --max-time 2 "http://${host}:${port}/health")
[[ "$actual" == "$expected" ]]
