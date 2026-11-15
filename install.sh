#!/bin/bash
# Build and install as the desktop user. Only keyboard setup runs as root.
set -euo pipefail

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/setup-keyboard-access.sh
source "$repo_dir/scripts/setup-keyboard-access.sh"

bin_dir="$HOME/.local/bin"
unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
unit="omarchy-expand.service"
desktop_file="${XDG_DATA_HOME:-$HOME/.local/share}/applications/omarchy-expand.desktop"
accept_keyboard_access=false

usage() {
  echo "Usage: ./install.sh [--accept-keyboard-access | uninstall]"
  echo "Run as your regular user in your Omarchy session."
  echo "--accept-keyboard-access explicitly approves input-group and uinput setup."
}

check_session() {
  local command
  for command in cargo gcc wl-copy wl-paste systemctl systemd-run; do
    if ! command -v "$command" >/dev/null; then
      echo "Missing $command. Install dependencies first:" >&2
      echo "  omarchy pkg add git rust gcc wl-clipboard libxkbcommon" >&2
      return 1
    fi
  done
  if [[ -z ${HYPRLAND_INSTANCE_SIGNATURE:-} || -z ${WAYLAND_DISPLAY:-} ]]; then
    echo "Run ./install.sh inside your Omarchy graphical session." >&2
    return 1
  fi
  # Probe a process launched by the actual user manager. A new terminal can have
  # the input group while the manager (and all its services) still lacks it.
  if ! manager_groups=$(systemd-run --user --quiet --wait --pipe --collect id -G); then
    echo "Cannot reach your systemd user manager. Run ./install.sh in your Omarchy session." >&2
    return 1
  fi
}

approve_keyboard_access() {
  local answer
  echo "Keyboard access is required to recognize keywords and insert snippets."
  echo "Setup adds $install_user to the input group, loads uinput at boot, and"
  echo "installs a udev rule giving the input group access to /dev/uinput."
  echo "Other programs running as your account can then read keyboard events."
  if [[ $accept_keyboard_access == true ]]; then
    return 0
  fi
  if [[ ! -t 0 ]]; then
    echo "Run ./install.sh in a terminal to approve this, or explicitly pass --accept-keyboard-access." >&2
    return 1
  fi
  read -r -p "Allow keyboard access? [y/N] " answer || return 1
  case "$answer" in
    y|Y|yes|YES) return 0 ;;
    *) echo "Installation cancelled; keyboard permissions were not changed."; return 1 ;;
  esac
}

run_keyboard_setup() {
  if [[ -t 0 ]]; then
    sudo /bin/bash "$repo_dir/scripts/setup-keyboard-access.sh" "$install_user"
  else
    pkexec /bin/bash "$repo_dir/scripts/setup-keyboard-access.sh" "$install_user"
  fi
}

start_service() {
  if ! systemctl --user restart "$unit"; then
    return 1
  fi
  # Type=simple can report success before the process exits on a startup error.
  sleep 1
  systemctl --user is-active --quiet "$unit"
}

install_service() {
  local install_user input_gid manager_groups
  install_user=$(id -un)
  input_gid=$(getent group input | cut -d: -f3 || true)
  if [[ -z $input_gid ]]; then
    echo "The input group is missing. This installer requires an Omarchy system." >&2
    return 1
  fi
  check_session
  # Finish the build before asking for administrator access or changing an
  # existing installation. A failed build leaves the installed app untouched.
  (cd "$repo_dir" && cargo build --release --locked --bin omarchy-expand)

  if ! keyboard_access_configured "$install_user"; then
    approve_keyboard_access
    run_keyboard_setup
    if ! keyboard_access_configured "$install_user"; then
      echo "Keyboard setup did not complete; installation stopped." >&2
      return 1
    fi
  fi

  install -Dm755 "$repo_dir/target/release/omarchy-expand" "$bin_dir/omarchy-expand"
  mkdir -p "$unit_dir" "$(dirname "$desktop_file")"
  sed "s|^ExecStart=/usr/bin/omarchy-expand|ExecStart=$bin_dir/omarchy-expand|" "$repo_dir/systemd/$unit" >"$unit_dir/$unit"
  sed "s|/usr/bin/omarchy-expand|$bin_dir/omarchy-expand|" "$repo_dir/share/omarchy-expand.desktop" >"$desktop_file"
  if command -v update-desktop-database >/dev/null; then
    update-desktop-database "$(dirname "$desktop_file")"
  fi
  systemctl --user daemon-reload
  systemctl --user enable --quiet "$unit"

  if ! grep -qw "$input_gid" <<<"$manager_groups"; then
    systemctl --user stop "$unit"
    echo
    echo "Installed. REBOOT REQUIRED to activate keyboard access."
    echo "Save your work and reboot when ready. Text expansion is not active yet."
    echo "The service will start automatically at your next graphical login."
    echo "You do not need to run the installer again."
  elif start_service; then
    echo
    echo "Installed. The omarchy-expand service is running."
  else
    echo "Installed, but the service failed to start. Check these diagnostics:" >&2
    systemctl --user status "$unit" --no-pager >&2 || true
    journalctl --user -u "$unit" -n 30 --no-pager >&2 || true
    return 1
  fi
  echo "Open Text Expander from the app launcher (Super+Space) to add combos."
  echo
  echo "Optional picker shortcut for ~/.config/hypr/bindings.lua:"
  echo "  o.bind(\"SUPER + ALT + E\", \"Text expander picker\", \"$bin_dir/omarchy-expand pick\")"
}

uninstall_service() {
  if [[ -e "$unit_dir/$unit" ]]; then
    systemctl --user disable --now --quiet "$unit"
    rm "$unit_dir/$unit"
    systemctl --user daemon-reload
  fi
  rm -f "$bin_dir/omarchy-expand" "$desktop_file"
  echo "omarchy-expand removed. Your combos and shared keyboard permissions are kept."
}

main() {
  if [[ $EUID -eq 0 ]]; then
    echo "Run install.sh as your regular user, not as root: it installs into your home directory." >&2
    return 1
  fi
  if [[ $# -gt 1 ]]; then usage >&2; return 1; fi
  case "${1:-}" in
    --help|-h) usage ;;
    uninstall) uninstall_service ;;
    --accept-keyboard-access) accept_keyboard_access=true; install_service ;;
    "") install_service ;;
    *) usage >&2; return 1 ;;
  esac
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  main "$@"
fi
