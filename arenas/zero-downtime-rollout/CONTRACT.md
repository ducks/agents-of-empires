# Zero-downtime rollout contract

A stateful v1 record service is live on the public endpoint at port 8080. It is
served by `rollout-v1.service` on port 8081 through the stable
`rollout-proxy.service`. Existing records in
`/var/lib/rollout/records.sqlite3` are customer state and must be preserved.

Deploy v2 using the dormant `rollout-v2.service` slot. It runs
`/opt/rollout/v2.py` and must listen on `127.0.0.1:8082`. v2 implements:

- `GET /health`: `200` with the exact body `ready`;
- `GET /version`: `200` with the exact body `v2`;
- `PUT /records/<id>`: store the UTF-8 request body and return `204`;
- `GET /records/<id>`: return the exact stored body, or `404` when absent.

The public proxy reads its upstream port from `/var/lib/rollout/upstream` for
every request. It initially contains `8081`. Switch it atomically to `8082`
only after v2 is ready. The public endpoint is monitored throughout the rollout.
Any failed health, version, or existing-record request makes the rollout
ineligible to win, even if the service later recovers.

Do not stop or restart `rollout-proxy.service` or `rollout-v1.service` during
cutover. After switching traffic, leave v2 and the proxy enabled and healthy.
The v2 deployment and all records must survive a v2 service restart and a full
host reboot without agent help.
