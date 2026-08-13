# Durable job queue contract

Deploy two existing systemd slots: `queue-api.service` runs
`/opt/job-queue/api.py`, and `queue-worker.service` runs
`/opt/job-queue/worker.py`. Both use the existing SQLite database at
`/var/lib/job-queue/jobs.sqlite3`.

The API listens on port 8080. `GET /health` returns `200` and the exact body
`ready`. `POST /jobs/<id>` accepts a UTF-8 request body and returns `202`.
`GET /jobs/<id>` returns JSON containing `id`, `payload`, `status`, `result`,
and integer `attempts`. The worker completes each queued job with
`result = "processed:" + payload` and exactly one attempt.

Jobs accepted before deployment already exist in the database. They must be
completed, not discarded or replaced. Existing and newly accepted jobs must
survive worker restart and host reboot. Both services must return without agent
help after reboot.
