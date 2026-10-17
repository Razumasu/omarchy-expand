//! The user's combos and settings in ~/.config/omarchy-expand. The daemon reloads them
//! whenever they change; the setup UI edits and saves them.
//!
//! combos.json holds groups of combos; only keyword and snippet are required:
//!
//! ```json
//! { "version": 1, "groups": [ { "name": "Personal", "combos": [
//!   { "keyword": "@@mail", "snippet": "razu@example.com" },
//!   { "keyword": "sig", "snippet": "Regards", "trigger": "delimiter" },
//!   { "keyword": "==>", "snippet": "⟹", "matching": "loose", "case_sensitive": false }
//! ] } ] }
//! ```
//!
//! settings.json is optional: `{ "version": 1, "excluded_apps": ["1password", "keepassxc"] }`
//!
//! Both files carry a format version so a later release can migrate them; a file written
//! by a newer release than the one reading it is refused rather than misread.

use std::collections::HashSet;
use std::env;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::Event;
use crate::typed::{Combo, Matching, Trigger};

const COMBOS_FILE: &str = "combos.json";
const SETTINGS_FILE: &str = "settings.json";

/// The layout of both files. Bump it when a change cannot be read by older releases.
const FORMAT_VERSION: u32 = 1;

/// Password managers, so their secrets never pass through the typed buffer. Named by
/// window class or process name; the Flatpak app id is listed where it differs.
const DEFAULT_EXCLUDED_APPS: &[&str] = &[
    "1password",
    "bitwarden",
    "com.bitwarden.desktop",
    "keepassxc",
    "org.keepassxc.keepassxc",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    #[serde(default = "format_version")]
    pub version: u32,
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
    #[serde(default, skip_serializing_if = "is_immediate")]
    pub trigger: Trigger,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub case_sensitive: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "format_version")]
    pub version: u32,
    #[serde(default = "default_excluded_apps")]
    pub excluded_apps: Vec<String>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            groups: Vec::new(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
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
    /// Every keyword must be typeable and unique, every group named and unique, and
    /// names must fit on one line because the picker shows them one per line.
    pub fn validate(&self) -> Result<(), String> {
        let mut group_names = HashSet::new();
        let mut keywords = HashSet::new();
        for group in &self.groups {
            if group.name.trim().is_empty() {
                return Err("a group has no name".into());
            }
            if has_line_break(&group.name) {
                return Err(format!("group name {:?} contains a line break", group.name));
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
                if combo.keyword.chars().any(char::is_control) {
                    return Err(format!(
                        "keyword {:?} contains a control character, which cannot be typed",
                        combo.keyword
                    ));
                }
                if has_line_break(&combo.name) {
                    return Err(format!(
                        "the name of combo {:?} contains a line break",
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
                trigger: combo.trigger,
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

fn is_immediate(trigger: &Trigger) -> bool {
    *trigger == Trigger::Immediate
}

fn format_version() -> u32 {
    FORMAT_VERSION
}

fn has_line_break(text: &str) -> bool {
    text.contains(['\n', '\r'])
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
    create_private_dir(dir)?;
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

/// Only the version is read first, loosely, so a file from a newer release gets a clear
/// refusal instead of an error about a field this release does not know.
#[derive(Deserialize)]
struct FormatVersion {
    #[serde(default = "format_version")]
    version: u32,
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(in_file(path, error.to_string())),
    };
    let FormatVersion { version } =
        serde_json::from_str(&text).map_err(|error| in_file(path, error.to_string()))?;
    if version > FORMAT_VERSION {
        return Err(in_file(
            path,
            format!(
                "format version {version} was written by a newer omarchy-expand; this one reads up to version {FORMAT_VERSION}"
            ),
        ));
    }
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| in_file(path, error.to_string()))
}

/// Writes a temporary file and renames it over the real one, so the daemon never
/// reads a half-written file and a crash never leaves one behind. Snippets are the
/// user's own text, so the files are readable by that user only.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let mut json = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    json.push('\n');
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    // Per process, so two editors saving at once cannot truncate each other's file.
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    let write = || -> io::Result<()> {
        if let Some(dir) = path.parent() {
            create_private_dir(dir)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    };
    write().map_err(|error| in_file(path, error.to_string()))
}

/// Creates the directory readable by the user only; an existing one keeps its mode.
fn create_private_dir(dir: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

fn in_file(path: &Path, error: String) -> String {
    format!("{}: {error}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

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
        assert_eq!(library.version, FORMAT_VERSION);
        assert_eq!(
            library.active_combos(),
            [Combo {
                keyword: "@@m".into(),
                snippet: "me".into(),
                matching: Matching::Strict,
                trigger: Trigger::Immediate,
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
                {"name":"Arrow","keyword":"==>","snippet":"⟹","matching":"loose","case_sensitive":false},
                {"keyword":"sig","snippet":"Regards","trigger":"delimiter"}]}]}"#,
        )
        .unwrap();
        let combo = &library.groups[0].combos[0];
        assert_eq!(combo.name, "Arrow");
        assert_eq!(combo.matching, Matching::Loose);
        assert_eq!(combo.trigger, Trigger::Immediate);
        assert!(!combo.case_sensitive);
        assert_eq!(library.groups[0].combos[1].trigger, Trigger::Delimiter);
    }

    #[test]
    fn untypeable_or_ambiguous_entries_are_rejected() {
        let file = |combos: &str| format!(r#"{{"groups":[{{"name":"g","combos":[{combos}]}}]}}"#);
        let empty = parse(&file(r#"{"keyword":"","snippet":"x"}"#)).unwrap_err();
        assert!(empty.contains("empty keyword"), "{empty}");
        let spaced = parse(&file(r#"{"keyword":"a b","snippet":"x"}"#)).unwrap_err();
        assert!(spaced.contains("whitespace"), "{spaced}");
        let control = parse(&file(r#"{"keyword":"a\u0007b","snippet":"x"}"#)).unwrap_err();
        assert!(control.contains("control character"), "{control}");
        let twice = parse(&file(
            r#"{"keyword":"a","snippet":"x"},{"keyword":"a","snippet":"y","enabled":false}"#,
        ))
        .unwrap_err();
        assert!(twice.contains("more than once"), "{twice}");
        let groups = parse(r#"{"groups":[{"name":"g"},{"name":"g"}]}"#).unwrap_err();
        assert!(groups.contains("two groups"), "{groups}");
    }

    #[test]
    fn names_must_fit_on_one_line() {
        let combo = parse(
            r#"{"groups":[{"name":"g","combos":[{"name":"a\nb","keyword":"k","snippet":"x"}]}]}"#,
        )
        .unwrap_err();
        assert!(combo.contains("line break"), "{combo}");
        let group = parse(r#"{"groups":[{"name":"a\rb"}]}"#).unwrap_err();
        assert!(group.contains("line break"), "{group}");
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
                {"keyword":"b","snippet":"B","matching":"loose","trigger":"delimiter","enabled":false}]}]}"#,
        )
        .unwrap();
        save_library(&dir, &library).unwrap();

        let saved = fs::read_to_string(dir.join(COMBOS_FILE)).unwrap();
        assert!(!saved.contains("case_sensitive"), "{saved}");
        assert!(saved.contains(r#""version": 1"#), "{saved}");
        assert!(saved.contains(r#""matching": "loose""#), "{saved}");
        assert!(saved.contains(r#""trigger": "delimiter""#), "{saved}");
        assert_eq!(saved.matches("trigger").count(), 1, "{saved}");
        assert_eq!(load_library(&dir).unwrap(), library);
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saved_files_are_private_to_the_user() {
        let dir = temp_dir("private").join("nested");
        save_settings(&dir, &Settings::default()).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.join(SETTINGS_FILE)), 0o600);
        assert_eq!(mode(&dir), 0o700);
        fs::remove_dir_all(dir.parent().unwrap()).unwrap();
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
    fn a_file_from_a_newer_release_is_refused_clearly() {
        let dir = temp_dir("newer");
        fs::write(
            dir.join(COMBOS_FILE),
            r#"{"version": 99, "groups": [], "sync": {"server": "x"}}"#,
        )
        .unwrap();
        let error = load_library(&dir).unwrap_err();
        fs::remove_dir_all(&dir).unwrap();
        assert!(error.contains("newer omarchy-expand"), "{error}");
        assert!(!error.contains("sync"), "{error}");
    }

    #[test]
    fn exclusions_match_class_or_process_ignoring_case() {
        let settings = Settings::default();
        assert!(settings.excludes("1Password", None));
        assert!(settings.excludes("org.keepassxc.KeePassXC", Some("keepassxc")));
        assert!(settings.excludes("some-wrapper", Some("keepassxc")));
        assert!(settings.excludes("com.bitwarden.desktop", Some("bitwarden")));
        assert!(!settings.excludes("foot", Some("foot")));

        let custom: Settings = serde_json::from_str(r#"{"excluded_apps":["Slack"]}"#).unwrap();
        assert!(custom.excludes("slack", None));
        assert!(!custom.excludes("1password", None));
    }
}
