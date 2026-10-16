#!/bin/bash
# Picks a combo from the Omarchy menu with wtype and checks the daemon pastes it into the
# window that had focus. Also checks cancelling, a missing daemon, and that a second
# daemon refuses to start. Menu screenshots go to the artifacts directory.
set -euo pipefail
cd "$(dirname "$0")/.."
source test/lib.sh

if systemctl --user is-active --quiet omarchy-expand; then
  echo "FAIL: the omarchy-expand service is running and would expand alongside the test daemon"
  exit 1
fi

cargo build --release --quiet --bin omarchy-expand

XDG_CONFIG_HOME=$(mktemp -d)
export XDG_CONFIG_HOME
mkdir -p "$XDG_CONFIG_HOME/omarchy-expand"
cat >"$XDG_CONFIG_HOME/omarchy-expand/combos.json" <<'EOF'
{ "groups": [
  { "name": "Test", "combos": [
    { "keyword": "oemail", "name": "Email", "snippet": "razu@example.com" },
    { "keyword": "oepick", "name": "Menu test", "snippet": "Picked from the menu æøå" },
    { "keyword": "oeoff", "snippet": "Disabled", "enabled": false }
  ] }
] }
EOF
printf 'clipboard before' | wl-copy

target/release/omarchy-expand daemon 2>"$artifacts/daemon.log" &
daemon=$!
cleanup() {
  kill "$daemon" 2>/dev/null || true
  omarchy-menu close >/dev/null 2>&1 || true
  close_targets
  rm -rf "$XDG_CONFIG_HOME"
}
trap cleanup EXIT
sleep 0.5

second_daemon=$(target/release/omarchy-expand daemon 2>&1 || true)
check "a second daemon refuses to start" "$second_daemon" "omarchy-expand: another omarchy-expand daemon is already running"

menu_open() {
  hyprctl -j layers | grep -qE '"namespace": *"omarchy-menu"'
}
menu_closed() {
  ! menu_open
}

open_target oe-picker
target/release/omarchy-expand pick &
picker=$!
wait_for "the menu to open" menu_open
sleep 0.4
grim "$artifacts/1-picker-menu.png"
wtype "menu test"
sleep 0.4
grim "$artifacts/2-picker-filtered.png"
wtype -k Return
picker_status=0
wait "$picker" || picker_status=$?
check "picker exits cleanly after a pick" "$picker_status" "0"
sleep 0.3
wtype -k Return
check "picked combo pasted into the focused window" "$(saved_line oe-picker)" "Picked from the menu æøå"
check "clipboard restored after the pick" "$(wl-paste --no-newline)" "clipboard before"

target/release/omarchy-expand pick &
picker=$!
wait_for "the menu to open again" menu_open
wtype -k Escape
picker_status=0
wait "$picker" || picker_status=$?
check "cancelling the picker is not an error" "$picker_status" "0"
wait_for "the menu to close" menu_closed

kill "$daemon"
wait "$daemon" 2>/dev/null || true
picker_status=0
target/release/omarchy-expand pick 2>/dev/null || picker_status=$?
check "picker without a daemon fails" "$picker_status" "1"
if menu_open; then
  echo "FAIL: the menu opened although nothing could be pasted"
  failures=$((failures + 1))
else
  echo "PASS: no menu without a daemon"
fi
exit "$failures"
