# Nixe Containers

A native GTK4 app for managing Incus and Docker on this computer and one remote
server. No agent, no web UI.

## Features

- **Overview** — host dashboard: engine chips, CPU / memory / engine disk /
  guest tiles with sparklines, running list.
- **Containers and VMs** — start, stop, restart, delete, inspect, logs, terminal.
- **Stacks** — Docker Compose projects from `com.docker.compose.project` labels.
  Up, Down, Restart, combined logs.
- **Images** — list, pull, inspect, remove.
- **Networks** — Docker and Incus bridges. Create and remove. Docker builtins
  and unmanaged Incus physical nets stay put.
- **Storage** — Docker volumes and Incus pool volumes. Create a named volume or
  an Incus custom volume. Restore-safe delete of custom volumes only.
- **Snapshots** — Incus instance snapshots on the details dialog: create, delete,
  restore.
- **Connections** — local engines plus one remote. Docker over SSH, Incus over
  HTTPS. SSH host key and TLS pin required before traffic.
- **Diagnostics** — Settings card with Fix (pkexec) or Copy for missing
  packages, groups, daemons, Incus storage, and VTE.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nixe-containers/main/install.sh | bash
```

From a checkout, `./install.sh` does the same.

Asks for sudo when Docker, Incus, or VTE is missing. Installs those packages,
adds your user to `docker`, `incus-admin`, and `incus`, starts the daemons, and
runs `incus admin init --auto` if Incus has no storage yet.

```
./install.sh --skip-engines
```

Builds and installs the app only. Use this when engines are already set up or
sudo is unavailable.

After a group change, log out of the desktop and back in. Reloading Hyprland
does not pick up new groups.

Binary: `~/.local/bin/nixe-containers`. Application id:
`io.github.design_nexus.nixe.Containers`.

```
./uninstall.sh
./uninstall.sh --purge
```

`--purge` also removes config and cache.

## Run

```
nixe-containers
cargo run -p nixe-containers
nixe-containers --section overview
```

`--section` accepts `overview`, `containers`, `vms`, `stacks`, `images`,
`networks`, `storage`, `connections`.

## Limits (v1.0)

- One remote profile.
- Compose from running container labels, not a YAML editor.
- No Kubernetes, clusters, extra remotes, agents, or web dashboard.

Suite registration and a public GitHub repo stay off until you ask.
