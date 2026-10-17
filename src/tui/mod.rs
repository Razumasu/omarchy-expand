//! The setup UI: browse groups and combos, edit them, and choose excluded apps.
//! Every confirmed change is saved straight away, and the running daemon reloads it.

mod form;
mod text_input;
mod view;

use std::error::Error;
use std::io::stdout;
use std::path::PathBuf;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
};
use ratatui::crossterm::execute;

use crate::config::{self, Group, Library, Settings};
use form::{ComboForm, GroupForm, Outcome, SettingsForm};

/// The group a first combo goes into when there are no groups yet.
const FIRST_GROUP_NAME: &str = "General";

pub fn run() -> Result<(), Box<dyn Error>> {
    let dir = config::config_dir()?;
    // Refuse to start on a broken file rather than overwrite it on the first save.
    let library = config::load_library(&dir)?;
    let settings = config::load_settings(&dir)?;
    // Asked over the daemon's socket, so a daemon run outside systemd counts too.
    let mut app = App::new(dir, library, settings, crate::ipc::daemon_reachable());

    let mut terminal = ratatui::init();
    execute!(stdout(), EnableBracketedPaste)?;
    let result = app.run(&mut terminal);
    // The terminal is restored whatever happened, before any error is reported.
    let paste = execute!(stdout(), DisableBracketedPaste);
    ratatui::restore();
    result?;
    paste?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pane {
    Groups,
    Combos,
}

pub enum Mode {
    Browse,
    Search,
    Combo(ComboForm),
    Group(GroupForm),
    Settings(SettingsForm),
    ConfirmDelete(DeleteTarget),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeleteTarget {
    Combo(usize, usize),
    Group(usize),
}

pub struct Status {
    pub message: String,
    pub is_error: bool,
}

pub struct App {
    dir: PathBuf,
    pub library: Library,
    pub settings: Settings,
    pub daemon_running: bool,
    pub pane: Pane,
    pub group: usize,
    /// Index into visible_combos().
    pub combo: usize,
    pub query: String,
    pub mode: Mode,
    pub status: Option<Status>,
    quit: bool,
}

impl App {
    pub fn new(dir: PathBuf, library: Library, settings: Settings, daemon_running: bool) -> Self {
        Self {
            dir,
            library,
            settings,
            daemon_running,
            pane: Pane::Combos,
            group: 0,
            combo: 0,
            query: String::new(),
            mode: Mode::Browse,
            status: None,
            quit: false,
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<(), Box<dyn Error>> {
        while !self.quit {
            terminal.draw(|frame| view::draw(frame, self))?;
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key),
                Event::Paste(text) => self.handle_paste(&text),
                _ => {}
            }
        }
        Ok(())
    }

    /// (group, combo) pairs shown in the combo list: the selected group's combos,
    /// or matches from every group while searching.
    pub fn visible_combos(&self) -> Vec<(usize, usize)> {
        let query = self.query.to_lowercase();
        self.library
            .groups
            .iter()
            .enumerate()
            .filter(|(index, _)| !query.is_empty() || *index == self.group)
            .flat_map(|(group_index, group)| {
                group
                    .combos
                    .iter()
                    .enumerate()
                    .filter(|(_, combo)| {
                        query.is_empty()
                            || [&combo.keyword, &combo.name, &combo.snippet]
                                .iter()
                                .any(|text| text.to_lowercase().contains(&query))
                    })
                    .map(move |(combo_index, _)| (group_index, combo_index))
            })
            .collect()
    }

    pub fn selected_combo(&self) -> Option<(usize, usize)> {
        self.visible_combos().get(self.combo).copied()
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.status = None;
        match std::mem::replace(&mut self.mode, Mode::Browse) {
            Mode::Browse => self.browse_key(key),
            Mode::Search => self.search_key(key),
            Mode::Combo(mut form) => match form.handle_key(key, self.library.groups.len()) {
                Outcome::Continue => self.mode = Mode::Combo(form),
                Outcome::Cancel => {}
                Outcome::Submit => self.submit_combo(form),
            },
            Mode::Group(mut form) => match form.handle_key(key) {
                Outcome::Continue => self.mode = Mode::Group(form),
                Outcome::Cancel => {}
                Outcome::Submit => self.submit_group(form),
            },
            Mode::Settings(mut form) => match form.handle_key(key) {
                Outcome::Continue => self.mode = Mode::Settings(form),
                Outcome::Cancel => {}
                Outcome::Submit => self.submit_settings(form),
            },
            Mode::ConfirmDelete(target) => {
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                    self.delete(target);
                }
            }
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        match &mut self.mode {
            Mode::Combo(form) => form.paste(text),
            Mode::Group(form) => form.name.paste(text),
            Mode::Settings(form) => form.excluded_apps.paste(text),
            Mode::Search => {
                self.query.push_str(&text.replace(['\n', '\r'], " "));
                self.combo = 0;
            }
            _ => {}
        }
    }

    fn browse_key(&mut self, key: KeyEvent) {
        match (self.pane, key.code) {
            (_, KeyCode::Char('q')) => self.quit = true,
            (_, KeyCode::Esc) if !self.query.is_empty() => {
                self.query.clear();
                self.combo = 0;
            }
            (_, KeyCode::Esc) => self.quit = true,
            (_, KeyCode::Char('/')) => {
                self.pane = Pane::Combos;
                self.combo = 0;
                self.mode = Mode::Search;
            }
            (_, KeyCode::Char('s')) => {
                self.mode = Mode::Settings(SettingsForm::new(&self.settings))
            }
            (_, KeyCode::Tab) => self.pane = other_pane(self.pane),
            (Pane::Combos, KeyCode::Left | KeyCode::Char('h')) => self.pane = Pane::Groups,
            (Pane::Groups, KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter) => {
                self.pane = Pane::Combos;
            }
            (_, KeyCode::Up | KeyCode::Char('k')) => self.move_selection(-1),
            (_, KeyCode::Down | KeyCode::Char('j')) => self.move_selection(1),
            (Pane::Groups, KeyCode::Char('a')) => self.mode = Mode::Group(GroupForm::new_group()),
            (Pane::Groups, KeyCode::Char('e')) => {
                if let Some(group) = self.library.groups.get(self.group) {
                    self.mode = Mode::Group(GroupForm::rename(self.group, &group.name));
                }
            }
            (Pane::Groups, KeyCode::Char(' ')) => self.toggle_group(),
            (Pane::Groups, KeyCode::Char('d')) if self.group < self.library.groups.len() => {
                self.mode = Mode::ConfirmDelete(DeleteTarget::Group(self.group));
            }
            (Pane::Combos, KeyCode::Char('a')) => {
                self.mode = Mode::Combo(ComboForm::new_combo(self.group));
            }
            (Pane::Combos, KeyCode::Char('e') | KeyCode::Enter) => {
                if let Some((group, index)) = self.selected_combo() {
                    let combo = &self.library.groups[group].combos[index];
                    self.mode = Mode::Combo(ComboForm::edit(group, index, combo));
                }
            }
            (Pane::Combos, KeyCode::Char(' ')) => self.toggle_combo(),
            (Pane::Combos, KeyCode::Char('d')) => {
                if let Some((group, index)) = self.selected_combo() {
                    self.mode = Mode::ConfirmDelete(DeleteTarget::Combo(group, index));
                }
            }
            _ => {}
        }
    }

    fn search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.query.clear(),
            KeyCode::Enter => {}
            KeyCode::Backspace => {
                self.query.pop();
                self.mode = Mode::Search;
            }
            KeyCode::Char(character) => {
                self.query.push(character);
                self.mode = Mode::Search;
            }
            KeyCode::Up | KeyCode::Down => {
                self.mode = Mode::Search;
                self.move_selection(if key.code == KeyCode::Up { -1 } else { 1 });
                return;
            }
            _ => self.mode = Mode::Search,
        }
        self.combo = 0;
    }

    fn move_selection(&mut self, step: isize) {
        let (selected, count) = match self.pane {
            Pane::Groups => (&mut self.group, self.library.groups.len()),
            Pane::Combos => {
                let count = self.visible_combos().len();
                (&mut self.combo, count)
            }
        };
        if count > 0 {
            *selected = selected.saturating_add_signed(step).min(count - 1);
        }
        if self.pane == Pane::Groups {
            self.combo = 0;
        }
    }

    fn submit_combo(&mut self, mut form: ComboForm) {
        let entry = match form.entry() {
            Ok(entry) => entry,
            Err(message) => {
                form.error = Some(message);
                self.mode = Mode::Combo(form);
                return;
            }
        };
        let mut library = self.library.clone();
        if library.groups.is_empty() {
            library.groups.push(new_group(FIRST_GROUP_NAME));
        }
        let target = form.group.min(library.groups.len() - 1);
        let position = match form.editing {
            Some((group, index)) if group == target => {
                library.groups[group].combos[index] = entry;
                index
            }
            Some((group, index)) => {
                library.groups[group].combos.remove(index);
                library.groups[target].combos.push(entry);
                library.groups[target].combos.len() - 1
            }
            None => {
                library.groups[target].combos.push(entry);
                library.groups[target].combos.len() - 1
            }
        };
        match self.save(library) {
            Ok(()) => {
                self.query.clear();
                self.group = target;
                self.combo = position;
                self.pane = Pane::Combos;
                self.report("Combo saved.");
            }
            Err(message) => {
                form.error = Some(message);
                self.mode = Mode::Combo(form);
            }
        }
    }

    fn submit_group(&mut self, mut form: GroupForm) {
        let name = form.name.text().trim().to_owned();
        let mut library = self.library.clone();
        let index = match form.editing {
            Some(index) => {
                library.groups[index].name = name;
                index
            }
            None => {
                library.groups.push(new_group(&name));
                library.groups.len() - 1
            }
        };
        match self.save(library) {
            Ok(()) => {
                self.group = index;
                self.combo = 0;
                self.report("Group saved.");
            }
            Err(message) => {
                form.error = Some(message);
                self.mode = Mode::Group(form);
            }
        }
    }

    fn submit_settings(&mut self, mut form: SettingsForm) {
        let settings = form.settings();
        match config::save_settings(&self.dir, &settings) {
            Ok(()) => {
                self.settings = settings;
                self.report("Settings saved.");
            }
            Err(message) => {
                form.error = Some(message);
                self.mode = Mode::Settings(form);
            }
        }
    }

    fn toggle_combo(&mut self) {
        let Some((group, index)) = self.selected_combo() else {
            return;
        };
        let mut library = self.library.clone();
        let combo = &mut library.groups[group].combos[index];
        combo.enabled = !combo.enabled;
        let message = if combo.enabled {
            "Combo on."
        } else {
            "Combo off."
        };
        self.save_and_report(library, message);
    }

    fn toggle_group(&mut self) {
        if self.group >= self.library.groups.len() {
            return;
        }
        let mut library = self.library.clone();
        let group = &mut library.groups[self.group];
        group.enabled = !group.enabled;
        let message = if group.enabled {
            "Group on."
        } else {
            "Group off."
        };
        self.save_and_report(library, message);
    }

    fn delete(&mut self, target: DeleteTarget) {
        let mut library = self.library.clone();
        match target {
            DeleteTarget::Combo(group, index) => {
                library.groups[group].combos.remove(index);
            }
            DeleteTarget::Group(group) => {
                library.groups.remove(group);
            }
        }
        self.save_and_report(library, "Deleted.");
        self.group = self.group.min(self.library.groups.len().saturating_sub(1));
        self.combo = self
            .combo
            .min(self.visible_combos().len().saturating_sub(1));
    }

    /// Writes the library and adopts it only once it is safely on disk.
    fn save(&mut self, library: Library) -> Result<(), String> {
        config::save_library(&self.dir, &library)?;
        self.library = library;
        Ok(())
    }

    fn save_and_report(&mut self, library: Library, message: &str) {
        match self.save(library) {
            Ok(()) => self.report(message),
            Err(error) => {
                self.status = Some(Status {
                    message: error,
                    is_error: true,
                })
            }
        }
    }

    fn report(&mut self, message: &str) {
        self.status = Some(Status {
            message: message.into(),
            is_error: false,
        });
    }
}

fn other_pane(pane: Pane) -> Pane {
    match pane {
        Pane::Groups => Pane::Combos,
        Pane::Combos => Pane::Groups,
    }
}

fn new_group(name: &str) -> Group {
    Group {
        name: name.into(),
        enabled: true,
        combos: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed::{Matching, Trigger};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyModifiers;
    use std::fs;

    /// Removes the test's config directory when the test ends.
    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn app(name: &str) -> (App, TempDir) {
        let dir =
            std::env::temp_dir().join(format!("omarchy-expand-tui-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let app = App::new(dir.clone(), Library::default(), Settings::default(), true);
        (app, TempDir(dir))
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn ctrl(app: &mut App, character: char) {
        app.handle_key(KeyEvent::new(
            KeyCode::Char(character),
            KeyModifiers::CONTROL,
        ));
    }

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character));
        }
    }

    fn add_combo(app: &mut App, keyword: &str, snippet: &str) {
        press(app, KeyCode::Char('a'));
        type_text(app, keyword);
        press(app, KeyCode::Tab);
        press(app, KeyCode::Tab);
        type_text(app, snippet);
        ctrl(app, 's');
    }

    fn saved(app: &App) -> Library {
        config::load_library(&app.dir).unwrap()
    }

    fn screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| view::draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_first_combo_creates_a_group_and_is_saved() {
        let (mut app, _dir) = app("first");
        add_combo(&mut app, "@@mail", "razu@example.com\nRoyal Arena");
        assert!(matches!(app.mode, Mode::Browse));
        let library = saved(&app);
        assert_eq!(library.groups[0].name, FIRST_GROUP_NAME);
        assert_eq!(library.groups[0].combos[0].keyword, "@@mail");
        assert_eq!(
            library.groups[0].combos[0].snippet,
            "razu@example.com\nRoyal Arena"
        );
    }

    #[test]
    fn a_duplicate_keyword_keeps_the_form_open_and_saves_nothing() {
        let (mut app, _dir) = app("duplicate");
        add_combo(&mut app, "sig", "One");
        add_combo(&mut app, "sig", "Two");
        let Mode::Combo(form) = &app.mode else {
            panic!("form should stay open")
        };
        assert!(form.error.as_deref().unwrap().contains("more than once"));
        assert_eq!(saved(&app).groups[0].combos.len(), 1);
    }

    #[test]
    fn editing_changes_options_and_keeps_position() {
        let (mut app, _dir) = app("edit");
        add_combo(&mut app, "a", "A");
        add_combo(&mut app, "b", "B");
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Char('e'));
        for _ in 0..4 {
            press(&mut app, KeyCode::Tab);
        }
        press(&mut app, KeyCode::Char(' ')); // matching: strict -> loose
        press(&mut app, KeyCode::Enter);
        let combos = &saved(&app).groups[0].combos;
        assert_eq!(combos[0].keyword, "a");
        assert_eq!(combos[0].matching, Matching::Loose);
        assert_eq!(combos[1].keyword, "b");
    }

    #[test]
    fn the_trigger_can_be_switched_to_a_delimiter() {
        let (mut app, _dir) = app("trigger");
        add_combo(&mut app, "sig", "Regards");
        press(&mut app, KeyCode::Char('e'));
        for _ in 0..5 {
            press(&mut app, KeyCode::Tab);
        }
        press(&mut app, KeyCode::Right); // expand: immediately -> after a delimiter
        press(&mut app, KeyCode::Enter);
        assert_eq!(saved(&app).groups[0].combos[0].trigger, Trigger::Delimiter);
        let preview = screen(&app, 100, 30);
        assert!(preview.contains("after a delimiter"), "{preview}");
    }

    #[test]
    fn whitespace_cannot_be_typed_into_a_keyword() {
        let (mut app, _dir) = app("keyword-space");
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "a b");
        let Mode::Combo(form) = &app.mode else {
            panic!("form should be open")
        };
        assert_eq!(form.keyword.text(), "ab");
    }

    #[test]
    fn toggling_and_deleting_are_saved() {
        let (mut app, _dir) = app("toggle");
        add_combo(&mut app, "a", "A");
        press(&mut app, KeyCode::Char(' '));
        assert!(!saved(&app).groups[0].combos[0].enabled);

        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(saved(&app).groups[0].combos.len(), 1);
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('y'));
        assert!(saved(&app).groups[0].combos.is_empty());
    }

    #[test]
    fn groups_can_be_added_and_combos_moved_between_them() {
        let (mut app, _dir) = app("groups");
        add_combo(&mut app, "a", "A");
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "Work");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.group, 1);

        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Char('e'));
        for _ in 0..3 {
            press(&mut app, KeyCode::Tab);
        }
        press(&mut app, KeyCode::Right);
        ctrl(&mut app, 's');
        let library = saved(&app);
        assert!(library.groups[0].combos.is_empty());
        assert_eq!(library.groups[1].combos[0].keyword, "a");
    }

    #[test]
    fn search_finds_combos_in_every_group() {
        let (mut app, _dir) = app("search");
        add_combo(&mut app, "mail", "razu@example.com");
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "Work");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Right);
        add_combo(&mut app, "sig", "Regards");

        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "EXAMPLE");
        assert_eq!(app.visible_combos(), [(0, 0)]);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.visible_combos(), [(1, 0)]);
    }

    #[test]
    fn excluded_apps_are_edited_one_per_line() {
        let (mut app, _dir) = app("settings");
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, "  Slack ");
        ctrl(&mut app, 's');
        let settings = config::load_settings(&app.dir).unwrap();
        assert_eq!(settings.excluded_apps.last().unwrap(), "Slack");
        assert_eq!(
            settings.excluded_apps.len(),
            Settings::default().excluded_apps.len() + 1
        );
    }

    #[test]
    fn the_screen_shows_combos_snippet_and_keys() {
        let (mut app, _dir) = app("render");
        add_combo(&mut app, "@@mail", "razu@example.com");
        let browse = screen(&app, 100, 30);
        assert!(browse.contains("General"), "{browse}");
        assert!(browse.contains("● expanding"), "{browse}");
        assert!(browse.contains("@@mail"), "{browse}");
        assert!(browse.contains("razu@example.com"), "{browse}");
        assert!(browse.contains("Combo saved."), "{browse}");
        press(&mut app, KeyCode::Down);
        let hints = screen(&app, 100, 30);
        assert!(hints.contains("new combo"), "{hints}");

        press(&mut app, KeyCode::Char('e'));
        let form = screen(&app, 100, 30);
        assert!(form.contains("Keyword"), "{form}");
        assert!(form.contains("ctrl+s"), "{form}");
    }

    #[test]
    fn tiny_terminals_do_not_break_drawing() {
        let (mut app, _dir) = app("tiny");
        add_combo(&mut app, "a", "A");
        for (width, height) in [(20, 5), (1, 1), (60, 12)] {
            screen(&app, width, height);
            press(&mut app, KeyCode::Char('e'));
            screen(&app, width, height);
            press(&mut app, KeyCode::Esc);
        }
    }
}
