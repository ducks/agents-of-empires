# Stateful primary failover contract

The public record service on port 8080 is unavailable because its proxy still
targets the failed old primary on `127.0.0.1:8081`. A healthy read-only replica
is running on `127.0.0.1:8082`. Existing customer state is stored in
`/var/lib/failover/records.sqlite3` and must not be discarded or replaced.

Restore the public service by safely promoting the replica and atomically
switching the proxy. The replica reads its role from
`/var/lib/failover/replica.role` on every request. It accepts writes only when
that file contains `primary`. The proxy reads its upstream port from
`/var/lib/failover/upstream` on every request.

The public API contract is:

- `GET /health`: `200` with exact body `ready`;
- `GET /role`: `200` with exact body `primary` after promotion;
- `PUT /records/<id>`: persist the UTF-8 body and return `204`;
- `GET /records/<id>`: return the exact body, or `404` when absent.

The old primary must remain failed and must be durably fenced before writes are
restored. Preserve `/var/lib/failover/primary.failed`, create
`/var/lib/failover/primary.fenced`, and do not make port 8081 listen again.
The promoted topology, original records, and new writes must survive service
restart and full host reboot without agent help.
