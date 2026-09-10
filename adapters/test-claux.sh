#!/usr/bin/env bash
set -euo pipefail

adapter="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/claux.sh"
root="$(mktemp -d "${TMPDIR:-/var/tmp}/agents-of-empires-claux-test.XXXXXX")"
trap 'rm -rf -- "$root"' EXIT
mkdir -p "$root/bin" "$root/run" "$root/remote"

cat >"$root/bin/python" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
while (( $# )); do
  if [[ "$1" == "--ready-file" ]]; then
    printf '%s\n' 41000 >"$2"
    break
  fi
  shift
done
sleep 30
EOF

cat >"$root/bin/ssh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
for argument in "$@"; do
  if [[ "$argument" == "-N" ]]; then
    sleep 30
  fi
  if [[ "${TEST_REMOTE_EXIT_255:-}" == 1 && "$argument" == *"--output-format json"* ]]; then
    exit 255
  fi
  if [[ -n "${TEST_REMOTE_EXIT:-}" && "$argument" == *"--output-format json"* ]]; then
    if [[ -n "${TEST_LIVE_TRANSCRIPT_JSON:-}" ]]; then
      printf '%s' "$TEST_LIVE_TRANSCRIPT_JSON" >"$(dirname "$AOE_RESULT_FILE")/transcript.live.json"
    fi
    exit "$TEST_REMOTE_EXIT"
  fi
done
exit 0
EOF

cat >"$root/bin/scp" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
source_path="${@: -2:1}"
destination="${@: -1}"
if [[ "${TEST_FAIL_SETUP_SCP_ONCE:-}" == 1 && "$source_path" != *:* && ! -e "${TEST_STATE_ROOT}/setup-scp-failed" ]]; then
  touch "${TEST_STATE_ROOT}/setup-scp-failed"
  exit 255
fi
if [[ "$source_path" == *:*/transcript.json && "${TEST_NO_TRANSCRIPT:-}" != 1 ]]; then
  printf '%s' '{"schema_version":2,"usage":{"input_tokens":120,"output_tokens":8,"cost_usd":0.0012}}' >"$destination"
elif [[ "$source_path" == *:*/result.json && "${TEST_NO_NATIVE_RESULT:-}" != 1 ]]; then
  if [[ -n "${TEST_NATIVE_RESULT_JSON:-}" ]]; then
    printf '%s' "$TEST_NATIVE_RESULT_JSON" >"$destination"
  else
    printf '%s' '{"result":"held the line","usage":{"input_tokens":120,"output_tokens":8,"cost_usd":0.0012}}' >"$destination"
  fi
fi
EOF

printf '#!/usr/bin/env bash\nexit 0\n' >"$root/claux"
printf '# fake controller proxy\n' >"$root/fake-proxy.py"
printf 'hold the line\n' >"$root/instruction.md"
printf '%s\n' \
  'OPENROUTER_API_KEY=controller-only-secret' \
  'AOE_SSH_PASSWORD=territory-password' \
  >"$root/credential.env"
chmod 0700 "$root/bin/python" "$root/bin/ssh" "$root/bin/scp" "$root/claux"
chmod 0600 "$root/credential.env"

PATH="$root/bin:$PATH" \
TEST_FAIL_SETUP_SCP_ONCE=1 \
TEST_STATE_ROOT="$root" \
AOE_AGENT_ID=test-agent \
AOE_TERRITORY_ID=test-territory \
AOE_TERRITORY_HOST=127.0.0.1 \
AOE_SSH_PORT=26000 \
AOE_MODEL=test/model \
AOE_REASONING_EFFORT=low \
AOE_INSTRUCTION_FILE="$root/instruction.md" \
AOE_RESULT_FILE="$root/run/result.json" \
AOE_USAGE_FILE="$root/run/usage.json" \
AOE_CREDENTIAL_FILE="$root/credential.env" \
AOE_CLAUX_BINARY="$root/claux" \
AOE_OPENROUTER_PROXY="$root/fake-proxy.py" \
  "$adapter"

[[ -e "$root/setup-scp-failed" ]]

jq -e '
  .schema_version == 1
  and .agent == "test-agent"
  and .territory == "test-territory"
  and .status == "completed"
  and .summary == "held the line"
  and .usage.input_tokens == 120
  and .usage.output_tokens == 8
  and .usage.cost_microusd == 1200
  and .usage.resource_units == 1
' "$root/run/result.json" >/dev/null
jq -e '
  .schema_version == 1
  and .agent == "test-agent"
  and .usage.input_tokens == 120
  and .usage.output_tokens == 8
  and .usage.cost_microusd == 1200
' "$root/run/usage.json" >/dev/null

rm -f "$root/run/result.json" "$root/run/claux-result.json" "$root/run/transcript.json" "$root/run/referee-reboot"
set +e
PATH="$root/bin:$PATH" \
TEST_REMOTE_EXIT_255=1 \
TEST_NO_NATIVE_RESULT=1 \
TEST_NO_TRANSCRIPT=1 \
AOE_AGENT_ID=test-agent \
AOE_TERRITORY_ID=test-territory \
AOE_TERRITORY_HOST=127.0.0.1 \
AOE_SSH_PORT=26000 \
AOE_MODEL=test/model \
AOE_REASONING_EFFORT=low \
AOE_INSTRUCTION_FILE="$root/instruction.md" \
AOE_RESULT_FILE="$root/run/result.json" \
AOE_USAGE_FILE="$root/run/usage.json" \
AOE_CREDENTIAL_FILE="$root/credential.env" \
AOE_CLAUX_BINARY="$root/claux" \
AOE_OPENROUTER_PROXY="$root/fake-proxy.py" \
  "$adapter"
generic_255_status=$?
set -e
[[ "$generic_255_status" == 255 ]]
if rg -q 'controller-only-secret' "$root/run"; then
  echo "controller credential leaked into adapter artifacts" >&2
  exit 1
fi

jq -e '
  .status == "harness_error"
  and .summary == "Claux harness exited with status 255 before producing a transcript"
' "$root/run/result.json" >/dev/null

rm -f "$root/run/result.json" "$root/run/claux-result.json" "$root/run/transcript.json"
printf 'host-reboot\n' >"$root/run/referee-reboot"
set +e
PATH="$root/bin:$PATH" \
TEST_REMOTE_EXIT_255=1 \
TEST_NO_NATIVE_RESULT=1 \
TEST_NO_TRANSCRIPT=1 \
AOE_AGENT_ID=test-agent \
AOE_TERRITORY_ID=test-territory \
AOE_TERRITORY_HOST=127.0.0.1 \
AOE_SSH_PORT=26000 \
AOE_MODEL=test/model \
AOE_REASONING_EFFORT=low \
AOE_INSTRUCTION_FILE="$root/instruction.md" \
AOE_RESULT_FILE="$root/run/result.json" \
AOE_USAGE_FILE="$root/run/usage.json" \
AOE_CREDENTIAL_FILE="$root/credential.env" \
AOE_CLAUX_BINARY="$root/claux" \
AOE_OPENROUTER_PROXY="$root/fake-proxy.py" \
  "$adapter"
interrupted_status=$?
set -e
[[ "$interrupted_status" == 255 ]]
jq -e '
  .status == "interrupted"
  and .summary == "agent session was interrupted by the referee\u0027s host reboot"
' "$root/run/result.json" >/dev/null

# Claux releases with classified failures write result.json on failure too,
# with a null result and an outcome. A present result.json must not be read
# as success, and provider failures must not be charged to the player.
run_adapter_case() {
  rm -f "$root/run/transcript.live.json" "$root/run/transcript.json.partial"
  rm -f "$root/run/result.json" "$root/run/claux-result.json" "$root/run/transcript.json" "$root/run/referee-reboot"
  set +e
  PATH="$root/bin:$PATH" \
  AOE_AGENT_ID=test-agent \
  AOE_TERRITORY_ID=test-territory \
  AOE_TERRITORY_HOST=127.0.0.1 \
  AOE_SSH_PORT=26000 \
  AOE_MODEL=test/model \
  AOE_REASONING_EFFORT=low \
  AOE_INSTRUCTION_FILE="$root/instruction.md" \
  AOE_RESULT_FILE="$root/run/result.json" \
  AOE_USAGE_FILE="$root/run/usage.json" \
  AOE_CREDENTIAL_FILE="$root/credential.env" \
  AOE_CLAUX_BINARY="$root/claux" \
  AOE_OPENROUTER_PROXY="$root/fake-proxy.py" \
    env "$@" "$adapter"
  last_status=$?
  set -e
}

# SSH loss after work preserves the last checkpoint without guessing its cause.
run_adapter_case \
  TEST_REMOTE_EXIT=255 TEST_NO_NATIVE_RESULT=1 TEST_NO_TRANSCRIPT=1 \
  TEST_LIVE_TRANSCRIPT_JSON='{"schema_version":2,"outcome":{"status":"running"},"usage":{"input_tokens":8753,"output_tokens":7106,"cost_usd":0.06368828}}'
[[ "$last_status" == 255 ]]
jq -e '.status == "harness_error" and (.summary | contains("disconnect cause unknown")) and .usage.cost_microusd == 63688 and .usage.input_tokens == 8753' "$root/run/result.json" >/dev/null
jq -e '.usage.cost_microusd == 63688' "$root/run/usage.json" >/dev/null
jq -e '.outcome.status == "running"' "$root/run/transcript.json" >/dev/null

# A partial checkpoint must not be mistaken for usable evidence.
run_adapter_case \
  TEST_REMOTE_EXIT=255 TEST_NO_NATIVE_RESULT=1 TEST_NO_TRANSCRIPT=1 \
  TEST_LIVE_TRANSCRIPT_JSON='{"usage":'
jq -e '.status == "harness_error" and (.summary | contains("before producing a transcript"))' "$root/run/result.json" >/dev/null

# Rate limited after retries: classified in the JSON, exit 11, usage kept.
run_adapter_case \
  TEST_REMOTE_EXIT=11 \
  TEST_NATIVE_RESULT_JSON='{"schema_version":1,"result":null,"model":"test/model","usage":{"input_tokens":9000,"output_tokens":300,"cost_usd":0.01},"outcome":{"status":"error","message":"API error: 429 Too Many Requests","failure":{"kind":"rate_limited","retryable":true,"http_status":429,"attempts":4}}}'
[[ "$last_status" == 11 ]]
jq -e '
  .status == "unavailable"
  and (.summary | startswith("Claux reported rate_limited (exit status 11): API error: 429"))
  and .usage.input_tokens == 9000
  and .usage.cost_microusd == 10000
' "$root/run/result.json" >/dev/null

# Context exhausted is a player outcome even though the request failed.
run_adapter_case \
  TEST_REMOTE_EXIT=13 \
  TEST_NATIVE_RESULT_JSON='{"schema_version":1,"result":null,"model":"test/model","usage":{"input_tokens":1,"output_tokens":1,"cost_usd":null},"outcome":{"status":"error","message":"context too long","failure":{"kind":"context_exceeded","retryable":false,"attempts":4}}}'
[[ "$last_status" == 13 ]]
jq -e '.status == "failed" and (.summary | startswith("Claux reported context_exceeded"))' "$root/run/result.json" >/dev/null

# Cancelled by a signal is an interruption, not a player failure.
run_adapter_case TEST_REMOTE_EXIT=10 TEST_NO_NATIVE_RESULT=1
[[ "$last_status" == 10 ]]
jq -e '.status == "interrupted" and (.summary | startswith("Claux reported cancelled"))' "$root/run/result.json" >/dev/null

# Exit code alone (older release without outcome in JSON, no result file).
run_adapter_case TEST_REMOTE_EXIT=12 TEST_NO_NATIVE_RESULT=1
[[ "$last_status" == 12 ]]
jq -e '.status == "unavailable" and .summary == "Claux reported authentication (exit status 12)"' "$root/run/result.json" >/dev/null

# An unclassified error with a result file still reads as a player failure.
run_adapter_case \
  TEST_REMOTE_EXIT=1 \
  TEST_NATIVE_RESULT_JSON='{"schema_version":1,"result":null,"model":"test/model","usage":{"input_tokens":1,"output_tokens":1,"cost_usd":null},"outcome":{"status":"error","message":"something broke"}}'
[[ "$last_status" == 1 ]]
jq -e '.status == "failed" and .summary == "Claux exited with status 1: something broke"' "$root/run/result.json" >/dev/null

# A pre-outcome release that only wrote result.json on success still completes.
run_adapter_case TEST_REMOTE_EXIT=0
[[ "$last_status" == 0 ]]
jq -e '.status == "completed" and .summary == "held the line"' "$root/run/result.json" >/dev/null
