# Phase 6 — Networks, storage, snapshots

Implemented:

- Networks page lists Docker and Incus networks. Create a bridge. Delete
  skips Docker builtins (`bridge`, `host`, `none`, `docker_gwbridge`) and
  unmanaged Incus physical networks.
- Storage page lists Docker volumes and Incus pool volumes. Create a Docker
  named volume or an Incus custom volume on a pool (default `default`). Only
  custom Incus volumes can be removed.
- Incus instance details list snapshots. Create, delete, and restore from
  the details dialog.

Restore uses `PUT /1.0/instances/{name}` with `{"restore": "<snap>"}`.

Not in this phase: packaging, docs polish, suite registration, GitHub.
