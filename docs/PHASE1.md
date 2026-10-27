# Phase 1 — Application foundation

Implemented:

- GTK4 application shell with Nyx chrome, theme, compact sidebar, settings card.
- Local Docker and Incus detection (socket probe + API ping).
- Unified workload inventory (name, host, engine, type, status).
- One remote profile: Docker over SSH, Incus over HTTPS with fingerprint pinning.
- Secret Service storage helper for later secrets (`secret-tool`).
- Connection status, Test connection, explicit SSH/TLS trust.

Not in this phase: start/stop/delete, create, metrics charts, VTE, logs, Compose,
networks, storage, snapshots, Control D-Bus, suite registration, GitHub publication.

Known limits:

- Incus on the development machine is installed but the daemon is not running; the
  app shows that as an unavailable engine.
- Live remote verification needs a host the user provides.
- Bollard's SSH connector uses the system OpenSSH client after the app has accepted
  the host key fingerprint.
