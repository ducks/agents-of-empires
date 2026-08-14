# Hello service contract

Deploy an HTTP service on port 8080. `GET /health` must return status 200 and the exact body `ready`.

The service must be managed by the existing `arena-app.service` systemd unit and remain healthy after that service is restarted and after the host reboots. Do not replace, disable, or bypass the existing unit.
