#!/usr/bin/env bash
set -euo pipefail

target=${1:?usage: oracle-gatekeeper.sh SSH_TARGET}
ssh "$target" "printf checkout > /var/lib/gatekeeper/route && systemctl restart gatekeeper-app.service nginx.service"
