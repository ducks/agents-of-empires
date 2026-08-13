# Durable service contract

Expose an HTTP service on port 8080. `GET /health` must return status 200 and
the exact body `ready`. `PUT /records/<id>` stores the request body and returns
status 204. `GET /records/<id>` returns the exact stored body with status 200.

The blank guest includes a dormant `builder-app.service` deployment slot whose
executable path is `/opt/builder/app.py`. Supply the application there and start
the existing unit. Stored records must survive restart of that service and a
host reboot. The deployment must return without agent help after reboot.
