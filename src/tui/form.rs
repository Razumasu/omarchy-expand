//! The modal forms: a combo, a group name, and the excluded apps.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::text_input::TextInput;
use crate::config::{ComboEntry, Settings};
use crate::typed::Matching;

pub enum Outcome {
    Continue,
    Cancel,
    Submit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ComboField {
    Keyword,
    Name,
    Snippet,
    Group,
    Matching,
    CaseSensitive,
    Enabled,
}

const COMBO_FIELDS: [ComboField; 7] = [
    ComboField::Keyword,
    ComboField::Name,
    ComboField::Snippet,
    ComboField::Group,
    ComboField::Matching,
    ComboField::CaseSensitive,
    ComboField::Enabled,
];

pub struct ComboForm {
    /// The (group, combo) being edited, or None for a new combo.
    pub editing: Option<(usize, usize)>,
    pub keyword: TextInput,
    pub name: TextInput,
    pub snippet: TextInput,
    pub group: usize,
    pub matching: Matching,
    pub case_sensitive: bool,
    pub enabled: bool,
    pub field: ComboField,
    pub error: Option<String>,
}

impl ComboForm {
    pub fn new_combo(group: usize) -> Self {
        Self {
            editing: None,
            keyword: TextInput::single_line(""),
            name: TextInput::single_line(""),
            snippet: TextInput::multiline(""),
            group,
            matching: Matching::Strict,
            case_sensitive: true,
            enabled: true,
            field: ComboField::Keyword,
            error: None,
        }
    }

    pub fn edit(group: usize, index: usize, combo: &ComboEntry) -> Self {
        Self {
            editing: Some((group, index)),
            keyword: TextInput::single_line(&combo.keyword),
            name: TextInput::single_line(&combo.name),
            snippet: TextInput::multiline(&combo.snippet),
            group,
            matching: combo.matching,
            case_sensitive: combo.case_sensitive,
            enabled: combo.enabled,
            field: ComboField::Keyword,
            error: None,
        }
    }

    /// The form's contents, or a message saying what is missing.
    pub fn entry(&self) -> Result<ComboEntry, String> {
        let entry = ComboEntry {
            name: self.name.text().trim().to_owned(),
            keyword: self.keyword.text(),
            snippet: self.snippet.text(),
            matching: self.matching,
            case_sensitive: self.case_sensitive,
            enabled: self.enabled,
        };
        if entry.keyword.is_empty() {
            return Err("Type a keyword.".into());
        }
        if entry.snippet.is_empty() {
            return Err("Type the snippet the keyword expands to.".into());
        }
        Ok(entry)
    }

    pub fn handle_key(&mut self, key: KeyEvent, group_count: usize) -> Outcome {
        if let Some(outcome) = common_keys(key) {
            return outcome;
        }
        match (self.field, key.code) {
            (_, KeyCode::Tab) => self.move_field(1),
            (_, KeyCode::BackTab) => self.move_field(-1),
            (ComboField::Snippet, _) => {
                self.snippet.handle_key(key);
            }
            (_, KeyCode::Enter) => return Outcome::Submit,
            (_, KeyCode::Up) => self.move_field(-1),
            (_, KeyCode::Down) => self.move_field(1),
            (ComboField::Keyword, _) => {
                self.keyword.handle_key(key);
            }
            (ComboField::Name, _) => {
                self.name.handle_key(key);
            }
            (ComboField::Group, KeyCode::Left) if group_count > 0 => {
                self.group = (self.group + group_count - 1) % group_count;
            }
            (ComboField::Group, KeyCode::Right | KeyCode::Char(' ')) if group_count > 0 => {
                self.group = (self.group + 1) % group_count;
            }
            (ComboField::Matching, KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')) => {
                self.matching = match self.matching {
                    Matching::Strict => Matching::Loose,
                    Matching::Loose => Matching::Strict,
                };
            }
            (ComboField::CaseSensitive, KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')) => {
                self.case_sensitive = !self.case_sensitive;
            }
            (ComboField::Enabled, KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')) => {
                self.enabled = !self.enabled;
            }
            _ => {}
        }
        Outcome::Continue
    }

    pub fn paste(&mut self, text: &str) {
        match self.field {
            ComboField::Keyword => self.keyword.paste(text),
            ComboField::Name => self.name.paste(text),
            ComboField::Snippet => self.snippet.paste(text),
            _ => {}
        }
    }

    fn move_field(&mut self, step: isize) {
        let current = COMBO_FIELDS
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        let next = (current as isize + step).rem_euclid(COMBO_FIELDS.len() as isize);
        self.field = COMBO_FIELDS[next as usize];
    }
}

pub struct GroupForm {
    /// The group being renamed, or None for a new group.
    pub editing: Option<usize>,
    pub name: TextInput,
    pub error: Option<String>,
}

impl GroupForm {
    pub fn new_group() -> Self {
        Self {
            editing: None,
            name: TextInput::single_line(""),
            error: None,
        }
    }

    pub fn rename(index: usize, name: &str) -> Self {
        Self {
            editing: Some(index),
            name: TextInput::single_line(name),
            error: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        if let Some(outcome) = common_keys(key) {
            return outcome;
        }
        if key.code == KeyCode::Enter {
            return Outcome::Submit;
        }
        self.name.handle_key(key);
        Outcome::Continue
    }
}

/// Excluded apps are edited as plain text, one app per line.
pub struct SettingsForm {
    pub excluded_apps: TextInput,
    pub error: Option<String>,
}

impl SettingsForm {
    pub fn new(settings: &Settings) -> Self {
        Self {
            excluded_apps: TextInput::multiline(&settings.excluded_apps.join("\n")),
            error: None,
        }
    }

    pub fn settings(&self) -> Settings {
        Settings {
            excluded_apps: self
                .excluded_apps
                .text()
                .lines()
                .map(str::trim)
                .filter(|app| !app.is_empty())
                .map(str::to_owned)
                .collect(),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        if let Some(outcome) = common_keys(key) {
            return outcome;
        }
        self.excluded_apps.handle_key(key);
        Outcome::Continue
    }
}

/// Esc cancels and Ctrl+S saves in every form.
fn common_keys(key: KeyEvent) -> Option<Outcome> {
    match key.code {
        KeyCode::Esc => Some(Outcome::Cancel),
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Outcome::Submit)
        }
        _ => None,
    }
}
