#!/bin/bash
# Builds omarchy-expand into ~/.local/bin, runs it as a systemd user service, and adds
# "Text Expander" to the app launcher for setting up combos.
#
#   ./install.sh            install, or update and restart
#   ./install.sh uninstall  stop and remove (combos in ~/.config/omarchy-expand are kept)
set -euo pipefail
cd "$(dirname "$0")"

if [[ $EUID -eq 0 ]]; then
  echo "Run install.sh as your regular user, not as root: it installs into your home directory."
  exit 1
fi

bin_dir="$HOME/.local/bin"
unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
unit="omarchy-expand.service"
desktop_file="${XDG_DATA_HOME:-$HOME/.local/share}/applications/omarchy-expand.desktop"

# The daemon reads keyboards and creates a virtual one, which needs the input group.
# Groups are fixed when a process starts, so the user's systemd manager must have it too.
check_permissions() {
  local user input_gid manager_pid
  user=$(id -un)

  if [[ ! -e /dev/uinput ]]; then
    echo "/dev/uinput does not exist, so the uinput kernel module is not loaded. Run these once:"
    echo "  sudo modprobe uinput"
    echo "  echo uinput | sudo tee /etc/modules-load.d/omarchy-expand.conf"
    echo "then run ./install.sh again."
    exit 1
  fi

  if ! id -nG | grep -qw input || [[ $(stat -c %G /dev/uinput) != "input" ]]; then
    echo "omarchy-expand needs keyboard access. Run these once, then reboot:"
    echo "  sudo usermod -aG input $user"
    echo "  echo 'KERNEL==\"uinput\", GROUP=\"input\", MODE=\"0660\", OPTIONS+=\"static_node=uinput\"' | sudo tee /etc/udev/rules.d/60-omarchy-expand-uinput.rules"
    exit 1
  fi

  input_gid=$(getent group input | cut -d: -f3)
  manager_pid=$(pgrep -u "$(id -u)" -x systemd | head -1 || true)
  if [[ -z $manager_pid ]]; then
    echo "No systemd user manager is running for $user. Log in to a graphical session and run ./install.sh there."
    exit 1
  fi
  if ! grep -E '^Groups:' "/proc/$manager_pid/status" | grep -qw "$input_gid"; then
    echo "You are in the input group, but this session started before you joined it."
    echo "Log out and back in (or reboot), then run ./install.sh again."
    exit 1
  fi
}

install_service() {
  check_permissions
  cargo build --release --locked --quiet --bin omarchy-expand
  install -Dm755 target/release/omarchy-expand "$bin_dir/omarchy-expand"
  mkdir -p "$unit_dir"
  sed "s|^ExecStart=/usr/bin/omarchy-expand|ExecStart=$bin_dir/omarchy-expand|" "systemd/$unit" >"$unit_dir/$unit"
  systemctl --user daemon-reload
  systemctl --user enable --quiet "$unit"
  systemctl --user restart "$unit"
  mkdir -p "$(dirname "$desktop_file")"
  sed "s|/usr/bin/omarchy-expand|$bin_dir/omarchy-expand|" share/omarchy-expand.desktop >"$desktop_file"
  echo "omarchy-expand is running. Open Text Expander from the app launcher to add combos."
  echo
  echo "To pick combos from the Omarchy menu with SUPER+ALT+E, add this line to ~/.config/hypr/bindings.lua:"
  echo "  o.bind(\"SUPER + ALT + E\", \"Text expander picker\", \"$bin_dir/omarchy-expand pick\")"
}

uninstall_service() {
  if [[ -e "$unit_dir/$unit" ]]; then
    systemctl --user disable --now --quiet "$unit"
    rm "$unit_dir/$unit"
    systemctl --user daemon-reload
  fi
  rm -f "$bin_dir/omarchy-expand" "$desktop_file"
  echo "omarchy-expand removed. Your combos are kept."
}

if [[ ${1:-} == "uninstall" ]]; then
  uninstall_service
else
  install_service
fi
