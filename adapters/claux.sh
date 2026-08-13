#!/usr/bin/env bash
set -euo pipefail

required=(
  AOE_AGENT_ID AOE_TERRITORY_ID AOE_TERRITORY_HOST AOE_SSH_PORT
  AOE_MODEL AOE_REASONING_EFFORT AOE_INSTRUCTION_FILE AOE_RESULT_FILE
  AOE_CREDENTIAL_FILE
)
for name in "${required[@]}"; do
  [[ -n "${!name:-}" ]] || {
    echo "missing adapter input: ${name}" >&2
    exit 2
  }
done

set -a
# The controller owns this mode-0600 file. It never enters the territory.
# shellcheck source=/dev/null
source "$AOE_CREDENTIAL_FILE"
set +a
: "${OPENROUTER_API_KEY:?credential file must set OPENROUTER_API_KEY}"
: "${AOE_SSH_PASSWORD:?credential file must set AOE_SSH_PASSWORD}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
claux="${AOE_CLAUX_BINARY:-${HOME}/.cache/replaybook/claux/v20260810.0.1/claux-linux-x86_64}"
proxy="${AOE_OPENROUTER_PROXY:-${repo_root}/../replaybook/integrations/host/openrouter_proxy.py}"
[[ -x "$claux" ]] || {
  echo "Claux binary is not executable: ${claux}" >&2
  exit 2
}
[[ -f "$proxy" ]] || {
  echo "OpenRouter credential proxy is missing: ${proxy}" >&2
  exit 2
}

run_root="$(dirname "$AOE_RESULT_FILE")"
mkdir -p "$run_root"
ready_file="${run_root}/proxy.port"
proxy_log="${run_root}/proxy.log"
askpass="${run_root}/ssh-askpass.sh"
remote_root="/var/tmp/agents-of-empires-${AOE_AGENT_ID}"
remote_port="$((18000 + AOE_SSH_PORT % 1000))"
transcript="${run_root}/transcript.json"
native_result="${run_root}/claux-result.json"

cat >"$askpass" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$AOE_SSH_PASSWORD"
EOF
chmod 0700 "$askpass"

proxy_pid=""
tunnel_pid=""
cleanup() {
  if [[ -n "$tunnel_pid" ]] && kill -0 "$tunnel_pid" 2>/dev/null; then
    kill "$tunnel_pid" 2>/dev/null || true
    wait "$tunnel_pid" 2>/dev/null || true
  fi
  if [[ -n "$proxy_pid" ]] && kill -0 "$proxy_pid" 2>/dev/null; then
    kill "$proxy_pid" 2>/dev/null || true
    wait "$proxy_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

ssh_options=(
  -p "$AOE_SSH_PORT"
  -o BatchMode=no
  -o ConnectTimeout=5
  -o LogLevel=ERROR
  -o PreferredAuthentications=password
  -o PubkeyAuthentication=no
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
)
ssh_command=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${ssh_options[@]}" "root@${AOE_TERRITORY_HOST}")
scp_command=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 scp -q -P "$AOE_SSH_PORT" "${ssh_options[@]:2}" )

deadline=$((SECONDS + 120))
until "${ssh_command[@]}" true 2>/dev/null; do
  (( SECONDS < deadline )) || {
    echo "territory SSH did not become ready" >&2
    exit 1
  }
  sleep 1
done

python "$proxy" --port 0 --ready-file "$ready_file" >"$proxy_log" 2>&1 &
proxy_pid=$!
deadline=$((SECONDS + 30))
until [[ -s "$ready_file" ]]; do
  kill -0 "$proxy_pid" 2>/dev/null || {
    cat "$proxy_log" >&2
    exit 1
  }
  (( SECONDS < deadline )) || {
    echo "credential proxy did not become ready" >&2
    exit 1
  }
  sleep 0.1
done
proxy_port="$(<"$ready_file")"

"${ssh_command[@]}" -N \
  -o ExitOnForwardFailure=yes \
  -R "127.0.0.1:${remote_port}:127.0.0.1:${proxy_port}" &
tunnel_pid=$!
sleep 1
kill -0 "$tunnel_pid"

"${ssh_command[@]}" "install -d -m 0700 '${remote_root}'"
"${scp_command[@]}" "$claux" "root@${AOE_TERRITORY_HOST}:${remote_root}/claux"
"${scp_command[@]}" "$AOE_INSTRUCTION_FILE" "root@${AOE_TERRITORY_HOST}:${remote_root}/instruction.md"

set +e
"${ssh_command[@]}" \
  "chmod 0700 '${remote_root}/claux' && \
   cd /root && \
   OPENROUTER_API_KEY=arena-proxy-placeholder '${remote_root}/claux' config init --provider openrouter --model '${AOE_MODEL}' >/dev/null && \
   sed -i 's#^base_url = .*#base_url = \"http://127.0.0.1:${remote_port}/api/v1\"#' /root/.config/claux/config.toml && \
   sed -i 's/^native_tool_filesystem_policy = .*/native_tool_filesystem_policy = \"unrestricted\"/' /root/.config/claux/config.toml && \
   sed -i 's/^bash_filesystem_policy = .*/bash_filesystem_policy = \"unrestricted\"/' /root/.config/claux/config.toml && \
   profile=\$(sed -n 's/^default_profile = \"\([^\"]*\)\"/\1/p' /root/.config/claux/config.toml) && \
   awk -v section=\"[model_profiles.\${profile}]\" -v effort='${AOE_REASONING_EFFORT}' '{ print; if (\$0 == section) print \"reasoning_effort = \\\"\" effort \"\\\"\" }' /root/.config/claux/config.toml > /root/.config/claux/config.toml.partial && \
   mv /root/.config/claux/config.toml.partial /root/.config/claux/config.toml && \
   OPENROUTER_API_KEY=arena-proxy-placeholder '${remote_root}/claux' --print \"\$(cat '${remote_root}/instruction.md')\" --permission-mode bypass --output-format json --transcript '${remote_root}/transcript.json' > '${remote_root}/result.json'"
status=$?
set -e

# A controller-owned durability check may reboot the guest while Claux still
# has an SSH session open. Wait for that same host to return before collecting
# artifacts, and distinguish the expected transport interruption from model or
# player failure.
referee_interrupted=false
if (( status == 255 )) && [[ -f "${run_root}/referee-reboot" ]]; then
  deadline=$((SECONDS + 60))
  until (( SECONDS >= deadline )); do
    if "${ssh_command[@]}" true >/dev/null 2>&1; then
      referee_interrupted=true
      break
    fi
    sleep 1
  done
fi

"${scp_command[@]}" "root@${AOE_TERRITORY_HOST}:${remote_root}/transcript.json" "$transcript" 2>/dev/null || true
"${scp_command[@]}" "root@${AOE_TERRITORY_HOST}:${remote_root}/result.json" "$native_result" 2>/dev/null || true

if [[ -s "$native_result" ]]; then
  jq \
    --arg agent "$AOE_AGENT_ID" \
    --arg territory "$AOE_TERRITORY_ID" \
    --arg transcript "$transcript" \
    '{
      schema_version: 1,
      agent: $agent,
      territory: $territory,
      status: "completed",
      summary: (.result // "agent completed"),
      usage: {
        rounds: null,
        tool_calls: null,
        input_tokens: (.usage.input_tokens // null),
        output_tokens: (.usage.output_tokens // null),
        cost_microusd: (if .usage.cost_usd == null then null else (.usage.cost_usd * 1000000 | round) end),
        resource_units: 1
      },
      transcript: $transcript
    }' "$native_result" >"${AOE_RESULT_FILE}.partial"
  mv "${AOE_RESULT_FILE}.partial" "$AOE_RESULT_FILE"
else
  normalized_status="failed"
  summary="Claux exited with status ${status}"
  if [[ "$referee_interrupted" == true ]]; then
    normalized_status="interrupted"
    summary="agent session was interrupted by the referee's host reboot"
  fi
  jq -n \
    --arg agent "$AOE_AGENT_ID" \
    --arg territory "$AOE_TERRITORY_ID" \
    --arg status "$normalized_status" \
    --arg summary "$summary" \
    --arg transcript "$transcript" \
    '{schema_version:1, agent:$agent, territory:$territory, status:$status, summary:$summary, usage:{resource_units:1}, transcript:$transcript}' \
    >"$AOE_RESULT_FILE"
fi
exit "$status"
