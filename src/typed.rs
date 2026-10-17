//! What the user has typed since the last combo breaker, and which combo it triggers.
//! Matching follows Beeftext: strict means the whole typed run equals the keyword,
//! loose means the typed run merely ends with it.

use serde::{Deserialize, Serialize};

const MAX_CHARS: usize = 128;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Matching {
    #[default]
    Strict,
    Loose,
}

/// When a matching keyword is replaced: as soon as it has been typed, or only once a
/// space or punctuation mark follows it, so "sig" can be typed inside "signal".
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    #[default]
    Immediate,
    Delimiter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Combo {
    pub keyword: String,
    pub snippet: String,
    pub matching: Matching,
    pub trigger: Trigger,
    pub case_sensitive: bool,
}

#[derive(Debug, Default)]
pub struct TypedText {
    text: String,
}

impl TypedText {
    pub fn push(&mut self, typed: &str) {
        self.text.push_str(typed);
        let excess = self.text.chars().count().saturating_sub(MAX_CHARS);
        if excess > 0 {
            let cut = self
                .text
                .char_indices()
                .nth(excess)
                .map_or(0, |(index, _)| index);
            self.text.drain(..cut);
        }
    }

    pub fn backspace(&mut self) {
        self.text.pop();
    }

    pub fn clear(&mut self) {
        self.text.clear();
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The combo with this trigger that the typed run matches. The longest keyword
    /// wins, so "sig" and "sigh" can coexist as loose combos.
    pub fn find_match<'a>(&self, combos: &'a [Combo], trigger: Trigger) -> Option<&'a Combo> {
        combos
            .iter()
            .filter(|combo| combo.trigger == trigger && self.matches(combo))
            .max_by_key(|combo| combo.keyword.chars().count())
    }

    fn matches(&self, combo: &Combo) -> bool {
        if combo.case_sensitive {
            keyword_matches(&self.text, &combo.keyword, combo.matching)
        } else {
            let text = self.text.to_lowercase();
            keyword_matches(&text, &combo.keyword.to_lowercase(), combo.matching)
        }
    }
}

fn keyword_matches(text: &str, keyword: &str, matching: Matching) -> bool {
    match matching {
        Matching::Strict => text == keyword,
        Matching::Loose => text.ends_with(keyword),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(keyword: &str, matching: Matching) -> Combo {
        Combo {
            keyword: keyword.into(),
            snippet: format!("<{keyword}>"),
            matching,
            trigger: Trigger::Immediate,
            case_sensitive: true,
        }
    }

    fn typed(text: &str) -> TypedText {
        let mut typed = TypedText::default();
        typed.push(text);
        typed
    }

    #[test]
    fn strict_needs_the_whole_run_to_be_the_keyword() {
        let combos = [combo("==>", Matching::Strict)];
        assert!(
            typed("==>")
                .find_match(&combos, Trigger::Immediate)
                .is_some()
        );
        assert!(
            typed("First==>")
                .find_match(&combos, Trigger::Immediate)
                .is_none()
        );
    }

    #[test]
    fn loose_matches_at_the_end_of_a_word() {
        let combos = [combo("==>", Matching::Loose)];
        assert!(
            typed("First==>")
                .find_match(&combos, Trigger::Immediate)
                .is_some()
        );
        assert!(
            typed("First==>x")
                .find_match(&combos, Trigger::Immediate)
                .is_none()
        );
    }

    #[test]
    fn case_insensitive_combos_ignore_case_on_both_sides() {
        let mut insensitive = combo("Sig", Matching::Strict);
        insensitive.case_sensitive = false;
        assert!(
            typed("sIG")
                .find_match(&[insensitive], Trigger::Immediate)
                .is_some()
        );
        assert!(
            typed("sIG")
                .find_match(&[combo("Sig", Matching::Strict)], Trigger::Immediate)
                .is_none()
        );
    }

    #[test]
    fn longest_keyword_wins() {
        let combos = [combo("ig", Matching::Loose), combo("sig", Matching::Loose)];
        assert_eq!(
            typed("sig")
                .find_match(&combos, Trigger::Immediate)
                .unwrap()
                .keyword,
            "sig"
        );
    }

    #[test]
    fn only_combos_with_the_asked_trigger_match() {
        let mut delimited = combo("sig", Matching::Strict);
        delimited.trigger = Trigger::Delimiter;
        let combos = [delimited];
        assert!(
            typed("sig")
                .find_match(&combos, Trigger::Immediate)
                .is_none()
        );
        assert!(
            typed("sig")
                .find_match(&combos, Trigger::Delimiter)
                .is_some()
        );
    }

    #[test]
    fn backspace_removes_whole_characters() {
        let mut text = typed("blåbær");
        text.backspace();
        text.push("r");
        let combos = [combo("blåbær", Matching::Strict)];
        assert!(text.find_match(&combos, Trigger::Immediate).is_some());
    }

    #[test]
    fn keeps_only_the_most_recent_characters() {
        let mut text = typed(&"ø".repeat(MAX_CHARS));
        text.push("abc");
        assert_eq!(text.text.chars().count(), MAX_CHARS);
        assert!(text.text.ends_with("øabc"));
    }
}
