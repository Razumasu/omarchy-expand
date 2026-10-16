//! Test helper: types lowercase text through its own uinput keyboard, so the daemon
//! sees it exactly like a physical keyboard plugged in after startup.
//!
//! usage: typist <text> [--enter-after-ms N]

use std::env;
use std::thread;
use std::time::Duration;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};

/// US positions; the lab guest uses the us layout.
const LETTERS: [KeyCode; 26] = [
    KeyCode::KEY_A,
    KeyCode::KEY_B,
    KeyCode::KEY_C,
    KeyCode::KEY_D,
    KeyCode::KEY_E,
    KeyCode::KEY_F,
    KeyCode::KEY_G,
    KeyCode::KEY_H,
    KeyCode::KEY_I,
    KeyCode::KEY_J,
    KeyCode::KEY_K,
    KeyCode::KEY_L,
    KeyCode::KEY_M,
    KeyCode::KEY_N,
    KeyCode::KEY_O,
    KeyCode::KEY_P,
    KeyCode::KEY_Q,
    KeyCode::KEY_R,
    KeyCode::KEY_S,
    KeyCode::KEY_T,
    KeyCode::KEY_U,
    KeyCode::KEY_V,
    KeyCode::KEY_W,
    KeyCode::KEY_X,
    KeyCode::KEY_Y,
    KeyCode::KEY_Z,
];

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let text = args
        .first()
        .expect("usage: typist <text> [--enter-after-ms N]");
    let enter_after = args
        .iter()
        .position(|arg| arg == "--enter-after-ms")
        .map(|index| Duration::from_millis(args[index + 1].parse().expect("milliseconds")));

    let mut keys = AttributeSet::<KeyCode>::new();
    for key in LETTERS.iter().chain([&KeyCode::KEY_ENTER]) {
        keys.insert(*key);
    }
    let mut keyboard = VirtualDevice::builder()?
        .name("omarchy-expand test typist")
        .with_keys(&keys)?
        .build()?;

    // Give Hyprland and the daemon time to notice the new device.
    thread::sleep(Duration::from_millis(1500));

    for letter in text.chars() {
        let index = (letter as u8).checked_sub(b'a').filter(|index| *index < 26);
        let key = LETTERS[usize::from(index.expect("typist only types a-z"))];
        tap(&mut keyboard, key)?;
    }
    if let Some(delay) = enter_after {
        thread::sleep(delay);
        tap(&mut keyboard, KeyCode::KEY_ENTER)?;
    }
    // Let the compositor read the last events before the device disappears.
    thread::sleep(Duration::from_millis(300));
    Ok(())
}

fn tap(keyboard: &mut VirtualDevice, key: KeyCode) -> std::io::Result<()> {
    keyboard.emit(&[InputEvent::new(EventType::KEY.0, key.0, 1)])?;
    thread::sleep(Duration::from_millis(15));
    keyboard.emit(&[InputEvent::new(EventType::KEY.0, key.0, 0)])?;
    thread::sleep(Duration::from_millis(15));
    Ok(())
}
