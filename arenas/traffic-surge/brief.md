# Mission

Keep a small stateful service correct and responsive through a traffic surge. You have root access and 900 seconds.

The existing deployment exposes HTTP on port `8080`:

- `GET /health` returns HTTP 200 with the exact body `ready`.
- `PUT /priority/<opaque-id>` durably stores the raw request body and returns HTTP 204.
- `GET /priority/<opaque-id>` returns the exact stored body.
- `GET /optional/<opaque-id>` performs nonessential work. It may return HTTP 200 with `optional:<opaque-id>`, or deliberately shed work with HTTP 429 or 503.

Historical priority records already exist and must not be lost. The controller will generate increasing optional traffic while submitting opaque priority writes. Priority writes must remain responsive, every accepted write must remain recoverable, and optional traffic must stay bounded rather than consuming the service indefinitely.

The public service lifecycle is `traffic-surge.service`. Your repaired deployment must survive that service being restarted while traffic continues, then survive a host reboot with historical and newly accepted priority state intact.

Choose the strategy yourself. You may optimize the application, introduce concurrency, bound admission, cache optional work, deliberately shed it, or replace the implementation. Inspect the host, test your assumptions, and leave a durable service running.
