//! Hyprland IPC over its sockets, read directly rather than by spawning hyprctl.

use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::Event;
use crate::keymap::KeymapNames;

/// The keymap of the keyboard Hyprland marks as main, including per-device overrides.
pub fn main_keyboard_names() -> io::Result<KeymapNames> {
    let devices = request_json("devices")?;
    let keyboards = devices["keyboards"].as_array().cloned().unwrap_or_default();
    let main = keyboards
        .iter()
        .find(|keyboard| keyboard["main"].as_bool() == Some(true))
        .or_else(|| keyboards.first())
        .ok_or_else(|| io::Error::other("Hyprland reports no keyboards"))?;
    let text = |field: &str| main[field].as_str().unwrap_or_default().to_owned();
    Ok(KeymapNames {
        rules: text("rules"),
        model: text("model"),
        layout: text("layout"),
        variant: text("variant"),
        options: text("options"),
        active_layout: main["active_layout_index"].as_u64().unwrap_or(0) as u32,
    })
}

#[derive(Debug, Default)]
pub struct ActiveWindow {
    pub class: String,
    pub pid: Option<u32>,
}

impl ActiveWindow {
    /// The program behind the window, which stays "foot" even when an app sets its own class.
    pub fn process_name(&self) -> Option<String> {
        let comm = std::fs::read_to_string(format!("/proc/{}/comm", self.pid?)).ok()?;
        Some(comm.trim_end().to_owned())
    }
}

/// The focused window, or an empty one when nothing has focus.
pub fn active_window() -> io::Result<ActiveWindow> {
    let window = request_json("activewindow")?;
    Ok(ActiveWindow {
        class: window["class"].as_str().unwrap_or_default().to_owned(),
        pid: window["pid"].as_u64().map(|pid| pid as u32),
    })
}

/// Hyprland has no lock flag, but a held session lock shows up as LOCK in a monitor's
/// solitaryBlockedBy. omarchy-hyprland-session-locked relies on the same signal.
pub fn session_locked() -> io::Result<bool> {
    Ok(monitors_show_lock(&request_json("monitors")?))
}

fn monitors_show_lock(monitors: &Value) -> bool {
    monitors.as_array().is_some_and(|monitors| {
        monitors.iter().any(|monitor| {
            monitor["solitaryBlockedBy"]
                .as_array()
                .is_some_and(|reasons| reasons.iter().any(|reason| reason == "LOCK"))
        })
    })
}

/// Waits until no layer surface with this namespace is mapped, so keyboard focus is back
/// with the window underneath. Bounded, and only used right after a pick.
pub fn wait_for_layer_to_close(namespace: &str, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    while has_layer(&request_json("layers")?, namespace) {
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!("{namespace} did not close")));
        }
        thread::sleep(Duration::from_millis(15));
    }
    Ok(())
}

/// Layers are nested by monitor and level; any object naming the namespace counts.
fn has_layer(value: &Value, namespace: &str) -> bool {
    match value {
        Value::Object(fields) => {
            fields.get("namespace").and_then(Value::as_str) == Some(namespace)
                || fields.values().any(|field| has_layer(field, namespace))
        }
        Value::Array(items) => items.iter().any(|item| has_layer(item, namespace)),
        _ => false,
    }
}

/// Sends FocusChanged whenever another window gets focus, and HyprlandLost if the
/// event stream ends.
pub fn watch_focus(events: Sender<Event>) -> io::Result<()> {
    let stream = UnixStream::connect(socket_dir()?.join(".socket2.sock"))?;
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        // Bytes, not lines of text: a window title is not guaranteed to be valid UTF-8.
        while reader
            .read_until(b'\n', &mut line)
            .is_ok_and(|read| read > 0)
        {
            if line.starts_with(b"activewindowv2>>") && events.send(Event::FocusChanged).is_err() {
                return;
            }
            line.clear();
        }
        // Fails only if the daemon is already shutting down.
        events.send(Event::HyprlandLost).ok();
    });
    Ok(())
}

fn request_json(command: &str) -> io::Result<Value> {
    let mut stream = UnixStream::connect(socket_dir()?.join(".socket.sock"))?;
    stream.write_all(format!("j/{command}").as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    serde_json::from_str(&response).map_err(|error| {
        io::Error::other(format!("Hyprland sent invalid JSON for {command}: {error}"))
    })
}

fn socket_dir() -> io::Result<PathBuf> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR");
    let instance = env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
    match (runtime_dir, instance) {
        (Some(runtime_dir), Some(instance)) => {
            Ok(PathBuf::from(runtime_dir).join("hypr").join(instance))
        }
        _ => Err(io::Error::other(
            "not inside a Hyprland session (XDG_RUNTIME_DIR or HYPRLAND_INSTANCE_SIGNATURE is unset)",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_are_found_at_any_level() {
        let layers = serde_json::json!({
            "DP-1": { "levels": {
                "2": [{ "namespace": "omarchy-bar" }],
                "3": [{ "namespace": "omarchy-menu" }]
            } }
        });
        assert!(has_layer(&layers, "omarchy-menu"));
        assert!(!has_layer(&layers, "omarchy-emojis"));
    }

    #[test]
    fn a_lock_on_any_monitor_means_locked() {
        let locked = serde_json::json!([
            { "name": "DP-1", "solitaryBlockedBy": ["WINDOWS"] },
            { "name": "DP-2", "solitaryBlockedBy": ["LOCK", "WINDOWS"] }
        ]);
        let unlocked = serde_json::json!([{ "name": "DP-1", "solitaryBlockedBy": ["WINDOWS"] }]);
        assert!(monitors_show_lock(&locked));
        assert!(!monitors_show_lock(&unlocked));
        assert!(!monitors_show_lock(
            &serde_json::json!([{ "name": "DP-1" }])
        ));
    }
}
