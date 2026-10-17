mod config;
mod devices;
mod expander;
mod hyprland;
mod injector;
mod ipc;
mod keymap;
mod pick;
mod tui;
mod typed;

use std::env;
use std::error::Error;
use std::io;
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use evdev::KeyCode;

use config::Config;
use expander::{Expander, Expansion};
use hyprland::ActiveWindow;
use injector::Injector;
use keymap::Keyboard;

/// The Omarchy menu's layer; the picker's paste waits for it to close.
const MENU_LAYER: &str = "omarchy-menu";

/// How long a daemon started at login waits for Hyprland's socket to answer.
const HYPRLAND_STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// Everything the daemon reacts to, from all listener threads.
#[derive(Debug)]
pub enum Event {
    /// value is 1 for press, 2 for autorepeat, 0 for release.
    Key {
        device: usize,
        code: u16,
        value: i32,
    },
    /// The device was unplugged; any keys it still had down will never be released.
    DeviceLost {
        device: usize,
    },
    Click,
    FocusChanged,
    /// A keyboard switched to another of its configured layouts.
    LayoutChanged,
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

    hyprland::wait_until_reachable(HYPRLAND_STARTUP_TIMEOUT)?;
    let mut names = hyprland::main_keyboard_names()?;
    let mut keyboard = Keyboard::new(&names)?;
    let mut injector = Injector::new(paste_key(&keyboard))?;
    let config_dir = config::config_dir()?;

    // Listen before loading so a change made in between is not missed.
    config::watch(&config_dir, sender.clone())?;
    hyprland::watch_focus(sender.clone())?;
    devices::watch(sender)?;

    let mut config = load_config(&config_dir).unwrap_or_default();
    let mut focus_excluded = focused_window_excluded(&config);
    let mut expander = Expander::default();

    for event in events {
        match event {
            Event::Key {
                device,
                code,
                value,
            } => {
                let due = expander.key(
                    &mut keyboard,
                    &config.combos,
                    focus_excluded,
                    device,
                    code,
                    value,
                );
                if let Some(expansion) = due {
                    expand(&mut injector, &expansion, &config);
                }
            }
            Event::DeviceLost { device } => expander.device_lost(&mut keyboard, device),
            Event::Click => expander.reset(),
            Event::FocusChanged => {
                expander.reset();
                focus_excluded = focused_window_excluded(&config);
            }
            Event::LayoutChanged => {
                expander.reset();
                match hyprland::main_keyboard_names() {
                    Ok(changed) if changed != names => match Keyboard::new(&changed) {
                        Ok(mut rebuilt) => {
                            for code in expander.held_codes() {
                                rebuilt.update_key(code, true);
                            }
                            keyboard = rebuilt;
                            names = changed;
                            eprintln!(
                                "omarchy-expand: following keyboard layout {} of {:?}",
                                names.active_layout + 1,
                                names.layout
                            );
                        }
                        Err(error) => {
                            eprintln!("omarchy-expand: keeping the previous keymap: {error}");
                        }
                    },
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("omarchy-expand: cannot read the new keyboard layout: {error}");
                    }
                }
            }
            Event::ConfigChanged => {
                if let Some(reloaded) = load_config(&config_dir) {
                    config = reloaded;
                    focus_excluded = focused_window_excluded(&config);
                }
                expander.reset();
            }
            Event::HyprlandLost => return Err("lost the connection to Hyprland".into()),
            Event::Paste { keyword, reply } => {
                let result = paste_picked(&mut injector, &config, &keyword);
                if let Err(error) = &result {
                    eprintln!("omarchy-expand: could not paste {keyword:?}: {error}");
                }
                // The picker may have given up waiting; the paste happened either way.
                reply.send(result).ok();
                expander.reset();
            }
        }
    }
    Err("the input listener stopped".into())
}

/// The key that types "v" on the layout Hyprland gives the virtual keyboard. That is the
/// global input layout, not a per-device override on the main keyboard, so it is read
/// separately; the main keyboard's layout is the fallback.
fn paste_key(main_keyboard: &Keyboard) -> u16 {
    let global = hyprland::global_keyboard_names()
        .and_then(|names| Keyboard::new(&names).map_err(io::Error::other));
    let key = match global {
        Ok(keyboard) => keyboard.paste_key(),
        Err(error) => {
            eprintln!(
                "omarchy-expand: cannot read the global keyboard layout, using the main keyboard's: {error}"
            );
            main_keyboard.paste_key()
        }
    };
    key.unwrap_or(KeyCode::KEY_V.0)
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
        Ok(window) => window_excluded(config, &window),
        Err(error) => {
            eprintln!("omarchy-expand: cannot read the focused window: {error}");
            true
        }
    }
}

/// A focused layer surface (the launcher, a menu, the lock screen) has no window class,
/// so it could never be excluded by name; nothing is recorded or expanded there.
fn window_excluded(config: &Config, window: &ActiveWindow) -> bool {
    window.class.is_empty() || config.excludes(&window.class, window.process_name().as_deref())
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

fn expand(injector: &mut Injector, expansion: &Expansion, config: &Config) {
    let window = match hyprland::active_window() {
        Ok(window) => window,
        Err(error) => {
            eprintln!("omarchy-expand: not expanding, cannot read the focused window: {error}");
            return;
        }
    };
    if window_excluded(config, &window) {
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
    let into_terminal = injector::is_terminal(&window.class, window.process_name().as_deref());
    if let Err(error) = injector.expand(expansion.erase, &expansion.text, into_terminal) {
        eprintln!("omarchy-expand: expansion failed: {error}");
    }
}
