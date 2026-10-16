#!/bin/bash
# End-to-end checks inside a live Omarchy session: expansion, clipboard restore,
# config reload, focus reset and excluded apps.
# Needs the user in the input group and a writable /dev/uinput.
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
config_dir=$XDG_CONFIG_HOME/omarchy-expand
mkdir -p "$config_dir"
write_combo() {
  printf '{"groups":[{"name":"Test","combos":[{"keyword":"%s","snippet":"%s"}]}]}' "$1" "$2" >"$config_dir/combos.json"
}
write_combo oetest "Hello æøå from omarchy-expand"
printf 'clipboard before' | wl-copy

target/release/omarchy-expand daemon 2>"$artifacts/daemon.log" &
daemon=$!
cleanup() {
  kill "$daemon" 2>/dev/null || true
  close_targets
  rm -rf "$XDG_CONFIG_HOME"
}
trap cleanup EXIT

# The class is not "foot", so the paste shortcut must come from the process name,
# as it does for Omarchy's TUIs.
open_target oe-expand
type_text oetest --enter-after-ms 800
check "keyword replaced by snippet" "$(saved_line oe-expand)" "Hello æøå from omarchy-expand"
check "clipboard restored" "$(wl-paste --no-newline)" "clipboard before"

write_combo oetest "Reloaded snippet"
sleep 0.5
open_target oe-reload
type_text oetest --enter-after-ms 800
check "saved combo used without a restart" "$(saved_line oe-reload)" "Reloaded snippet"

# Without a reset on focus change, "oe" here plus "test" there would expand.
open_target oe-focus-first
type_text oe
open_target oe-focus-second
type_text test --enter-after-ms 800
check "keyword split across windows not expanded" "$(saved_line oe-focus-second)" "test"

printf '{"excluded_apps":["oe-excluded"]}' >"$config_dir/settings.json"
sleep 0.5
open_target oe-excluded
type_text oetest --enter-after-ms 800
check "keyword left alone in an excluded app" "$(saved_line oe-excluded)" "oetest"

# Expansions must stay out of Omarchy's clipboard history. A plain copy that does
# appear there proves history was recording while the expansion ran.
history_file=$HOME/.local/state/omarchy/clipboard-history.json
in_history() {
  python3 -c 'import json, sys; sys.exit(0 if any(e.get("text") == sys.argv[2] for e in json.load(open(sys.argv[1]))) else 1)' "$history_file" "$1"
}
run_id=$RANDOM$RANDOM
printf 'history control %s' "$run_id" | wl-copy
write_combo oeprivate "private snippet $run_id"
rm "$config_dir/settings.json"
sleep 1
open_target oe-history
type_text oeprivate --enter-after-ms 800
check "private snippet expanded" "$(saved_line oe-history)" "private snippet $run_id"
sleep 1
check "a plain copy is recorded in clipboard history" "$(in_history "history control $run_id" && echo yes || echo no)" "yes"
check "the expanded snippet is not recorded in clipboard history" "$(in_history "private snippet $run_id" && echo yes || echo no)" "no"

if kill -0 "$daemon" 2>/dev/null; then
  echo "PASS: daemon still running"
else
  echo "FAIL: daemon exited"
  failures=$((failures + 1))
fi
exit "$failures"
