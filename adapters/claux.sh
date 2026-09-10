#!/usr/bin/env bash
set -euo pipefail

required=(
  AOE_AGENT_ID AOE_TERRITORY_ID AOE_TERRITORY_HOST AOE_SSH_PORT
  AOE_MODEL AOE_REASONING_EFFORT AOE_INSTRUCTION_FILE AOE_RESULT_FILE AOE_USAGE_FILE
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
claux_version="20260908.0.0"
claux_sha256="8e386c95dc489f3388c7c8dfb6fdf79d774d706125fa02af79b4d0f4116de33e"
default_claux="${HOME}/.cache/agents-of-empires/claux/v${claux_version}/claux-linux-x86_64"
claux="${AOE_CLAUX_BINARY:-$default_claux}"
proxy="${AOE_OPENROUTER_PROXY:-${repo_root}/../replaybook/integrations/host/openrouter_proxy.py}"

if [[ -z "${AOE_CLAUX_BINARY:-}" ]]; then
  actual_sha256=""
  if [[ -f "$claux" ]]; then
    actual_sha256="$(sha256sum "$claux" | cut -d' ' -f1)"
  fi
  if [[ "$actual_sha256" != "$claux_sha256" ]]; then
    mkdir -p "$(dirname "$claux")"
    download="${claux}.download.$$"
    if ! curl -fsSL \
      "https://github.com/ducks/claux/releases/download/v${claux_version}/claux-linux-x86_64" \
      -o "$download"; then
      rm -f -- "$download"
      echo "could not download pinned Claux v${claux_version}" >&2
      exit 2
    fi
    actual_sha256="$(sha256sum "$download" | cut -d' ' -f1)"
    if [[ "$actual_sha256" != "$claux_sha256" ]]; then
      rm -f -- "$download"
      echo "checksum mismatch for pinned Claux v${claux_version}" >&2
      exit 2
    fi
    chmod 0700 "$download"
    mv -f -- "$download" "$claux"
  fi
fi
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
live_transcript="${run_root}/transcript.live.json"

valid_transcript() {
  jq -e 'type == "object" and (.usage | type == "object")' "$1" >/dev/null 2>&1
}

normalize_usage() {
  local source="$1"
  [[ -s "$source" ]] || return 0
  jq \
    --arg agent "$AOE_AGENT_ID" \
    --arg territory "$AOE_TERRITORY_ID" \
    '{schema_version:1,agent:$agent,territory:$territory,usage:{rounds:(.usage.rounds // null),tool_calls:(.usage.tool_calls // null),input_tokens:(.usage.input_tokens // null),output_tokens:(.usage.output_tokens // null),cost_microusd:(if .usage.cost_usd == null then null else (.usage.cost_usd * 1000000 | round) end),resource_units:1}}' \
    "$source" >"${AOE_USAGE_FILE}.partial" 2>/dev/null || return 0
  mv "${AOE_USAGE_FILE}.partial" "$AOE_USAGE_FILE"
}

checkpoint_usage() {
  while true; do
    sleep 2
    "${scp_command[@]}" "root@${AOE_TERRITORY_HOST}:${remote_root}/transcript.json" "${live_transcript}.partial" 2>/dev/null || continue
    valid_transcript "${live_transcript}.partial" || continue
    mv "${live_transcript}.partial" "$live_transcript"
    normalize_usage "$live_transcript"
  done
}

cat >"$askpass" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$AOE_SSH_PASSWORD"
EOF
chmod 0700 "$askpass"

proxy_pid=""
tunnel_pid=""
checkpoint_pid=""
cleanup() {
  if [[ -n "$checkpoint_pid" ]] && kill -0 "$checkpoint_pid" 2>/dev/null; then
    kill "$checkpoint_pid" 2>/dev/null || true
    wait "$checkpoint_pid" 2>/dev/null || true
  fi
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

retry_setup() {
  local attempt=1
  local status=0
  while true; do
    if "$@"; then
      return 0
    else
      status=$?
    fi
    if (( attempt >= 3 )); then
      return "$status"
    fi
    echo "setup transport failed (attempt ${attempt}/3); retrying" >&2
    sleep "$attempt"
    (( attempt += 1 ))
  done
}

start_tunnel() {
  local attempt=1
  while true; do
    "${ssh_command[@]}" -N \
      -o ExitOnForwardFailure=yes \
      -R "127.0.0.1:${remote_port}:127.0.0.1:${proxy_port}" &
    tunnel_pid=$!
    sleep 1
    if kill -0 "$tunnel_pid" 2>/dev/null; then
      return 0
    fi
    wait "$tunnel_pid" 2>/dev/null || true
    tunnel_pid=""
    if (( attempt >= 3 )); then
      echo "credential tunnel failed after 3 attempts" >&2
      return 1
    fi
    echo "credential tunnel failed (attempt ${attempt}/3); retrying" >&2
    sleep "$attempt"
    (( attempt += 1 ))
  done
}

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

start_tunnel

retry_setup "${ssh_command[@]}" "install -d -m 0700 '${remote_root}'"
retry_setup "${scp_command[@]}" "$claux" "root@${AOE_TERRITORY_HOST}:${remote_root}/claux"
retry_setup "${scp_command[@]}" "$AOE_INSTRUCTION_FILE" "root@${AOE_TERRITORY_HOST}:${remote_root}/instruction.md"

remote_image_args=""
mapfile -t player_artifacts < <(jq -r '.[]' <<<"${AOE_PLAYER_ARTIFACTS_JSON:-[]}")
for index in "${!player_artifacts[@]}"; do
  artifact="${player_artifacts[$index]}"
  [[ -f "$artifact" ]] || {
    echo "player artifact does not exist: ${artifact}" >&2
    exit 2
  }
  extension="${artifact##*.}"
  case "$extension" in
    png|jpg|jpeg|gif|webp) ;;
    *) extension="bin" ;;
  esac
  remote_artifact="${remote_root}/player-artifact-${index}.${extension}"
  retry_setup "${scp_command[@]}" "$artifact" "root@${AOE_TERRITORY_HOST}:${remote_artifact}"
  remote_image_args+=" --image '${remote_artifact}'"
done

checkpoint_usage &
checkpoint_pid=$!

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
   OPENROUTER_API_KEY=arena-proxy-placeholder '${remote_root}/claux' --print \"\$(cat '${remote_root}/instruction.md')\"${remote_image_args} --permission-mode bypass --output-format json --transcript '${remote_root}/transcript.json' > '${remote_root}/result.json'"
status=$?
set -e

if [[ -n "$checkpoint_pid" ]] && kill -0 "$checkpoint_pid" 2>/dev/null; then
  kill "$checkpoint_pid" 2>/dev/null || true
  wait "$checkpoint_pid" 2>/dev/null || true
fi
checkpoint_pid=""

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

if "${scp_command[@]}" "root@${AOE_TERRITORY_HOST}:${remote_root}/transcript.json" "${transcript}.partial" 2>/dev/null && valid_transcript "${transcript}.partial"; then
  mv "${transcript}.partial" "$transcript"
elif valid_transcript "$live_transcript"; then
  cp "$live_transcript" "$transcript"
fi
"${scp_command[@]}" "root@${AOE_TERRITORY_HOST}:${remote_root}/result.json" "$native_result" 2>/dev/null || true
normalize_usage "$transcript"

# Claux exit codes 10-18 encode the failure kind (see the claux README);
# they never collide with SSH's 255 or the shell's 126-128+signal range.
failure_kind_for_exit() {
  case "$1" in
    10) printf cancelled ;;
    11) printf unavailable ;;
    12) printf authentication ;;
    13) printf context_exceeded ;;
    14) printf policy_rejection ;;
    15) printf protocol_error ;;
    16) printf model_not_found ;;
    17) printf network ;;
    18) printf output_limit_exceeded ;;
    *) printf '' ;;
  esac
}

# Prefer the classified failure claux wrote into its JSON outputs; fall back
# to the exit code for releases that only encode the kind there.
read_failure_kind() {
  local file kind
  for file in "$native_result" "$transcript"; do
    [[ -s "$file" ]] || continue
    kind="$(jq -r 'if type == "object" then (.outcome.failure.kind // "") else "" end' "$file" 2>/dev/null || true)"
    if [[ -n "$kind" ]]; then
      printf '%s' "$kind"
      return 0
    fi
  done
  failure_kind_for_exit "$status"
}

# Map a claux failure kind onto the normalized agent status. Provider and
# transport failures are not evidence about the player; running out of
# context or budget, or being refused on policy grounds, is.
status_for_failure_kind() {
  case "$1" in
    cancelled) printf interrupted ;;
    rate_limited|unavailable|network|authentication|model_not_found|protocol_error) printf unavailable ;;
    *) printf failed ;;
  esac
}

native_outcome_status=""
native_outcome_message=""
if [[ -s "$native_result" ]]; then
  native_outcome_status="$(jq -r 'if type == "object" then (.outcome.status // "") else "" end' "$native_result" 2>/dev/null || true)"
  native_outcome_message="$(jq -r 'if type == "object" then (.outcome.message // "") else "" end' "$native_result" 2>/dev/null | head -c 300 || true)"
  if [[ -z "$native_outcome_status" ]]; then
    # Releases before outcome reporting only wrote result.json on success.
    native_outcome_status="completed"
  fi
fi

write_usage_result() {
  local normalized_status="$1" summary="$2" source="$3"
  if [[ ! -s "$source" ]] && valid_transcript "$transcript"; then
    source="$transcript"
  fi
  if [[ -s "$source" ]]; then
    jq \
      --arg agent "$AOE_AGENT_ID" \
      --arg territory "$AOE_TERRITORY_ID" \
      --arg status "$normalized_status" \
      --arg summary "$summary" \
      --arg transcript "$transcript" \
      '{
        schema_version: 1,
        agent: $agent,
        territory: $territory,
        status: $status,
        summary: $summary,
        usage: {
          rounds: null,
          tool_calls: null,
          input_tokens: (.usage.input_tokens // null),
          output_tokens: (.usage.output_tokens // null),
          cost_microusd: (if .usage.cost_usd == null then null else (.usage.cost_usd * 1000000 | round) end),
          resource_units: 1
        },
        transcript: $transcript
      }' "$source" >"${AOE_RESULT_FILE}.partial"
  else
    jq -n \
      --arg agent "$AOE_AGENT_ID" \
      --arg territory "$AOE_TERRITORY_ID" \
      --arg status "$normalized_status" \
      --arg summary "$summary" \
      --arg transcript "$transcript" \
      '{schema_version:1, agent:$agent, territory:$territory, status:$status, summary:$summary, usage:{resource_units:1}, transcript:$transcript}' \
      >"${AOE_RESULT_FILE}.partial"
  fi
  mv "${AOE_RESULT_FILE}.partial" "$AOE_RESULT_FILE"
}

if [[ "$native_outcome_status" == "completed" ]]; then
  summary="$(jq -r '.result // "agent completed"' "$native_result")"
  write_usage_result completed "$summary" "$native_result"
elif [[ "$referee_interrupted" == true ]]; then
  write_usage_result interrupted "agent session was interrupted by the referee's host reboot" "$native_result"
else
  failure_kind="$(read_failure_kind)"
  if [[ -n "$failure_kind" ]]; then
    normalized_status="$(status_for_failure_kind "$failure_kind")"
    summary="Claux reported ${failure_kind} (exit status ${status})"
    if [[ -n "$native_outcome_message" ]]; then
      summary+=": ${native_outcome_message}"
    fi
  elif (( status == 255 )) && valid_transcript "$transcript"; then
    # Evidence of work is not evidence of who caused an SSH disconnect.
    # Preserve usage without charging an unproven transport failure to the player.
    normalized_status="harness_error"
    summary="Claux SSH session disconnected (exit status 255); retained transcript and usage, disconnect cause unknown"
  elif [[ ! -s "$transcript" ]]; then
    # No classification and no evidence: the harness never got going.
    normalized_status="harness_error"
    summary="Claux harness exited with status ${status} before producing a transcript"
  else
    normalized_status="failed"
    summary="Claux exited with status ${status}"
    if [[ -n "$native_outcome_message" ]]; then
      summary+=": ${native_outcome_message}"
    fi
  fi
  write_usage_result "$normalized_status" "$summary" "$native_result"
fi
exit "$status"
