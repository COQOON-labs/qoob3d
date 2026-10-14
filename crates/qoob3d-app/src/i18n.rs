//! UI translations with Project Fluent (ADR-0020).
//!
//! English is the base language; every other language is a "language pack"
//! `locales/<code>/main.ftl` with exactly the same keys (checked by
//! `cargo xtask check`). UI code never contains user-visible literals; it
//! calls [`tr`] / [`tr_args`] with a key. Adding a language = one `.ftl`
//! file plus one [`Language`] variant.

use std::cell::Cell;
use std::sync::OnceLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use serde::{Deserialize, Serialize};
use unic_langid::LanguageIdentifier;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    English,
    German,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::German];

    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::German => "de",
        }
    }

    /// Name of the language in that language (for the picker).
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::German => "Deutsch",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Language::English => include_str!("../locales/en/main.ftl"),
            Language::German => include_str!("../locales/de/main.ftl"),
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&l| l == self).expect("listed in ALL")
    }

    pub fn decimal_separator(self) -> char {
        match self {
            Language::English => '.',
            Language::German => ',',
        }
    }
}

thread_local! {
    /// Per thread: all UI runs on one thread, and parallel UI tests stay
    /// independent of each other.
    static CURRENT: Cell<Language> = const { Cell::new(Language::English) };
}
static BUNDLES: OnceLock<Vec<FluentBundle<FluentResource>>> = OnceLock::new();

fn bundles() -> &'static [FluentBundle<FluentResource>] {
    BUNDLES.get_or_init(|| {
        Language::ALL
            .iter()
            .map(|&lang| {
                let id: LanguageIdentifier = lang.code().parse().expect("valid language code");
                let resource = FluentResource::try_new(lang.source().to_owned())
                    .unwrap_or_else(|(_, errs)| panic!("invalid {}.ftl: {errs:?}", lang.code()));
                let mut bundle = FluentBundle::new_concurrent(vec![id]);
                // No Unicode isolation marks around arguments: egui would show them.
                bundle.set_use_isolating(false);
                bundle.add_resource(resource).expect("no duplicate keys");
                bundle
            })
            .collect()
    })
}

/// Sets the language for the calling (UI) thread.
pub fn set_language(lang: Language) {
    CURRENT.with(|c| c.set(lang));
}

pub fn language() -> Language {
    CURRENT.with(Cell::get)
}

/// Translates `key` in the current language.
pub fn tr(key: &str) -> String {
    tr_args(key, &[])
}

/// Translates `key` with named arguments, e.g. `tr_args("status-model", &[("edges", 14.into())])`.
/// Falls back to English, then to `«key»` (never happens when the xtask check is green).
pub fn tr_args(key: &str, args: &[(&str, FluentValue<'_>)]) -> String {
    let mut fluent_args = FluentArgs::new();
    for (name, value) in args {
        fluent_args.set(*name, value.clone());
    }
    let args = (!args.is_empty()).then_some(&fluent_args);
    for lang in [language(), Language::English] {
        let bundle = &bundles()[lang.index()];
        if let Some(pattern) = bundle.get_message(key).and_then(|m| m.value()) {
            let mut errors = Vec::new();
            let text = bundle.format_pattern(pattern, args, &mut errors);
            if !errors.is_empty() {
                tracing::warn!(key, ?errors, "fluent formatting problems");
            }
            return text.into_owned();
        }
    }
    tracing::warn!(key, "missing translation key");
    format!("«{key}»")
}

/// Formats a number with `digits` decimals and the language's separator.
pub fn decimal(value: f64, digits: usize) -> String {
    format!("{value:.digits$}").replace('.', &language().decimal_separator().to_string())
}

#[cfg(test)]
mod tests;
