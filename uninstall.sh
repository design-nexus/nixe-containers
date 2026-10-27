#!/usr/bin/env bash
# Remove Nyx Containers.
set -euo pipefail
purge=false
for a in "$@"; do
  case $a in
    --purge) purge=true ;;
  esac
done
bin="${HOME}/.local/bin"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
config="${XDG_CONFIG_HOME:-$HOME/.config}"
rm -f "$bin/nyx-containers"
rm -f "$share/applications/io.github.design_nexus.nyx.Containers.desktop"
rm -f "$share/icons/hicolor/scalable/apps/io.github.design_nexus.nyx.Containers.svg"
gtk-update-icon-cache -f -t "$share/icons/hicolor" >/dev/null 2>&1 || true
update-desktop-database "$share/applications" >/dev/null 2>&1 || true
if $purge; then
  rm -rf "$config/nyx-containers" "$share/nyx-containers"
fi
printf '\033[1m==>\033[0m Removed nyx-containers\n'
