# omarchy-expand

**Beta · GitHub source installation only · MIT licensed**

A text expander built for [Omarchy](https://omarchy.org/). Turn short keywords
into the text you type repeatedly: email addresses, signatures, replies, links,
or multiline notes.

For example, save `@@mail` as a keyword for your email address. Type `@@mail`
in a text field and omarchy-expand replaces it with the saved text. You can also
choose a snippet from an Omarchy menu and paste it into the focused app.

This is an independent community project. It is in early beta and needs testing
across applications and keyboard layouts. Install from this repository if you
want to try it and help find bugs. There is currently no official AUR package,
package-repository release, or prebuilt binary.

## What it does

- Expands typed keywords into plain text, including Unicode and multiple lines.
- Provides a terminal UI for creating, editing, searching, and grouping snippets.
- Lets you enable or disable individual snippets or whole groups.
- Supports strict or loose matching and optional case-insensitive matching.
- Offers a searchable snippet picker through the Omarchy menu.
- Reloads saved snippets and settings without restarting the daemon.
- Supports app exclusions for automatic expansion.
- Runs locally as a systemd user service and starts with your graphical session.

## How it works

There are three parts:

1. **Setup UI:** `omarchy-expand` opens the snippet editor. The app launcher
   lists it as **Text Expander**.
2. **Background service:** `omarchy-expand daemon` reads keyboard events and
   translates them using the main keyboard layout reported by Hyprland. When
   a keyword matches, it waits for held keys to be released, erases the keyword
   with a virtual keyboard, and pastes the snippet through the Wayland clipboard.
3. **Picker:** `omarchy-expand pick` opens an Omarchy menu. Selecting a snippet
   asks the running daemon to paste it into the app underneath the menu.

The daemon uses Ctrl+V for ordinary apps and Ctrl+Shift+V for recognized terminals.
It attempts to restore the previous text clipboard after pasting and marks its
clipboard writes as sensitive to keep them out of Omarchy's clipboard history.

Snippet configuration stays on your machine. The app has no built-in cloud sync,
account system, or telemetry. Building from source downloads Rust dependencies.

## Install the beta

### Requirements

- Omarchy with its Hyprland session, systemd user services, and menu/TUI helpers.
- Rust and Cargo **1.98 or newer**, a C linker, `libxkbcommon`, and `wl-clipboard`.
- Permission to read input devices and create a virtual keyboard through `/dev/uinput`.

The initial smoke test passed on Omarchy 4.0.4, Hyprland 0.56.2, and a Danish
keyboard layout. This is a test baseline, not a guarantee for every app or layout.

Install build and runtime dependencies from a terminal:

```bash
omarchy pkg add git rust gcc wl-clipboard libxkbcommon
```

### 1. Get the source

```bash
git clone --branch v0.1.0-beta.1 --depth 1 https://github.com/Razumasu/omarchy-expand.git
cd omarchy-expand
```

Use a tagged beta from [Releases](https://github.com/Razumasu/omarchy-expand/releases)
for a reproducible test. The `main` branch may contain changes after the tagged beta.

### 2. Set up keyboard access

The app reads keyboard events to recognize keywords and uses a virtual keyboard
to insert expansions. The `input` group provides this access. Membership also
allows other programs running as your account to read input events; this is a
meaningful system permission.

Run these once on a normal Omarchy installation:

```bash
sudo usermod -aG input "$USER"
printf '%s\n' 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' \
  | sudo tee /etc/udev/rules.d/60-omarchy-expand-uinput.rules
```

**Reboot before continuing** so the device rule and your systemd user manager both
have the new permissions. Save any open work first. If that rule file already
exists with custom content, review it before replacing it.

### 3. Build and install

Return to the cloned `omarchy-expand` directory in your graphical session:

```bash
./install.sh
systemctl --user status omarchy-expand --no-pager
```

Run the installer as your regular user. It builds the binary, installs it to
`~/.local/bin/omarchy-expand`, enables and starts the user service, and adds
**Text Expander** to your app launcher. Snippets are added separately; a fresh
installation starts with an empty library.

## Try your first expansion

1. Open **Text Expander** from the app launcher, or run `~/.local/bin/omarchy-expand`.
2. Press **a** to add a combo.
3. Set the keyword to `oetest`, the name to `Test`, and the snippet to
   `Text expansion works — æøå!`. Use **Tab** to move between fields.
4. Press **Ctrl+S** to save, then **q** to close the editor.
5. Open a blank text editor or a browser text field and type `oetest` by hand.

It should be replaced immediately with `Text expansion works — æøå!`. You do not
need to type a trailing space or press Enter. Test in a scratch document first.

The first saved combo creates a **General** group automatically. Further useful
checks are multiline text, typing in a terminal, and saving an edited snippet
while the daemon is running. Please report the app and layout if a check fails.

### Matching and editor controls

**Strict** matching, the default, requires the whole current typed run to equal
the keyword. **Loose** matching allows the keyword at the end of a longer run.
Whitespace, navigation keys, shortcuts, mouse clicks, and window focus changes
break the current run. Keywords cannot contain whitespace.

| Key | Action |
| --- | --- |
| Tab / left / right | Switch between groups and combos |
| Up / down | Move through the selected list |
| a | Add a group or combo in the selected pane |
| e / Enter | Edit the selected combo; `e` renames a group |
| Space | Enable or disable the selected group or combo |
| d | Delete, with confirmation |
| / | Search combos |
| s | Edit app exclusions |
| Ctrl+S | Save a form |
| Esc | Cancel a form or search; close the editor when browsing normally |
| q | Close the editor when browsing |

### Optional picker shortcut

The installer prints a suggested binding. To use **Super+Alt+E**, first check
that the shortcut is available with `omarchy menu keybindings --print`. Add the
following to `~/.config/hypr/bindings.lua`, replacing `/home/YOUR_USER` with your
actual home directory:

```lua
o.bind("SUPER + ALT + E", "Text expander picker", "/home/YOUR_USER/.local/bin/omarchy-expand pick")
```

Apply and check the configuration:

```bash
hyprctl reload
hyprctl configerrors
```

Focus a scratch text field, press the shortcut, search for a combo, and press
Enter. The picker pastes the snippet without requiring its keyword to be typed.

## Configuration and beta limitations

The editor saves `combos.json` and optional `settings.json` in
`~/.config/omarchy-expand/` (or `$XDG_CONFIG_HOME/omarchy-expand/` when set).
These are plain-text files. Back them up before testing updates and avoid storing
passwords or other secrets as snippets.

Current limitations to be aware of:

- Clipboard restoration preserves **text only**. An image or other non-text
  clipboard content is not restored after an expansion.
- Pasting depends on the focused app accepting its expected paste shortcut.
  Some applications or slow clipboard consumers may behave differently.
- The keymap is read when the daemon starts. Restart the service after changing
  layouts. IME/composed-text input and unusual keyboard setups need more testing.
- The matching buffer is limited to 128 characters. Keep keywords short.
- The default exclusions cover 1Password, Bitwarden, and KeePassXC by app class
  or process name. The daemon does not identify password fields inside browsers
  or other apps. Exclusions are editable in the setup UI.
- Explicit picker selections bypass app exclusions. Automatic expansions check
  Hyprland's session-lock state; do not treat this beta as a security boundary.

## Troubleshooting, updates, and removal

Check the service and recent diagnostics:

```bash
systemctl --user status omarchy-expand --no-pager
journalctl --user -u omarchy-expand -n 50 --no-pager
systemctl --user restart omarchy-expand
```

If the installer reports missing permissions, check `id -nG` and
`ls -l /dev/uinput`. Joining the group in a new terminal alone does not update
an already-running systemd user manager; reboot and try again.

For a later beta, back up `~/.config/omarchy-expand/`, fetch its tag in your clone,
and install it:

```bash
git fetch --tags origin
git checkout <new-beta-tag>
./install.sh
```

To remove the app, run this from your clone:

```bash
./install.sh uninstall
```

Removal keeps your snippet configuration. Remove an optional picker keybinding
separately. The installer also leaves the `input` group membership and uinput
rule in place; review whether other software needs them before removing those
permissions.

## Bugs, ideas, and contributions

Use [Issues](https://github.com/Razumasu/omarchy-expand/issues) for bugs and feature
requests. Include your beta tag or commit, Omarchy/Hyprland versions, keyboard
layout, affected app, and clear steps to reproduce. Review logs for personal
information before posting them.

Pull requests are welcome. The project is maintained by **@Razumasu**, who decides
which changes are merged and released. Discuss substantial changes in an issue
first. See [CONTRIBUTING.md](CONTRIBUTING.md) for development and testing guidance.

Distribution during this beta is through GitHub source checkouts only. Please
discuss packaging proposals before submitting packaging work to this project.
This is the project's release policy; the MIT license permits third-party
redistribution.

## License

[MIT](LICENSE). Copyright (c) 2026 Razumasu.

You may use, modify, share, and sell the software under the license terms, including
preserving its copyright and license notices. Third-party dependencies retain
their own licenses.
