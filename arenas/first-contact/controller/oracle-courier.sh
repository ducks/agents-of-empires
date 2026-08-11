#!/usr/bin/env bash
set -euo pipefail

target=${1:?usage: oracle-courier.sh SSH_TARGET}
ssh "$target" "systemctl restart redis-arena.service courier-worker.service courier-app.service && redis-cli SET empire:alive courier PX 5000 >/dev/null"
