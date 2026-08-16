#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

missing=()
for command in curl jq nix qemu-system-x86_64 ssh; do
  if ! command -v "$command" >/dev/null 2>&1; then
    missing+=("$command")
  fi
done
if ((${#missing[@]})); then
  printf 'demo requires these commands in PATH: %s\n' "${missing[*]}" >&2
  printf 'enter the pinned environment with: nix-shell\n' >&2
  exit 1
fi

run_id="$(date -u +%Y%m%d-%H%M%S)-$$"
demo_root="${AOE_DEMO_OUTPUT_DIR:-$repo_root/.agents-of-empires/demos/$run_id}"
match_dir="$demo_root/match"
site_dir="$demo_root/site"
base_port="${AOE_DEMO_BASE_PORT:-26000}"

case "$base_port" in
  ''|*[!0-9]*)
    printf 'AOE_DEMO_BASE_PORT must be an integer\n' >&2
    exit 1
    ;;
esac
if ((base_port < 1024 || base_port > 65529)); then
  printf 'AOE_DEMO_BASE_PORT must be between 1024 and 65529\n' >&2
  exit 1
fi

printf '[demo] building the release controller\n'
cargo build --release --quiet --bin agents-of-empires

credentials="$(scripts/prepare-first-build-credentials.sh)"
cleanup() {
  case "$credentials" in
    /var/tmp/agents-of-empires-build-credentials.*|/tmp/agents-of-empires-build-credentials.*)
      rm -rf -- "$credentials"
      ;;
  esac
}
trap cleanup EXIT

printf '[demo] racing three deterministic builders on ports %s-%s\n' \
  "$base_port" "$((base_port + 5))"
target/release/agents-of-empires run \
  arenas/first-build/arena.toml \
  --adapter oracle=adapters/oracle-build.sh \
  --credential "builder-one=$credentials/builder-one.env" \
  --credential "builder-two=$credentials/builder-two.env" \
  --credential "builder-three=$credentials/builder-three.env" \
  --base-port "$base_port" \
  --no-color \
  --output "$match_dir"

printf '[demo] generating the match report\n'
target/release/agents-of-empires report "$match_dir" --output "$site_dir"

report="$site_dir/index.html"
printf '\nDemo complete. Open:\n  file://%s\n\nArtifacts:\n  %s\n' \
  "$report" "$demo_root"
