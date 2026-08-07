//! Platform-independent, grapheme-safe token state.

use unicode_segmentation::UnicodeSegmentation;
use zonkey_types::EditPlan;

/// Raw input and rendered output are intentionally stored separately.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TokenState {
    /// ASCII keys as entered by the user.
    pub raw_ascii: String,
    /// Current Unicode representation shown to the user.
    pub rendered: String,
}

impl TokenState {
    /// Creates an empty token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether both raw and rendered state are empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.raw_ascii.is_empty() && self.rendered.is_empty()
    }

    /// Adds one verified ASCII character to the raw sequence.
    ///
    /// # Errors
    ///
    /// Returns the rejected character when it is not ASCII.
    pub fn push_raw(&mut self, character: char) -> Result<(), char> {
        if !character.is_ascii() {
            return Err(character);
        }
        self.raw_ascii.push(character);
        Ok(())
    }

    /// Removes the last raw ASCII character.
    pub fn pop_raw(&mut self) -> Option<char> {
        self.raw_ascii.pop()
    }

    /// Replaces rendered text and returns the abstract visible edit required.
    pub fn replace_rendered(&mut self, rendered: String) -> EditPlan {
        let plan = edit_plan_between(&self.rendered, &rendered);
        self.rendered = rendered;
        plan
    }

    /// Clears raw and rendered state after a commit or reset.
    pub fn clear(&mut self) {
        self.raw_ascii.clear();
        self.rendered.clear();
    }
}

/// Counts extended grapheme clusters according to Unicode segmentation.
#[must_use]
pub fn grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

/// Removes up to `count` visible grapheme clusters from the end of a string.
///
/// Returns the number actually removed. The string is rebuilt from grapheme
/// slices; no byte or code-unit deletion is used.
pub fn delete_last_graphemes(text: &mut String, count: usize) -> usize {
    let total = grapheme_count(text);
    let keep = total.saturating_sub(count);
    let rebuilt: String = text.graphemes(true).take(keep).collect();
    *text = rebuilt;
    total - keep
}

/// Creates the smallest suffix replacement after a common grapheme prefix.
#[must_use]
pub fn edit_plan_between(before: &str, after: &str) -> EditPlan {
    let before_graphemes: Vec<&str> = before.graphemes(true).collect();
    let after_graphemes: Vec<&str> = after.graphemes(true).collect();
    let common = before_graphemes
        .iter()
        .zip(&after_graphemes)
        .take_while(|(left, right)| left == right)
        .count();

    EditPlan {
        delete_graphemes: before_graphemes.len() - common,
        insert_text: after_graphemes[common..].concat(),
    }
}

/// Applies an abstract edit to an in-memory visible-text model.
pub fn apply_edit_plan(text: &mut String, plan: &EditPlan) {
    delete_last_graphemes(text, plan.delete_graphemes);
    text.push_str(&plan.insert_text);
}

#[cfg(test)]
mod tests {
    use super::{
        TokenState, apply_edit_plan, delete_last_graphemes, edit_plan_between, grapheme_count,
    };

    #[test]
    fn raw_and_rendered_text_are_independent() {
        let token = TokenState {
            raw_ascii: "dungf".into(),
            rendered: "dùng".into(),
        };
        assert_ne!(token.raw_ascii, token.rendered);
    }

    #[test]
    fn counts_ascii_and_precomposed_vietnamese() {
        assert_eq!(grapheme_count("hello"), 5);
        assert_eq!(grapheme_count("dùng"), 4);
    }

    #[test]
    fn counts_combining_sequence_as_one_grapheme() {
        assert_eq!(grapheme_count("a\u{0301}"), 1);
    }

    #[test]
    fn counts_emoji_zwj_sequence_as_one_grapheme() {
        assert_eq!(grapheme_count("👩‍💻"), 1);
    }

    #[test]
    fn deletes_mixed_unicode_by_grapheme() {
        let mut text = String::from("A🇻🇳a\u{0301}👩‍💻");
        assert_eq!(delete_last_graphemes(&mut text, 2), 2);
        assert_eq!(text, "A🇻🇳");
    }

    #[test]
    fn plans_and_applies_grapheme_suffix_replacement() {
        let plan = edit_plan_between("ca", "cá");
        assert_eq!(plan.delete_graphemes, 1);
        assert_eq!(plan.insert_text, "á");
        let mut visible = String::from("ca");
        apply_edit_plan(&mut visible, &plan);
        assert_eq!(visible, "cá");
    }

    #[test]
    fn token_lifecycle_clears_both_representations() {
        let mut token = TokenState::new();
        token.push_raw('a').expect("ASCII must be accepted");
        let plan = token.replace_rendered(String::from("ă"));
        assert_eq!(token.raw_ascii, "a");
        assert_eq!(token.rendered, "ă");
        assert_eq!(plan.insert_text, "ă");
        token.clear();
        assert!(token.is_empty());
    }
}
