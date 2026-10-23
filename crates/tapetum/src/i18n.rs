//! German and English. Texts stay where they are used, as pairs:
//! `tr("Qualität", "Quality")`, or with arguments `trf!("{n} Geräte", "{n} devices", n = n)`
//! (named arguments must be passed explicitly: a format string coming through
//! a macro can't capture variables).
//!
//! The language is a per-user setting (not per show), kept in the user's
//! config directory; the first start follows the system language.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

static ENGLISH: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "de")]
    German,
    #[serde(rename = "en")]
    English,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::German, Language::English];

    pub fn name(self) -> &'static str {
        match self {
            Language::German => "Deutsch",
            Language::English => "English",
        }
    }

    pub fn current() -> Self {
        if english() { Language::English } else { Language::German }
    }

    /// German for German system languages, English otherwise.
    fn from_system() -> Self {
        match sys_locale::get_locale() {
            Some(l) if l.to_ascii_lowercase().starts_with("de") => Language::German,
            _ => Language::English,
        }
    }
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

pub fn set(language: Language) {
    ENGLISH.store(language == Language::English, Ordering::Relaxed);
}

/// The text in the current language.
pub fn tr(de: &'static str, en: &'static str) -> &'static str {
    if english() { en } else { de }
}

#[macro_export]
macro_rules! trf {
    ($de:literal, $en:literal $(, $($arg:tt)*)?) => {
        if $crate::i18n::english() { format!($en $(, $($arg)*)?) } else { format!($de $(, $($arg)*)?) }
    };
}

/// Settings of this user, the same for every show.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserSettings {
    pub language: Option<Language>,
    /// Opened when Tapetum starts without a show (from the dock, the start menu).
    pub last_show: Option<PathBuf>,
    /// The fixture library folder; `None` is `Documents/Tapetum/Fixtures`.
    pub fixture_library: Option<PathBuf>,
}

/// Tapetum's folder for per-user files (settings, the built-in example).
pub fn config_dir() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "APPDATA" } else { "HOME" }).map(PathBuf::from)?;
    Some(if cfg!(windows) {
        home.join("Tapetum")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Tapetum")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from).join("tapetum")
    })
}

fn settings_path() -> Option<PathBuf> {
    Some(config_dir()?.join("settings.json"))
}

impl UserSettings {
    pub fn load() -> Self {
        settings_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Sets the language from the settings, else from the system.
    pub fn apply(&self) {
        set(self.language.unwrap_or_else(Language::from_system));
    }
}
