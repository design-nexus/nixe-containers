# Phase 3 — Terminal and logs

Implemented:

- Logs dialog with follow, search, and Docker stdout/stderr filter.
- Interactive VTE terminal via Docker exec and Incus exec.
- Incus console for containers and VMs.
- Remote sessions use the same SSH Docker and pinned HTTPS Incus transports.

Not in this phase: Compose, networks, volumes, snapshots, Control D-Bus, suite
registration, GitHub publication.

Known limits:

- `./install.sh` installs Docker, Incus, and `vte4` when missing, starts the
  daemons, and initializes Incus. A new login is needed after group changes.
- Exec needs a running workload. Logs work for stopped Docker containers and for
  Incus log files when the daemon is up.
- Incus log follow polls every two seconds. Docker uses the native follow stream.
- Live Incus and remote checks still need a running daemon and a host the user
  provides.
