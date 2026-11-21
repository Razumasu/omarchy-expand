#!/bin/bash
# Installer integration tests. All files live in a temporary directory; account,
# device, privilege, build, and service commands are fakes. Never needs sudo.
# Fakes are invoked indirectly by the sourced installer.
# shellcheck disable=SC2317
set -euo pipefail
test_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

fixture() {
  # shellcheck source=install.sh
  source "$test_repo/install.sh"
  repo_dir="$case_dir/source"
  bin_dir="$case_dir/user/bin"
  unit_dir="$case_dir/user/systemd"
  desktop_file="$case_dir/user/applications/omarchy-expand.desktop"
  keyboard_rule="$case_dir/etc/udev/60-omarchy-expand-uinput.rules"
  keyboard_module="$case_dir/etc/modules-load/omarchy-expand.conf"
  uinput_device="$case_dir/missing-uinput"
  accept_keyboard_access=true
  export HYPRLAND_INSTANCE_SIGNATURE=installer-test WAYLAND_DISPLAY=wayland-test
  mkdir -p "$repo_dir" "$(dirname "$keyboard_rule")" "$(dirname "$keyboard_module")"
  cp -r "$test_repo/systemd" "$test_repo/share" "$repo_dir/"
  : >"$case_dir/commands"
}

record() { echo "$*" >>"$case_dir/commands"; }
id() {
  case "$1" in
    -un) echo testuser ;;
    -u) echo 1000 ;;
    -nG)
      if [[ -f $case_dir/group ]]; then echo 'testuser input'; else echo testuser; fi ;;
    *) return 1 ;;
  esac
}
getent() { echo 'input:x:992:'; }
systemd-run() {
  record "systemd-run $*"
  if [[ $scenario == no-manager ]]; then return 1; fi
  echo "$session_groups"
}
cargo() {
  record "cargo $*"
  if [[ $scenario == build-failure ]]; then return 1; fi
  mkdir -p target/release
  printf '#!/bin/sh\nexit 0\n' >target/release/omarchy-expand
}
# Dependencies only need to be discoverable for the preflight check.
gcc() { :; }
wl-copy() { :; }
wl-paste() { :; }
pkexec() {
  record "pkexec $*"
  if [[ $scenario == auth-denied ]]; then return 126; fi
  configure_keyboard_access testuser
}
sudo() { pkexec "$@"; }
modprobe() {
  record "modprobe $*"
  if [[ $scenario == module-failure ]]; then return 1; fi
  uinput_device=/dev/null
}
usermod() { record "usermod $*"; touch "$case_dir/group"; }
udevadm() {
  record "udevadm $*"
  if [[ $1 == trigger && $scenario != udev-conflict ]]; then touch "$case_dir/device-access"; fi
}
stat() {
  if [[ ${3:-} == /dev/null ]]; then
    if [[ ! -f $case_dir/device-access ]]; then echo root
    elif [[ $2 == %G ]]; then echo input
    else echo 660
    fi
  else
    command stat "$@"
  fi
}
systemctl() {
  record "systemctl $*"
  case "$*" in
    *is-active*) [[ $scenario != start-failure ]] ;;
    *) return 0 ;;
  esac
}
journalctl() { record "journalctl $*"; echo 'fixture startup failure'; }
update-desktop-database() { record "update-desktop-database $*"; }
sleep() { :; }

preset_access() {
  touch "$case_dir/group" "$case_dir/device-access"
  uinput_device=/dev/null
  printf '%s\n' "$keyboard_rule_text" >"$keyboard_rule"
  echo uinput >"$keyboard_module"
}

run_fixture() {
  fixture
  session_groups=1000
  case "$scenario" in
    ready|repeat|start-failure|uninstall) preset_access; session_groups='1000 992' ;;
    stale-session) preset_access ;;
    rule-only) touch "$case_dir/group"; session_groups='1000 992' ;;
    no-consent|interactive-accept|interactive-refuse) accept_keyboard_access=false ;;
    custom-rule) echo 'custom rule' >"$keyboard_rule" ;;
    custom-module) echo 'custom module' >"$keyboard_module" ;;
    symlink-rule) echo keep >"$case_dir/preserve"; ln -s "$case_dir/preserve" "$keyboard_rule" ;;
  esac
  if [[ $scenario == build-failure ]]; then
    mkdir -p "$bin_dir"
    echo 'existing binary' >"$bin_dir/omarchy-expand"
  fi
  if [[ $scenario == unknown-argument ]]; then main --typo; return; fi
  install_service
  if [[ $scenario == rebooted ]]; then
    session_groups='1000 992'
    install_service
  fi
  if [[ $scenario == repeat ]]; then install_service; fi
  if [[ $scenario == uninstall ]]; then uninstall_service; fi
}

if [[ ${1:-} == --case ]]; then
  scenario=$2
  case_dir=$3
  # This is a separate shell: failures exercise errexit exactly as install.sh
  # does, rather than disabling it by calling the installer in an if/! context.
  run_fixture
  exit
fi

suite_dir=$(mktemp -d)
trap 'rm -rf "$suite_dir"' EXIT
assert_contains() { grep -Fq -- "$2" "$1" || { echo "Missing '$2' in $1" >&2; exit 1; }; }
assert_absent() { if grep -Fq -- "$2" "$1"; then echo "Unexpected '$2' in $1" >&2; exit 1; fi; }

for scenario in fresh ready repeat rebooted stale-session rule-only no-consent interactive-accept interactive-refuse auth-denied build-failure no-manager custom-rule custom-module symlink-rule module-failure udev-conflict start-failure uninstall unknown-argument; do
  case_dir="$suite_dir/$scenario"
  mkdir -p "$case_dir"
  result=0
  if [[ $scenario == interactive-* ]]; then
    answer=n
    if [[ $scenario == interactive-accept ]]; then answer=y; fi
    printf -v fixture_command 'bash %q --case %q %q' "$test_repo/test/installer.sh" "$scenario" "$case_dir"
    printf '%s\n' "$answer" | script --quiet --return --command "$fixture_command" /dev/null >"$case_dir/output" 2>&1 || result=$?
  else
    bash "$test_repo/test/installer.sh" --case "$scenario" "$case_dir" >"$case_dir/output" 2>&1 || result=$?
  fi
  case "$scenario" in
    fresh|ready|repeat|rebooted|stale-session|rule-only|interactive-accept|uninstall) expected=0 ;;
    *) expected=1 ;;
  esac
  if [[ ($expected == 0 && $result != 0) || ($expected != 0 && $result == 0) ]]; then
    cat "$case_dir/output" >&2
    echo "FAIL: $scenario exited $result" >&2
    exit 1
  fi
  commands="$case_dir/commands"
  output="$case_dir/output"
  case "$scenario" in
    fresh|ready|repeat|rebooted|stale-session|rule-only|interactive-accept|start-failure)
      [[ -x $case_dir/user/bin/omarchy-expand && -f $case_dir/user/applications/omarchy-expand.desktop ]]
      assert_contains "$commands" 'systemctl --user enable --quiet omarchy-expand.service'
      assert_contains "$case_dir/user/systemd/omarchy-expand.service" "ExecStart=$case_dir/user/bin/omarchy-expand daemon"
      ;;
  esac
  case "$scenario" in
    fresh|stale-session|interactive-accept)
      assert_contains "$output" 'REBOOT REQUIRED'
      assert_contains "$output" 'You do not need to run the installer again.'
      assert_absent "$commands" 'systemctl --user restart'
      assert_absent "$output" 'service is running'
      ;;
    ready|repeat|rule-only)
      assert_contains "$commands" 'systemctl --user restart omarchy-expand.service'
      assert_contains "$output" 'service is running'
      assert_absent "$output" 'REBOOT REQUIRED'
      ;;
    rebooted)
      assert_contains "$output" 'REBOOT REQUIRED'
      assert_contains "$output" 'service is running'
      [[ $(grep -c '^pkexec ' "$commands") == 1 ]]
      ;;
    start-failure)
      assert_contains "$output" 'service failed to start'
      assert_contains "$output" 'fixture startup failure'
      assert_absent "$output" 'service is running'
      ;;
    uninstall)
      [[ ! -e $case_dir/user/bin/omarchy-expand && ! -e $case_dir/user/applications/omarchy-expand.desktop ]]
      [[ -f $case_dir/etc/udev/60-omarchy-expand-uinput.rules && -f $case_dir/group ]]
      assert_contains "$commands" 'systemctl --user disable --now'
      ;;
    build-failure)
      assert_contains "$case_dir/user/bin/omarchy-expand" 'existing binary'
      assert_absent "$commands" pkexec
      ;;
    *)
      [[ ! -e $case_dir/user/bin/omarchy-expand && ! -e $case_dir/user/applications/omarchy-expand.desktop ]]
      assert_absent "$commands" 'systemctl --user enable'
      ;;
  esac
  case "$scenario" in
    ready|repeat|stale-session|no-consent|interactive-refuse|no-manager|unknown-argument) assert_absent "$commands" pkexec ;;
    fresh)
      assert_contains "$commands" 'usermod -aG input testuser'
      assert_contains "$commands" 'udevadm trigger --subsystem-match=misc --sysname-match=uinput'
      [[ $(cat "$case_dir/etc/modules-load/omarchy-expand.conf") == uinput ]]
      [[ $(command stat -c %a "$case_dir/etc/udev/60-omarchy-expand-uinput.rules") == 644 ]]
      ;;
    custom-rule|custom-module|symlink-rule)
      assert_contains "$output" 'Refusing to'
      assert_absent "$commands" modprobe
      assert_absent "$commands" usermod
      ;;
    auth-denied) assert_absent "$commands" modprobe ;;
    module-failure) assert_absent "$commands" usermod ;;
  esac
  echo "PASS: $scenario"
done
