//! Interface language of the text Rust shows: the tray and app menus and the
//! messages commands return to the pages. Chinese is the source text and
//! every call site carries its English next to it (`tr!`); the pages work the
//! same way (common.js `t`, `data-en` in app.html).
//!
//! The choice lives in settings.json (`language`); `apply` resolves
//! "follow the system" against the OS locale and is called at start-up and
//! whenever the setting changes.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

/// `tr!("中文", "English")`: the text in the current interface language.
/// Both arms must have the same type (two `&str` or two `String`s).
macro_rules! tr {
    ($zh:expr, $en:expr $(,)?) => {
        if $crate::i18n::english() {
            $en
        } else {
            $zh
        }
    };
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    System,
    Zh,
    En,
}

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// The resolved interface language for the pages: "zh" or "en".
pub fn code() -> &'static str {
    tr!("zh", "en")
}

pub fn apply(choice: Language) {
    ENGLISH.store(is_english(choice, sys_locale::get_locale().as_deref()), Ordering::Relaxed);
}

/// Following the system: Chinese for any zh locale, English otherwise. An
/// unknown locale keeps Chinese, the language Coquou was written in.
fn is_english(choice: Language, locale: Option<&str>) -> bool {
    match choice {
        Language::Zh => false,
        Language::En => true,
        Language::System => locale.is_some_and(|l| !l.to_ascii_lowercase().starts_with("zh")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_language_follows_the_locale() {
        assert!(!is_english(Language::System, Some("zh-Hans-CN")));
        assert!(!is_english(Language::System, Some("zh_TW")));
        assert!(is_english(Language::System, Some("en-US")));
        assert!(is_english(Language::System, Some("ja-JP")));
        assert!(!is_english(Language::System, None));
        assert!(is_english(Language::En, Some("zh-CN")));
        assert!(!is_english(Language::Zh, Some("en-US")));
    }

    #[test]
    fn language_reads_from_settings_json() {
        let l: Language = serde_json::from_str("\"en\"").unwrap();
        assert_eq!(l, Language::En);
        assert_eq!(serde_json::to_string(&Language::System).unwrap(), "\"system\"");
    }
}
