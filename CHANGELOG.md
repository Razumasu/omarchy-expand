# Changelog

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
