#!/bin/bash
# Drives the setup UI in a floating foot window with wtype, then checks that the combo
# it saved is expanded by a running daemon without a restart. Screenshots of each step
# go to the artifacts directory for visual review.
set -euo pipefail
cd "$(dirname "$0")/.."
source test/lib.sh

if systemctl --user is-active --quiet omarchy-expand; then
  echo "FAIL: the omarchy-expand service is running and would expand alongside the test daemon"
  exit 1
fi

cargo build --release --quiet --bin omarchy-expand --example typist

XDG_CONFIG_HOME=$(mktemp -d)
export XDG_CONFIG_HOME
combos=$XDG_CONFIG_HOME/omarchy-expand/combos.json

target/release/omarchy-expand daemon 2>"$artifacts/daemon.log" &
daemon=$!
cleanup() {
  kill "$daemon" 2>/dev/null || true
  hyprctl dispatch 'hl.dsp.window.close({ window = "class:^TUI.float$" })' >/dev/null 2>&1 || true
  close_targets
  rm -rf "$XDG_CONFIG_HOME"
}
trap cleanup EXIT

screenshot() {
  sleep 0.4
  grim "$artifacts/$1.png"
}
keys() {
  wtype "$@"
  sleep 0.2
}

# The window gets Omarchy's floating TUI treatment through its TUI.float app id.
hyprctl dispatch "hl.dsp.exec_cmd([[foot --app-id TUI.float env XDG_CONFIG_HOME=$XDG_CONFIG_HOME $PWD/target/release/omarchy-expand]])" >/dev/null
wait_for "the setup UI to get focus" focused TUI.float
screenshot 1-empty

keys a
keys oetui
keys -k Tab
keys "TUI test"
keys -k Tab
keys "Hej fra TUI æøå"
screenshot 2-new-combo-form
keys -M ctrl s -m ctrl
screenshot 3-combo-list

check "combo saved by the UI" "$(python3 -c 'import json,sys; c=json.load(open(sys.argv[1]))["groups"][0]["combos"][0]; print(c["keyword"], c["name"], c["snippet"], sep="|")' "$combos")" "oetui|TUI test|Hej fra TUI æøå"

keys s
screenshot 4-settings
keys -k Escape
keys /
keys fra
screenshot 5-search
keys -k Escape
keys q
wait_for "the setup UI to close" bash -c '! hyprctl -j clients | grep -q "\"class\": *\"TUI.float\""'

open_target oe-tui
type_text oetui --enter-after-ms 800
check "daemon expands the combo saved by the UI" "$(saved_line oe-tui)" "Hej fra TUI æøå"
exit "$failures"
