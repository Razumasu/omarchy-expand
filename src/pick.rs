//! The picker: choose a combo from the Omarchy menu, and the daemon pastes it into the
//! window that had focus. Filtering covers keyword, name and snippet.

use std::error::Error;
use std::process::{Command, Stdio};

use crate::config::{self, ComboEntry};
use crate::ipc;

const PROMPT: &str = "Combos";
/// Nerd Font keyboard glyph, as used by Omarchy's own menus.
const GLYPH: &str = "󰌌";
const PREVIEW_CHARS: usize = 60;

pub fn run() -> Result<(), Box<dyn Error>> {
    // Check first, so nobody picks a combo that cannot be pasted.
    if !ipc::daemon_reachable() {
        return Err(notify_failure("the omarchy-expand daemon is not running").into());
    }
    let library = config::load_library(&config::config_dir()?)?;
    let options: Vec<String> = library
        .groups
        .iter()
        .filter(|group| group.enabled)
        .flat_map(|group| &group.combos)
        .filter(|combo| combo.enabled)
        .map(menu_option)
        .collect();
    if options.is_empty() {
        notify("No combos yet", "Open Text Expander to add some.");
        return Ok(());
    }

    let output = Command::new("omarchy-menu-select")
        .arg(PROMPT)
        .args(&options)
        .args(["--", "--width", "640"])
        .stderr(Stdio::inherit())
        .output()?;
    match output.status.code() {
        Some(0) => {}
        Some(1) => return Ok(()), // closed without choosing
        _ => return Err(format!("omarchy-menu-select failed with {}", output.status).into()),
    }
    let selection = String::from_utf8_lossy(&output.stdout);
    ipc::request_paste(selected_keyword(&selection)).map_err(|error| notify_failure(&error).into())
}

/// `glyph<TAB>keyword<TAB>subtext`: the menu shows the glyph and filters on keyword and subtext.
fn menu_option(combo: &ComboEntry) -> String {
    let first_line = combo.snippet.lines().next().unwrap_or_default();
    let mut preview: String = first_line.chars().take(PREVIEW_CHARS).collect();
    if first_line.chars().count() > PREVIEW_CHARS || combo.snippet.lines().nth(1).is_some() {
        preview.push('…');
    }
    let subtext = if combo.name.is_empty() {
        preview
    } else {
        format!("{} · {preview}", combo.name)
    };
    format!("{GLYPH}\t{}\t{}", combo.keyword, subtext.replace('\t', " "))
}

/// The menu answers `keyword<TAB>subtext`; keywords are unique, so they are the key.
fn selected_keyword(selection: &str) -> &str {
    selection
        .trim_end_matches('\n')
        .split('\t')
        .next()
        .unwrap_or_default()
}

fn notify_failure(message: &str) -> String {
    notify("Text Expander could not paste", message);
    message.to_owned()
}

fn notify(headline: &str, description: &str) {
    // The message is also returned or printed, so a failed notification loses nothing.
    Command::new("omarchy-notification-send")
        .args([headline, description])
        .status()
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed::Matching;

    fn combo(name: &str, keyword: &str, snippet: &str) -> ComboEntry {
        ComboEntry {
            name: name.into(),
            keyword: keyword.into(),
            snippet: snippet.into(),
            matching: Matching::Strict,
            case_sensitive: true,
            enabled: true,
        }
    }

    #[test]
    fn options_show_keyword_name_and_snippet_preview() {
        assert_eq!(
            menu_option(&combo("Email", "@@mail", "razu@example.com")),
            "󰌌\t@@mail\tEmail · razu@example.com"
        );
        assert_eq!(
            menu_option(&combo("", "sig", "Regards\nRazu")),
            "󰌌\tsig\tRegards…"
        );
        let long = "x".repeat(80);
        let option = menu_option(&combo("", "long", &long));
        assert!(
            option.ends_with(&format!("{}…", "x".repeat(PREVIEW_CHARS))),
            "{option}"
        );
    }

    #[test]
    fn the_keyword_is_read_back_from_the_selection() {
        assert_eq!(
            selected_keyword("@@mail\tEmail · razu@example.com\n"),
            "@@mail"
        );
        assert_eq!(selected_keyword("sig\n"), "sig");
    }
}
