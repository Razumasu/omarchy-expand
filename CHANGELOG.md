# Changelog

## Unreleased

Fixes from the first code review of the beta.

- A key pressed before the keyword's last key was released, or an autorepeat of
  that key, no longer corrupts the expansion: the extra text is erased and
  re-added after the snippet. The daemon's typing logic now lives in a tested
  state machine.
- New per-combo option **Expand after a delimiter**: the keyword is replaced only
  once a space or punctuation mark follows it, so `sig` can be typed inside
  `signal`. The default, expanding immediately, is unchanged.
- Switching between configured keyboard layouts is followed live.
- The paste shortcut is taken from Hyprland's global keyboard layout, which is
  what the virtual keyboard gets, instead of a per-device override.
- Nothing is recorded or expanded while no window has focus (launcher, menu,
  lock screen) or the focused window reports no class.
- If Hyprland stops reporting the session-lock state, expansions stop instead
  of continuing.
- Ctrl+Backspace and other modified Backspace keys end the typed run instead of
  removing one character from it.
- A clipboard owner that never answers no longer stalls the daemon; reading the
  clipboard gives up after one second.
- Keys held on a keyboard that is unplugged are released, so expansions do not
  silently stop afterwards.
- The daemon waits up to 15 seconds for Hyprland at login, and the service
  allows more restart attempts before giving up.
- `combos.json` and `settings.json` are created readable by the user only, are
  written through a per-process temporary file, and carry a `version` field; a
  file from a newer release is refused with a clear message.
- Combo and group names must fit on one line, so the picker's menu cannot be split.
- The picker gives up after ten seconds if the daemon does not answer.
- The setup UI reports the daemon's state from its socket, restores the terminal
  before reporting an error, and refuses whitespace in the keyword field.
- Bitwarden's Flatpak app id is excluded by default.
- The installer refuses to run as root, explains a missing `/dev/uinput`, and no
  longer depends on `$USER`.
- The minimum Rust version is 1.88, which the code actually needs, and a GitHub
  Actions workflow runs formatting, clippy, the tests, shellcheck, and an MSRV
  build.

## 0.1.0-beta.1 — 2026-10-17

First public beta for testing from GitHub source checkouts.

- Expand typed keywords into Unicode and multiline plain text.
- Manage snippets and groups in a terminal UI.
- Choose snippets through an Omarchy menu picker.
- Configure matching, enable/disable groups or combos, and exclude apps.
- Reload saved configuration while the daemon is running.
- Install a systemd user service and app-launcher entry with `install.sh`.
- Publish under the MIT license with installation, testing, and contribution guides.

The initial validation includes 38 passing unit tests and a live expansion test
on Omarchy 4.0.4 / Hyprland 0.56.2 with a Danish keyboard layout. Other apps and
layouts need more testing. This beta has no official AUR package or prebuilt binaries.

See the README for required keyboard permissions and beta limitations, including
text-only clipboard restoration and the need to restart after layout changes.
