//! The user's combos and settings in ~/.config/omarchy-expand. The daemon reloads them
//! whenever they change; the setup UI edits and saves them.
//!
//! combos.json holds groups of combos; only keyword and snippet are required:
//!
//! ```json
//! { "groups": [ { "name": "Personal", "combos": [
//!   { "keyword": "@@mail", "snippet": "razu@example.com" },
//!   { "keyword": "==>", "snippet": "⟹", "matching": "loose", "case_sensitive": false }
//! ] } ] }
//! ```
//!
//! settings.json is optional: `{ "excluded_apps": ["1password", "keepassxc"] }`

use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::Event;
use crate::typed::{Combo, Matching};

const COMBOS_FILE: &str = "combos.json";
const SETTINGS_FILE: &str = "settings.json";

/// Password managers, so their secrets never pass through the typed buffer.
const DEFAULT_EXCLUDED_APPS: &[&str] = &[
    "1password",
    "bitwarden",
    "keepassxc",
    "org.keepassxc.keepassxc",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default)]
    pub combos: Vec<ComboEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComboEntry {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub keyword: String,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "is_strict")]
    pub matching: Matching,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub case_sensitive: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "default_excluded_apps")]
    pub excluded_apps: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            excluded_apps: default_excluded_apps(),
        }
    }
}

/// What the daemon works from: the combos it can expand and the apps it must ignore.
#[derive(Debug, Default, PartialEq)]
pub struct Config {
    pub combos: Vec<Combo>,
    pub settings: Settings,
}

impl Library {
    /// Every keyword must be typeable and unique, and every group named and unique.
    pub fn validate(&self) -> Result<(), String> {
        let mut group_names = HashSet::new();
        let mut keywords = HashSet::new();
        for group in &self.groups {
            if group.name.trim().is_empty() {
                return Err("a group has no name".into());
            }
            if !group_names.insert(group.name.as_str()) {
                return Err(format!("there are two groups named {:?}", group.name));
            }
            for combo in &group.combos {
                if combo.keyword.is_empty() {
                    return Err(format!(
                        "a combo in group {:?} has an empty keyword",
                        group.name
                    ));
                }
                if combo.keyword.chars().any(char::is_whitespace) {
                    return Err(format!(
                        "keyword {:?} contains whitespace, which ends a keyword while typing",
                        combo.keyword
                    ));
                }
                if !keywords.insert(combo.keyword.as_str()) {
                    return Err(format!(
                        "keyword {:?} is used more than once",
                        combo.keyword
                    ));
                }
            }
        }
        Ok(())
    }

    /// The combos the daemon should expand: enabled, in enabled groups.
    pub fn active_combos(&self) -> Vec<Combo> {
        self.groups
            .iter()
            .filter(|group| group.enabled)
            .flat_map(|group| &group.combos)
            .filter(|combo| combo.enabled)
            .map(|combo| Combo {
                keyword: combo.keyword.clone(),
                snippet: combo.snippet.clone(),
                matching: combo.matching,
                case_sensitive: combo.case_sensitive,
            })
            .collect()
    }
}

impl Settings {
    /// Apps are named by window class or process name, ignoring case.
    pub fn excludes(&self, class: &str, process_name: Option<&str>) -> bool {
        self.excluded_apps.iter().any(|app| {
            app.eq_ignore_ascii_case(class)
                || process_name.is_some_and(|name| app.eq_ignore_ascii_case(name))
        })
    }
}

impl Config {
    pub fn excludes(&self, class: &str, process_name: Option<&str>) -> bool {
        self.settings.excludes(class, process_name)
    }
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_strict(matching: &Matching) -> bool {
    *matching == Matching::Strict
}

fn default_excluded_apps() -> Vec<String> {
    DEFAULT_EXCLUDED_APPS
        .iter()
        .map(|app| app.to_string())
        .collect()
}

pub fn config_dir() -> io::Result<PathBuf> {
    let base = match env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".config"))
            .ok_or_else(|| io::Error::other("neither XDG_CONFIG_HOME nor HOME is set"))?,
    };
    Ok(base.join("omarchy-expand"))
}

pub fn load(dir: &Path) -> Result<Config, String> {
    Ok(Config {
        combos: load_library(dir)?.active_combos(),
        settings: load_settings(dir)?,
    })
}

/// A missing file means no combos yet; an invalid one is an error naming the file.
pub fn load_library(dir: &Path) -> Result<Library, String> {
    let path = dir.join(COMBOS_FILE);
    let library: Library = read_json(&path)?.unwrap_or_default();
    library.validate().map_err(|error| in_file(&path, error))?;
    Ok(library)
}

pub fn load_settings(dir: &Path) -> Result<Settings, String> {
    Ok(read_json(&dir.join(SETTINGS_FILE))?.unwrap_or_default())
}

pub fn save_library(dir: &Path, library: &Library) -> Result<(), String> {
    library.validate()?;
    write_json(&dir.join(COMBOS_FILE), library)
}

pub fn save_settings(dir: &Path, settings: &Settings) -> Result<(), String> {
    write_json(&dir.join(SETTINGS_FILE), settings)
}

/// Sends ConfigChanged when either file is saved, replaced or deleted.
pub fn watch(dir: &Path, events: Sender<Event>) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let inotify = Inotify::init(InitFlags::empty())?;
    inotify.add_watch(
        dir,
        AddWatchFlags::IN_CLOSE_WRITE | AddWatchFlags::IN_MOVED_TO | AddWatchFlags::IN_DELETE,
    )?;
    thread::spawn(move || {
        while let Ok(changes) = inotify.read_events() {
            let relevant = changes.iter().any(|change| {
                change
                    .name
                    .as_deref()
                    .is_some_and(|name| name == COMBOS_FILE || name == SETTINGS_FILE)
            });
            if relevant && events.send(Event::ConfigChanged).is_err() {
                return;
            }
        }
    });
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(in_file(path, error.to_string())),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| in_file(path, error.to_string()))
}

/// Writes a temporary file and renames it over the real one, so the daemon never
/// reads a half-written file and a crash never leaves one behind.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let mut json = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    json.push('\n');
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let temporary = path.with_file_name(format!(".{file_name}.tmp"));
    let write = || -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut file = File::create(&temporary)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    };
    write().map_err(|error| in_file(path, error.to_string()))
}

fn in_file(path: &Path, error: String) -> String {
    format!("{}: {error}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Library, String> {
        let library: Library = serde_json::from_str(text).map_err(|error| error.to_string())?;
        library.validate()?;
        Ok(library)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("omarchy-expand-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_combo_needs_only_keyword_and_snippet() {
        let library =
            parse(r#"{"groups":[{"name":"g","combos":[{"keyword":"@@m","snippet":"me"}]}]}"#)
                .unwrap();
        assert_eq!(
            library.active_combos(),
            [Combo {
                keyword: "@@m".into(),
                snippet: "me".into(),
                matching: Matching::Strict,
                case_sensitive: true,
            }]
        );
    }

    #[test]
    fn disabled_combos_and_groups_are_not_active() {
        let library = parse(
            r#"{"groups":[
                {"name":"on","combos":[
                    {"keyword":"a","snippet":"A"},
                    {"keyword":"b","snippet":"B","enabled":false}]},
                {"name":"off","enabled":false,"combos":[{"keyword":"c","snippet":"C"}]}]}"#,
        )
        .unwrap();
        let active = library.active_combos();
        let keywords: Vec<_> = active.iter().map(|combo| combo.keyword.as_str()).collect();
        assert_eq!(keywords, ["a"]);
    }

    #[test]
    fn options_are_read() {
        let library = parse(
            r#"{"groups":[{"name":"g","combos":[
                {"name":"Arrow","keyword":"==>","snippet":"⟹","matching":"loose","case_sensitive":false}]}]}"#,
        )
        .unwrap();
        let combo = &library.groups[0].combos[0];
        assert_eq!(combo.name, "Arrow");
        assert_eq!(combo.matching, Matching::Loose);
        assert!(!combo.case_sensitive);
    }

    #[test]
    fn untypeable_or_ambiguous_entries_are_rejected() {
        let file = |combos: &str| format!(r#"{{"groups":[{{"name":"g","combos":[{combos}]}}]}}"#);
        let empty = parse(&file(r#"{"keyword":"","snippet":"x"}"#)).unwrap_err();
        assert!(empty.contains("empty keyword"), "{empty}");
        let spaced = parse(&file(r#"{"keyword":"a b","snippet":"x"}"#)).unwrap_err();
        assert!(spaced.contains("whitespace"), "{spaced}");
        let twice = parse(&file(
            r#"{"keyword":"a","snippet":"x"},{"keyword":"a","snippet":"y","enabled":false}"#,
        ))
        .unwrap_err();
        assert!(twice.contains("more than once"), "{twice}");
        let groups = parse(r#"{"groups":[{"name":"g"},{"name":"g"}]}"#).unwrap_err();
        assert!(groups.contains("two groups"), "{groups}");
    }

    #[test]
    fn misspelt_fields_are_rejected() {
        let error = parse(r#"{"groups":[{"name":"g","combos":[{"keyword":"a","snipet":"x"}]}]}"#)
            .unwrap_err();
        assert!(error.contains("snipet"), "{error}");
    }

    #[test]
    fn saving_round_trips_and_leaves_defaults_out_of_the_file() {
        let dir = temp_dir("save");
        let library = parse(
            r#"{"groups":[{"name":"g","combos":[
                {"keyword":"a","snippet":"A"},
                {"keyword":"b","snippet":"B","matching":"loose","enabled":false}]}]}"#,
        )
        .unwrap();
        save_library(&dir, &library).unwrap();

        let saved = fs::read_to_string(dir.join(COMBOS_FILE)).unwrap();
        assert!(!saved.contains("case_sensitive"), "{saved}");
        assert!(saved.contains(r#""matching": "loose""#), "{saved}");
        assert_eq!(load_library(&dir).unwrap(), library);
        assert!(!dir.join(".combos.json.tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_invalid_library_is_never_saved() {
        let dir = temp_dir("invalid");
        let mut library = Library::default();
        library.groups.push(Group {
            name: String::new(),
            enabled: true,
            combos: Vec::new(),
        });
        assert!(save_library(&dir, &library).is_err());
        assert!(!dir.join(COMBOS_FILE).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_files_give_defaults_and_errors_name_the_file() {
        let dir = temp_dir("load");
        assert_eq!(load(&dir).unwrap(), Config::default());

        fs::write(dir.join(SETTINGS_FILE), "{ not json").unwrap();
        let error = load(&dir).unwrap_err();
        fs::remove_dir_all(&dir).unwrap();
        assert!(error.contains("settings.json"), "{error}");
    }

    #[test]
    fn exclusions_match_class_or_process_ignoring_case() {
        let settings = Settings::default();
        assert!(settings.excludes("1Password", None));
        assert!(settings.excludes("org.keepassxc.KeePassXC", Some("keepassxc")));
        assert!(settings.excludes("some-wrapper", Some("keepassxc")));
        assert!(!settings.excludes("foot", Some("foot")));

        let custom: Settings = serde_json::from_str(r#"{"excluded_apps":["Slack"]}"#).unwrap();
        assert!(custom.excludes("slack", None));
        assert!(!custom.excludes("1password", None));
    }
}
