# Phase 2 — Core management

Implemented:

- Start, stop, restart, and delete for Docker containers and Incus containers/VMs.
- Workload details (status, image, created, command/address, live CPU and memory).
- Create container or VM from a name and image, with start-after-create.
- Image list, pull, inspect, and remove for Docker and Incus.
- Confirmation dialogs for delete/remove that name the target host.

Not in this phase: VTE terminal, log streaming, Compose, networks, volumes,
snapshots, Control D-Bus, suite registration, GitHub publication.

Known limits:

- Incus on the development machine is installed but the daemon is not running.
- Live remote verification needs a host the user provides.
- Incus create uses a local image alias or fingerprint. Pull images first for
  aliases that are not already on the host. Incus pull uses
  `images.linuxcontainers.org`.
- Docker virtual machines are refused. VMs are Incus only.
- Container and image delete cannot be undone, so they keep a confirmation.
