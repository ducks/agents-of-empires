# Blueprint Build

Build and leave running the production system specified by the attached
blueprint. The image is the authoritative service and topology contract. Choose
the implementation language, persistence layer, queue design, and internal
protocols yourself.

The blank NixOS host provides three lifecycle slots which the referee may stop
or restart independently:

- `blueprint-edge.service` launches `/opt/blueprint/edge`
- `blueprint-api.service` launches `/opt/blueprint/api`
- `blueprint-worker.service` launches `/opt/blueprint/worker`

Install executable launchers at those paths and enable the units. Persistent
state belongs beneath `/var/lib/blueprint`. Public internet access is disabled.
You have root access and may use the software already installed on the host.
