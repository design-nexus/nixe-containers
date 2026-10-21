#!/usr/bin/env bash
# Install Nixe Containers from this checkout. Installs and starts Docker and
# Incus when they are missing; asks for a sudo password once.
#
#   ./install.sh                 build, install engines if needed, install the app
#   ./install.sh --skip-engines  build and install the app only
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf '\033[31m==>\033[0m %s\n' "$*" >&2; exit 1; }

app_info() {
  dir=$1 pkg="nixe-$1" bins="nixe-$1" deps="" ids=""
  case $1 in
    containers) ids="Containers" deps="gtk4 vte4 pkgconf gcc git curl" ;;
    *) return 1 ;;
  esac
}

if [[ $# == 0 || ${1:-} == --* ]]; then set -- containers "$@"; fi
skip_engines=false
for a in "$@"; do
  case $a in
    containers) ;;
    --skip-engines) skip_engines=true ;;
    -h | --help)
      sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) die "Unknown option: $a (try --help)" ;;
  esac
done

user=${USER:-$(id -un)}
groups_changed=false

in_group() { id -nG "$user" | grep -qw "$1"; }
pkg_ok() { pacman -Q "$1" >/dev/null 2>&1; }
svc_ok() { systemctl is-active --quiet "$1"; }

run() {
  if [[ ${EUID} -eq 0 ]]; then
    "$@"
  else
    sudo "$@"
  fi
}

ensure_sudo() {
  if [[ ${EUID} -eq 0 ]]; then
    return
  fi
  command -v sudo >/dev/null || die "sudo is needed to install Docker and Incus."
  if sudo -n true 2>/dev/null; then
    return
  fi
  say "Administrator access is needed to install Docker, Incus, and VTE."
  sudo -v || die "sudo is required to finish setup."
}

engines_ready() {
  pkg_ok docker && pkg_ok docker-compose && pkg_ok incus && pkg_ok vte4 || return 1
  in_group docker && in_group incus-admin && in_group incus || return 1
  svc_ok docker.socket || svc_ok docker || return 1
  svc_ok incus || return 1
  incus info >/dev/null 2>&1 || return 1
  [[ -n "$(incus storage list --format csv 2>/dev/null || true)" ]]
}

setup_engines() {
  command -v pacman >/dev/null || die "pacman is needed to install Docker and Incus."
  if engines_ready; then
    say "Docker and Incus are already set up"
    return
  fi
  ensure_sudo

  local pkgs=()
  pkg_ok docker || pkgs+=(docker)
  pkg_ok docker-compose || pkgs+=(docker-compose)
  pkg_ok incus || pkgs+=(incus)
  pkg_ok vte4 || pkgs+=(vte4)
  if ((${#pkgs[@]})); then
    say "Installing ${pkgs[*]}"
    run pacman -S --needed --noconfirm "${pkgs[@]}"
  fi

  local add=()
  in_group docker || add+=(docker)
  in_group incus-admin || add+=(incus-admin)
  in_group incus || add+=(incus)
  if ((${#add[@]})); then
    local joined
    joined=$(IFS=,; echo "${add[*]}")
    say "Adding $user to $joined"
    run usermod -aG "$joined" "$user"
    groups_changed=true
  fi

  say "Starting Docker and Incus"
  run systemctl enable --now docker.socket docker.service
  run systemctl enable --now lxcfs.service incus.socket incus.service

  if [[ -z "$(run incus storage list --format csv 2>/dev/null || true)" ]]; then
    say "Initializing Incus"
    run incus admin init --auto
  fi

  run incus info >/dev/null
  if $groups_changed; then
    say "Log out and back in so Docker and Incus group membership takes effect."
  fi
}

command -v cargo >/dev/null || die "Rust (cargo) is needed to build Containers."
if $skip_engines; then
  say "Skipping Docker and Incus setup"
else
  setup_engines
fi
pkg-config --exists vte-2.91-gtk4 || die "vte4 is needed to build the terminal (pacman -S vte4)."

say "Building nixe-containers"
(cd "$root" && cargo build --release --locked -p nixe-containers)

bin="${HOME}/.local/bin"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
desk="$share/applications"
icons="$share/icons/hicolor/scalable/apps"
mkdir -p "$bin" "$desk" "$icons"

src="${CARGO_TARGET_DIR:-$root/target}/release/nixe-containers"
[[ -f $src ]] || die "Build did not produce $src"
say "Installing nixe-containers"
install -m 755 "$src" "$bin/nixe-containers"
install -m 644 "$root/apps/containers/data/io.github.design_nexus.nixe.Containers.desktop" "$desk/"
install -m 644 "$root/crates/nixe-icons/apps/io.github.design_nexus.nixe.Containers.svg" "$icons/"
gtk-update-icon-cache -f -t "$share/icons/hicolor" >/dev/null 2>&1 || true
update-desktop-database "$desk" >/dev/null 2>&1 || true
say "Installed $bin/nixe-containers"
