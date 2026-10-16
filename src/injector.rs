//! Replaces a typed keyword with its snippet: erase with backspaces, paste through the
//! clipboard (instant for long text, any Unicode), then give the user their clipboard back.

use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};

pub const VIRTUAL_DEVICE_NAME: &str = "omarchy-expand virtual keyboard";

/// How long the focused app gets to read the pasted snippet before the clipboard is restored.
const PASTE_SETTLE_TIME: Duration = Duration::from_millis(250);

/// Terminals paste with Ctrl+Shift+V because Ctrl+V is a control character to them.
/// Matched by class, or by process for terminals running an app under its own class.
const TERMINAL_CLASSES: &[&str] = &[
    "foot",
    "footclient",
    "Alacritty",
    "com.mitchellh.ghostty",
    "kitty",
    "org.wezfurlong.wezterm",
];
const TERMINAL_PROCESSES: &[&str] = &["foot", "alacritty", "ghostty", "kitty", "wezterm-gui"];

pub fn is_terminal(class: &str, process_name: Option<&str>) -> bool {
    TERMINAL_CLASSES.contains(&class)
        || process_name.is_some_and(|name| TERMINAL_PROCESSES.contains(&name))
}

pub struct Injector {
    device: VirtualDevice,
    paste_key: KeyCode,
}

impl Injector {
    pub fn new(paste_key: u16) -> io::Result<Self> {
        let paste_key = KeyCode(paste_key);
        let mut keys = AttributeSet::<KeyCode>::new();
        for key in [
            KeyCode::KEY_BACKSPACE,
            KeyCode::KEY_LEFTCTRL,
            KeyCode::KEY_LEFTSHIFT,
            paste_key,
        ] {
            keys.insert(key);
        }
        let device = VirtualDevice::builder()?
            .name(VIRTUAL_DEVICE_NAME)
            .with_keys(&keys)?
            .build()?;
        Ok(Self { device, paste_key })
    }

    pub fn expand(&mut self, erase: usize, snippet: &str, into_terminal: bool) -> io::Result<()> {
        let previous_clipboard = read_clipboard();
        write_clipboard(snippet)?;

        for _ in 0..erase {
            self.tap(&[KeyCode::KEY_BACKSPACE])?;
        }
        if into_terminal {
            self.tap(&[
                KeyCode::KEY_LEFTCTRL,
                KeyCode::KEY_LEFTSHIFT,
                self.paste_key,
            ])?;
        } else {
            self.tap(&[KeyCode::KEY_LEFTCTRL, self.paste_key])?;
        }

        thread::sleep(PASTE_SETTLE_TIME);
        match previous_clipboard {
            Some(text) => write_clipboard(&text),
            None => clear_clipboard(),
        }
    }

    /// Presses the keys in order, then releases them in reverse.
    fn tap(&mut self, keys: &[KeyCode]) -> io::Result<()> {
        for key in keys {
            self.device.emit(&[key_event(*key, 1)])?;
        }
        for key in keys.iter().rev() {
            self.device.emit(&[key_event(*key, 0)])?;
        }
        Ok(())
    }
}

fn key_event(key: KeyCode, value: i32) -> InputEvent {
    InputEvent::new(EventType::KEY.0, key.0, value)
}

/// Only text is preserved for now; an image on the clipboard is not restored.
fn read_clipboard() -> Option<String> {
    let output = Command::new("wl-paste")
        .args(["--no-newline", "--type", "text"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn write_clipboard(text: &str) -> io::Result<()> {
    // Sensitive keeps snippets and restores out of Omarchy's clipboard history.
    let mut child = Command::new("wl-copy")
        .arg("--sensitive")
        .stdin(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(text.as_bytes())?;
    check_status("wl-copy", child.wait()?)
}

fn clear_clipboard() -> io::Result<()> {
    check_status(
        "wl-copy --clear",
        Command::new("wl-copy").arg("--clear").status()?,
    )
}

fn check_status(command: &str, status: std::process::ExitStatus) -> io::Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{command} failed with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_are_recognised_by_class_or_process() {
        assert!(is_terminal("foot", None));
        assert!(is_terminal("org.omarchy.wiremix", Some("foot")));
        assert!(!is_terminal("chromium", Some("chromium")));
        assert!(!is_terminal("", None));
    }
}
