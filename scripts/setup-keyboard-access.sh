#!/bin/bash
# The only privileged part of installation. No build, user files, or user
# services run as root. Paths are fixed in normal use; tests source the functions
# with temporary paths and mock device/account commands.
set -euo pipefail

keyboard_rule=/etc/udev/rules.d/60-omarchy-expand-uinput.rules
keyboard_rule_text='KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"'
keyboard_module=/etc/modules-load.d/omarchy-expand.conf
uinput_device=/dev/uinput

uinput_ready() {
  [[ -c $uinput_device ]] &&
    [[ $(stat -c %G "$uinput_device") == input ]] &&
    [[ $(stat -c %a "$uinput_device") == 660 ]]
}

keyboard_access_configured() {
  id -nG "$1" | grep -qw input &&
    [[ -f $keyboard_rule && ! -L $keyboard_rule && $(cat "$keyboard_rule") == "$keyboard_rule_text" ]] &&
    [[ -f $keyboard_module && ! -L $keyboard_module && $(cat "$keyboard_module") == uinput ]] &&
    uinput_ready
}

check_managed_file() {
  local path=$1 expected=$2
  if [[ -L $path || ( -e $path && ! -f $path ) ]]; then
    echo "Refusing to replace $path: it is not a regular file." >&2
    return 1
  fi
  if [[ -f $path && $(cat "$path") != "$expected" ]]; then
    echo "Refusing to overwrite custom content in $path. Review it before installing." >&2
    return 1
  fi
}

write_managed_file() {
  local path=$1 content=$2
  if [[ ! -e $path ]]; then
    install -Dm644 /dev/null "$path"
    printf '%s\n' "$content" >"$path"
  fi
}

configure_keyboard_access() {
  local user=$1 target_uid
  target_uid=$(id -u -- "$user") || return 1
  if [[ $target_uid == 0 ]]; then
    echo "Keyboard access must be configured for a regular user, not root." >&2
    return 1
  fi
  getent group input >/dev/null
  # Check both files before making any change, including adding group access.
  check_managed_file "$keyboard_rule" "$keyboard_rule_text"
  check_managed_file "$keyboard_module" uinput
  modprobe uinput
  write_managed_file "$keyboard_rule" "$keyboard_rule_text"
  write_managed_file "$keyboard_module" uinput
  if ! id -nG "$user" | grep -qw input; then
    usermod -aG input "$user"
  fi
  udevadm control --reload-rules
  udevadm trigger --subsystem-match=misc --sysname-match=uinput
  udevadm settle
  if ! uinput_ready; then
    echo "/dev/uinput still lacks input-group read/write access. Check for conflicting udev rules." >&2
    return 1
  fi
}

keyboard_setup_main() {
  if [[ $EUID -ne 0 || $# -ne 1 ]]; then
    echo "This helper is run as root by install.sh with the desktop user's name." >&2
    return 1
  fi
  configure_keyboard_access "$1"
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  keyboard_setup_main "$@"
fi
