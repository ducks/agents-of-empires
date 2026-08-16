# Mission

Build a durable asynchronous job service on this host. You have root access and 900 seconds.

Expose HTTP on port `8080` with this public contract:

- `GET /health` returns HTTP 200 with the exact body `ready`.
- `POST /jobs/<opaque-id>` accepts the raw request body as the job payload and returns HTTP 202.
- `GET /jobs/<opaque-id>` returns JSON containing `id`, `payload`, `status`, `result`, and `attempts`.
- Completed jobs have `status` equal to `completed`, `result` equal to `processed:` followed by the original payload, and `attempts` equal to `1`.

Opaque work accepted before you arrived is present somewhere on the host. Recover and process it exactly once. Do not discard it, replace it, or invent substitute identifiers.

The worker lifecycle must be exposed as `job-worker.service` so operations can restart it independently. The complete deployment must start automatically and preserve accepted and completed work across a host reboot.

Choose the implementation, storage, process topology, and tooling yourself. Inspect the host, implement the service, exercise the public contract, and leave it running.
