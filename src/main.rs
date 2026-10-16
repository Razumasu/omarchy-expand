mod config;
mod devices;
mod hyprland;
mod injector;
mod ipc;
mod keymap;
mod pick;
mod tui;
mod typed;

use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use evdev::KeyCode;

use config::Config;
use injector::Injector;
use keymap::Keyboard;
use typed::{Combo, TypedText};

/// The Omarchy menu's layer; the picker's paste waits for it to close.
const MENU_LAYER: &str = "omarchy-menu";

/// Everything the daemon reacts to, from all listener threads.
#[derive(Debug)]
pub enum Event {
    /// value is 1 for press, 2 for autorepeat, 0 for release.
    Key {
        code: u16,
        value: i32,
    },
    Click,
    FocusChanged,
    ConfigChanged,
    HyprlandLost,
    /// The picker chose a combo; the result goes back to it.
    Paste {
        keyword: String,
        reply: Sender<Result<(), String>>,
    },
}

fn main() -> ExitCode {
    let result = match env::args().nth(1).as_deref() {
        None => tui::run(),
        Some("daemon") => run_daemon(),
        Some("pick") => pick::run(),
        _ => {
            eprintln!("usage: omarchy-expand          open the setup UI");
            eprintln!(
                "       omarchy-expand pick     choose a combo from the Omarchy menu and paste it"
            );
            eprintln!("       omarchy-expand daemon   expand combos (run by the systemd service)");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("omarchy-expand: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_daemon() -> Result<(), Box<dyn Error>> {
    // First, so a second daemon stops before it creates a second virtual keyboard.
    let (sender, events) = mpsc::channel();
    ipc::listen(sender.clone())?;

    let mut keyboard = Keyboard::new(&hyprland::main_keyboard_names()?)?;
    let mut injector = Injector::new(keyboard.paste_key().unwrap_or(KeyCode::KEY_V.0))?;
    let config_dir = config::config_dir()?;

    // Listen before loading so a change made in between is not missed.
    config::watch(&config_dir, sender.clone())?;
    hyprland::watch_focus(sender.clone())?;
    devices::watch(sender)?;

    let mut config = load_config(&config_dir).unwrap_or_default();
    let mut focus_excluded = focused_window_excluded(&config);
    let mut typed = TypedText::default();
    let mut held_keys = HashSet::new();
    let mut pending: Option<Combo> = None;

    for event in events {
        match event {
            Event::Key { code, value } => {
                if value != 0 && pending.is_none() && !focus_excluded {
                    record_keystroke(&keyboard, &mut typed, code);
                    pending = typed.find_match(&config.combos).cloned();
                }
                match value {
                    1 => {
                        held_keys.insert(code);
                        keyboard.update_key(code, true);
                    }
                    0 => {
                        held_keys.remove(&code);
                        keyboard.update_key(code, false);
                    }
                    _ => {}
                }
                // A held Shift would turn our Ctrl+V into Ctrl+Shift+V, so wait for a clean keyboard.
                if held_keys.is_empty()
                    && let Some(combo) = pending.take()
                {
                    expand(&mut injector, &combo, &config);
                    typed.clear();
                }
            }
            Event::Click => typed.clear(),
            Event::FocusChanged => {
                typed.clear();
                pending = None;
                focus_excluded = focused_window_excluded(&config);
            }
            Event::ConfigChanged => {
                if let Some(reloaded) = load_config(&config_dir) {
                    config = reloaded;
                    focus_excluded = focused_window_excluded(&config);
                }
                typed.clear();
                pending = None;
            }
            Event::HyprlandLost => return Err("lost the connection to Hyprland".into()),
            Event::Paste { keyword, reply } => {
                let result = paste_picked(&mut injector, &config, &keyword);
                if let Err(error) = &result {
                    eprintln!("omarchy-expand: could not paste {keyword:?}: {error}");
                }
                // The picker may have given up waiting; the paste happened either way.
                reply.send(result).ok();
                typed.clear();
            }
        }
    }
    Err("the input listener stopped".into())
}

/// An invalid file is reported and skipped, so a typo never stops a running daemon.
fn load_config(dir: &Path) -> Option<Config> {
    match config::load(dir) {
        Ok(config) => {
            eprintln!("omarchy-expand: loaded {} combos", config.combos.len());
            Some(config)
        }
        Err(error) => {
            eprintln!("omarchy-expand: ignoring invalid config: {error}");
            None
        }
    }
}

/// Fails closed: if Hyprland cannot say what has focus, nothing is recorded.
fn focused_window_excluded(config: &Config) -> bool {
    match hyprland::active_window() {
        Ok(window) => config.excludes(&window.class, window.process_name().as_deref()),
        Err(error) => {
            eprintln!("omarchy-expand: cannot read the focused window: {error}");
            true
        }
    }
}

/// A combo chosen in the picker. It was picked on purpose, so excluded apps do not apply.
fn paste_picked(injector: &mut Injector, config: &Config, keyword: &str) -> Result<(), String> {
    let combo = config
        .combos
        .iter()
        .find(|combo| combo.keyword == keyword)
        .ok_or_else(|| format!("no active combo has the keyword {keyword:?}"))?;
    hyprland::wait_for_layer_to_close(MENU_LAYER, Duration::from_secs(2))
        .map_err(|error| error.to_string())?;
    let window = hyprland::active_window().map_err(|error| error.to_string())?;
    let into_terminal = injector::is_terminal(&window.class, window.process_name().as_deref());
    injector
        .expand(0, &combo.snippet, into_terminal)
        .map_err(|error| error.to_string())
}

fn expand(injector: &mut Injector, combo: &Combo, config: &Config) {
    let window = match hyprland::active_window() {
        Ok(window) => window,
        Err(error) => {
            eprintln!("omarchy-expand: not expanding, cannot read the focused window: {error}");
            return;
        }
    };
    let process_name = window.process_name();
    if config.excludes(&window.class, process_name.as_deref()) {
        return;
    }
    match hyprland::session_locked() {
        Ok(false) => {}
        Ok(true) => return,
        Err(error) => {
            eprintln!("omarchy-expand: not expanding, cannot read the lock state: {error}");
            return;
        }
    }
    let into_terminal = injector::is_terminal(&window.class, process_name.as_deref());
    let erase = combo.keyword.chars().count();
    if let Err(error) = injector.expand(erase, &combo.snippet, into_terminal) {
        eprintln!("omarchy-expand: expansion failed: {error}");
    }
}

/// Printable characters extend the typed run; anything else (space, Enter, arrows,
/// shortcuts) breaks it, as in Beeftext.
fn record_keystroke(keyboard: &Keyboard, typed: &mut TypedText, code: u16) {
    if code == KeyCode::KEY_BACKSPACE.0 {
        typed.backspace();
        return;
    }
    if keyboard.shortcut_modifier_active() {
        typed.clear();
        return;
    }
    let keysym = keyboard.keysym(code);
    if keymap::is_modifier(keysym) || keymap::is_dead_key(keysym) {
        return;
    }
    let text = keyboard.text(code);
    if !text.is_empty() && !text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        typed.push(&text);
    } else {
        typed.clear();
    }
}
