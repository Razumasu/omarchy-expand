#!/bin/bash
# Installs omarchy-expand as a systemd user service with install.sh, checks that the
# service expands a combo from ~/.config/omarchy-expand, then uninstalls it.
set -euo pipefail
cd "$(dirname "$0")/.."
source test/lib.sh

config_dir=$HOME/.config/omarchy-expand
if [[ -e $config_dir/combos.json ]]; then
  echo "FAIL: refusing to overwrite the existing $config_dir/combos.json"
  exit 1
fi

cleanup() {
  journalctl --user -u omarchy-expand --since "-5min" --no-pager >"$artifacts/service-journal.log" 2>&1 || true
  ./install.sh uninstall >/dev/null
  rm -f "$config_dir/combos.json"
  close_targets
}
trap cleanup EXIT

./install.sh
cargo build --release --quiet --example typist
wait_for "the service to be active" systemctl --user is-active --quiet omarchy-expand

mkdir -p "$config_dir"
printf '{"groups":[{"name":"Test","combos":[{"keyword":"oeservice","snippet":"Expanded by the service"}]}]}' >"$config_dir/combos.json"
sleep 0.5

open_target oe-service
type_text oeservice --enter-after-ms 800
check "service expands a combo" "$(saved_line oe-service)" "Expanded by the service"
check "service still active" "$(systemctl --user is-active omarchy-expand)" "active"
desktop_file=$HOME/.local/share/applications/omarchy-expand.desktop
check "launcher entry opens the installed binary" "$(grep '^Exec=' "$desktop_file")" "Exec=omarchy-launch-tui --app-id=TUI.float $HOME/.local/bin/omarchy-expand"
exit "$failures"
