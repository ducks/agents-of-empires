#!/usr/bin/env bash
set -euo pipefail

target=${1:?usage: oracle-archivist.sh SSH_TARGET}
ssh "$target" "psql --username arena --dbname empire --set ON_ERROR_STOP=1 --command \"CREATE TABLE IF NOT EXISTS empire_state (key text PRIMARY KEY, value text NOT NULL); INSERT INTO empire_state (key, value) VALUES ('status', 'archivist:ok') ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value\" && systemctl restart archivist-app.service"
