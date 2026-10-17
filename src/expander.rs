//! The daemon's typing state machine: which keys are down, what has been typed since the
//! last combo breaker, and the expansion waiting for a clean keyboard. It is pure logic so
//! that overlapping key presses, autorepeat and other timing cases are unit-tested.
//!
//! An expansion is armed when the typed run matches a combo and injected once every key
//! is released, because a held Shift would turn the paste chord into something else.
//! Text typed in between (the next key pressed before the last one is released, or an
//! autorepeat) is erased and re-added after the snippet, so fast typing never leaves a
//! half-erased keyword behind.

use std::collections::HashSet;

use evdev::KeyCode;

use crate::keymap::{self, Keyboard};
use crate::typed::{Combo, Trigger, TypedText};

/// A replacement ready to inject: erase this many characters, then paste this text.
#[derive(Debug, Clone, PartialEq)]
pub struct Expansion {
    pub erase: usize,
    pub text: String,
}

#[derive(Default)]
pub struct Expander {
    typed: TypedText,
    /// Every key currently down, as (device, key code), across all keyboards.
    held: HashSet<(usize, u16)>,
    pending: Option<Pending>,
}

/// A matched combo waiting for the keyboard to be clean, plus whatever was typed since.
struct Pending {
    snippet: String,
    erase: usize,
    suffix: String,
}

enum Keystroke {
    Backspace,
    /// The text this key types right now, given the modifiers held.
    Text(String),
    /// A modifier or dead key: it changes later keys and types nothing itself.
    Silent,
    /// Anything else (Enter, Tab, arrows, shortcuts) ends the typed run.
    Break,
}

impl Expander {
    /// Feeds one key event and returns the expansion to inject, if one is due now.
    /// `value` is 1 for press, 2 for autorepeat, 0 for release. Nothing is recorded while
    /// `excluded`, but key state is still tracked so a clean keyboard is recognised later.
    pub fn key(
        &mut self,
        keyboard: &mut Keyboard,
        combos: &[Combo],
        excluded: bool,
        device: usize,
        code: u16,
        value: i32,
    ) -> Option<Expansion> {
        if value != 0 && !excluded {
            // Before update_key: what a key types depends on the modifiers held before it.
            self.record(classify(keyboard, code), combos);
        }
        match value {
            1 => {
                self.held.insert((device, code));
                keyboard.update_key(code, true);
            }
            0 => {
                self.held.remove(&(device, code));
                keyboard.update_key(code, false);
            }
            _ => {}
        }
        self.take_when_clean()
    }

    /// A click, focus change or config change: whatever was typed no longer counts.
    pub fn reset(&mut self) {
        self.typed.clear();
        self.pending = None;
    }

    /// Releases every key the device had down, so a keyboard unplugged mid-press does
    /// not leave the daemon waiting forever for a release that never comes. The typed run
    /// starts over too: what it holds was typed against modifiers that are now gone.
    pub fn device_lost(&mut self, keyboard: &mut Keyboard, device: usize) {
        self.reset();
        let lost: Vec<u16> = self
            .held
            .iter()
            .filter(|(held_device, _)| *held_device == device)
            .map(|(_, code)| *code)
            .collect();
        for code in lost {
            self.held.remove(&(device, code));
            keyboard.update_key(code, false);
        }
    }

    /// The key codes currently down, for re-pressing them on a rebuilt keyboard.
    pub fn held_codes(&self) -> impl Iterator<Item = u16> + '_ {
        self.held.iter().map(|(_, code)| *code)
    }

    fn record(&mut self, keystroke: Keystroke, combos: &[Combo]) {
        match keystroke {
            Keystroke::Silent => {}
            Keystroke::Break => self.reset(),
            Keystroke::Backspace => self.backspace(combos),
            Keystroke::Text(text) => self.text(&text, combos),
        }
    }

    fn text(&mut self, text: &str, combos: &[Combo]) {
        if let Some(pending) = &mut self.pending {
            // Typed before the keyboard was clean: the app has it, so erase and re-add it.
            pending.erase += text.chars().count();
            pending.suffix.push_str(text);
            return;
        }
        if is_delimiter(text)
            && let Some(combo) = self.typed.find_match(combos, Trigger::Delimiter)
        {
            self.pending = Some(Pending {
                snippet: combo.snippet.clone(),
                erase: combo.keyword.chars().count() + text.chars().count(),
                suffix: text.to_owned(),
            });
            return;
        }
        if text.chars().any(char::is_whitespace) {
            self.typed.clear();
            return;
        }
        self.typed.push(text);
        if let Some(combo) = self.typed.find_match(combos, Trigger::Immediate) {
            self.pending = Some(Pending {
                snippet: combo.snippet.clone(),
                erase: combo.keyword.chars().count(),
                suffix: String::new(),
            });
        }
    }

    fn backspace(&mut self, combos: &[Combo]) {
        let Some(pending) = self.pending.take() else {
            self.typed.backspace();
            return;
        };
        // The app holds the run plus the suffix, minus the character just deleted.
        // Replaying what is left re-evaluates the match against the real text.
        let mut remaining = format!("{}{}", self.typed.text(), pending.suffix);
        remaining.pop();
        self.typed.clear();
        for character in remaining.chars() {
            self.text(&character.to_string(), combos);
        }
    }

    fn take_when_clean(&mut self) -> Option<Expansion> {
        if !self.held.is_empty() {
            return None;
        }
        let pending = self.pending.take()?;
        self.typed.clear();
        Some(Expansion {
            erase: pending.erase,
            text: pending.snippet + &pending.suffix,
        })
    }
}

/// Printable characters extend the typed run; anything else (Enter, arrows, shortcuts)
/// breaks it, as in Beeftext. Shortcuts are checked first, so Ctrl+Backspace (delete a
/// word) is a break rather than a single backspace.
fn classify(keyboard: &Keyboard, code: u16) -> Keystroke {
    if keyboard.shortcut_modifier_active() {
        return Keystroke::Break;
    }
    if code == KeyCode::KEY_BACKSPACE.0 {
        return Keystroke::Backspace;
    }
    let keysym = keyboard.keysym(code);
    if keymap::is_modifier(keysym) || keymap::is_dead_key(keysym) {
        return Keystroke::Silent;
    }
    let text = keyboard.text(code);
    if text.is_empty() || text.chars().any(char::is_control) {
        Keystroke::Break
    } else {
        Keystroke::Text(text)
    }
}

/// What may follow a delimiter-triggered keyword: a space or a punctuation mark. Enter and
/// Tab are not delimiters, because the app may act on them (send, move focus) before the
/// snippet could replace the keyword.
fn is_delimiter(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_whitespace() || c.is_ascii_punctuation())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::KeymapNames;
    use crate::typed::Matching;

    const DEVICE: usize = 1;

    fn combo(keyword: &str, snippet: &str, trigger: Trigger) -> Combo {
        Combo {
            keyword: keyword.into(),
            snippet: snippet.into(),
            matching: Matching::Strict,
            trigger,
            case_sensitive: true,
        }
    }

    fn key_code(letter: char) -> u16 {
        match letter {
            'a' => KeyCode::KEY_A,
            'e' => KeyCode::KEY_E,
            'g' => KeyCode::KEY_G,
            'i' => KeyCode::KEY_I,
            'l' => KeyCode::KEY_L,
            'n' => KeyCode::KEY_N,
            'o' => KeyCode::KEY_O,
            's' => KeyCode::KEY_S,
            't' => KeyCode::KEY_T,
            'x' => KeyCode::KEY_X,
            ' ' => KeyCode::KEY_SPACE,
            '.' => KeyCode::KEY_DOT,
            '\n' => KeyCode::KEY_ENTER,
            '<' => KeyCode::KEY_BACKSPACE,
            other => panic!("no key for {other:?}"),
        }
        .0
    }

    struct Lab {
        keyboard: Keyboard,
        combos: Vec<Combo>,
        expander: Expander,
    }

    impl Lab {
        fn new(combos: Vec<Combo>) -> Self {
            let keyboard = Keyboard::new(&KeymapNames {
                layout: "us".into(),
                ..Default::default()
            })
            .unwrap();
            Self {
                keyboard,
                combos,
                expander: Expander::default(),
            }
        }

        fn event(&mut self, code: u16, value: i32) -> Option<Expansion> {
            self.expander
                .key(&mut self.keyboard, &self.combos, false, DEVICE, code, value)
        }

        fn press(&mut self, letter: char) -> Option<Expansion> {
            self.event(key_code(letter), 1)
        }

        fn release(&mut self, letter: char) -> Option<Expansion> {
            self.event(key_code(letter), 0)
        }

        fn repeat(&mut self, letter: char) -> Option<Expansion> {
            self.event(key_code(letter), 2)
        }

        /// Types each character with a full press and release, returning the last result.
        fn tap(&mut self, text: &str) -> Option<Expansion> {
            let mut result = None;
            for letter in text.chars() {
                assert_eq!(self.press(letter), None);
                result = self.release(letter);
            }
            result
        }
    }

    fn expansion(erase: usize, text: &str) -> Option<Expansion> {
        Some(Expansion {
            erase,
            text: text.into(),
        })
    }

    #[test]
    fn a_typed_keyword_is_expanded_when_the_last_key_is_released() {
        let mut lab = Lab::new(vec![combo("oetest", "Hello", Trigger::Immediate)]);
        assert_eq!(lab.tap("oetes"), None);
        assert_eq!(lab.press('t'), None);
        assert_eq!(lab.release('t'), expansion(6, "Hello"));
        assert_eq!(lab.tap("oetest"), expansion(6, "Hello"));
    }

    #[test]
    fn a_key_pressed_before_the_last_one_is_released_is_kept_after_the_snippet() {
        let mut lab = Lab::new(vec![combo("oetest", "Hello", Trigger::Immediate)]);
        lab.tap("oetes");
        lab.press('t');
        assert_eq!(lab.press(' '), None); // rollover: the app already has "oetest "
        assert_eq!(lab.release('t'), None);
        assert_eq!(lab.release(' '), expansion(7, "Hello "));
    }

    #[test]
    fn autorepeat_on_the_last_key_is_erased_too() {
        let mut lab = Lab::new(vec![combo("oetest", "Hello", Trigger::Immediate)]);
        lab.tap("oetes");
        lab.press('t');
        assert_eq!(lab.repeat('t'), None);
        assert_eq!(lab.repeat('t'), None);
        assert_eq!(lab.release('t'), expansion(8, "Hellott"));
    }

    #[test]
    fn enter_during_rollover_cancels_rather_than_corrupts() {
        let mut lab = Lab::new(vec![combo("oetest", "Hello", Trigger::Immediate)]);
        lab.tap("oetes");
        lab.press('t');
        lab.press('\n');
        assert_eq!(lab.release('t'), None);
        assert_eq!(lab.release('\n'), None);
    }

    #[test]
    fn a_held_shift_delays_the_expansion_until_it_is_released() {
        let mut lab = Lab::new(vec![combo("SIG", "Regards", Trigger::Immediate)]);
        assert_eq!(lab.event(KeyCode::KEY_LEFTSHIFT.0, 1), None);
        assert_eq!(lab.tap("sig"), None);
        assert_eq!(
            lab.event(KeyCode::KEY_LEFTSHIFT.0, 0),
            expansion(3, "Regards")
        );
    }

    #[test]
    fn a_shortcut_breaks_the_run() {
        let mut lab = Lab::new(vec![combo("sig", "Regards", Trigger::Immediate)]);
        lab.tap("si");
        lab.event(KeyCode::KEY_LEFTCTRL.0, 1);
        lab.tap("a");
        lab.event(KeyCode::KEY_LEFTCTRL.0, 0);
        assert_eq!(lab.tap("g"), None);
        lab.tap(" ");
        assert_eq!(lab.tap("sig"), expansion(3, "Regards"));
    }

    #[test]
    fn ctrl_backspace_breaks_the_run_instead_of_deleting_one_character() {
        let mut lab = Lab::new(vec![combo("sig", "Regards", Trigger::Immediate)]);
        lab.tap("xsi");
        lab.event(KeyCode::KEY_LEFTCTRL.0, 1);
        lab.tap("<");
        lab.event(KeyCode::KEY_LEFTCTRL.0, 0);
        // The app deleted the whole word; "g" alone must not complete anything.
        assert_eq!(lab.tap("g"), None);
    }

    #[test]
    fn backspace_during_rollover_replays_what_the_app_really_has() {
        let mut lab = Lab::new(vec![combo("sig", "Regards", Trigger::Immediate)]);
        lab.tap("si");
        lab.press('g');
        lab.press('n'); // the app has "sign"
        assert_eq!(lab.release('g'), None);
        assert_eq!(lab.press('<'), None); // the app has "sig" again, n still down
        assert_eq!(lab.release('n'), None);
        assert_eq!(lab.release('<'), expansion(3, "Regards"));

        lab.tap("si");
        lab.press('g');
        lab.press('<'); // the app has "si"
        assert_eq!(lab.release('g'), None);
        assert_eq!(lab.release('<'), None);
        assert_eq!(lab.tap("g"), expansion(3, "Regards"));
    }

    #[test]
    fn delimiter_combos_wait_for_a_space_or_punctuation() {
        let mut lab = Lab::new(vec![combo("sig", "Regards", Trigger::Delimiter)]);
        assert_eq!(lab.tap("signal"), None);
        assert_eq!(lab.tap(" "), None);
        assert_eq!(lab.tap("sig"), None);
        assert_eq!(lab.tap(" "), expansion(4, "Regards "));
        assert_eq!(lab.tap("sig"), None);
        assert_eq!(lab.tap("."), expansion(4, "Regards."));
        // Enter is not a delimiter: the app may have acted on it already.
        assert_eq!(lab.tap("sig"), None);
        assert_eq!(lab.tap("\n"), None);
    }

    #[test]
    fn text_typed_right_after_a_delimiter_is_kept() {
        let mut lab = Lab::new(vec![combo("sig", "Regards", Trigger::Delimiter)]);
        lab.tap("sig");
        lab.press(' ');
        lab.press('n');
        lab.release(' ');
        assert_eq!(lab.release('n'), expansion(5, "Regards n"));
    }

    #[test]
    fn nothing_is_recorded_in_an_excluded_app() {
        let mut lab = Lab::new(vec![combo("sig", "Regards", Trigger::Immediate)]);
        for letter in "sig".chars() {
            let code = key_code(letter);
            for value in [1, 0] {
                let result =
                    lab.expander
                        .key(&mut lab.keyboard, &lab.combos, true, DEVICE, code, value);
                assert_eq!(result, None);
            }
        }
    }

    #[test]
    fn a_reset_cancels_an_armed_expansion() {
        let mut lab = Lab::new(vec![combo("sig", "Regards", Trigger::Immediate)]);
        lab.tap("si");
        lab.press('g');
        lab.expander.reset(); // a click or focus change while g is still down
        assert_eq!(lab.release('g'), None);
    }

    #[test]
    fn a_lost_device_releases_its_keys() {
        let mut lab = Lab::new(vec![combo("sig", "Regards", Trigger::Immediate)]);
        let other_device = DEVICE + 1;
        lab.expander.key(
            &mut lab.keyboard,
            &lab.combos,
            false,
            other_device,
            KeyCode::KEY_LEFTSHIFT.0,
            1,
        );
        assert_eq!(lab.tap("sig"), None); // typed as SIG while the other Shift is down
        lab.expander.device_lost(&mut lab.keyboard, other_device);
        assert!(lab.expander.held_codes().next().is_none());
        assert_eq!(lab.tap("sig"), expansion(3, "Regards"));
    }
}
