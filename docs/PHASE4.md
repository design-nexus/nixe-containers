# Phase 4 — Overview, diagnostics, dashboard UI

Implemented:

- Overview is a host dashboard: engine chips, CPU/memory/engine-disk/guest
  tiles with bars and sparklines, running list.
- Local CPU/memory from `/proc`. Engine disk from Docker `df` and Incus
  storage pools. Remote memory/disk from Docker/Incus APIs.
- Settings → Diagnostics with Fix (pkexec) and Copy.
- Connections has an Open diagnostics button. Workload rows have a status dot.

Not in this phase: Compose, networks, volumes, snapshots, suite registration.
