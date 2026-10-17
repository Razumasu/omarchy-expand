# shellcheck shell=bash
# Shared helpers for the guest end-to-end tests; source from a test script.
# Targets are foot terminals that save the first line typed into them.

eval "$(systemctl --user show-environment |
  grep -E '^(WAYLAND_DISPLAY|HYPRLAND_INSTANCE_SIGNATURE|XDG_RUNTIME_DIR|OMARCHY_PATH)=' | sed 's/^/export /')"
# Omarchy's own commands (the menu, notifications) as the desktop session sees them.
PATH="$PATH:$OMARCHY_PATH/bin"

# shellcheck disable=SC2034 # used by the scripts that source this file
artifacts=${OMARCHY_LAB_ARTIFACTS:-$(mktemp -d)}
results_dir=$(mktemp -d)
targets=()
failures=0

wait_for() {
  local description=$1
  shift
  for _ in $(seq 50); do
    if "$@"; then
      return 0
    fi
    sleep 0.2
  done
  echo "FAIL: timed out waiting for $description"
  exit 1
}

focused() {
  hyprctl -j activewindow | grep -qE "\"class\": *\"$1\""
}

# Hyprland 0.56 dispatchers are Lua; [[...]] is a Lua string that needs no escaping.
open_target() {
  local class=$1
  hyprctl dispatch "hl.dsp.exec_cmd([[foot --app-id $class sh -c 'IFS= read -r line; printf %s \"\$line\" > $results_dir/$class']])" >/dev/null
  targets+=("$class")
  wait_for "$class to get focus" focused "$class"
}

close_targets() {
  local class
  for class in "${targets[@]}"; do
    hyprctl dispatch "hl.dsp.window.close({ window = \"class:^$class\$\" })" >/dev/null 2>&1 || true
  done
  rm -rf "$results_dir"
}

type_text() {
  target/release/examples/typist "$@"
}

saved_line() {
  wait_for "$1 to save its line" test -s "$results_dir/$1"
  cat "$results_dir/$1"
}

check() {
  if [[ $2 == "$3" ]]; then
    echo "PASS: $1"
  else
    echo "FAIL: $1: expected '$3', got '$2'"
    failures=$((failures + 1))
  fi
}
