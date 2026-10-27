# Containers

GTK4 manager for Incus and Docker. Application source lives in apps/containers.
Shared dependencies use pinned Git sources.

Commands: `cargo build --release --locked -p nyx-containers`; `./install.sh`

Use the public, pinned shared crates; never import another application's internals.
Keep application names, IDs, paths, D-Bus interfaces, and data formats stable.
Reuse chrome, theme tokens, widgets, icons, and shortcuts from the pinned
nyx-design crates. Page layout is a hypervisor dashboard, not STYLE.md's
settings list.
Run essential targeted checks only. Do not download models or run inference for
routine verification. Install app updates without opening/restarting them.
Common guidance: https://github.com/design-nexus/nyx/blob/main/docs/AGENTS_COMMON.md
This file carries the essential common rules so offline work remains possible.

Shared dependencies use immutable GitHub revisions. Coordinate public API changes
with the suite manifest and affected consumers in design-nexus/nyx.
Never force-push, delete history, or publish a new GitHub repository without
explicit approval.
