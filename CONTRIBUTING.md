# Contributing to omarchy-expand

Bug reports, feature suggestions, documentation improvements, and pull requests
are welcome. This is an early beta for Omarchy, currently tested through GitHub
source installations. There is no official AUR package or binary distribution.

## Project direction

@Razumasu maintains the official repository and has final say over project
direction, merged changes, and releases. Please open an issue before starting a
large feature, changing the permissions model, or proposing distribution packages.
Small fixes can go straight to a pull request.

## Reporting a bug or proposing a feature

Search existing issues first, then use the relevant issue template. For bugs,
include the beta tag or commit, Omarchy and Hyprland versions, keyboard layout,
affected app, reproduction steps, and expected versus actual behavior.

Use harmless example snippets. Review logs and screenshots before posting them;
remove personal information, secrets, and unrelated window contents. Never attach
raw keyboard-event recordings.

For features, describe the problem you need to solve and how you expect the feature
to behave. Explain how it fits a small, local text expander for Omarchy.

## Development

Fork the repository and create a branch for your change. Rust/Cargo 1.98 or newer
and a C linker are needed to build; see the README for runtime dependencies.

```bash
cargo build --locked
cargo test --locked
git diff --check
```

The unit tests do not need a live Hyprland session or keyboard-device access.
Keep dependency changes intentional and include `Cargo.lock` when they change.
Apply formatting consistently with nearby code and keep PRs focused.

Changes to input handling, matching, clipboard behavior, or the picker also need
a manual test inside Omarchy. Describe the apps and layouts you tested and any
checks you could not run. Check that the service stays running and that edits to
snippets reload as expected.

The `test/guest-*.sh` scripts are intended for a **disposable Omarchy test session**.
They create windows, type through a virtual keyboard, modify the clipboard, and
can install or uninstall the app. Read each script before running it; do not run
the whole suite against your everyday session. Its screenshot commands also need
`grim`, and UI-driving scripts need `wtype` and `foot`.

## Pull requests

Explain the problem, the resulting behavior, and how you verified the change.
Link the relevant issue when there is one. Update documentation when behavior or
installation changes. Add meaningful tests for logic changes where appropriate.

Submitting a contribution means licensing it under the project's MIT license.
Contributors retain copyright in their contributions. No copyright assignment is
required, and submitting a PR does not grant write access to the official repo.
