//! Listens to every keyboard and mouse under /dev/input, including ones plugged in later.
//! Devices are read, never grabbed, so typing reaches applications untouched.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use evdev::{Device, EventSummary, KeyCode};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};

use crate::Event;
use crate::injector::VIRTUAL_DEVICE_NAME;

const INPUT_DIR: &str = "/dev/input";

type OpenDevices = Arc<Mutex<HashSet<PathBuf>>>;

pub fn watch(events: Sender<Event>) -> io::Result<()> {
    let open_devices = OpenDevices::default();

    // Watch before scanning so a device appearing in between is not missed.
    // New nodes are often unreadable until udev sets their group, hence IN_ATTRIB.
    let inotify = Inotify::init(InitFlags::empty())?;
    inotify.add_watch(
        INPUT_DIR,
        AddWatchFlags::IN_CREATE | AddWatchFlags::IN_ATTRIB,
    )?;

    for entry in std::fs::read_dir(INPUT_DIR)? {
        try_open(&entry?.path(), &open_devices, &events);
    }

    thread::spawn(move || {
        while let Ok(changes) = inotify.read_events() {
            for name in changes.into_iter().filter_map(|change| change.name) {
                try_open(&Path::new(INPUT_DIR).join(name), &open_devices, &events);
            }
        }
    });
    Ok(())
}

fn try_open(path: &Path, open_devices: &OpenDevices, events: &Sender<Event>) {
    let is_event_node = path
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.starts_with("event"));
    if !is_event_node || open_devices.lock().unwrap().contains(path) {
        return;
    }
    // Unreadable nodes are retried when udev changes their permissions.
    let Ok(device) = Device::open(path) else {
        return;
    };
    if device.name() == Some(VIRTUAL_DEVICE_NAME) || !is_keyboard_or_mouse(&device) {
        return;
    }
    if !open_devices.lock().unwrap().insert(path.to_owned()) {
        return;
    }
    eprintln!(
        "omarchy-expand: listening to {}",
        device.name().unwrap_or("unnamed device")
    );

    let path = path.to_owned();
    let open_devices = Arc::clone(open_devices);
    let events = events.clone();
    thread::spawn(move || {
        forward_events(device, &events);
        open_devices.lock().unwrap().remove(&path);
    });
}

fn is_keyboard_or_mouse(device: &Device) -> bool {
    device.supported_keys().is_some_and(|keys| {
        (keys.contains(KeyCode::KEY_A) && keys.contains(KeyCode::KEY_ENTER))
            || keys.contains(KeyCode::BTN_LEFT)
    })
}

/// Returns when the device disappears or the daemon stops listening.
fn forward_events(mut device: Device, events: &Sender<Event>) {
    while let Ok(input_events) = device.fetch_events() {
        for input_event in input_events {
            let EventSummary::Key(_, key, value) = input_event.destructure() else {
                continue;
            };
            let event = match key {
                KeyCode::BTN_LEFT | KeyCode::BTN_RIGHT | KeyCode::BTN_MIDDLE if value == 1 => {
                    Event::Click
                }
                // Codes below the first button are keyboard keys.
                key if key.0 < KeyCode::BTN_0.0 => Event::Key { code: key.0, value },
                _ => continue,
            };
            if events.send(event).is_err() {
                return;
            }
        }
    }
}
