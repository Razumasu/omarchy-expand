//! Turns evdev key codes into text using the same xkb keymap Hyprland compiled,
//! so keywords containing layout-specific characters (æ, ø, å) match.

use std::ffi::{CString, c_char};
use std::ptr;

use xkbcommon_dl::{
    XKB_MOD_NAME_ALT, XKB_MOD_NAME_CTRL, XKB_MOD_NAME_LOGO, xkb_context, xkb_context_flags,
    xkb_key_direction, xkb_keymap, xkb_keymap_compile_flags, xkb_rule_names, xkb_state,
    xkb_state_component, xkbcommon_handle,
};

/// evdev numbers keys from 0, xkb from 8 (an X11 legacy).
const EVDEV_TO_XKB: u32 = 8;

const XKB_KEY_V: u32 = 0x0076;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct KeymapNames {
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: String,
    pub active_layout: u32,
}

pub struct Keyboard {
    context: *mut xkb_context,
    keymap: *mut xkb_keymap,
    state: *mut xkb_state,
}

impl Keyboard {
    pub fn new(names: &KeymapNames) -> Result<Self, String> {
        let xkb = xkbcommon_handle();
        let rules = optional_cstring(&names.rules)?;
        let model = optional_cstring(&names.model)?;
        let layout = optional_cstring(&names.layout)?;
        let variant = optional_cstring(&names.variant)?;
        let options = optional_cstring(&names.options)?;
        let rule_names = xkb_rule_names {
            rules: as_ptr(&rules),
            model: as_ptr(&model),
            layout: as_ptr(&layout),
            variant: as_ptr(&variant),
            options: as_ptr(&options),
        };

        // SAFETY: every pointer handed to xkb is either null or a live CString above,
        // and each returned object is checked before use and released in Drop.
        unsafe {
            let context = (xkb.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS);
            if context.is_null() {
                return Err("could not create an xkb context".into());
            }
            let keymap = (xkb.xkb_keymap_new_from_names)(
                context,
                &rule_names,
                xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
            );
            if keymap.is_null() {
                (xkb.xkb_context_unref)(context);
                return Err(format!("xkb could not compile the keymap {names:?}"));
            }
            let state = (xkb.xkb_state_new)(keymap);
            if state.is_null() {
                (xkb.xkb_keymap_unref)(keymap);
                (xkb.xkb_context_unref)(context);
                return Err("could not create an xkb state".into());
            }
            (xkb.xkb_state_update_mask)(state, 0, 0, 0, 0, 0, names.active_layout);
            Ok(Self {
                context,
                keymap,
                state,
            })
        }
    }

    pub fn update_key(&mut self, code: u16, pressed: bool) {
        let direction = if pressed {
            xkb_key_direction::XKB_KEY_DOWN
        } else {
            xkb_key_direction::XKB_KEY_UP
        };
        // SAFETY: state is valid for the lifetime of self.
        unsafe { (xkbcommon_handle().xkb_state_update_key)(self.state, xkb_code(code), direction) };
    }

    /// The text this key would type right now, given the modifiers currently held.
    pub fn text(&self, code: u16) -> String {
        let mut buffer = [0u8; 64];
        // SAFETY: xkb writes at most buffer.len() bytes including the NUL terminator.
        let written = unsafe {
            (xkbcommon_handle().xkb_state_key_get_utf8)(
                self.state,
                xkb_code(code),
                buffer.as_mut_ptr().cast::<c_char>(),
                buffer.len(),
            )
        };
        let length = (written.max(0) as usize).min(buffer.len() - 1);
        String::from_utf8_lossy(&buffer[..length]).into_owned()
    }

    pub fn keysym(&self, code: u16) -> u32 {
        // SAFETY: state is valid for the lifetime of self.
        unsafe { (xkbcommon_handle().xkb_state_key_get_one_sym)(self.state, xkb_code(code)) }
    }

    /// Ctrl, Alt or Super held means the user is pressing a shortcut, not typing.
    pub fn shortcut_modifier_active(&self) -> bool {
        [XKB_MOD_NAME_CTRL, XKB_MOD_NAME_ALT, XKB_MOD_NAME_LOGO]
            .iter()
            .any(|name| {
                // SAFETY: the modifier names are NUL-terminated constants.
                unsafe {
                    (xkbcommon_handle().xkb_state_mod_name_is_active)(
                        self.state,
                        name.as_ptr().cast::<c_char>(),
                        xkb_state_component::XKB_STATE_MODS_EFFECTIVE,
                    ) == 1
                }
            })
    }

    /// The evdev code of the key that types "v", so Ctrl+V pastes on Dvorak and Colemak too.
    pub fn paste_key(&self) -> Option<u16> {
        let xkb = xkbcommon_handle();
        // SAFETY: keymap and state are valid; xkb owns the returned keysym array.
        unsafe {
            let layout = (xkb.xkb_state_serialize_layout)(
                self.state,
                xkb_state_component::XKB_STATE_LAYOUT_EFFECTIVE,
            );
            let first = (xkb.xkb_keymap_min_keycode)(self.keymap);
            let last = (xkb.xkb_keymap_max_keycode)(self.keymap);
            (first.max(EVDEV_TO_XKB)..=last).find_map(|keycode| {
                let mut syms: *const u32 = ptr::null();
                let count = (xkb.xkb_keymap_key_get_syms_by_level)(
                    self.keymap,
                    keycode,
                    layout,
                    0,
                    &mut syms,
                );
                let types_v = count > 0
                    && std::slice::from_raw_parts(syms, count as usize).contains(&XKB_KEY_V);
                types_v.then(|| (keycode - EVDEV_TO_XKB) as u16)
            })
        }
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        let xkb = xkbcommon_handle();
        // SAFETY: each pointer was created in new() and is released exactly once.
        unsafe {
            (xkb.xkb_state_unref)(self.state);
            (xkb.xkb_keymap_unref)(self.keymap);
            (xkb.xkb_context_unref)(self.context);
        }
    }
}

/// Keys that change how other keys type without typing anything themselves.
pub fn is_modifier(keysym: u32) -> bool {
    matches!(keysym,
        0xffe1..=0xffee // Shift, Control, Caps Lock, Meta, Alt, Super, Hyper
        | 0xfe01..=0xfe0f // ISO level and group shifts (AltGr)
        | 0xff7e | 0xff7f // Mode switch, Num Lock
        | 0xff20 // Compose
    )
}

/// Dead keys (´ ¨ ^) wait for the next key to produce a character.
pub fn is_dead_key(keysym: u32) -> bool {
    (0xfe50..=0xfe93).contains(&keysym)
}

fn xkb_code(code: u16) -> u32 {
    u32::from(code) + EVDEV_TO_XKB
}

/// xkb treats a null name as "use the default", which is what Hyprland's empty strings mean.
fn optional_cstring(value: &str) -> Result<Option<CString>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    CString::new(value)
        .map(Some)
        .map_err(|_| format!("keymap name {value:?} contains a NUL byte"))
}

fn as_ptr(value: &Option<CString>) -> *const c_char {
    value.as_ref().map_or(ptr::null(), |name| name.as_ptr())
}

#[cfg(test)]
mod tests {
    use super::*;
    use evdev::KeyCode;

    fn keyboard(layout: &str) -> Keyboard {
        Keyboard::new(&KeymapNames {
            layout: layout.into(),
            ..Default::default()
        })
        .unwrap()
    }

    fn tap(keyboard: &mut Keyboard, key: KeyCode) -> String {
        let text = keyboard.text(key.0);
        keyboard.update_key(key.0, true);
        keyboard.update_key(key.0, false);
        text
    }

    #[test]
    fn danish_layout_types_danish_letters() {
        let mut danish = keyboard("dk");
        assert_eq!(tap(&mut danish, KeyCode::KEY_SEMICOLON), "æ");
        assert_eq!(tap(&mut danish, KeyCode::KEY_APOSTROPHE), "ø");
        assert_eq!(tap(&mut danish, KeyCode::KEY_LEFTBRACE), "å");
    }

    #[test]
    fn shift_produces_capitals_and_ctrl_counts_as_a_shortcut() {
        let mut us = keyboard("us");
        us.update_key(KeyCode::KEY_LEFTSHIFT.0, true);
        assert_eq!(us.text(KeyCode::KEY_A.0), "A");
        assert!(!us.shortcut_modifier_active());
        us.update_key(KeyCode::KEY_LEFTSHIFT.0, false);

        us.update_key(KeyCode::KEY_LEFTCTRL.0, true);
        assert!(us.shortcut_modifier_active());
    }

    #[test]
    fn paste_key_follows_the_layout() {
        assert_eq!(keyboard("us").paste_key(), Some(KeyCode::KEY_V.0));
        let dvorak = Keyboard::new(&KeymapNames {
            layout: "us".into(),
            variant: "dvorak".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(dvorak.paste_key(), Some(KeyCode::KEY_DOT.0));
    }

    #[test]
    fn modifiers_and_dead_keys_are_recognised() {
        let us = keyboard("us");
        assert!(is_modifier(us.keysym(KeyCode::KEY_LEFTSHIFT.0)));
        assert!(!is_modifier(us.keysym(KeyCode::KEY_A.0)));
        let danish = keyboard("dk");
        assert!(is_dead_key(danish.keysym(KeyCode::KEY_EQUAL.0)));
    }
}
