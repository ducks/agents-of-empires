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
done
exit 0
EOF

cat >"$root/bin/scp" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
source_path="${@: -2:1}"
destination="${@: -1}"
if [[ "$source_path" == *:*/transcript.json ]]; then
  printf '%s' '{"schema_version":2}' >"$destination"
elif [[ "$source_path" == *:*/result.json ]]; then
  printf '%s' '{"result":"held the line","usage":{"input_tokens":120,"output_tokens":8,"cost_usd":0.0012}}' >"$destination"
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
AOE_AGENT_ID=test-agent \
AOE_TERRITORY_ID=test-territory \
AOE_TERRITORY_HOST=127.0.0.1 \
AOE_SSH_PORT=26000 \
AOE_MODEL=test/model \
AOE_REASONING_EFFORT=low \
AOE_INSTRUCTION_FILE="$root/instruction.md" \
AOE_RESULT_FILE="$root/run/result.json" \
AOE_CREDENTIAL_FILE="$root/credential.env" \
AOE_CLAUX_BINARY="$root/claux" \
AOE_OPENROUTER_PROXY="$root/fake-proxy.py" \
  "$adapter"

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

if rg -q 'controller-only-secret' "$root/run"; then
  echo "controller credential leaked into adapter artifacts" >&2
  exit 1
fi
